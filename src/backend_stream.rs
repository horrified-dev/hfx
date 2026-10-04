//! Coalesce adjacent UI deltas within a network chunk, never across an await.
use super::Event;
use std::{
    cell::{Cell, RefCell},
    sync::mpsc::Sender,
};

#[derive(Debug)]
pub(super) struct ClosedStream;

pub(super) trait StreamSink {
    fn send(&self, event: Event) -> Result<(), ClosedStream>;
}

impl StreamSink for Sender<Event> {
    fn send(&self, event: Event) -> Result<(), ClosedStream> {
        Sender::send(self, event).map_err(|_| ClosedStream)
    }
}

const MAX_COALESCED_BYTES: usize = 16 * 1024;

/// First answer/reasoning deltas are immediate. Later adjacent deltas share an
/// event only until this chunk ends, their type changes, a control event arrives,
/// or the byte cap is reached. Drop flushes partial text even on a parsing error.
pub(super) struct StreamBatch<'a> {
    tx: &'a Sender<Event>,
    pending: RefCell<Option<Event>>,
    text_started: Cell<bool>,
    reasoning_started: Cell<bool>,
}

impl<'a> StreamBatch<'a> {
    pub(super) fn new(tx: &'a Sender<Event>, text_started: bool, reasoning_started: bool) -> Self {
        Self {
            tx,
            pending: RefCell::new(None),
            text_started: Cell::new(text_started),
            reasoning_started: Cell::new(reasoning_started),
        }
    }

    fn flush(&self) -> Result<(), ClosedStream> {
        if let Some(event) = self.pending.take() {
            self.tx.send(event).map_err(|_| ClosedStream)?;
        }
        Ok(())
    }
}

impl StreamSink for StreamBatch<'_> {
    fn send(&self, event: Event) -> Result<(), ClosedStream> {
        let started = match &event {
            Event::Text(text) if !text.is_empty() => Some(&self.text_started),
            Event::Reasoning(text) if !text.is_empty() => Some(&self.reasoning_started),
            Event::Text(_) | Event::Reasoning(_) => None,
            _ => {
                self.flush()?;
                return self.tx.send(event).map_err(|_| ClosedStream);
            }
        };
        if let Some(started) = started
            && !started.replace(true)
        {
            self.flush()?;
            return self.tx.send(event).map_err(|_| ClosedStream);
        }
        let mut pending = self.pending.borrow_mut();
        if let Some(prior) = pending.as_mut() {
            let pair = match (prior, &event) {
                (Event::Text(prior), Event::Text(next))
                | (Event::Reasoning(prior), Event::Reasoning(next)) => Some((prior, next)),
                _ => None,
            };
            if let Some((prior, next)) = pair
                && prior.len().saturating_add(next.len()) <= MAX_COALESCED_BYTES
            {
                prior.push_str(next);
                return Ok(());
            }
        }
        let prior = pending.replace(event);
        drop(pending);
        if let Some(prior) = prior {
            self.tx.send(prior).map_err(|_| ClosedStream)?;
        }
        Ok(())
    }
}

impl Drop for StreamBatch<'_> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_and_direct_parsing_preserve_text_reasoning_usage_and_tool_arguments() {
        use super::super::{SseDecoder, Turn, parse_event, parse_stream_events};
        use crate::state::Provider;
        use serde_json::json;
        for provider in [
            Provider::Codex,
            Provider::OpenAI,
            Provider::OpenRouter,
            Provider::Llama,
        ] {
            let responses = matches!(provider, Provider::Codex | Provider::OpenAI);
            let events = if responses {
                vec![
                    json!({"type":"response.reasoning_summary_text.delta","delta":"thinking 🌿"}),
                    json!({"type":"response.reasoning_summary_text.delta","delta":" more"}),
                    json!({"type":"response.output_text.delta","delta":"answer café"}),
                    json!({"type":"response.output_text.delta","delta":" continued"}),
                    json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"item-1","call_id":"call-1","name":"read_file","arguments":""}}),
                    json!({"type":"response.function_call_arguments.delta","item_id":"item-1","delta":"{\"path\":"}),
                    json!({"type":"response.function_call_arguments.delta","item_id":"item-1","delta":"\"source🌿.rs\"}"}),
                    json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"reasoning","id":"reasoning-1","encrypted_content":"opaque state"}],"usage":{"input_tokens":42,"output_tokens":10}}}),
                ]
            } else {
                vec![
                    json!({"choices":[{"delta":{"reasoning_content":"thinking 🌿","content":"answer café","reasoning_details":[{"index":0,"type":"reasoning.encrypted","data":"opaque "}],"tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}]}),
                    json!({"choices":[{"delta":{"reasoning_content":" more","content":" continued","reasoning_details":[{"index":0,"data":"state"}],"tool_calls":[{"index":0,"function":{"arguments":"\"source🌿.rs\"}"}}]},"finish_reason":null}]}),
                    json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":42,"completion_tokens":10}}),
                ]
            };
            let stream = super::super::tests::sse(&events) + "data: [DONE]\n\n";
            for chunk_size in [1, 7, 16 * 1024] {
                let mut expected = None;
                for batched in [false, true] {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let mut decoder = SseDecoder::default();
                    let mut turn = Turn::default();
                    for chunk in stream.as_bytes().chunks(chunk_size) {
                        let events = decoder.push(chunk).unwrap();
                        if batched {
                            parse_stream_events(provider, events, &mut turn, &tx).unwrap();
                        } else {
                            for event in events {
                                parse_event(provider, &event, &mut turn, &tx).unwrap();
                            }
                        }
                    }
                    assert!(decoder.finish().unwrap().is_empty());
                    let mut text = String::new();
                    let mut reasoning = String::new();
                    let mut usage = Vec::new();
                    for event in rx.try_iter() {
                        match event {
                            Event::Text(delta) => text.push_str(&delta),
                            Event::Reasoning(delta) => reasoning.push_str(&delta),
                            Event::Usage(tokens) => usage.push(tokens),
                            _ => panic!("unexpected parser event"),
                        }
                    }
                    assert_eq!(text, "answer café continued");
                    assert_eq!(reasoning, "thinking 🌿 more");
                    assert!(turn.terminal);
                    assert_eq!(turn.input_tokens, Some(42));
                    assert_eq!(turn.output_tokens, 10);
                    let calls: Vec<_> = turn
                        .calls
                        .values()
                        .map(|call| (&call.id, &call.name, &call.arguments))
                        .collect();
                    assert_eq!(calls.len(), 1);
                    assert_eq!(calls[0].0, "call-1");
                    assert_eq!(calls[0].1, "read_file");
                    assert_eq!(calls[0].2, "{\"path\":\"source🌿.rs\"}");
                    let result = (text, reasoning, usage, turn.output, turn.details);
                    if let Some(expected) = &expected {
                        assert_eq!(&result, expected);
                    } else {
                        expected = Some(result);
                    }
                }
            }
        }
    }

    #[test]
    fn first_tokens_are_immediate_and_adjacent_unicode_deltas_flush_at_chunk_end() {
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let batch = StreamBatch::new(&tx, false, false);
            batch.send(Event::Text("first 🌿".into())).unwrap();
            assert!(matches!(rx.try_recv().unwrap(), Event::Text(text) if text == "first 🌿"));
            batch.send(Event::Text(" café".into())).unwrap();
            batch.send(Event::Text(" 世界".into())).unwrap();
            assert!(matches!(
                rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
        }
        assert!(matches!(rx.try_recv().unwrap(), Event::Text(text) if text == " café 世界"));
        {
            let batch = StreamBatch::new(&tx, true, false);
            batch.send(Event::Reasoning("think 🌿".into())).unwrap();
            assert!(matches!(rx.try_recv().unwrap(), Event::Reasoning(text) if text == "think 🌿"));
            batch.send(Event::Reasoning(" then".into())).unwrap();
            batch.send(Event::Reasoning(" answer".into())).unwrap();
        }
        assert!(matches!(rx.try_recv().unwrap(), Event::Reasoning(text) if text == " then answer"));
    }

    #[test]
    fn types_and_control_events_are_ordered_and_text_is_not_buffered_across_chunks() {
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let batch = StreamBatch::new(&tx, true, true);
            batch.send(Event::Text("answer".into())).unwrap();
            batch.send(Event::Reasoning("reasoning".into())).unwrap();
            batch.send(Event::Reasoning(" continued".into())).unwrap();
            batch.send(Event::Usage(42)).unwrap();
            batch.send(Event::Text("tail".into())).unwrap();
        }
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], Event::Text(text) if text == "answer"));
        assert!(matches!(&events[1], Event::Reasoning(text) if text == "reasoning continued"));
        assert!(matches!(events[2], Event::Usage(42)));
        assert!(matches!(&events[3], Event::Text(text) if text == "tail"));
        {
            let batch = StreamBatch::new(&tx, true, true);
            batch.send(Event::Text("next chunk".into())).unwrap();
        }
        assert!(matches!(rx.try_recv().unwrap(), Event::Text(text) if text == "next chunk"));
    }

    #[test]
    fn byte_caps_limit_merged_events_without_splitting_unicode_or_losing_text() {
        let (tx, rx) = std::sync::mpsc::channel();
        let delta = "🌿".repeat(2048);
        {
            let batch = StreamBatch::new(&tx, true, true);
            for _ in 0..7 {
                batch.send(Event::Text(delta.clone())).unwrap();
            }
        }
        let mut text = String::new();
        let mut events = 0;
        for event in rx.try_iter() {
            let Event::Text(part) = event else { panic!() };
            assert!(part.len() <= MAX_COALESCED_BYTES);
            text.push_str(&part);
            events += 1;
        }
        assert_eq!(events, 4);
        assert_eq!(text, delta.repeat(7));
    }

    #[test]
    fn failed_parsing_flushes_partial_deltas_before_the_error_and_never_authorizes_tools() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut turn = super::super::Turn::default();
        let result = (|| {
            let batch = StreamBatch::new(&tx, false, false);
            super::super::parse_event(
                crate::state::Provider::Codex,
                r#"{"type":"response.output_text.delta","delta":"first "}"#,
                &mut turn,
                &batch,
            )?;
            super::super::parse_event(
                crate::state::Provider::Codex,
                r#"{"type":"response.output_text.delta","delta":"partial 🌿"}"#,
                &mut turn,
                &batch,
            )?;
            super::super::parse_event(
                crate::state::Provider::Codex,
                r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"not-authorized","name":"write_file","arguments":"{}"}}"#,
                &mut turn,
                &batch,
            )?;
            super::super::parse_event(
                crate::state::Provider::Codex,
                r#"{"error":{"message":"fixture failure"}}"#,
                &mut turn,
                &batch,
            )
        })();
        tx.send(Event::Error(result.unwrap_err())).unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], Event::Text(text) if text == "first "));
        assert!(matches!(&events[1], Event::Text(text) if text == "partial 🌿"));
        assert!(matches!(&events[2], Event::Error(error) if error.contains("fixture failure")));
        assert!(!turn.terminal);
        assert!(turn.calls.is_empty());
    }

    #[test]
    fn a_disconnected_receiver_does_not_panic_during_flush_or_drop() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(rx);
        let batch = StreamBatch::new(&tx, true, true);
        batch.send(Event::Text("pending".into())).unwrap();
        assert!(batch.send(Event::Completed).is_err());
    }

    #[test]
    #[ignore = "manual streaming parser and UI event batching measurement"]
    fn profile_stream_event_batching() {
        use std::{hint::black_box, time::Instant};
        for provider in [crate::state::Provider::Codex, crate::state::Provider::Llama] {
            let line = if provider == crate::state::Provider::Codex {
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\" café 🌿\"}\n\n"
            } else {
                "data: {\"choices\":[{\"delta\":{\"content\":\" café 🌿\"},\"finish_reason\":null}]}\n\n"
            };
            let input = line.repeat(50_000);
            for batched in [false, true] {
                let (tx, rx) = std::sync::mpsc::channel();
                let mut decoder = super::super::SseDecoder::default();
                let mut turn = super::super::Turn::default();
                let started = Instant::now();
                for chunk in input.as_bytes().chunks(16 * 1024) {
                    if batched {
                        let batch = StreamBatch::new(
                            &tx,
                            !turn.text.is_empty(),
                            !turn.reasoning.is_empty(),
                        );
                        for event in decoder.push(black_box(chunk)).unwrap() {
                            super::super::parse_event(provider, &event, &mut turn, &batch).unwrap();
                        }
                    } else {
                        for event in decoder.push(black_box(chunk)).unwrap() {
                            super::super::parse_event(provider, &event, &mut turn, &tx).unwrap();
                        }
                    }
                }
                assert!(decoder.finish().unwrap().is_empty());
                let elapsed = started.elapsed();
                let mut events = 0;
                let mut visible = String::new();
                for event in rx.try_iter() {
                    if let Event::Text(text) = event {
                        visible.push_str(&text);
                        events += 1;
                    }
                }
                assert_eq!(visible, turn.text);
                assert_eq!(visible, " café 🌿".repeat(50_000));
                println!(
                    "{provider:?} 50,000 Unicode deltas batched={batched}: {:.2} ms decode/parse/enqueue, {events} UI text events",
                    elapsed.as_secs_f64() * 1000.0
                );
            }
        }
    }
}

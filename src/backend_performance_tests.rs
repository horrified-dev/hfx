//! Synthetic wire-context publication measurements.
use super::tests::{mock_server, sse, text_completion};
use super::*;

#[test]
fn tool_context_publications_contain_only_new_items_and_skip_empty_suffixes() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut context = Vec::new();
    let mut saved = Vec::new();
    for round in 0..14 {
        let start = context.len();
        context.push(json!({"type":"function_call","call_id":format!("call-{round}"),"name":"read_file","arguments":"{}"}));
        context.push(json!({"type":"function_call_output","call_id":format!("call-{round}"),"output":"tool output 🌿"}));
        publish_tool_context(&tx, &context, start);
        let Event::ResponsesContextAppend(items) = rx.try_recv().unwrap() else {
            panic!("tool updates must append, not republish their whole prefix");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items, context[start..]);
        saved.extend(items);
    }
    assert_eq!(saved, context);
    publish_tool_context(&tx, &context, context.len());
    assert!(matches!(
        rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_round_deltas_reconstruct_the_wire_prefix_and_survive_later_provider_errors() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        for failed in [false, true] {
            let responses = matches!(provider, Provider::Codex | Provider::OpenAI);
            let mut streams = Vec::new();
            for round in 0..2 {
                let id = format!("read-{round}");
                let args = json!({"path":"sample.txt"}).to_string();
                streams.push(if responses {
                    sse(&[json!({"type":"response.completed","response":{"status":"completed","output":[
                        {"type":"function_call","call_id":id,"name":"read_file","arguments":args}
                    ]}})])
                } else {
                    sse(&[json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"function":{"name":"read_file","arguments":args}}]},"finish_reason":"tool_calls"}]})])
                });
            }
            streams.push(if failed {
                sse(&[json!({"error":{"message":"fixture failure"}})])
            } else {
                text_completion(provider, "Done", 1000)
            });
            let (url, requests, server) = mock_server(streams);
            let root = tempfile::tempdir().unwrap();
            std::fs::write(
                root.path().join("sample.txt"),
                "tool data 🌿\n".repeat(2000),
            )
            .unwrap();
            let settings = Settings {
                provider,
                openai_url: url.clone(),
                openrouter_url: url.clone(),
                llama_url: url.clone(),
                openai_key: "fixture".into(),
                openrouter_key: "fixture".into(),
                auto_compact: false,
                ..Default::default()
            };
            let (tx, rx) = std::sync::mpsc::channel();
            run(
                Request {
                    project_trusted: true,
                    settings,
                    workspace: root.path().into(),
                    history: vec![Message::new(
                        true,
                        "Read the file twice.".into(),
                        0.0,
                        String::new(),
                    )],
                    session: "delta-regression".into(),
                    codex_auth: (provider == Provider::Codex)
                        .then(|| crate::codex::Auth::fixture(url)),
                },
                tx,
            )
            .await;
            server.join().unwrap();
            let mut prefix = Vec::new();
            let mut rounds = 0;
            let mut completed = false;
            let mut error = false;
            for event in rx.try_iter() {
                match event {
                    Event::ResponsesContextAppend(items) => {
                        assert_eq!(
                            items.len(),
                            2,
                            "{provider:?}: each tool round is published once"
                        );
                        prefix.extend(items);
                        rounds += 1;
                    }
                    Event::ResponsesContext(items) => {
                        assert!(
                            !failed,
                            "failed responses must leave completed tool deltas intact"
                        );
                        assert_eq!(prefix, items[..prefix.len()]);
                        assert_eq!(items.len(), prefix.len() + 1);
                    }
                    Event::Completed => completed = true,
                    Event::Error(message) => {
                        assert!(message.contains("fixture failure"), "{message}");
                        error = true;
                    }
                    _ => {}
                }
            }
            assert_eq!(rounds, 2);
            assert_eq!(prefix.len(), 4);
            assert_eq!(completed, !failed);
            assert_eq!(error, failed);
            let bodies: Vec<_> = requests.try_iter().collect();
            assert_eq!(bodies.len(), 3);
            let wire = if responses {
                &bodies[2]["input"]
            } else {
                &bodies[2]["messages"]
            };
            assert!(
                wire.as_array()
                    .unwrap()
                    .windows(prefix.len())
                    .any(|window| window == prefix),
                "published context must match the next provider request"
            );
        }
    }
}

#[test]
#[ignore = "manual tool-context publication measurement"]
fn profile_tool_context_publishing() {
    use std::{hint::black_box, time::Instant};
    let payload = "recorded tool output\n".repeat(3276);
    for rounds in [16, 64, 128] {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut context = Vec::new();
        let mut published_items = 0;
        let mut published_text_bytes = 0;
        let start = Instant::now();
        for round in 0..rounds {
            let offset = context.len();
            context.push(
                json!({"role":"tool","tool_call_id":format!("tool-{round}"),"content":payload}),
            );
            publish_tool_context(&tx, black_box(&context), offset);
            match rx.recv().unwrap() {
                Event::ResponsesContext(items) | Event::ResponsesContextAppend(items) => {
                    published_items += items.len();
                    published_text_bytes += items
                        .iter()
                        .map(|item| item["content"].as_str().unwrap().len())
                        .sum::<usize>();
                    black_box(items);
                }
                _ => panic!("Expected a context publication"),
            }
        }
        println!(
            "{rounds} tool rounds: {:.2} ms, {published_items} published items, {:.2} MiB published text",
            start.elapsed().as_secs_f64() * 1000.0,
            published_text_bytes as f64 / (1024.0 * 1024.0)
        );
    }
}

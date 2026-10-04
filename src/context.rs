//! Provider-neutral context budgeting. Visible messages are never removed.
use crate::state::{ContextCheckpoint, Message, Provider, Settings};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const COMPACT_PERCENT: u64 = 75;

/// A provider count can anchor later estimates without re-tokenizing the old
/// transcript. Any estimated growth must remain visibly approximate until the
/// provider supplies a fresh usage report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub tokens: u64,
    pub estimated: bool,
}

impl Usage {
    pub fn reported(tokens: u64) -> Self {
        Self {
            tokens,
            estimated: false,
        }
    }

    pub fn estimated(tokens: u64) -> Self {
        Self {
            tokens,
            estimated: true,
        }
    }

    pub fn with_growth(self, tokens: u64) -> Self {
        Self {
            tokens: self.tokens.saturating_add(tokens),
            estimated: self.estimated || tokens > 0,
        }
    }
}

pub fn at_threshold(tokens: u64, limit: u64) -> bool {
    tokens.saturating_mul(100) >= limit.saturating_mul(COMPACT_PERCENT)
}

/// Conservative fallback, not a tokenizer. Provider usage supersedes this when available.
/// Do not count image base64 as text; reserve a visual token allowance instead.
pub fn estimate(value: &Value) -> u64 {
    match value {
        Value::String(s) if s.starts_with("data:image/") => 4096,
        Value::String(s) => (s.len() as u64).div_ceil(3),
        Value::Array(values) => estimate_items(values),
        Value::Object(fields) if value["type"] == "image_generation_call" => {
            4096 + fields
                .iter()
                .filter(|(key, _)| key.as_str() != "result")
                .map(|(_, value)| estimate(value))
                .sum::<u64>()
        }
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| (key.len() as u64).div_ceil(3) + estimate(value) + 4)
            .sum(),
        _ => 4,
    }
}

/// Estimate borrowed history directly, without cloning multi-megabyte tool
/// outputs into a temporary JSON array on every request/tool boundary.
pub fn estimate_items(values: &[Value]) -> u64 {
    values.iter().map(estimate).sum::<u64>() + values.len() as u64 * 8
}

/// Summary requests receive readable reference data, not opaque state or image payloads.
pub fn readable(value: &Value) -> Value {
    match value {
        Value::String(s) if s.starts_with("data:image/") => {
            json!("[image; pixels retained in recent context when applicable]")
        }
        Value::Array(values) => Value::Array(values.iter().map(readable).collect()),
        Value::Object(fields) if value["type"] == "image_generation_call" => {
            let mut result = fields.clone();
            result.insert(
                "result".into(),
                json!("[generated image; pixels omitted from summary]"),
            );
            Value::Object(result)
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| {
                    !matches!(key.as_str(), "encrypted_content" | "reasoning_details")
                })
                .map(|(key, value)| (key.clone(), readable(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub fn compatible(checkpoint: &ContextCheckpoint, settings: &Settings) -> bool {
    checkpoint.connection == settings.context_key()
}

/// A checkpoint replaces only model input, including tool rounds already summarized.
/// Never replay the checkpoint message's old activities or flattened visible text.
pub fn replay(
    messages: &[Message],
    settings: &Settings,
    items: impl Fn(&Message, Provider) -> Vec<Value>,
) -> Vec<Value> {
    let checkpoint = messages.iter().rposition(|m| {
        m.context_checkpoint
            .as_ref()
            .is_some_and(|c| compatible(c, settings))
    });
    if let Some(index) = checkpoint {
        let mut history = (*messages[index].context_checkpoint.as_ref().unwrap().history).clone();
        history.extend(messages[index].response_items.iter().cloned());
        history.extend(
            messages[index + 1..]
                .iter()
                .flat_map(|m| items(m, settings.provider)),
        );
        history
    } else {
        messages
            .iter()
            .flat_map(|m| items(m, settings.provider))
            .collect()
    }
}

fn user_request(value: &Value) -> bool {
    value["role"] == "user"
        && !value["content"]
            .as_array()
            .and_then(|parts| parts.first())
            .and_then(|part| part["text"].as_str())
            .is_some_and(|text| text.starts_with("Reference image returned by the tool."))
}

pub struct Plan {
    pub prefix: Vec<Value>,
    pub tail: Vec<Value>,
}

/// Cut only at boundaries with all tool calls answered. Retain the latest request
/// verbatim, plus as much recent history (including images) as the budget permits.
pub fn plan(history: &[Value], limit: u64, fixed_cost: u64) -> Option<Plan> {
    let latest_user = history.iter().rposition(user_request)?;
    let target = limit / 2;
    let mut pending = HashSet::new();
    // Maintain the remaining cost instead of rescanning every suffix (O(n²)).
    let mut remaining_cost = estimate_items(history);
    let latest_user_cost = estimate(&history[latest_user]) + 8;
    for (index, item) in history.iter().enumerate() {
        remaining_cost -= estimate(item) + 8;
        if item["type"] == "function_call"
            && let Some(id) = item["call_id"].as_str()
        {
            pending.insert(id);
        }
        if let Some(calls) = item["tool_calls"].as_array() {
            for call in calls {
                if let Some(id) = call["id"].as_str() {
                    pending.insert(id);
                }
            }
        }
        if item["type"] == "function_call_output"
            && let Some(id) = item["call_id"].as_str()
        {
            pending.remove(id);
        }
        if item["role"] == "tool"
            && let Some(id) = item["tool_call_id"].as_str()
        {
            pending.remove(id);
        }
        let cut = index + 1;
        // There must be older context to summarize; never summarize away the
        // latest user input itself, nor manufacture an empty final conversation.
        if !pending.is_empty()
            || (cut == history.len() && latest_user == index)
            || (cut == 1 && latest_user == 0)
        {
            continue;
        }
        let repeated_user = latest_user < cut;
        let tail_cost = remaining_cost + if repeated_user { latest_user_cost } else { 0 };
        if tail_cost + fixed_cost <= target {
            let mut tail = history[cut..].to_vec();
            if repeated_user {
                tail.insert(0, history[latest_user].clone());
            }
            return Some(Plan {
                prefix: history[..cut].to_vec(),
                tail,
            });
        }
    }
    None
}

pub fn checkpoint(settings: &Settings, history: Vec<Value>) -> ContextCheckpoint {
    ContextCheckpoint {
        connection: settings.context_key(),
        history: history.into(),
    }
}

/// Bound summarization inputs without splitting UTF-8. Chunking the readable
/// transcript allows compaction even when old history is already over the limit.
pub fn chunks(text: &str, max_bytes: usize) -> Vec<&str> {
    let mut rest = text;
    let mut parts = Vec::new();
    while !rest.is_empty() {
        let mut end = max_bytes.max(4).min(rest.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&rest[..end]);
        rest = &rest[end..];
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_growth_preserves_provenance_and_saturates_without_overflow() {
        let usage = Usage::reported(180_000);
        assert_eq!(usage.with_growth(0), usage);
        assert_eq!(usage.with_growth(100), Usage::estimated(180_100));
        assert_eq!(
            Usage::estimated(180_100).with_growth(0),
            Usage::estimated(180_100)
        );
        assert_eq!(
            Usage::reported(u64::MAX).with_growth(1),
            Usage::estimated(u64::MAX)
        );
    }

    #[test]
    fn planning_chooses_the_earliest_affordable_safe_cut() {
        for responses in [false, true] {
            let call = if responses {
                json!({"type":"function_call","call_id":"a","name":"read_file","arguments":"{}"})
            } else {
                json!({"role":"assistant","tool_calls":[{"id":"a","function":{"name":"read_file","arguments":"{}"}}]})
            };
            let result = if responses {
                json!({"type":"function_call_output","call_id":"a","output":"old result"})
            } else {
                json!({"role":"tool","tool_call_id":"a","content":"old result"})
            };
            let user = json!({"role":"user","content":"latest request"});
            let history = vec![
                json!({"role":"user","content":"old context"}),
                call,
                result,
                user.clone(),
                json!({"role":"assistant","content":"recent response"}),
            ];
            for fixed in [0, 100] {
                for limit in (0..1000).step_by(7) {
                    let expected = [1, 3, 4, 5].into_iter().find(|cut| {
                        let repeated = if *cut > 3 { estimate(&user) + 8 } else { 0 };
                        estimate_items(&history[*cut..]) + repeated + fixed <= limit / 2
                    });
                    let actual = plan(&history, limit, fixed);
                    assert_eq!(actual.as_ref().map(|p| p.prefix.len()), expected);
                    if let Some(plan) = actual {
                        let cut = plan.prefix.len();
                        let mut tail = history[cut..].to_vec();
                        if cut > 3 {
                            tail.insert(0, user.clone());
                        }
                        assert_eq!(plan.prefix, history[..cut]);
                        assert_eq!(plan.tail, tail);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "manual context compaction planner latency measurement"]
    fn profile_compaction_planning() {
        use std::{hint::black_box, time::Instant};
        let mut history = vec![json!({"role":"user","content":"keep this request"})];
        history.extend(
            (0..3000).map(|_| json!({"role":"assistant","content":"response".repeat(100)})),
        );
        let started = Instant::now();
        assert!(plan(black_box(&history), 1024, 100).is_some());
        println!(
            "compaction plan, {} messages: {:.2} ms",
            history.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    #[test]
    fn threshold_and_images_are_budgeted_without_base64_inflation() {
        assert!(!at_threshold(7499, 10000));
        assert!(at_threshold(7500, 10000));
        assert!(at_threshold(9000, 10000));
        assert_eq!(
            estimate(&json!(format!(
                "data:image/png;base64,{}",
                "x".repeat(100000)
            ))),
            4096
        );
        assert!(
            !readable(
                &json!({"encrypted_content":"secret","image_url":"data:image/png;base64,pixels"})
            )
            .to_string()
            .contains("pixels\"")
        );
        let generated = json!({"type":"image_generation_call","result":"RAW_PIXELS".repeat(10000)});
        assert!(estimate(&generated) < 4200);
        assert!(!readable(&generated).to_string().contains("RAW_PIXELS"));
        assert_eq!(chunks("a🌿b🌿c", 5).concat(), "a🌿b🌿c");
    }
    #[test]
    fn cuts_never_split_tool_pairs_and_preserve_latest_request_and_image() {
        for responses in [false, true] {
            let call = if responses {
                json!({"type":"function_call","call_id":"a","name":"read_file","arguments":"{}"})
            } else {
                json!({"role":"assistant","tool_calls":[{"id":"a","function":{"name":"read_file","arguments":"{}"}}]})
            };
            let result = if responses {
                json!({"type":"function_call_output","call_id":"a","output":"old result"})
            } else {
                json!({"role":"tool","tool_call_id":"a","content":"old result"})
            };
            let latest = json!({"role":"user","content":[{"type":"text","text":"continue"},{"type":"image_url","image_url":{"url":"data:image/png;base64,live"}}]});
            let history = vec![
                json!({"role":"user","content":"old".repeat(15000)}),
                call,
                result,
                latest.clone(),
            ];
            let plan = plan(&history, 30000, 100).unwrap();
            assert!(
                matches!(plan.prefix.len(), 1 | 3),
                "never cut between call and result"
            );
            assert_eq!(plan.tail.last(), Some(&latest));
        }
    }
    #[test]
    fn checkpoint_roundtrip_keeps_ui_history_but_does_not_replay_old_actions() {
        let settings = Settings {
            provider: Provider::Llama,
            ..Default::default()
        };
        let mut old = Message::new(false, "visible old answer".into(), 0.0, "llama.cpp".into());
        old.context_checkpoint = Some(checkpoint(
            &settings,
            vec![json!({"role":"user","content":"summary"})],
        ));
        old.response_items = vec![json!({"role":"assistant","content":"after summary"})].into();
        let new = Message::new(true, "follow up".into(), 0.0, String::new());
        let messages: Vec<Message> =
            serde_json::from_str(&serde_json::to_string(&vec![old, new]).unwrap()).unwrap();
        let wire = replay(&messages, &settings, |message, _| {
            vec![json!({"role":"user","content":message.text})]
        });
        assert_eq!(wire.len(), 3);
        assert_eq!(messages[0].text, "visible old answer");
        assert!(!json!(wire).to_string().contains("visible old answer"));
    }
    #[test]
    #[ignore = "manual allocation/latency measurement"]
    fn profile_large_history_budgeting() {
        use std::{hint::black_box, time::Instant};
        let body = "source and command output\n".repeat(2600);
        let history: Vec<Value> = (0..64)
            .map(|_| json!({"role":"tool","content":body}))
            .collect();
        assert_eq!(estimate(&json!(history)), estimate_items(&history));
        for borrowed in [false, true] {
            let started = Instant::now();
            for _ in 0..500 {
                black_box(if borrowed {
                    estimate_items(black_box(&history))
                } else {
                    estimate(&json!(black_box(&history)))
                });
            }
            println!(
                "context estimate borrowed={borrowed}: {:.2} ms / 500 iterations ({} KiB history)",
                started.elapsed().as_secs_f64() * 1000.0,
                history.len() * body.len() / 1024
            );
        }
    }
}

//! Borrowed provider request bodies: serialize history once, without JSON copies.
use crate::state::{Provider, Settings};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use serde_json::Value;

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum RequestBody<'a> {
    Responses(ResponsesBody<'a>),
    Chat(ChatBody<'a>),
}

#[derive(Serialize)]
pub(super) struct ResponsesBody<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [Value],
    stream: bool,
    store: bool,
    reasoning: Reasoning<'a>,
    include: [&'static str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Value]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
}

#[derive(Serialize)]
struct Reasoning<'a> {
    effort: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude: Option<bool>,
}

#[derive(Serialize)]
pub(super) struct ChatBody<'a> {
    model: &'a str,
    messages: SystemHistory<'a>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_format: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    max_tokens: u64,
    // Match json!(settings.temperature)'s f32-to-f64 representation exactly.
    temperature: f64,
    top_k: i32,
    top_p: f32,
    presence_penalty: f32,
    repeat_penalty: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Value]>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

struct SystemHistory<'a> {
    instructions: &'a str,
    history: &'a [Value],
}

impl Serialize for SystemHistory<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct SystemMessage<'a> {
            role: &'static str,
            content: &'a str,
        }
        let mut sequence = serializer.serialize_seq(Some(self.history.len() + 1))?;
        sequence.serialize_element(&SystemMessage {
            role: "system",
            content: self.instructions,
        })?;
        for item in self.history {
            sequence.serialize_element(item)?;
        }
        sequence.end()
    }
}

impl<'a> RequestBody<'a> {
    pub(super) fn new(
        settings: &'a Settings,
        instructions: &'a str,
        history: &'a [Value],
        tools: &'a [Value],
        session: &'a str,
    ) -> Self {
        let tools = settings.tools_enabled.then_some(tools);
        let output_limit = super::output_reserve(settings);
        if matches!(settings.provider, Provider::OpenAI | Provider::Codex) {
            let codex = settings.provider == Provider::Codex;
            Self::Responses(ResponsesBody {
                model: settings.model(),
                instructions,
                input: history,
                stream: true,
                store: false,
                reasoning: Reasoning {
                    effort: &settings.effort,
                    summary: settings.show_reasoning.then_some("auto"),
                    exclude: None,
                },
                include: ["reasoning.encrypted_content"],
                max_output_tokens: (!codex).then_some(output_limit),
                tools,
                prompt_cache_key: codex.then_some(session),
                tool_choice: codex.then_some("auto"),
                parallel_tool_calls: codex.then_some(true),
            })
        } else {
            let router = settings.provider == Provider::OpenRouter;
            Self::Chat(ChatBody {
                model: settings.model(),
                messages: SystemHistory {
                    instructions,
                    history,
                },
                stream: true,
                stream_options: StreamOptions {
                    include_usage: true,
                },
                reasoning: router.then_some(Reasoning {
                    effort: &settings.effort,
                    summary: None,
                    exclude: Some(!settings.show_reasoning),
                }),
                reasoning_format: (!router).then_some("deepseek"),
                reasoning_effort: (!router).then_some(&settings.effort),
                max_tokens: output_limit,
                temperature: f64::from(settings.temperature),
                top_p: f32::from(settings.top_p),
                top_k: i32::from(settings.top_k),
                presence_penalty: f32::from(settings.presence_penalty),
                repeat_penalty: f32::from(settings.repeat_penalty),
                tools,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The previous production body construction, kept only for parity checks
    // and comparative benchmarks. Never sends a request to a real provider.
    fn legacy_body(
        settings: &Settings,
        instructions: &str,
        history: &[Value],
        tools: &[Value],
        session: &str,
    ) -> Value {
        let openai = matches!(settings.provider, Provider::OpenAI | Provider::Codex);
        let mut body = if openai {
            let mut reasoning = json!({"effort":settings.effort});
            if settings.show_reasoning {
                reasoning["summary"] = json!("auto");
            }
            json!({"model":settings.model(),"instructions":instructions,"input":history,"stream":true,"store":false,
                "reasoning":reasoning,"include":["reasoning.encrypted_content"],"max_output_tokens":super::super::output_reserve(settings)})
        } else if settings.provider == Provider::OpenRouter {
            json!({"model":settings.model(),"messages":super::super::with_system(history, instructions),"stream":true,"stream_options":{"include_usage":true},
                "reasoning":{"effort":settings.effort,"exclude":!settings.show_reasoning},
                "max_tokens":super::super::output_reserve(settings),"temperature":settings.temperature})
        } else {
            json!({"model":settings.model(),"messages":super::super::with_system(history, instructions),"stream":true,"stream_options":{"include_usage":true},
                "reasoning_format":"deepseek","reasoning_effort":settings.effort,
                "max_tokens":super::super::output_reserve(settings),"temperature":settings.temperature})
        };
        if settings.tools_enabled {
            body["tools"] = json!(tools);
        }
        if settings.provider == Provider::Codex {
            body.as_object_mut().unwrap().remove("max_output_tokens");
            body["prompt_cache_key"] = json!(session);
            body["tool_choice"] = json!("auto");
            body["parallel_tool_calls"] = json!(true);
        }
        body
    }

    #[test]
    fn borrowed_bodies_match_legacy_wire_fields_for_all_providers_and_flags() {
        let history = vec![
            json!({"role":"user","content":"Read café 🌿\nquoted \"source\""}),
            json!({"type":"function_call_output","call_id":"tool-1","output":"line 1\nline 2"}),
            json!({"type":"reasoning","encrypted_content":"opaque state"}),
            json!({"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,fixture"}}]}),
        ];
        let tools =
            vec![json!({"type":"function","name":"read_file","parameters":{"type":"object"}})];
        for provider in [
            Provider::Codex,
            Provider::OpenAI,
            Provider::OpenRouter,
            Provider::Llama,
        ] {
            for show_reasoning in [false, true] {
                for tools_enabled in [false, true] {
                    for max_tokens in [0, 300, 32768] {
                        for temperature in [0.0, 0.7, 1.0] {
                            let settings = Settings {
                                provider,
                                show_reasoning,
                                tools_enabled,
                                max_tokens,
                                temperature,
                                context_window: 1024,
                                ..Default::default()
                            };
                            let body = RequestBody::new(
                                &settings,
                                "System instructions 🌿",
                                &history,
                                &tools,
                                "session-1",
                            );
                            let serialized = serde_json::to_vec(&body).unwrap();
                            let value: Value = serde_json::from_slice(&serialized).unwrap();
                            assert_eq!(
                                value,
                                legacy_body(
                                    &settings,
                                    "System instructions 🌿",
                                    &history,
                                    &tools,
                                    "session-1"
                                ),
                                "{provider:?}, reasoning={show_reasoning}, tools={tools_enabled}, max={max_tokens}, temperature={temperature}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn request_history_is_borrowed_and_chat_serialization_preserves_order() {
        let history = vec![
            json!({"role":"user","content":"first"}),
            json!({"role":"tool","content":"second"}),
        ];
        let tools = vec![json!({"type":"function","name":"read_file"})];
        let mut settings = Settings {
            provider: Provider::OpenAI,
            ..Default::default()
        };
        let RequestBody::Responses(body) =
            RequestBody::new(&settings, "instructions", &history, &tools, "session")
        else {
            panic!()
        };
        assert_eq!(body.input.as_ptr(), history.as_ptr());
        assert_eq!(body.tools.unwrap().as_ptr(), tools.as_ptr());
        settings.provider = Provider::Llama;
        let RequestBody::Chat(body) =
            RequestBody::new(&settings, "instructions", &history, &tools, "session")
        else {
            panic!()
        };
        assert_eq!(body.messages.history.as_ptr(), history.as_ptr());
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(
            value["messages"][0],
            json!({"role":"system","content":"instructions"})
        );
        assert_eq!(value["messages"].as_array().unwrap()[1..], history);
    }

    #[test]
    #[ignore = "manual borrowed request serialization measurement"]
    fn profile_request_serialization() {
        use std::{hint::black_box, time::Instant};
        let payload = "source line with quoted \"output\" 🌿\n".repeat(1900);
        let history: Vec<Value> = (0..64).map(|index| json!({"role":"tool","tool_call_id":format!("tool-{index}"),"content":payload})).collect();
        let tools =
            vec![json!({"type":"function","name":"read_file","parameters":{"type":"object"}})];
        for provider in [
            Provider::Codex,
            Provider::OpenAI,
            Provider::OpenRouter,
            Provider::Llama,
        ] {
            let settings = Settings {
                provider,
                ..Default::default()
            };
            for borrowed in [false, true] {
                let start = Instant::now();
                let mut bytes = 0;
                for _ in 0..50 {
                    let serialized = if borrowed {
                        serde_json::to_vec(&RequestBody::new(
                            &settings,
                            "instructions",
                            black_box(&history),
                            &tools,
                            "session",
                        ))
                        .unwrap()
                    } else {
                        serde_json::to_vec(&legacy_body(
                            &settings,
                            "instructions",
                            black_box(&history),
                            &tools,
                            "session",
                        ))
                        .unwrap()
                    };
                    bytes += serialized.len();
                    black_box(serialized);
                }
                println!(
                    "{provider:?} request borrowed={borrowed}: {:.2} ms / 50 requests, {:.2} MiB/request",
                    start.elapsed().as_secs_f64() * 1000.0,
                    bytes as f64 / 50.0 / (1024.0 * 1024.0)
                );
            }
        }
    }
}

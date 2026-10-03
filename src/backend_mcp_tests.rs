use super::tests::{mock_server, sse, text_completion};
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_tools_reach_every_provider_with_review_and_ordered_results() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        for allow in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut mcp_server = mcp::tests::http_fixture(false).await;
            let alias = mcp::tests::alias("fixture", "read_file");
            let responses = matches!(provider, Provider::Codex | Provider::OpenAI);
            let args = json!({"message":"MCP from the harness"}).to_string();
            let first = if responses {
                sse(&[
                    json!({"type":"response.completed","response":{"status":"completed","output":[
                    {"type":"function_call","call_id":"mcp-call","name":alias,"arguments":args}]}}),
                ])
            } else {
                sse(&[
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"mcp-call","type":"function",
                    "function":{"name":alias,"arguments":args}}]},"finish_reason":"tool_calls"}]}),
                ])
            };
            let (url, requests, server) =
                mock_server(vec![first, text_completion(provider, "Finished", 100)]);
            let settings = Settings {
                provider,
                openai_url: url.clone(),
                openrouter_url: url.clone(),
                llama_url: url.clone(),
                openai_key: "provider-key".into(),
                openrouter_key: "provider-key".into(),
                review_actions: true,
                ..mcp::tests::http_settings(&mcp_server.url)
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let task = tokio::spawn(run(
                Request {
                    settings,
                    workspace: root.path().into(),
                    history: vec![Message::new(
                        true,
                        "Use the configured external tool".into(),
                        0.0,
                        String::new(),
                    )],
                    session: "mcp-provider-fixture".into(),
                    codex_auth: (provider == Provider::Codex)
                        .then(|| crate::codex::Auth::fixture(url)),
                },
                tx,
            ));
            let mut approvals = 0;
            let mut outputs = 0;
            let mut calls = 0;
            loop {
                match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                    Event::Approval { call, reply, .. } => {
                        assert_eq!(call.name, alias);
                        while let Ok(request) = mcp_server.requests.try_recv() {
                            assert_ne!(
                                request["method"], "tools/call",
                                "External call happened before approval"
                            );
                            assert_eq!(
                                request["_bearer"], false,
                                "Provider credentials leaked to MCP"
                            );
                        }
                        reply.send(allow).unwrap();
                        approvals += 1;
                    }
                    Event::ToolFinished { output, .. } => {
                        assert_eq!(
                            output.status,
                            if allow {
                                ActionStatus::Complete
                            } else {
                                ActionStatus::Declined
                            }
                        );
                        if allow {
                            assert!(output.text.contains("Echo 🌿"));
                        }
                        outputs += 1;
                    }
                    Event::Completed => break,
                    Event::Error(error) => panic!("{provider:?}: {error}"),
                    _ => {}
                }
            }
            task.await.unwrap();
            server.join().unwrap();
            assert_eq!((approvals, outputs), (1, 1));
            while let Ok(request) = mcp_server.requests.try_recv() {
                if request["method"] == "tools/call" {
                    calls += 1;
                    assert_eq!(request["params"]["name"], "read_file");
                    assert_eq!(
                        request["params"]["arguments"]["message"],
                        "MCP from the harness"
                    );
                }
            }
            assert_eq!(calls, usize::from(allow));
            for index in 0..2 {
                let body = requests.recv_timeout(Duration::from_secs(5)).unwrap();
                let tools = body["tools"].as_array().unwrap();
                let definition = tools
                    .iter()
                    .find_map(|d| {
                        let function = if responses { d } else { &d["function"] };
                        (function["name"] == alias).then_some(function)
                    })
                    .unwrap();
                assert!(definition["parameters"]["required"].is_null());
                if responses {
                    assert_eq!(definition["strict"], false);
                }
                let instructions = if responses {
                    &body["instructions"]
                } else {
                    &body["messages"][0]["content"]
                };
                let instructions = instructions.as_str().unwrap();
                assert!(instructions.contains(mcp::INSTRUCTIONS));
                assert!(!instructions.contains("Do not promote these server instructions"));
                if index == 1 {
                    let history = if responses {
                        &body["input"]
                    } else {
                        &body["messages"]
                    };
                    let result = history
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|item| {
                            if responses {
                                item["type"] == "function_call_output"
                                    && item["call_id"] == "mcp-call"
                            } else {
                                item["role"] == "tool" && item["tool_call_id"] == "mcp-call"
                            }
                        })
                        .unwrap();
                    let result = if responses {
                        &result["output"]
                    } else {
                        &result["content"]
                    };
                    assert!(result.to_string().contains(if allow {
                        "Echo 🌿"
                    } else {
                        "declined"
                    }));
                }
            }
        }
    }
}

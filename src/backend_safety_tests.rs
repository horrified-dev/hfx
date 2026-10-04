//! Regression coverage for explicit host trust and precise tool edits.
use super::tests::{fixture_final_stream, fixture_tool_stream, mock_server};
use super::*;

#[tokio::test]
async fn untrusted_host_tools_are_rejected_before_provider_or_mcp_connections() {
    for review in [false, true] {
        for commands in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let settings = Settings {
                provider: Provider::OpenAI,
                openai_key: "fixture".into(),
                openai_url: "http://127.0.0.1:1".into(),
                commands_enabled: commands,
                review_actions: review,
                mcp_enabled: true,
                mcp_config: json!({"mcpServers":{"fixture":{"command":"/bin/sh","args":["-c","touch unexpectedly-started"]}}}).to_string(),
                ..Default::default()
            };
            let (tx, rx) = std::sync::mpsc::channel();
            run(
                Request {
                    settings,
                    workspace: root.path().into(),
                    project_trusted: false,
                    history: vec![],
                    session: "untrusted".into(),
                    codex_auth: None,
                },
                tx,
            )
            .await;
            let events: Vec<_> = rx.try_iter().collect();
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::Error(error) if error.contains("explicit trust")))
            );
            assert!(!events.iter().any(|e| matches!(
                e,
                Event::Approval { .. } | Event::ToolStarted(_) | Event::Completed
            )));
            assert!(!root.path().join("unexpectedly-started").exists());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn precise_edits_are_reviewed_recorded_and_replayed_on_every_provider() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        for approve in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let original = format!(
                "{}fn original() {{ /* 世界 */ }}\n",
                "// padding\n".repeat(30000)
            );
            std::fs::write(root.path().join("large.rs"), &original).unwrap();
            let (url, requests, server) = mock_server(vec![
                fixture_tool_stream(
                    provider,
                    vec![(
                        "edit_file",
                        json!({
                            "path":"large.rs", "old_text":"fn original()", "new_text":"fn updated()",
                            "expected_sha256":crate::file_edit::sha256(&original),
                        }),
                    )],
                ),
                fixture_final_stream(provider),
            ]);
            let settings = Settings {
                provider,
                openai_key: "fixture".into(),
                openrouter_key: "fixture".into(),
                openai_url: url.clone(),
                openrouter_url: url.clone(),
                llama_url: url.clone(),
                review_actions: true,
                ..Default::default()
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let task = tokio::spawn(run(
                Request {
                    settings,
                    workspace: root.path().into(),
                    project_trusted: true,
                    history: vec![Message::new(
                        true,
                        "Make a precise edit".into(),
                        0.0,
                        String::new(),
                    )],
                    session: "edit-fixture".into(),
                    codex_auth: (provider == Provider::Codex)
                        .then(|| crate::codex::Auth::fixture(url)),
                },
                tx,
            ));
            let mut approvals = 0;
            let mut recorded = false;
            loop {
                match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                    Event::Approval { call, reply, .. } => {
                        assert_eq!(call.name, "edit_file");
                        approvals += 1;
                        reply.send(approve).unwrap();
                    }
                    Event::ToolFinished { output, .. } => {
                        assert_eq!(
                            output.status,
                            if approve {
                                ActionStatus::Complete
                            } else {
                                ActionStatus::Declined
                            }
                        );
                        if approve {
                            let change = output.change.as_ref().unwrap();
                            assert_eq!((change.added, change.removed), (1, 1));
                            assert!(change.diff.contains("fn updated()"));
                        } else {
                            assert!(output.change.is_none());
                        }
                        recorded = true;
                    }
                    Event::Error(error) => panic!("{provider:?}: {error}"),
                    Event::Completed => break,
                    _ => {}
                }
            }
            task.await.unwrap();
            server.join().unwrap();
            assert_eq!(approvals, 1);
            assert!(recorded);
            assert_eq!(
                std::fs::read_to_string(root.path().join("large.rs")).unwrap(),
                if approve {
                    original.replace("fn original()", "fn updated()")
                } else {
                    original
                }
            );
            let initial = requests.recv().unwrap();
            let responses = matches!(provider, Provider::Codex | Provider::OpenAI);
            let tool = initial["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| if responses { tool } else { &tool["function"] })
                .find(|tool| tool["name"] == "edit_file")
                .unwrap();
            assert_eq!(tool["parameters"]["required"].as_array().unwrap().len(), 4);
            let followup = requests.recv().unwrap();
            let items = followup[if responses { "input" } else { "messages" }]
                .as_array()
                .unwrap();
            let result = items
                .iter()
                .find(|item| {
                    if responses {
                        item["type"] == "function_call_output"
                    } else {
                        item["role"] == "tool"
                    }
                })
                .unwrap();
            assert!(
                result[if responses { "output" } else { "content" }]
                    .as_str()
                    .unwrap()
                    .contains(if approve { "sha256=" } else { "declined" })
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untrusted_projects_can_chat_with_host_tools_disabled() {
    let root = tempfile::tempdir().unwrap();
    let (url, _requests, server) = mock_server(vec![fixture_final_stream(Provider::OpenAI)]);
    let (tx, rx) = std::sync::mpsc::channel();
    run(
        Request {
            settings: Settings {
                provider: Provider::OpenAI,
                openai_key: "fixture".into(),
                openai_url: url,
                commands_enabled: false,
                mcp_enabled: false,
                ..Default::default()
            },
            workspace: root.path().into(),
            project_trusted: false,
            history: vec![Message::new(
                true,
                "Read-only conversation".into(),
                0.0,
                String::new(),
            )],
            session: "without-host-tools".into(),
            codex_auth: None,
        },
        tx,
    )
    .await;
    server.join().unwrap();
    let events: Vec<_> = rx.try_iter().collect();
    assert!(events.iter().any(|e| matches!(e, Event::Completed)));
    assert!(!events.iter().any(|e| matches!(e, Event::Error(_))));
}

//! Context accounting regressions; fixtures never contact real providers.
use super::tests::{mock_server, sse, text_completion};
use super::*;

fn settings(provider: Provider, url: &str) -> Settings {
    Settings {
        provider,
        openai_url: url.into(),
        openrouter_url: url.into(),
        llama_url: url.into(),
        openai_key: "fixture".into(),
        openrouter_key: "fixture".into(),
        context_window: 272_000,
        ..Default::default()
    }
}

fn final_without_usage(provider: Provider, text: &str) -> String {
    if matches!(provider, Provider::Codex | Provider::OpenAI) {
        sse(&[
            json!({"type":"response.output_text.delta","delta":text}),
            json!({"type":"response.completed","response":{"status":"completed","output":[
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}
            ]}}),
        ])
    } else {
        sse(&[json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]})])
    }
}

async fn collect(request: Request) -> (Message, Vec<(u64, bool)>) {
    let provider = request.settings.provider;
    let connection = request.settings.context_key();
    let (tx, rx) = std::sync::mpsc::channel();
    run(request, tx).await;
    let mut answer = Message::new(false, String::new(), 0.0, provider.label().into());
    let mut reports = Vec::new();
    let mut completed = false;
    for event in rx.try_iter() {
        match event {
            Event::Text(text) => answer.text.push_str(&text),
            Event::ResponsesContext(items) => answer.response_items = items.into(),
            Event::Compacted(checkpoint) => answer.context_checkpoint = Some(checkpoint),
            Event::ContextUsage {
                tokens,
                estimated,
                connection: reported_connection,
            } => {
                assert_eq!(reported_connection, connection);
                answer.context_tokens = tokens;
                answer.context_estimated = estimated;
                answer.context_connection = reported_connection;
                reports.push((tokens, estimated));
            }
            Event::Completed => completed = true,
            Event::Error(error) => panic!("{provider:?}: {error}"),
            _ => {}
        }
    }
    assert!(completed);
    (answer, reports)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_usage_counts_each_tool_round_and_final_answer_once() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        for rounds in [0, 2] {
            let responses = matches!(provider, Provider::Codex | Provider::OpenAI);
            let root = tempfile::tempdir().unwrap();
            std::fs::write(
                root.path().join("large.txt"),
                "file contents ".repeat(15000),
            )
            .unwrap();
            let mut streams = Vec::new();
            for index in 0..rounds {
                let id = format!("read-{index}");
                let args = json!({"path":"large.txt"}).to_string();
                streams.push(if responses {
                    sse(&[json!({"type":"response.completed","response":{"status":"completed","output":[
                        {"type":"function_call","call_id":id,"name":"read_file","arguments":args}
                    ]}})])
                } else {
                    sse(&[json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":"read_file","arguments":args}}]},"finish_reason":"tool_calls"}]})])
                });
            }
            let final_text = "Final answer ".repeat(1200);
            streams.push(final_without_usage(provider, &final_text));
            let (url, requests, server) = mock_server(streams);
            let mut settings = settings(provider, &url);
            // Isolate accounting from summarization; the actual history fits the window.
            settings.auto_compact = false;
            let user = Message::new(true, "original context ".repeat(9000), 0.0, String::new());
            let mut wire = history_items(&user, provider);
            let (answer, reports) = collect(Request {
                project_trusted: true,
                settings,
                workspace: root.path().into(),
                history: vec![user],
                session: "missing-usage".into(),
                codex_auth: (provider == Provider::Codex).then(|| crate::codex::Auth::fixture(url)),
            })
            .await;
            server.join().unwrap();
            let bodies: Vec<_> = requests.try_iter().collect();
            assert_eq!(bodies.len(), rounds + 1);
            let instructions = if responses {
                &bodies[0]["instructions"]
            } else {
                &bodies[0]["messages"][0]["content"]
            };
            let fixed = context::estimate(instructions)
                + context::estimate_items(bodies[0]["tools"].as_array().unwrap());
            wire.extend(answer.response_items.iter().cloned());
            let expected = context::estimate_items(&wire) + fixed;
            assert!(expected < 272_000, "fixture history fits the real window");
            assert!(reports.iter().all(|(_, estimated)| *estimated));
            assert_eq!(
                answer.context_tokens, expected,
                "{provider:?}, {rounds} tool rounds: the saved suffix is already in history, and final text is already in its assistant item"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_usage_keeps_the_last_provider_count_and_estimates_only_new_content() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        for reported in [false, true] {
            let final_text = "A short continuation.";
            let stream = if reported {
                text_completion(provider, final_text, 180_050)
            } else {
                final_without_usage(provider, final_text)
            };
            let (url, requests, server) = mock_server(vec![stream]);
            let root = tempfile::tempdir().unwrap();
            let mut settings = settings(provider, &url);
            settings.auto_compact = false;
            let mut previous = Message::new(
                false,
                "Previous answer".into(),
                0.0,
                provider.label().into(),
            );
            previous.context_tokens = 180_000;
            previous.context_connection = settings.context_key();
            let next = Message::new(true, "Continue the task.".into(), 0.0, String::new());
            let growth = context::estimate_items(&history_items(&next, provider));
            let (answer, reports) = collect(Request {
                project_trusted: true,
                settings,
                workspace: root.path().into(),
                history: vec![
                    Message::new(
                        true,
                        "long previous context ".repeat(40000),
                        0.0,
                        String::new(),
                    ),
                    previous,
                    next,
                ],
                session: "provider-count-anchor".into(),
                codex_auth: (provider == Provider::Codex).then(|| crate::codex::Auth::fixture(url)),
            })
            .await;
            server.join().unwrap();
            assert_eq!(requests.try_iter().count(), 1);
            assert_eq!(reports[0].0, 180_000 + growth);
            if reported {
                assert_eq!(answer.context_tokens, 180_060);
                assert!(!answer.context_estimated);
            } else {
                assert_eq!(
                    answer.context_tokens,
                    180_000 + growth + context::estimate_items(&answer.response_items),
                    "a missing usage event must not replace a real 180K count with a fresh estimate of the entire old transcript"
                );
                assert!(answer.context_estimated);
            }
            assert!(
                reports[0].1,
                "new input is estimated until the provider reports usage"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_away_and_back_reuses_the_codex_checkpoint_not_full_visible_history() {
    let provider = Provider::Codex;
    let (url, requests, server) = mock_server(vec![text_completion(provider, "Resumed", 180_000)]);
    let root = tempfile::tempdir().unwrap();
    let mut settings = settings(provider, &url);
    let mut previous = Message::new(
        false,
        "Visible original answer".into(),
        0.0,
        provider.label().into(),
    );
    previous.context_tokens = 180_000;
    previous.context_connection = settings.context_key();
    previous.context_checkpoint = Some(context::checkpoint(
        &settings,
        vec![json!({"role":"user","content":"SUMMARY: preserve prior decisions"})],
    ));
    previous.response_items = vec![json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"After summary"}]})].into();
    settings
        .context_windows
        .insert(settings.context_key(), 272_000);
    settings.provider = Provider::Llama;
    settings
        .context_windows
        .insert(settings.context_key(), 32_768);
    settings.provider = Provider::Codex;
    assert_eq!(settings.context_limit(), 272_000);
    let history: Vec<Message> = serde_json::from_str(
        &serde_json::to_string(&vec![
            Message::new(true, "OLD_TRANSCRIPT ".repeat(80000), 0.0, String::new()),
            previous,
            Message::new(true, "Resume Codex".into(), 0.0, String::new()),
        ])
        .unwrap(),
    )
    .unwrap();
    let (answer, reports) = collect(Request {
        project_trusted: true,
        settings,
        workspace: root.path().into(),
        history,
        session: "switch-back".into(),
        codex_auth: Some(crate::codex::Auth::fixture(url)),
    })
    .await;
    server.join().unwrap();
    let bodies: Vec<_> = requests.try_iter().collect();
    assert_eq!(
        bodies.len(),
        1,
        "switching alone must not recompact a valid checkpoint"
    );
    let wire = bodies[0]["input"].to_string();
    assert!(wire.contains("SUMMARY:"));
    assert!(wire.contains("After summary"));
    assert!(wire.contains("Resume Codex"));
    assert!(!wire.contains("OLD_TRANSCRIPT"));
    assert!(!wire.contains("Visible original answer"));
    assert!(reports[0].0 < 181_000);
    assert_eq!(answer.context_tokens, 180_010);
}

#[test]
fn usage_anchor_ignores_old_estimates_but_stops_at_compaction_or_connection_changes() {
    let mut settings = settings(Provider::Codex, "http://127.0.0.1:1234/v1");
    let connection = settings.context_key();
    let mut original = Message::new(false, "Reported answer".into(), 0.0, "Codex".into());
    original.context_connection = connection.clone();
    original.context_tokens = 180_000;
    let mut estimated = Message::new(false, "Later unmeasured answer".into(), 0.0, "Codex".into());
    estimated.context_connection = connection.clone();
    estimated.context_tokens = 320_000; // A stale count from the old double-counting bug.
    estimated.context_estimated = true;
    let user = Message::new(true, "Continue".into(), 0.0, String::new());
    let expected_growth = context::estimate_items(&history_items(&estimated, Provider::Codex))
        + context::estimate_items(&history_items(&user, Provider::Codex));
    let mut messages = vec![original, estimated, user];
    let restored: Vec<Message> =
        serde_json::from_str(&serde_json::to_string(&messages).unwrap()).unwrap();
    assert_eq!(
        previous_context_usage(&restored, &settings),
        Some(context::Usage::estimated(180_000 + expected_growth)),
        "estimated turns must not destroy the last real measurement or preserve inflated old counts"
    );

    settings.codex_model = "different-model".into();
    assert_eq!(previous_context_usage(&messages, &settings), None);
    settings.codex_model = Settings::default().codex_model;
    messages[1].context_checkpoint = Some(context::checkpoint(&settings, vec![]));
    assert_eq!(
        previous_context_usage(&messages, &settings),
        None,
        "a summary invalidates a pre-summary measurement"
    );
    messages[1].context_checkpoint = None;
    messages[1].context_connection = "Llama|other-endpoint|model".into();
    assert_eq!(previous_context_usage(&messages, &settings), None);
    messages[1].context_connection.clear();
    messages[1].provider = "llama.cpp".into();
    assert_eq!(
        previous_context_usage(&messages, &settings),
        None,
        "legacy foreign replies also invalidate the anchor"
    );
    messages[1].provider = "Codex".into();
    messages[1].context_connection = connection;
    messages[1].context_estimated = false;
    messages[1].context_tokens = 0;
    let latest_growth = context::estimate_items(&history_items(&messages[2], Provider::Codex));
    assert_eq!(
        previous_context_usage(&messages, &settings),
        Some(context::Usage::estimated(latest_growth)),
        "an explicit zero report must not resurrect the earlier 180K report"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_measured_tool_round_anchors_later_rounds_that_omit_usage() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        let responses = matches!(provider, Provider::Codex | Provider::OpenAI);
        let args = json!({"path":"large.txt"}).to_string();
        let tool = if responses {
            sse(&[
                json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":180000,"output_tokens":10},"output":[
                    {"type":"function_call","call_id":"read-1","name":"read_file","arguments":args}
                ]}}),
            ])
        } else {
            sse(&[
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"read-1","function":{"name":"read_file","arguments":args}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":180000,"completion_tokens":10}}),
            ])
        };
        let (url, requests, server) = mock_server(vec![
            tool,
            final_without_usage(provider, "Finished reading."),
        ]);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("large.txt"),
            "file contents ".repeat(15000),
        )
        .unwrap();
        let mut settings = settings(provider, &url);
        settings.auto_compact = false;
        let (answer, reports) = collect(Request {
            project_trusted: true,
            settings,
            workspace: root.path().into(),
            history: vec![Message::new(
                true,
                "Read the file".into(),
                0.0,
                String::new(),
            )],
            session: "measured-tool-round".into(),
            codex_auth: (provider == Provider::Codex).then(|| crate::codex::Auth::fixture(url)),
        })
        .await;
        server.join().unwrap();
        assert_eq!(requests.try_iter().count(), 2);
        let result = answer
            .response_items
            .iter()
            .find(|item| item["type"] == "function_call_output" || item["role"] == "tool")
            .unwrap();
        let tool_growth = context::estimate_items(std::slice::from_ref(result));
        let final_growth =
            context::estimate_items(std::slice::from_ref(answer.response_items.last().unwrap()));
        assert_eq!(reports[1], (180_010 + tool_growth, true));
        assert_eq!(answer.context_tokens, 180_010 + tool_growth + final_growth);
        assert!(answer.context_estimated);
    }
}

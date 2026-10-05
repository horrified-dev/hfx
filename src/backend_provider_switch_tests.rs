//! Real alternating connections and saved checkpoints, using local HTTP only.
use super::tests::{mock_server, text_completion};
use super::*;

#[test]
fn opaque_state_requires_the_exact_connection_not_just_the_provider_label() {
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::OpenRouter,
        Provider::Llama,
    ] {
        let mut settings = Settings {
            provider,
            ..Default::default()
        };
        let mut answer = Message::new(false, "Visible answer".into(), 0.0, provider.label().into());
        answer.context_connection = settings.context_key();
        answer.response_items = vec![
            json!({"type":"reasoning","encrypted_content":"PRIVATE_STATE","id":"private-id"}),
            json!({"role":"assistant","content":"Visible answer","reasoning_content":"PRIVATE_STATE","reasoning_details":[{"data":"PRIVATE_STATE"}]}),
            json!({"type":"function_call_output","call_id":"private-call-id","output":"READABLE_RESULT"}),
        ].into();
        answer.reasoning_details = vec![json!({"data":"PRIVATE_STATE"})].into();
        assert!(
            json!(history_items_for_settings(&answer, &settings))
                .to_string()
                .contains("PRIVATE_STATE")
        );
        for change in ["model", "endpoint", "legacy"] {
            let mut changed = settings.clone();
            if change == "model" {
                *changed.model_mut() = "another-model".into();
            } else if change == "endpoint" {
                // Codex has one fixed authenticated endpoint; use a new model instead.
                if provider == Provider::Codex {
                    changed.codex_model = "another-model".into();
                } else {
                    *changed.base_url_mut() = "http://localhost:9999/v1".into();
                }
            } else {
                answer.context_connection.clear();
            }
            let wire = json!(history_items_for_settings(&answer, &changed)).to_string();
            assert!(wire.contains("Visible answer"));
            assert!(wire.contains("READABLE_RESULT"));
            assert!(!wire.contains("PRIVATE_STATE"), "{provider:?}, {change}");
            assert!(!wire.contains("private-call-id"));
            assert!(!wire.contains("private-id"));
        }
        settings.provider = Provider::Llama;
        assert!(
            !json!(history_items_for_settings(&answer, &settings))
                .to_string()
                .contains("PRIVATE_STATE")
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn alternating_models_use_the_newest_summary_without_resurrecting_500k_history() {
    let root = tempfile::tempdir().unwrap();
    let (url, requests, server) = mock_server(vec![
        text_completion(Provider::Llama, "Local continuation", 4000),
        text_completion(Provider::Codex, "Codex continuation", 5000),
        text_completion(Provider::Llama, "Local again", 6000),
        text_completion(Provider::Codex, "Codex again", 7000),
    ]);
    let mut settings = Settings {
        provider: Provider::Codex,
        llama_url: url.clone(),
        context_window: 272_000,
        ..Default::default()
    };
    let auth = crate::codex::Auth::fixture(url);
    let mut old = Message::new(false, "VISIBLE_OLD_REPLY".into(), 0.0, "Codex".into());
    old.context_connection = settings.context_key();
    old.context_checkpoint = Some(context::checkpoint(
        &settings,
        vec![json!({"role":"user","content":"OLDER_CODEX_SUMMARY"})],
    ));
    old.context_tokens = 180_000;
    old.response_items =
        vec![json!({"type":"reasoning","encrypted_content":"PRIVATE_CODEX_STATE"})].into();
    let mut history = vec![
        Message::new(true, "OLD_TRANSCRIPT ".repeat(150000), 0.0, String::new()),
        old,
    ];
    // A later local-model compaction supersedes the older Codex checkpoint.
    settings.provider = Provider::Llama;
    let mut local = Message::new(
        false,
        "VISIBLE_PRE_SUMMARY_LOCAL_TEXT".into(),
        0.0,
        "llama.cpp".into(),
    );
    local.context_connection = settings.context_key();
    local.context_estimated = true;
    local.context_tokens = 500_000; // Legacy inflated usage must not be carried forward.
    local.context_checkpoint = Some(context::checkpoint(
        &settings,
        vec![
            json!({"role":"user","content":"NEWEST_SUMMARY: all earlier decisions retained"}),
            json!({"role":"assistant","tool_calls":[{"id":"local-tool","function":{"name":"read_file","arguments":"{}"}}],"reasoning_content":"PRIVATE_LOCAL_STATE"}),
            json!({"role":"tool","tool_call_id":"local-tool","content":"RECENT_TOOL_RESULT"}),
        ],
    ));
    local.response_items = vec![json!({"role":"assistant","content":"LOCAL_SUFFIX_ONCE","reasoning_content":"PRIVATE_LOCAL_STATE"})].into();
    history.push(Message::new(
        true,
        "INTERVENING_OLD_HISTORY ".repeat(80000),
        0.0,
        String::new(),
    ));
    history.push(local);
    for provider in [
        Provider::Llama,
        Provider::Codex,
        Provider::Llama,
        Provider::Codex,
    ] {
        settings.provider = provider;
        settings.context_windows.insert(
            settings.context_key(),
            if provider == Provider::Llama {
                32768
            } else {
                272000
            },
        );
        history.push(Message::new(
            true,
            "Continue the same task".into(),
            0.0,
            String::new(),
        ));
        let (tx, rx) = std::sync::mpsc::channel();
        run(
            Request {
                settings: settings.clone(),
                workspace: root.path().into(),
                project_trusted: true,
                history: history.clone(),
                session: "alternating-chat".into(),
                codex_auth: (provider == Provider::Codex).then(|| auth.clone()),
            },
            tx,
        )
        .await;
        let mut answer = Message::new(false, String::new(), 0.0, provider.label().into());
        for event in rx.try_iter() {
            match event {
                Event::Text(text) => answer.text.push_str(&text),
                Event::ResponsesContext(items) => answer.response_items = items.into(),
                Event::ContextUsage {
                    tokens,
                    estimated,
                    connection,
                } => {
                    assert!(
                        tokens < settings.context_limit(),
                        "{provider:?}: stale inflated usage {tokens}"
                    );
                    answer.context_tokens = tokens;
                    answer.context_estimated = estimated;
                    answer.context_connection = connection;
                }
                Event::CompactionStarted | Event::Compacted(_) => {
                    panic!("Already-small latest summary must not be compacted again")
                }
                Event::Error(error) => panic!("{provider:?}: {error}"),
                _ => {}
            }
        }
        assert!(!answer.text.is_empty());
        history.push(answer);
        // Exercise persistence between every connection switch.
        history = serde_json::from_str(&serde_json::to_string(&history).unwrap()).unwrap();
    }
    server.join().unwrap();
    let bodies: Vec<_> = requests.try_iter().collect();
    assert_eq!(bodies.len(), 4);
    for (index, body) in bodies.iter().enumerate() {
        let wire = &body[if index % 2 == 0 { "messages" } else { "input" }];
        let text = wire.to_string();
        assert_eq!(text.matches("NEWEST_SUMMARY").count(), 1);
        assert_eq!(text.matches("LOCAL_SUFFIX_ONCE").count(), 1);
        assert_eq!(text.matches("RECENT_TOOL_RESULT").count(), 1);
        assert!(!text.contains("OLD_TRANSCRIPT"));
        assert!(!text.contains("INTERVENING_OLD_HISTORY"));
        assert!(!text.contains("OLDER_CODEX_SUMMARY"));
        assert!(!text.contains("VISIBLE_PRE_SUMMARY_LOCAL_TEXT"));
        if index % 2 == 1 {
            assert!(!text.contains("PRIVATE_LOCAL_STATE"));
            assert!(!text.contains("tool_calls"));
            assert!(!text.contains("tool_call_id"));
        }
        assert!(!text.contains("PRIVATE_CODEX_STATE"));
    }
}

#[test]
fn portable_checkpoint_images_keep_pixels_without_base64_token_inflation() {
    let pixels = "x".repeat(200000);
    let items = vec![
        json!({"role":"user","content":[{"type":"input_text","text":"Original request"},{"type":"input_image","image_url":format!("data:image/png;base64,{pixels}")}]}),
        json!({"type":"function_call_output","call_id":"a","output":[{"type":"input_text","text":"Tool reference"},{"type":"input_image","image_url":"data:image/png;base64,tool-image"}]}),
    ];
    for provider in [
        Provider::Codex,
        Provider::OpenAI,
        Provider::Llama,
        Provider::OpenRouter,
    ] {
        let wire = context::portable_items(&items, provider);
        let text = json!(wire).to_string();
        assert!(text.contains(&pixels));
        assert!(text.contains("tool-image"));
        assert!(text.contains("Original request"));
        assert!(text.contains("Tool reference"));
        assert!(context::estimate_items(&wire) < 9000);
        assert!(!text.contains("call_id"));
    }
}

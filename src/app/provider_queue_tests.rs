//! Provider selection and queued/steered turns stay with their original chats.
use super::*;

fn fixture() -> (Harness, egui::Context) {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    (Harness::new(&cc, Some("chat".into())), ctx)
}

#[test]
fn chat_switches_restore_provider_model_endpoint_and_limits_without_copying_history() {
    let (mut app, ctx) = fixture();
    app.saved.settings.provider = Provider::Codex;
    app.saved.settings.codex_model = "codex-chat-model".into();
    let first = app.saved.selected;
    let first_messages = app.saved.chats[0].messages.len();
    app.new_chat(app.project().id, 0.0);
    let second = app.saved.selected;
    app.saved.settings.provider = Provider::Llama;
    app.saved.settings.llama_model = "local-chat-model".into();
    app.saved.settings.llama_url = "http://127.0.0.1:9999/v1".into();
    let local_key = app.saved.settings.context_key();
    app.saved
        .settings
        .context_windows
        .insert(local_key.clone(), 32768);
    for _ in 0..5 {
        app.select(first, 0.2);
        assert_eq!(app.saved.settings.provider, Provider::Codex);
        assert_eq!(app.saved.settings.codex_model, "codex-chat-model");
        assert_eq!(app.saved.chats[0].messages.len(), first_messages);
        app.select(second, 0.4);
        assert_eq!(app.saved.settings.provider, Provider::Llama);
        assert_eq!(app.saved.settings.context_key(), local_key);
        assert_eq!(app.saved.settings.context_limit(), 32768);
        assert!(app.saved.chats[1].messages.is_empty());
    }
    app.remember_connection();
    let mut restored: Saved =
        serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
    restored.restore();
    assert_eq!(restored.settings.context_key(), local_key);
    app.saved = restored;
    app.select(first, 0.6);
    assert_eq!(app.saved.settings.provider, Provider::Codex);
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.8,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.saved.settings.codex_model, "codex-chat-model");
}

#[test]
fn a_changed_provider_cannot_steer_the_old_run_and_queued_work_keeps_its_chosen_provider() {
    let (mut app, ctx) = fixture();
    app.saved.settings.provider = Provider::Codex;
    let original_chat = app.saved.selected;
    let (events, mut steering) = fake_active(&mut app);
    let active_key = app.saved.settings.context_key();
    app.saved.settings.provider = Provider::Demo;
    app.saved.chats[0].draft = "Run this using the newly selected provider".into();
    app.submit(&ctx, true);
    assert!(steering.try_recv().is_err());
    assert!(!app.saved.chats[0].queue[0].dispatched);
    assert_eq!(
        app.saved.chats[0].queue[0]
            .connection
            .as_ref()
            .unwrap()
            .provider,
        Provider::Demo
    );
    assert_eq!(
        app.saved.chats[0]
            .messages
            .last()
            .unwrap()
            .context_connection,
        active_key
    );
    let durable: Saved = serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
    assert_eq!(
        durable.chats[0].queue[0]
            .connection
            .as_ref()
            .unwrap()
            .provider,
        Provider::Demo
    );
    app.new_chat(app.project().id, 0.2);
    app.saved.settings.provider = Provider::Codex;
    app.saved.settings.codex_model = "unrelated-chat-model".into();
    let selected = app.saved.selected;
    events.send(Event::Completed).unwrap();
    app.poll(&ctx);
    assert_eq!(app.saved.selected, selected);
    assert_eq!(app.active.as_ref().unwrap().chat, original_chat);
    assert_eq!(app.saved.chats[0].messages.last().unwrap().provider, "Demo");
    assert!(app.saved.chats[0].queue.is_empty());
    assert!(app.saved.chats[1].messages.is_empty());
    assert_eq!(app.saved.settings.provider, Provider::Codex);
    app.stop();
}

#[test]
fn model_and_endpoint_changes_cannot_steer_an_inflight_request_either() {
    for change in ["model", "endpoint"] {
        let (mut app, ctx) = fixture();
        app.saved.settings.provider = Provider::Llama;
        let (_events, mut steering) = fake_active(&mut app);
        if change == "model" {
            app.saved.settings.llama_model = "different-model".into();
        } else {
            app.saved.settings.llama_url = "http://localhost:9999/v1".into();
        }
        app.saved.chats[0].draft = "A new connection must wait".into();
        app.submit(&ctx, true);
        assert!(steering.try_recv().is_err(), "{change}");
        assert!(!app.saved.chats[0].queue[0].dispatched);
        app.stop();
    }
}

#[test]
fn legacy_chat_connections_migrate_from_their_own_reports_not_global_provider() {
    let mut saved = Saved::default();
    saved.settings.provider = Provider::Codex;
    saved.chats[0].messages.push(Message::new(
        false,
        "Codex answer".into(),
        0.0,
        "Codex".into(),
    ));
    let mut local_settings = saved.settings.clone();
    local_settings.provider = Provider::Llama;
    local_settings.llama_model = "legacy-local-model".into();
    local_settings.llama_url = "http://localhost:9999/v1".into();
    let mut local = Chat::new(saved.projects[0].id);
    let mut reply = Message::new(false, "Local answer".into(), 0.0, "llama.cpp".into());
    reply.context_connection = local_settings.context_key();
    local.messages.push(reply);
    local
        .queue
        .push(QueuedMessage::new("Legacy follow-up".into(), vec![]));
    saved.chats.push(local);
    saved.restore();
    assert_eq!(
        saved.chats[0].connection.as_ref().unwrap().provider,
        Provider::Codex
    );
    assert_eq!(
        saved.chats[1].connection.as_ref().unwrap().context_key(),
        local_settings.context_key()
    );
    assert_eq!(
        saved.chats[1].queue[0].connection,
        saved.chats[1].connection
    );
    assert!(saved.chats[1].queue_paused);
    assert_eq!(saved.settings.provider, Provider::Codex);
}

//! Chat/provider switching must never move usage reports between conversations.
use super::*;

#[test]
fn codex_llama_new_chat_roundtrip_preserves_usage_and_connection_limits() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    app.saved.settings.provider = Provider::Codex;
    let codex_connection = app.saved.settings.context_key();
    app.saved
        .settings
        .context_windows
        .insert(codex_connection.clone(), 272_000);
    let original_chat = app.saved.selected;
    let mut reply = Message::new(false, "Codex answer".into(), 0.0, "Codex".into());
    reply.context_tokens = 180_000;
    reply.context_connection = codex_connection;
    app.saved.chats[0].messages.push(reply);
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        ContextMeter::for_chat(&app.saved.chats[0], &app.saved.settings).used,
        Some(180_000)
    );

    app.new_chat(app.project().id, 0.2);
    let llama_chat = app.saved.selected;
    assert_ne!(llama_chat, original_chat);
    app.saved.settings.provider = Provider::Llama;
    let llama_connection = app.saved.settings.context_key();
    app.saved
        .settings
        .context_windows
        .insert(llama_connection.clone(), 32_768);
    let index = app.selected_index();
    let meter = ContextMeter::for_chat(&app.saved.chats[index], &app.saved.settings);
    assert_eq!(meter.used, None);
    assert_eq!(meter.maximum, 32_768);
    assert_eq!(meter.fraction(), 0.0);
    let mut reply = Message::new(false, "Local answer".into(), 0.2, "llama.cpp".into());
    reply.context_tokens = 16_000;
    reply.context_connection = llama_connection;
    app.saved.chats[index].messages.push(reply);
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.4,
        vec![],
        egui::Modifiers::NONE,
    );

    app.select(original_chat, 0.6);
    assert_eq!(
        ContextMeter::for_chat(&app.saved.chats[0], &app.saved.settings).used,
        None
    );
    app.saved.settings.provider = Provider::Codex;
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.8,
        vec![],
        egui::Modifiers::NONE,
    );
    let meter = ContextMeter::for_chat(&app.saved.chats[0], &app.saved.settings);
    assert_eq!(meter.used, Some(180_000));
    assert_eq!(meter.maximum, 272_000);
    assert!(!meter.estimated);
    assert_eq!(meter.usage_text(), "180,000 / 272,000 tokens");

    let restored: Saved =
        serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
    assert_eq!(
        ContextMeter::for_chat(&restored.chats[0], &restored.settings).used,
        Some(180_000)
    );
    app.select(llama_chat, 1.0);
    assert_eq!(
        ContextMeter::for_chat(&app.saved.chats[index], &app.saved.settings).used,
        None
    );
    app.saved.settings.provider = Provider::Llama;
    let meter = ContextMeter::for_chat(&app.saved.chats[index], &app.saved.settings);
    assert_eq!(meter.used, Some(16_000));
    assert_eq!(meter.maximum, 32_768);
}

#[test]
fn codex_usage_arriving_after_a_provider_and_chat_switch_updates_only_its_original_reply() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("chat".into()));
    app.saved.settings.provider = Provider::Codex;
    let connection = app.saved.settings.context_key();
    let original_chat = app.saved.selected;
    let message = app.saved.chats[0].messages[1].id;
    app.saved.chats[0].messages[1].provider = "Codex".into();
    app.saved.chats[0].messages[1].status = Status::Streaming;
    let (tx, rx) = mpsc::channel();
    app.active = Some(Active {
        chat: original_chat,
        message,
        workspace: PathBuf::from("."),
        rx,
        task: app.runtime.spawn(std::future::pending::<()>()),
        started: Instant::now(),
        steering: None,
    });
    app.new_chat(app.project().id, 0.2);
    app.saved.settings.provider = Provider::Llama;
    tx.send(Event::ContextUsage {
        tokens: 180_000,
        estimated: false,
        connection,
    })
    .unwrap();
    tx.send(Event::Completed).unwrap();
    app.poll(&ctx);
    assert!(app.active.is_none());
    assert!(app.saved.chats[app.selected_index()].messages.is_empty());
    assert_eq!(
        ContextMeter::for_chat(&app.saved.chats[app.selected_index()], &app.saved.settings).used,
        None
    );
    assert_eq!(app.saved.chats[0].messages[1].context_tokens, 180_000);
    app.select(original_chat, 0.4);
    app.saved.settings.provider = Provider::Codex;
    assert_eq!(
        ContextMeter::for_chat(&app.saved.chats[0], &app.saved.settings).used,
        Some(180_000)
    );
}

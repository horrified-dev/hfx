//! Synthetic performance checks; no saved chats or provider connections are used.
use super::*;
use serde_json::json;

#[test]
fn reply_views_borrow_message_storage_without_advancing_unicode_cursors() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    let mut message = Message::new(false, "Hello 🌿 世界".into(), 0.0, "llama.cpp".into());
    message.reasoning = "Reasoning café 🌿".into();
    let (answer, reasoning) = app.reply_text(&message);
    assert_eq!(answer, message.text);
    assert_eq!(reasoning, message.reasoning);
    assert_eq!(answer.as_ptr(), message.text.as_ptr());
    assert_eq!(reasoning.as_ptr(), message.reasoning.as_ptr());

    let mut answer_cursor = Reveal::default();
    let mut reasoning_cursor = Reveal::default();
    answer_cursor.step(&message.text, 0.02, 50.0, false);
    reasoning_cursor.step(&message.reasoning, 0.02, 50.0, false);
    app.reveals
        .insert(message.id, (answer_cursor, reasoning_cursor));
    let (answer, reasoning) = app.reply_text(&message);
    assert_eq!(answer, "H");
    assert_eq!(reasoning, "R");
    assert_eq!(answer.as_ptr(), message.text.as_ptr());
    assert_eq!(reasoning.as_ptr(), message.reasoning.as_ptr());
    assert_eq!(app.reveals[&message.id].0.visible(&message.text), answer);
    assert_eq!(
        app.reveals[&message.id].1.visible(&message.reasoning),
        reasoning
    );
}

fn active_preview(ctx: &egui::Context) -> (Harness, Active) {
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("chat".into()));
    let (_, rx) = mpsc::channel();
    let reply = &mut app.saved.chats[0].messages[1];
    reply.status = Status::Streaming;
    let active = Active {
        chat: app.saved.selected,
        message: reply.id,
        workspace: PathBuf::from("."),
        rx,
        task: app.runtime.spawn(std::future::pending::<()>()),
        started: Instant::now(),
        steering: None,
    };
    (app, active)
}

#[test]
fn wire_context_appends_target_the_original_reply_and_preserve_save_snapshots_on_error() {
    let ctx = egui::Context::default();
    let (mut app, mut active) = active_preview(&ctx);
    let original = json!({"role":"tool","content":"original tool output"});
    app.apply_event(&mut active, Event::ResponsesContext(vec![original.clone()]));
    let snapshot = app.saved.clone();
    app.new_chat(app.project().id, 0.2);
    app.saved.settings.provider = Provider::Llama;
    let next = json!({"role":"tool","content":"next tool output"});
    app.apply_event(
        &mut active,
        Event::ResponsesContextAppend(vec![next.clone()]),
    );
    assert_eq!(
        *snapshot.chats[0].messages[1].response_items,
        vec![original.clone()]
    );
    assert_eq!(
        *app.saved.chats[0].messages[1].response_items,
        vec![original.clone(), next.clone()]
    );
    assert!(app.saved.chats[app.selected_index()].messages.is_empty());
    assert!(!std::sync::Arc::ptr_eq(
        &snapshot.chats[0].messages[1].response_items,
        &app.saved.chats[0].messages[1].response_items
    ));

    let prefix_pointer = app.saved.chats[0].messages[1].response_items[0]["content"]
        .as_str()
        .unwrap()
        .as_ptr();
    let final_tool = json!({"role":"tool","content":"final tool output"});
    app.apply_event(
        &mut active,
        Event::ResponsesContextAppend(vec![final_tool.clone()]),
    );
    assert_eq!(
        prefix_pointer,
        app.saved.chats[0].messages[1].response_items[0]["content"]
            .as_str()
            .unwrap()
            .as_ptr(),
        "an unshared context must not clone the earlier prefix on append"
    );
    assert_eq!(
        app.apply_event(&mut active, Event::Error("fixture failure".into())),
        Some(false)
    );
    let restored: Saved =
        serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
    assert_eq!(
        *restored.chats[0].messages[1].response_items,
        vec![original, next, final_tool]
    );
    assert_eq!(restored.chats[0].messages[1].status, Status::Failed);
}

#[test]
fn compaction_and_final_snapshots_replace_instead_of_duplicate_appended_context() {
    let ctx = egui::Context::default();
    let (mut app, mut active) = active_preview(&ctx);
    app.apply_event(
        &mut active,
        Event::ResponsesContextAppend(vec![json!({"role":"tool","content":"old context"})]),
    );
    let checkpoint = crate::context::checkpoint(
        &app.saved.settings,
        vec![json!({"role":"user","content":"summary"})],
    );
    app.apply_event(&mut active, Event::Compacted(checkpoint));
    assert!(app.saved.chats[0].messages[1].response_items.is_empty());
    let tool = json!({"role":"tool","content":"after summary"});
    app.apply_event(
        &mut active,
        Event::ResponsesContextAppend(vec![tool.clone()]),
    );
    let answer = json!({"role":"assistant","content":"done"});
    app.apply_event(
        &mut active,
        Event::ResponsesContext(vec![tool.clone(), answer.clone()]),
    );
    assert_eq!(app.apply_event(&mut active, Event::Completed), Some(true));
    assert_eq!(
        *app.saved.chats[0].messages[1].response_items,
        vec![tool, answer]
    );
}

#[test]
fn stop_drains_pending_tool_context_deltas_into_the_original_reply() {
    let ctx = egui::Context::default();
    let (mut app, mut active) = active_preview(&ctx);
    let (tx, rx) = mpsc::channel();
    active.rx = rx;
    let first = json!({"role":"tool","content":"first completed tool"});
    let second = json!({"role":"tool","content":"second completed tool"});
    tx.send(Event::ResponsesContextAppend(vec![first.clone()]))
        .unwrap();
    tx.send(Event::ResponsesContextAppend(vec![second.clone()]))
        .unwrap();
    app.active = Some(active);
    app.new_chat(app.project().id, 0.2);
    app.stop();
    assert!(app.active.is_none());
    assert!(app.saved.chats[app.selected_index()].messages.is_empty());
    assert_eq!(
        *app.saved.chats[0].messages[1].response_items,
        vec![first, second]
    );
    assert_eq!(app.saved.chats[0].messages[1].status, Status::Cancelled);
}

#[test]
#[ignore = "manual long-transcript layout measurement"]
fn profile_long_transcript_frames() {
    use std::hint::black_box;
    for messages in [16, 64, 256] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        app.saved.settings.reduced_motion = true;
        let body = "A completed reply with **formatted prose**, Unicode café 🌿, and a [documentation link](https://example.com/docs).\n".repeat(8);
        for index in 0..messages {
            let message = Message::new(
                false,
                format!("Reply {index}\n{body}"),
                0.0,
                "llama.cpp".into(),
            );
            app.reveals.insert(
                message.id,
                (Reveal::complete(&message.text), Reveal::default()),
            );
            app.saved.chats[0].messages.push(message);
        }
        let mut render = |frame| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(1180.0, 820.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| app.conversation(ui, frame as f64 / 60.0),
            );
            output.textures_delta.clear();
            black_box(output);
        };
        for frame in 0..10 {
            render(frame);
        }
        let frames = 30;
        let start = Instant::now();
        for frame in 10..frames + 10 {
            render(frame);
        }
        println!(
            "{messages} completed prose replies: {:.3} ms/frame",
            start.elapsed().as_secs_f64() * 1000.0 / frames as f64
        );
    }
}

#[test]
#[ignore = "manual completed-reply rendering measurement"]
fn profile_hidden_reasoning_frames() {
    use std::hint::black_box;
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    app.saved.settings.show_reasoning = false;
    app.saved.settings.reduced_motion = true;
    let mut message = Message::new(false, "A completed reply.".into(), 0.0, "llama.cpp".into());
    message.reasoning = "reasoning details 🌿\n".repeat(400_000);
    app.reveals.insert(
        message.id,
        (
            Reveal::complete(&message.text),
            Reveal::complete(&message.reasoning),
        ),
    );
    let frames = 300;
    let mut render = |frame| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(1180.0, 820.0),
                )),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            },
            |ui| app.message(ui, black_box(&message), frame as f64 / 60.0),
        );
        output.textures_delta.clear();
        black_box(output);
    };
    for frame in 0..10 {
        render(frame);
    }
    let start = Instant::now();
    for frame in 10..frames + 10 {
        render(frame);
    }
    let elapsed = start.elapsed();
    println!(
        "completed reply with {} MiB hidden reasoning: {:.2} ms / {frames} frames, {:.3} ms/frame",
        message.reasoning.len() as f64 / (1024.0 * 1024.0),
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1000.0 / frames as f64
    );
}

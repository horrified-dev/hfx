use super::*;
use crate::state::Activity;
use eframe::App;

#[path = "safety_tests.rs"]
mod safety_regressions;

#[path = "edit_badge_tests.rs"]
mod edit_badge;

#[path = "edit_badge_live_tests.rs"]
mod edit_badge_live;

fn draw(
    app: &mut Harness,
    ctx: &egui::Context,
    width: f32,
    height: f32,
    time: f64,
    mut events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> egui::FullOutput {
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            pos2(0.0, 0.0),
            vec2(width, height),
        )),
        time: Some(time),
        events,
        ..Default::default()
    };
    app.raw_input_hook(ctx, &mut input);
    let mut output = ctx.run_ui(input, |ui| {
        let mut frame = eframe::Frame::_new_kittest();
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    });
    // Headless layout tests intentionally have no GPU texture consumer.
    output.textures_delta.clear();
    output
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn text_position(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Pos2 {
    shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(shape) if shape.galley.job.text == text => {
                Some(shape.pos + vec2(4.0, shape.galley.size().y / 2.0))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing UI text: {text}"))
}

fn context_ring_position(shapes: &[egui::epaint::ClippedShape]) -> egui::Pos2 {
    shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle)
                if circle.radius == 9.0 && circle.stroke.color == theme::OUTLINE =>
            {
                Some(circle.center)
            }
            _ => None,
        })
        .expect("context ring is visible")
}

fn pointer_button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn timeline_fixture() -> Message {
    let mut reply = Message::new(false, String::new(), 0.0, "Demo preview".into());
    let tool = |reply: &mut Message, name: &str, target: &str| {
        let index = reply.activities.len();
        reply.push_activity(Activity {
            id: format!("timeline-{index}"),
            name: name.into(),
            arguments: if name == "run_command" {
                serde_json::json!({"command":target})
            } else {
                serde_json::json!({"path":target})
            }
            .to_string()
            .into(),
            result: format!("Completed {target}").into(),
            status: ActionStatus::Complete,
            elapsed: 0.2,
            change: None,
            images: Vec::new(),
        });
    };
    reply.push_text(
        "I’ll inspect the existing file and command history before changing the layout.",
    );
    tool(&mut reply, "read_file", "src/app.rs");
    tool(&mut reply, "run_command", "rg action_history src");
    reply.push_text("\n\nThe history is now split into compact groups. Progress updates stay in the conversation where they happened.");
    tool(&mut reply, "write_file", "src/state.rs");
    tool(&mut reply, "read_file", "src/state.rs");
    tool(&mut reply, "run_command", "cargo test --locked timeline");
    reply.record_compaction();
    reply.context_checkpoint = Some(crate::state::ContextCheckpoint {
        connection: "demo".into(),
        history: Default::default(),
    });
    tool(&mut reply, "read_file", "README.md");
    tool(&mut reply, "run_command", "cargo fmt --all --check");
    reply.push_text("\n\nI’m checking saved history and streaming updates next. Each new batch of tools has its own disclosure.");
    tool(&mut reply, "write_file", "src/app.rs");
    tool(&mut reply, "read_file", "src/app.rs");
    tool(
        &mut reply,
        "run_command",
        "cargo test --locked app::tests::",
    );
    reply.push_text(
        "\n\nDone — progress updates and tool groups now alternate in chronological order.",
    );
    reply.elapsed = 52.0;
    reply
}

#[test]
fn reply_timeline_renders_in_order_and_tool_groups_expand_independently() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.settings.show_reasoning = false;
    app.saved.settings.reduced_motion = true;
    app.saved.chats[0].messages = vec![timeline_fixture()];
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    // Test message geometry without the composer's viewport clipping: this
    // reply fits, but tests may inspect text in a multi-line wrapped galley.
    let positions = |output: &egui::FullOutput, prefix: &str| -> Vec<egui::Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with(prefix) => {
                    Some(text.pos + vec2(4.0, text.galley.size().y / 2.0))
                }
                _ => None,
            })
            .collect()
    };
    let reads = positions(&output, "Read files, Ran commands");
    let edits = positions(&output, "File edits, Read files, Ran commands");
    assert_eq!(reads.len(), 2);
    assert_eq!(edits.len(), 2);
    let first = positions(&output, "I’ll inspect")[0];
    let second = positions(&output, "The history is now")[0];
    let compacted = positions(&output, "Context automatically compacted")[0];
    let third = positions(&output, "I’m checking saved")[0];
    let done = positions(&output, "Done —")[0];
    assert!(first.y < reads[0].y && reads[0].y < second.y && second.y < edits[0].y);
    assert!(edits[0].y < compacted.y && compacted.y < reads[1].y && reads[1].y < third.y);
    assert!(third.y < edits[1].y && edits[1].y < done.y);
    assert!(app.action_targets.is_empty(), "all groups begin collapsed");
    click(&mut app, &ctx, reads[0], 0.2);
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.4,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        app.action_targets.len(),
        2,
        "only the first group's arguments are parsed"
    );
    assert!(positions(&output, "Read src/app.rs").len() == 1);
    assert!(
        positions(&output, "Read README.md").is_empty(),
        "the other read group stays collapsed"
    );
    // Result/status updates must not reset the first group's open state.
    app.saved.chats[0].messages[0].activities[0].status = ActionStatus::Failed;
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.6,
        vec![],
        egui::Modifiers::NONE,
    );
    assert_eq!(positions(&output, "Read src/app.rs").len(), 1);
    assert_eq!(positions(&output, "Read README.md").len(), 0);
}

#[test]
fn growing_a_live_tool_group_keeps_its_disclosure_open_when_the_label_changes() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.settings.show_reasoning = false;
    app.saved.settings.reduced_motion = true;
    let mut reply = timeline_fixture();
    reply.timeline.truncate(2);
    reply.text.truncate(match reply.timeline[0] {
        ReplyBlock::Text { end, .. } => end,
        _ => unreachable!(),
    });
    reply.activities.truncate(1);
    reply.timeline[1] = ReplyBlock::Tools { start: 0, end: 1 };
    reply.context_checkpoint = None;
    app.saved.chats[0].messages = vec![reply];
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    click(&mut app, &ctx, text_center(&output, "Read files"), 0.2);
    let reply = &mut app.saved.chats[0].messages[0];
    reply.push_activity(Activity {
        id: "new-call".into(),
        name: "run_command".into(),
        arguments: r#"{"command":"cargo test"}"#.into(),
        result: "ok".into(),
        status: ActionStatus::Running,
        elapsed: 0.0,
        change: None,
        images: Vec::new(),
    });
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.4,
        vec![],
        egui::Modifiers::NONE,
    );
    text_center(&output, "Read files, Ran commands");
    text_center(&output, "Ran cargo test · running");
    assert_eq!(app.action_targets.len(), 2);
}

#[test]
fn progress_events_split_tool_batches_in_the_original_chat_without_losing_results() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
    let (events, _) = fake_active(&mut app);
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    events
        .send(Event::Text("First progress 🌿.".into()))
        .unwrap();
    for (id, name) in [("one", "read_file"), ("two", "run_command")] {
        events
            .send(Event::ToolStarted(ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: "{}".into(),
            }))
            .unwrap();
        events
            .send(Event::ToolFinished {
                id: id.into(),
                output: tools::ToolOutput {
                    text: format!("Result {id}"),
                    status: ActionStatus::Complete,
                    change: None,
                    images: Vec::new(),
                },
                elapsed: 0.3,
                show_in_reply: false,
            })
            .unwrap();
    }
    events.send(Event::Text("\n\n".into())).unwrap();
    events.send(Event::Text("Second progress.".into())).unwrap();
    events
        .send(Event::ToolStarted(ToolCall {
            id: "three".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        }))
        .unwrap();
    events
        .send(Event::Compacted(crate::state::ContextCheckpoint {
            connection: "demo".into(),
            history: Default::default(),
        }))
        .unwrap();
    events
        .send(Event::Image(attachments::test_image()))
        .unwrap();
    events
        .send(Event::Text("\n\nFinal answer.".into()))
        .unwrap();
    app.poll(&ctx);
    let reply = &app.saved.chats[0].messages[1];
    assert!(reply.timeline_valid());
    assert_eq!(reply.timeline.len(), 7);
    assert!(matches!(
        reply.timeline[1],
        ReplyBlock::Tools { start: 0, end: 2 }
    ));
    assert!(matches!(
        reply.timeline[3],
        ReplyBlock::Tools { start: 2, end: 3 }
    ));
    assert_eq!(reply.activities[0].result.as_ref(), "Result one");
    assert_eq!(reply.activities[1].result.as_ref(), "Result two");
    assert!(app.saved.chats[1].messages.is_empty());
    app.stop();
    let reply = &app.saved.chats[0].messages[1];
    assert_eq!(reply.activities[2].status, ActionStatus::Cancelled);
    assert!(reply.timeline_valid());
    assert_eq!(reply.status, Status::Cancelled);
}

#[test]
fn invalid_reply_timeline_falls_back_to_full_text_and_tools_without_panicking() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.settings.reduced_motion = true;
    let reply = &mut app.saved.chats[0].messages[1];
    reply.text = "👋 The full reply remains readable.".into();
    reply.timeline = vec![ReplyBlock::Text { start: 1, end: 3 }];
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    text_center(&output, "👋 The full reply remains readable.");
    text_center(&output, "File edits, Read files, Ran commands");
}

#[test]
fn tool_activity_starts_collapsed_and_can_be_expanded_and_collapsed() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.settings.reduced_motion = true;
    let mut output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    assert!(
        app.action_targets.is_empty(),
        "tool rows are not rendered by default"
    );
    let header = "File edits, Read files, Ran commands";
    let pos = text_position(&output.shapes, header);
    for frame in 1..=3 {
        let events = if frame < 3 {
            vec![
                egui::Event::PointerMoved(pos),
                pointer_button(pos, egui::PointerButton::Primary, frame == 1),
            ]
        } else {
            vec![]
        };
        output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            frame as f64 * 0.2,
            events,
            egui::Modifiers::NONE,
        );
    }
    assert_eq!(
        app.action_targets.len(),
        3,
        "all tool rows can still be expanded"
    );
    text_position(&output.shapes, "Ran cargo check --locked · 1.2s");
    let pos = text_position(&output.shapes, header);
    for frame in 4..=6 {
        let events = if frame < 6 {
            vec![
                egui::Event::PointerMoved(pos),
                pointer_button(pos, egui::PointerButton::Primary, frame == 4),
            ]
        } else {
            vec![]
        };
        output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            frame as f64 * 0.2,
            events,
            egui::Modifiers::NONE,
        );
    }
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Ran cargo check"))));
    assert_eq!(
        app.saved.chats[0].messages[1].activities.len(),
        3,
        "collapse never erases activity"
    );
}

#[test]
fn context_meter_tracks_current_chat_and_connection_and_handles_missing_or_oversized_usage() {
    let mut settings = Settings {
        provider: Provider::OpenAI,
        ..Default::default()
    };
    let mut chat = Chat::new(Uuid::new_v4());
    assert_eq!(ContextMeter::for_chat(&chat, &settings).used, None);
    let mut message = Message::new(false, "answer".into(), 0.0, "OpenAI API".into());
    message.context_connection = settings.context_key();
    message.context_tokens = 64000;
    chat.messages.push(message);
    let meter = ContextMeter::for_chat(&chat, &settings);
    assert_eq!(meter.fraction(), 0.5);
    assert_eq!(meter.usage_text(), "64,000 / 128,000 tokens");
    settings
        .context_windows
        .insert(settings.context_key(), 100000);
    assert_eq!(ContextMeter::for_chat(&chat, &settings).maximum, 100000);
    chat.messages[0].context_estimated = true;
    chat.messages[0].context_tokens = 150000;
    let meter = ContextMeter::for_chat(&chat, &settings);
    assert_eq!(
        meter.fraction(),
        1.0,
        "over-limit usage cannot overdraw the ring"
    );
    assert_eq!(meter.usage_text(), "~150,000 / 100,000 tokens");
    chat.messages[0].context_tokens = 0;
    assert_eq!(ContextMeter::for_chat(&chat, &settings).fraction(), 0.0);
    settings.openai_model = "another-model".into();
    assert_eq!(ContextMeter::for_chat(&chat, &settings).used, None);
    settings.openai_model = Settings::default().openai_model;
    let mut newer = Message::new(false, "other provider".into(), 0.0, "llama.cpp".into());
    newer.context_connection = "Llama|other-url|model".into();
    newer.context_tokens = 999;
    chat.messages.push(newer);
    assert_eq!(
        ContextMeter::for_chat(&chat, &settings).used,
        None,
        "never show an old connection's report as current"
    );
    assert_eq!(grouped_tokens(0), "0");
    assert_eq!(grouped_tokens(u64::MAX), "18,446,744,073,709,551,615");
}

#[test]
fn context_ring_is_left_of_reasoning_and_hover_shows_used_and_maximum_tokens() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.settings.provider = Provider::OpenAI;
    let connection = app.saved.settings.context_key();
    app.saved.chats[0].messages[1].context_connection = connection;
    app.saved.chats[0].messages[1].context_tokens = 64000;
    let mut output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let ring = context_ring_position(&output.shapes);
    let effort = text_position(&output.shapes, "high");
    assert!(
        ring.x + 14.0 < effort.x,
        "context meter sits immediately left of effort"
    );
    assert!((ring.y - effort.y).abs() < 2.0);
    for frame in 1..=7 {
        output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            frame as f64 * 0.2,
            if frame == 1 {
                vec![egui::Event::PointerMoved(ring)]
            } else {
                vec![]
            },
            egui::Modifiers::NONE,
        );
    }
    text_position(&output.shapes, "Context usage");
    text_position(&output.shapes, "64,000 / 128,000 tokens");
    text_position(
        &output.shapes,
        "Maximum source: Fallback · not reported by provider",
    );
    text_position(
        &output.shapes,
        "50.0% of context window · provider reported",
    );
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    let meter = ContextMeter::for_chat(&app.saved.chats[1], &app.saved.settings);
    assert_eq!(meter.used, None, "switching chats resets displayed usage");
    assert_eq!(meter.fraction(), 0.0);
}

#[test]
fn chat_context_menu_renames_deletes_and_protects_running_chats() {
    for (action, running) in [
        ("Rename chat", false),
        ("Delete chat", false),
        ("Delete chat", true),
    ] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("chat".into()));
        let chat_id = app.saved.selected;
        // Keep the channel alive so polling doesn't complete the running chat.
        let (_tx, rx) = mpsc::channel();
        if running {
            app.active = Some(Active {
                chat: chat_id,
                message: app.saved.chats[0].messages[1].id,
                workspace: PathBuf::from("."),
                rx,
                task: app.runtime.spawn(std::future::pending::<()>()),
                started: Instant::now(),
                steering: None,
            });
        }
        let mut output = draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let pos = text_position(&output.shapes, &app.saved.chats[0].title);
        for frame in 1..=3 {
            let events = if frame < 3 {
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, egui::PointerButton::Secondary, frame == 1),
                ]
            } else {
                vec![]
            };
            output = draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                frame as f64 * 0.2,
                events,
                egui::Modifiers::NONE,
            );
        }
        let pos = text_position(&output.shapes, action);
        for frame in 4..=5 {
            draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                frame as f64 * 0.2,
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, egui::PointerButton::Primary, frame == 4),
                ],
                egui::Modifiers::NONE,
            );
        }
        if action == "Rename chat" {
            assert_eq!(app.rename.as_ref().map(|rename| rename.0), Some(chat_id));
        } else {
            assert_eq!(
                app.saved.chats.iter().any(|chat| chat.id == chat_id),
                running
            );
        }
    }
}

#[test]
fn deleting_another_chat_preserves_the_live_turn_tools_and_steering() {
    for select_deleted in [false, true] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let original = app.saved.selected;
        let reply_id = app.saved.chats[0].messages[1].id;
        app.saved.settings.provider = Provider::OpenAI;
        app.saved.settings.reduced_motion = true;
        app.saved.settings.show_reasoning = false;
        app.saved.chats[0].messages[1].text = "Live progress".into();
        let (events, mut steering) = fake_active(&mut app);
        app.saved.chats[0].messages[1].activities[0].status = ActionStatus::Running;
        app.saved.chats[0].messages[1].activities[1].status = ActionStatus::Waiting;
        app.saved.chats[0].draft = "Keep this guidance".into();
        app.submit(&ctx, true);
        let accepted = steering.try_recv().unwrap();

        let mut other = Chat::new(app.project().id);
        other.title = "Chat to delete".into();
        let other_id = other.id;
        let user = Message::new(true, "Old request".into(), 0.0, String::new());
        let removed_message = user.id;
        app.reveals
            .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
        other.messages.push(user);
        app.saved.chats.push(other);
        if select_deleted {
            app.saved.selected = other_id;
        }
        let mut output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let pos = text_position(&output.shapes, "Chat to delete");
        for frame in 1..=3 {
            output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                frame as f64 * 0.2,
                if frame < 3 {
                    vec![
                        egui::Event::PointerMoved(pos),
                        pointer_button(pos, egui::PointerButton::Secondary, frame == 1),
                    ]
                } else {
                    vec![]
                },
                egui::Modifiers::NONE,
            );
        }
        let pos = text_position(&output.shapes, "Delete chat");
        let output = click(&mut app, &ctx, pos, 0.8);

        assert!(!app.saved.chats.iter().any(|c| c.id == other_id));
        assert!(!app.reveals.contains_key(&removed_message));
        assert_eq!(app.saved.selected, original);
        let active = app.active.as_ref().unwrap();
        assert_eq!((active.chat, active.message), (original, reply_id));
        assert!(!active.task.is_finished());
        let chat = &app.saved.chats[0];
        assert_eq!(chat.messages[1].status, Status::Streaming);
        assert_eq!(chat.messages[1].activities[0].status, ActionStatus::Running);
        assert_eq!(chat.messages[1].activities[1].status, ActionStatus::Waiting);
        assert!(!chat.queue_paused);
        assert_eq!(chat.queue[0].id, accepted.id);
        assert!(chat.queue[0].dispatched);
        text_center(&output, "Working");
        let tool_ids = chat.messages[1].activities[..2]
            .iter()
            .map(|a| a.id.clone())
            .collect::<Vec<_>>();

        // Deletion must not allow an already dispatched steer to be sent twice.
        app.steer_queued(original, accepted.id, &ctx);
        assert!(steering.try_recv().is_err());
        for id in tool_ids {
            events
                .send(Event::ToolFinished {
                    id,
                    output: tools::ToolOutput {
                        text: "Finished after deletion".into(),
                        status: ActionStatus::Complete,
                        change: None,
                        images: Vec::new(),
                    },
                    elapsed: 0.2,
                    show_in_reply: false,
                })
                .unwrap();
        }
        events.send(Event::Steered(vec![accepted])).unwrap();
        events
            .send(Event::Text("Continued after deletion".into()))
            .unwrap();
        app.poll(&ctx);
        let chat = &app.saved.chats[0];
        assert!(chat.queue.is_empty());
        assert_eq!(chat.messages[1].status, Status::Complete);
        for activity in &chat.messages[1].activities[..2] {
            assert_eq!(activity.status, ActionStatus::Complete);
            assert_eq!(activity.result.as_ref(), "Finished after deletion");
        }
        assert_eq!(chat.messages[3].status, Status::Streaming);
        assert_eq!(chat.messages[3].text, "Continued after deletion");
        events.send(Event::Completed).unwrap();
        app.poll(&ctx);
        assert_eq!(app.saved.chats[0].messages[3].status, Status::Complete);
        assert!(app.active.is_none());
    }
}

#[test]
fn compaction_events_update_the_original_chat_and_persist_without_erasing_visible_history() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("chat".into()));
    let chat = app.saved.selected;
    let message = app.saved.chats[0].messages[1].id;
    let visible_text = app.saved.chats[0].messages[1].text.clone();
    let (tx, rx) = mpsc::channel();
    app.active = Some(Active {
        chat,
        message,
        workspace: PathBuf::from("."),
        rx,
        task: app.runtime.spawn(std::future::pending::<()>()),
        started: Instant::now(),
        steering: None,
    });
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    tx.send(Event::CompactionStarted).unwrap();
    app.poll(&ctx);
    assert!(app.saved.chats[0].messages[1].compacting);
    let snapshot = crate::context::checkpoint(
        &app.saved.settings,
        vec![serde_json::json!({"role":"user","content":"Summary: keep the workspace decisions"})],
    );
    tx.send(Event::Compacted(snapshot)).unwrap();
    tx.send(Event::ContextUsage {
        tokens: 3000,
        estimated: true,
        connection: app.saved.settings.context_key(),
    })
    .unwrap();
    tx.send(Event::ResponsesContext(vec![
        serde_json::json!({"role":"assistant","content":"Continued answer"}),
    ]))
    .unwrap();
    tx.send(Event::Completed).unwrap();
    app.poll(&ctx);
    assert!(app.active.is_none());
    assert!(!app.saved.chats[0].messages[1].compacting);
    assert_eq!(app.saved.chats[0].messages[1].text, visible_text);
    assert!(app.saved.chats[1].messages.is_empty());
    let restored: Saved =
        serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
    let message = &restored.chats[0].messages[1];
    assert!(message.context_checkpoint.is_some());
    assert_eq!(message.context_tokens, 3000);
    assert!(message.response_items[0]["content"] == "Continued answer");
}

#[test]
fn project_dialog_focus_validation_add_duplicates_and_cancel() {
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    let initial_projects = app.saved.projects.len();
    app.project_modal = true;
    app.project_focus = true;
    draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    assert!(ctx.memory(|m| m.has_focus(Id::new("project_folder_path"))));
    let output = draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        0.2,
        vec![],
        egui::Modifiers::NONE,
    );
    let pos = text_position(&output.shapes, "Add project");
    for frame in 2..=3 {
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            frame as f64 * 0.2,
            vec![
                egui::Event::PointerMoved(pos),
                pointer_button(pos, egui::PointerButton::Primary, frame == 2),
            ],
            egui::Modifiers::NONE,
        );
    }
    assert!(app.project_modal, "blank submission is disabled");
    assert_eq!(app.saved.projects.len(), initial_projects);
    app.project_path = root.path().join("missing").display().to_string();
    ctx.memory_mut(|m| m.request_focus(Id::new("project_folder_path")));
    draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        0.8,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(app.project_modal);
    assert!(app.project_error.is_some());
    app.project_path = root.path().display().to_string();
    ctx.memory_mut(|m| m.request_focus(Id::new("project_folder_path")));
    draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        1.0,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(!app.project_modal);
    assert!(app.project_error.is_none());
    assert_eq!(app.saved.projects.len(), initial_projects + 1);
    assert_eq!(
        app.project().path,
        root.path().canonicalize().unwrap().display().to_string()
    );
    app.project_modal = true;
    app.project_path = root.path().display().to_string();
    ctx.memory_mut(|m| m.request_focus(Id::new("project_folder_path")));
    draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        1.2,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        app.saved.projects.len(),
        initial_projects + 1,
        "existing folders aren't duplicated"
    );
    app.project_modal = true;
    draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        1.4,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(!app.project_modal);
}

#[test]
fn rename_dialog_focus_validation_save_and_cancel() {
    for action in ["Enter", "Save", "Cancel", "Escape", "Blank"] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("chat".into()));
        let original = app.saved.chats[0].title.clone();
        app.rename = Some((app.saved.selected, original.clone()));
        app.rename_focus = true;
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.2,
            vec![],
            egui::Modifiers::NONE,
        );
        assert!(ctx.memory(|memory| memory.has_focus(Id::new("rename_chat_title"))));
        let name = if action == "Blank" {
            "   "
        } else {
            "  A better name 🌿  "
        };
        let output = draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.4,
            vec![egui::Event::Text(name.into())],
            egui::Modifiers::NONE,
        );
        assert_eq!(
            app.rename.as_ref().unwrap().1,
            name,
            "opening selects the old name"
        );
        match action {
            "Enter" | "Escape" => {
                draw(
                    &mut app,
                    &ctx,
                    720.0,
                    540.0,
                    0.6,
                    vec![key(
                        if action == "Enter" {
                            egui::Key::Enter
                        } else {
                            egui::Key::Escape
                        },
                        egui::Modifiers::NONE,
                    )],
                    egui::Modifiers::NONE,
                );
            }
            "Save" | "Cancel" | "Blank" => {
                let pos = text_position(
                    &output.shapes,
                    if action == "Cancel" { "Cancel" } else { "Save" },
                );
                for frame in 3..=4 {
                    draw(
                        &mut app,
                        &ctx,
                        720.0,
                        540.0,
                        frame as f64 * 0.2,
                        vec![
                            egui::Event::PointerMoved(pos),
                            pointer_button(pos, egui::PointerButton::Primary, frame == 3),
                        ],
                        egui::Modifiers::NONE,
                    );
                }
            }
            _ => unreachable!(),
        }
        if matches!(action, "Enter" | "Save") {
            assert_eq!(app.saved.chats[0].title, "A better name 🌿");
            assert!(app.rename.is_none());
        } else {
            assert_eq!(app.saved.chats[0].title, original);
            assert_eq!(app.rename.is_some(), action == "Blank");
        }
    }
}

#[test]
fn typing_keeps_large_tool_history_shared_while_background_saving() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    let mut message = Message::new(
        false,
        "Finished the workspace changes.".into(),
        0.0,
        "OpenRouter".into(),
    );
    message.response_items = vec![
        serde_json::json!({"role":"tool","tool_call_id":"large","content":"x".repeat(8*1024*1024)}),
    ]
    .into();
    let arguments: std::sync::Arc<str> =
        serde_json::json!({"path":"large.rs","content":"y".repeat(256*1024)})
            .to_string()
            .into();
    for index in 0..20 {
        message.activities.push(Activity {
            id: format!("call-{index}"),
            name: "write_file".into(),
            arguments: arguments.clone(),
            result: "z".repeat(65536).into(),
            status: ActionStatus::Complete,
            elapsed: 0.1,
            change: None,
            images: Vec::new(),
        });
    }
    let wire = message.response_items.clone();
    let id = message.id;
    app.reveals.insert(
        id,
        (
            Reveal::complete(&message.text),
            Reveal::complete(&message.reasoning),
        ),
    );
    app.saved.chats[0].messages.push(message);
    let root = tempfile::tempdir().unwrap();
    app.store = persistence::Store::new(Some(root.path().join("chats.json")));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    app.store.queue(&app.saved);
    let typed = "Typing stays responsive 🌿";
    let mut timings = Vec::new();
    for (index, character) in typed.chars().enumerate() {
        let start = Instant::now();
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.2 + index as f64 * 0.1,
            vec![egui::Event::Text(character.to_string())],
            egui::Modifiers::NONE,
        );
        timings.push(start.elapsed());
    }
    assert_eq!(app.saved.chats[0].draft, typed);
    assert!(std::sync::Arc::ptr_eq(
        &wire,
        &app.saved.chats[0].messages[0].response_items
    ));
    assert!(std::sync::Arc::ptr_eq(
        &arguments,
        &app.saved.chats[0].messages[0].activities[0].arguments
    ));
    assert!(
        app.action_targets.is_empty(),
        "collapsed tools do not parse large arguments while typing"
    );
    assert_eq!(app.saved.chats[0].messages[0].activities.len(), 20);
    timings.sort();
    eprintln!(
        "Typing with 14 MiB of tool history: median {:?}, slowest {:?}",
        timings[timings.len() / 2],
        timings.last().unwrap()
    );
    app.store.flush(&app.saved).unwrap();
    assert_eq!(
        persistence::read(&root.path().join("chats.json"))
            .unwrap()
            .unwrap()
            .chats[0]
            .draft,
        typed
    );
}

// Software rasterization keeps visual QA off the user's live desktop.
// This opt-in test writes previews from real egui geometry and font textures.
#[test]
#[ignore = "writes headless visual QA artifacts"]
fn export_headless_previews() {
    for (preview, width, height) in [
        ("edits-settled", 1180, 820),
        ("edits-settled", 720, 540),
        ("git-branch", 1180, 820),
        ("git-branch", 720, 540),
        ("git-no-repo", 720, 540),
        ("git-detached", 720, 540),
        ("git-bare", 720, 540),
        ("git-long-branch", 720, 540),
        ("git-error", 720, 540),
        ("trust-changed", 720, 540),
        ("recovery-busy", 720, 540),
        ("trust", 1180, 820),
        ("trust", 720, 540),
        ("recovery", 1180, 820),
        ("recovery", 720, 540),
        ("edit-approval", 1180, 820),
        ("edit-approval", 720, 540),
        ("settings", 1180, 820),
        ("settings", 720, 540),
        ("appearance", 1180, 820),
        ("appearance", 720, 540),
        ("tools", 1180, 820),
        ("tools", 720, 540),
        ("codex", 1180, 820),
        ("settings-details", 1180, 820),
        ("settings-details", 720, 540),
        ("appearance-details", 1180, 820),
        ("appearance-details", 720, 540),
        ("tools-details", 1180, 820),
        ("tools-details", 720, 540),
        ("tools-advanced", 1180, 820),
        ("markdown", 1180, 820),
        ("markdown", 720, 540),
        ("agent-tools", 1180, 820),
        ("agent-tools", 720, 540),
        ("web-tools", 1180, 820),
        ("web-tools", 720, 540),
        ("git-attribution", 1180, 820),
        ("git-attribution", 720, 540),
        ("icon-alignment", 1180, 820),
        ("icon-alignment", 720, 540),
        ("tool-timeline", 1180, 980),
        ("tool-timeline", 720, 540),
        ("message-queue", 1180, 820),
        ("message-queue", 720, 540),
        ("message-queue-paused", 720, 540),
        ("context-meter", 1180, 820),
        ("context-meter", 720, 540),
        ("context-meter-hover", 1180, 820),
        ("welcome", 1180, 820),
        ("welcome", 1180, 600),
        ("attachments", 1180, 820),
        ("question", 1180, 820),
        ("question", 720, 540),
        ("images", 1180, 820),
        ("images", 720, 540),
        ("image-viewer", 1180, 820),
        ("image-viewer", 720, 540),
        ("chat-menu", 1180, 820),
        ("chat-menu", 720, 540),
        ("chat-menu-hover", 720, 540),
        ("rename-dialog", 1180, 820),
        ("rename-dialog", 720, 540),
        ("rename-dialog-empty", 720, 540),
        ("project-dialog", 1180, 820),
        ("project-dialog", 720, 540),
        ("project-dialog-error", 720, 540),
    ] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        if let Ok(filter) = std::env::var("HFX_PREVIEW_FILTER")
            && !preview.starts_with(&filter)
        {
            continue;
        }
        let is_chat_menu = preview.starts_with("chat-menu");
        let mut app = Harness::new(
            &cc,
            Some(
                if preview == "icon-alignment"
                    || preview.starts_with("context-meter")
                    || preview.starts_with("message-queue")
                    || preview.starts_with("tool-timeline")
                {
                    "actions"
                } else if is_chat_menu
                    || preview.starts_with("rename-dialog")
                    || preview.starts_with("project-dialog")
                {
                    "chat"
                } else {
                    preview
                }
                .into(),
            ),
        );
        app.saved.settings.reduced_motion = true;
        if preview == "agent-tools" {
            app.settings_open = true;
            app.settings_tab = 2;
        }
        if preview == "web-tools" {
            app.saved.settings.show_reasoning = false;
            let mut message = Message::new(false, "I searched the web and checked the source documentation.\n\nThe results include their original URLs, so you can verify the details.".into(), 0.0, "Codex".into());
            for (id, name, args, result) in [
                (
                    "search",
                    "web_search",
                    serde_json::json!({"query":"Rust official documentation"}),
                    serde_json::json!({"kind":"web_search","results":[{"title":"Rust Documentation","url":"https://doc.rust-lang.org/","snippet":"Official language and standard library references."}]}),
                ),
                (
                    "fetch",
                    "web_fetch",
                    serde_json::json!({"url":"https://doc.rust-lang.org/"}),
                    serde_json::json!({"kind":"web_fetch","title":"Rust Documentation","url":"https://doc.rust-lang.org/","content":"Official Rust documentation, including the book, standard library, and compiler reference.","truncated":false}),
                ),
            ] {
                message.activities.push(crate::state::Activity {
                    id: id.into(),
                    name: name.into(),
                    arguments: args.to_string().into(),
                    result: result.to_string().into(),
                    status: ActionStatus::Complete,
                    elapsed: 0.3,
                    change: None,
                    images: Vec::new(),
                });
            }
            app.reveals.insert(
                message.id,
                (Reveal::complete(&message.text), Reveal::default()),
            );
            app.saved.chats[0].messages = vec![message];
        }
        if preview == "icon-alignment" {
            app.saved.projects[0].name = "hfx".into();
            app.saved.settings.provider = Provider::Codex;
            let reply = Message::new(false, "The reply icon, project header, and composer folder share their text row’s vertical center.".into(), 0.0, "Codex".into());
            app.saved.chats[0].messages = vec![reply];
        }
        if preview.starts_with("tool-timeline") {
            app.saved.settings.show_reasoning = false;
            app.saved.chats[0].messages = vec![timeline_fixture()];
        }
        if preview.starts_with("context-meter") {
            app.saved.settings.provider = Provider::Codex;
            let connection = app.saved.settings.context_key();
            app.saved
                .settings
                .discovered_context_windows
                .insert(connection.clone(), 272000);
            let message = &mut app.saved.chats[0].messages[1];
            message.context_tokens = 136000;
            message.context_connection = connection;
        }
        if preview.starts_with("message-queue") {
            app.saved.settings.provider = Provider::Codex;
            let connection = app.saved.settings.context_key();
            app.saved
                .settings
                .discovered_context_windows
                .insert(connection.clone(), 272000);
            let message = &mut app.saved.chats[0].messages[1];
            message.text = "I’m checking the remaining changes and tests.".into();
            message.context_connection = connection;
            message.context_tokens = 45000;
            app.reveals.insert(
                message.id,
                (Reveal::complete(&message.text), Reveal::default()),
            );
            app.saved.chats[0].queue.push(QueuedMessage::new(
                "That’s awesome — also add keyboard shortcuts.".into(),
                Vec::new(),
            ));
            if preview.ends_with("paused") {
                app.saved.chats[0].queue_paused = true;
            } else {
                let (events, rx) = mpsc::channel();
                let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                let message = &mut app.saved.chats[0].messages[1];
                message.status = Status::Streaming;
                app.active = Some(Active {
                    chat: app.saved.selected,
                    message: message.id,
                    workspace: PathBuf::from("."),
                    rx,
                    steering: Some(sender),
                    started: Instant::now(),
                    task: app.runtime.spawn(async move {
                        let _channels = (events, receiver);
                        std::future::pending::<()>().await;
                    }),
                });
            }
        }
        if preview.starts_with("rename-dialog") {
            app.rename = Some((
                app.saved.selected,
                if preview.ends_with("empty") {
                    String::new()
                } else {
                    "hey".into()
                },
            ));
            app.rename_focus = true;
        }
        if preview.starts_with("project-dialog") {
            app.project_modal = true;
            app.project_focus = true;
            app.project_path = app.project().path.clone();
            if preview.ends_with("error") {
                app.project_path = "/missing/project".into();
                app.project_error = Some("Enter the path of an existing folder.".into());
            }
        }
        let mut menu_anchor = pos2(0.0, 0.0);
        let mut textures: HashMap<egui::TextureId, egui::ColorImage> = HashMap::new();
        let mut shapes = Vec::new();
        for frame in 0..8 {
            let mut events = Vec::new();
            if is_chat_menu {
                if frame == 2 || frame == 3 {
                    events.push(egui::Event::PointerMoved(menu_anchor));
                    events.push(pointer_button(
                        menu_anchor,
                        egui::PointerButton::Secondary,
                        frame == 2,
                    ));
                } else if frame >= 5 {
                    let pos = if preview.ends_with("hover") {
                        text_position(&shapes, "Delete chat")
                    } else {
                        pos2(600.0, 400.0)
                    };
                    events.push(egui::Event::PointerMoved(pos));
                }
            }
            if preview == "web-tools" && matches!(frame, 2 | 3) {
                let position = text_position(&shapes, "Searched web, Fetched pages");
                events.push(egui::Event::PointerMoved(position));
                events.push(pointer_button(
                    position,
                    egui::PointerButton::Primary,
                    frame == 2,
                ));
            }
            if (preview.ends_with("-details") || preview.ends_with("-advanced")) && frame == 2 {
                events.push(egui::Event::PointerMoved(pos2(width as f32 - 175.0, 260.0)));
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(
                        0.0,
                        if preview == "tools-details" {
                            -325.0
                        } else {
                            -4000.0
                        },
                    ),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if preview == "git-attribution" && frame == 2 {
                events.push(egui::Event::PointerMoved(pos2(width as f32 - 175.0, 260.0)));
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, -360.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if preview == "context-meter-hover" && frame == 2 {
                events.push(egui::Event::PointerMoved(context_ring_position(&shapes)));
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(width as f32, height as f32),
                )),
                time: Some(frame as f64 * 0.2),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let mut frame = eframe::Frame::_new_kittest();
                app.logic(ui.ctx(), &mut frame);
                app.ui(ui, &mut frame);
            });
            for (id, updates) in &output.textures_delta.set {
                for update in updates {
                    let egui::ImageData::Color(image) = &update.image;
                    if let Some([x, y]) = update.pos {
                        let destination = textures.get_mut(id).expect("Texture was allocated");
                        for row in 0..image.height() {
                            let start = (y + row) * destination.width() + x;
                            destination.pixels[start..start + image.width()].copy_from_slice(
                                &image.pixels[row * image.width()..(row + 1) * image.width()],
                            );
                        }
                    } else {
                        textures.insert(*id, (**image).clone());
                    }
                }
            }
            output.textures_delta.clear();
            shapes = output.shapes;
            if is_chat_menu && frame == 1 {
                menu_anchor = text_position(&shapes, &app.saved.chats[0].title);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let meshes = ctx.tessellate(shapes, 1.0);
        let mut canvas =
            image::RgbaImage::from_pixel(width, height, image::Rgba([23, 24, 25, 255]));
        for primitive in meshes {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            let texture = textures.get(&mesh.texture_id).expect("Mesh texture exists");
            let clip = primitive.clip_rect.intersect(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(width as f32, height as f32),
            ));
            for triangle in mesh.indices.as_chunks::<3>().0 {
                let v = [
                    mesh.vertices[triangle[0] as usize],
                    mesh.vertices[triangle[1] as usize],
                    mesh.vertices[triangle[2] as usize],
                ];
                let cross = |a: egui::Pos2, b: egui::Pos2, p: egui::Pos2| {
                    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
                };
                let area = cross(v[0].pos, v[1].pos, v[2].pos);
                if area.abs() < 0.0001 {
                    continue;
                }
                let min_x = v
                    .iter()
                    .map(|v| v.pos.x)
                    .fold(f32::INFINITY, f32::min)
                    .floor()
                    .max(clip.left())
                    .max(0.0) as u32;
                let max_x = v
                    .iter()
                    .map(|v| v.pos.x)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .ceil()
                    .min(clip.right())
                    .min(width as f32) as u32;
                let min_y = v
                    .iter()
                    .map(|v| v.pos.y)
                    .fold(f32::INFINITY, f32::min)
                    .floor()
                    .max(clip.top())
                    .max(0.0) as u32;
                let max_y = v
                    .iter()
                    .map(|v| v.pos.y)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .ceil()
                    .min(clip.bottom())
                    .min(height as f32) as u32;
                for y in min_y..max_y {
                    for x in min_x..max_x {
                        let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                        let weights = [
                            cross(v[1].pos, v[2].pos, p) / area,
                            cross(v[2].pos, v[0].pos, p) / area,
                            cross(v[0].pos, v[1].pos, p) / area,
                        ];
                        if weights.iter().any(|&w| w < 0.0) {
                            continue;
                        }
                        let u = (0..3).map(|i| weights[i] * v[i].uv.x).sum::<f32>();
                        let t = (0..3).map(|i| weights[i] * v[i].uv.y).sum::<f32>();
                        let tx = ((u * texture.width() as f32) as usize).min(texture.width() - 1);
                        let ty = ((t * texture.height() as f32) as usize).min(texture.height() - 1);
                        let sampled = texture.pixels[ty * texture.width() + tx].to_array();
                        let mut color = [0.0; 4];
                        for channel in 0..4 {
                            color[channel] = (0..3)
                                .map(|i| weights[i] * v[i].color.to_array()[channel] as f32)
                                .sum::<f32>()
                                * sampled[channel] as f32
                                / 255.0;
                        }
                        let pixel = canvas.get_pixel_mut(x, y);
                        for channel in 0..3 {
                            pixel[channel] =
                                (color[channel] + pixel[channel] as f32 * (1.0 - color[3] / 255.0))
                                    .clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        std::fs::create_dir_all("artifacts").unwrap();
        canvas
            .save(format!("artifacts/headless-{preview}-{width}x{height}.png"))
            .unwrap();
    }
}

#[test]
fn reply_and_project_icons_are_centered_on_their_rendered_text_rows() {
    for (width, height) in [(1180.0, 820.0), (720.0, 540.0)] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("chat".into()));
        app.saved.settings.reduced_motion = true;
        app.saved.projects[0].name = "Alignment project".into();
        app.saved.chats[0].messages = vec![Message::new(
            false,
            "Aligned reply".into(),
            0.0,
            "Codex".into(),
        )];
        let output = draw(
            &mut app,
            &ctx,
            width,
            height,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let text_rect = |label: &str, size: f32| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.job.text == label
                            && text.galley.job.sections[0].format.font_id.size == size =>
                    {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing text row: {label} ({size})"))
        };
        for (label, size, color, pieces) in [
            ("hfx", 13.0, theme::ACCENT, 3),
            ("Alignment project", 14.0, theme::MUTED, 2),
            ("Alignment project", 11.0, theme::MUTED, 2),
        ] {
            let text = text_rect(label, size);
            let region = egui::Rect::from_min_max(
                text.min - vec2(30.0, 15.0),
                pos2(text.left(), text.bottom() + 15.0),
            );
            let glyphs: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    let stroke_color = match &shape.shape {
                        egui::Shape::LineSegment { stroke, .. } => stroke.color,
                        egui::Shape::Path(path) => match path.stroke.color {
                            egui::epaint::ColorMode::Solid(color) => color,
                            _ => return None,
                        },
                        _ => return None,
                    };
                    let bounds = shape.shape.visual_bounding_rect();
                    (stroke_color == color && region.contains(bounds.center())).then_some(bounds)
                })
                .collect();
            assert_eq!(
                glyphs.len(),
                pieces,
                "missing leading icon for {label} at {width}x{height}"
            );
            let bounds = glyphs.into_iter().reduce(|a, b| a.union(b)).unwrap();
            assert!(
                (bounds.center().y - text.center().y).abs() < 1.0,
                "{label} icon at {:?} is not centered on text {:?}",
                bounds,
                text
            );
        }
    }
}

#[test]
fn git_attribution_email_ui_edits_validate_without_changing_custom_instructions() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("git-attribution".into()));
    app.saved.settings.system_prompt = "Keep my instructions".into();
    assert!(app.settings_open);
    assert_eq!(app.settings_tab, 2);
    let mut render = |time, mut events: Vec<egui::Event>, modifiers| {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(340.0, 600.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| app.git_attribution_settings(ui),
        );
        output.textures_delta.clear();
        output
    };
    render(0.0, vec![], egui::Modifiers::NONE);
    ctx.memory_mut(|m| m.request_focus(Id::new("git_coauthor_email")));
    for (time, address, trailer, warning) in [
        (
            0.2,
            "12345+fixture@users.noreply.github.com",
            "Co-authored-by: hfx <12345+fixture@users.noreply.github.com>",
            false,
        ),
        (
            0.4,
            "invalid address",
            "Co-authored-by: hfx <hfx@local.invalid>",
            true,
        ),
    ] {
        let output = render(
            time,
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Paste(address.into()),
            ],
            egui::Modifiers::COMMAND,
        );
        text_position(&output.shapes, trailer);
        assert_eq!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Enter a plain email address"))), warning);
    }
    assert_eq!(app.saved.settings.git_coauthor_email, "invalid address");
    assert_eq!(app.saved.settings.system_prompt, "Keep my instructions");
}

#[test]
fn settings_cards_fit_the_drawer_for_all_providers_and_tool_modes() {
    use crate::state::{CommandMode, SearchProvider};
    for width in [304.0, 372.0] {
        for font_size in [13.0, 15.5, 19.0] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("settings".into()));
            app.saved.settings.font_size = font_size;
            app.saved.settings.reveal_speed = 137.5;
            for tab in 0..3 {
                for (provider, search, mode) in [
                    (
                        Provider::Codex,
                        SearchProvider::DuckDuckGo,
                        CommandMode::Trusted,
                    ),
                    (
                        Provider::OpenAI,
                        SearchProvider::Brave,
                        CommandMode::Sandbox,
                    ),
                    (
                        Provider::OpenRouter,
                        SearchProvider::Searxng,
                        CommandMode::Trusted,
                    ),
                    (
                        Provider::Llama,
                        SearchProvider::DuckDuckGo,
                        CommandMode::Sandbox,
                    ),
                    (Provider::Demo, SearchProvider::Brave, CommandMode::Trusted),
                ] {
                    app.saved.settings.provider = provider;
                    app.saved.settings.search_provider = search;
                    app.saved.settings.command_mode = mode;
                    let mut output = ctx.run_ui(egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 2000.0))),
                            ..Default::default()
                        }, |ui| {
                            prefs::style(ui);
                            let right = ui.max_rect().right();
                            prefs::tabs(ui, &mut app.settings_tab);
                            match tab {
                                0 => app.provider_settings(ui),
                                1 => app.appearance_settings(ui),
                                _ => app.tool_settings(ui),
                            }
                            assert!(ui.min_rect().right() <= right + 0.5,
                                "tab {tab}, {provider:?}, width {width}: content extends to {}, expected <= {right}", ui.min_rect().right());
                        });
                    output.textures_delta.clear();
                    assert_eq!(app.saved.settings.font_size, font_size);
                    assert_eq!(app.saved.settings.reveal_speed, 137.5);
                }
            }
        }
    }
}

#[test]
fn settings_tabs_provider_selection_review_state_and_footer_are_interactive() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("settings".into()));
    app.saved.settings.reduced_motion = true;
    let mut output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    output = click(
        &mut app,
        &ctx,
        text_position(&output.shapes, "llama.cpp"),
        0.2,
    );
    assert_eq!(app.saved.settings.provider, Provider::Llama);
    output = click(
        &mut app,
        &ctx,
        text_position(&output.shapes, "Appearance"),
        0.4,
    );
    assert_eq!(app.settings_tab, 1);
    text_position(&output.shapes, "READING PREVIEW");
    output = click(
        &mut app,
        &ctx,
        text_position(&output.shapes, "Reasoning summaries"),
        0.6,
    );
    assert!(!app.saved.settings.show_reasoning);
    output = click(&mut app, &ctx, text_position(&output.shapes, "Tools"), 0.8);
    assert_eq!(app.settings_tab, 2);
    output = click(
        &mut app,
        &ctx,
        text_position(&output.shapes, "Review each tool action"),
        1.0,
    );
    assert!(app.saved.settings.review_actions);
    text_position(&output.shapes, "REVIEW MODE");
    output = click(
        &mut app,
        &ctx,
        text_position(&output.shapes, "Enable agent tools"),
        1.2,
    );
    assert!(!app.saved.settings.tools_enabled);
    text_position(&output.shapes, "TOOLS PAUSED");
    output = click(
        &mut app,
        &ctx,
        text_position(&output.shapes, "Review each tool action"),
        1.4,
    );
    assert!(
        app.saved.settings.review_actions,
        "disabled preferences must be retained"
    );
    click(&mut app, &ctx, text_position(&output.shapes, "Done"), 1.6);
    assert!(!app.settings_open);
}

#[test]
fn settings_footer_stays_visible_when_scrolling_a_small_window() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("tools".into()));
    app.saved.settings.reduced_motion = true;
    let mut last = Vec::new();
    for frame in 0..5 {
        let events = if frame == 2 {
            vec![
                egui::Event::PointerMoved(pos2(600.0, 300.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: vec2(0.0, -4000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            vec![]
        };
        let output = draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            frame as f64 * 0.2,
            events,
            egui::Modifiers::NONE,
        );
        let done = text_position(&output.shapes, "Done");
        assert!(done.y > 470.0 && done.y < 540.0);
        last = output.shapes;
    }
    let instructions = text_position(&last, "Assistant instructions");
    assert!(
        instructions.y > 170.0 && instructions.y < 470.0,
        "bottom settings must be reachable: {instructions:?}"
    );
}

#[test]
fn panels_render_at_small_and_large_sizes() {
    for preview in [
        "welcome",
        "chat",
        "markdown",
        "settings",
        "codex",
        "openrouter",
        "approval",
        "appearance",
        "git-attribution",
        "menu",
        "actions",
        "attachments",
        "question",
        "images",
        "image-viewer",
    ] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some(preview.into()));
        for (width, height) in [(1180.0, 820.0), (720.0, 540.0)] {
            for frame in 0..3 {
                let output = draw(
                    &mut app,
                    &ctx,
                    width,
                    height,
                    frame as f64 * 0.2,
                    vec![],
                    egui::Modifiers::NONE,
                );
                assert!(!output.shapes.is_empty(), "{preview} should render");
            }
        }
    }
}

fn fake_active(
    app: &mut Harness,
) -> (
    mpsc::Sender<Event>,
    tokio::sync::mpsc::UnboundedReceiver<Message>,
) {
    let chat = &mut app.saved.chats[0];
    let message = chat.messages.last_mut().unwrap();
    message.status = Status::Streaming;
    let (tx, rx) = mpsc::channel();
    let (steering, receive) = tokio::sync::mpsc::unbounded_channel();
    app.active = Some(Active {
        chat: chat.id,
        message: message.id,
        workspace: PathBuf::from("."),
        rx,
        task: app.runtime.spawn(std::future::pending::<()>()),
        started: Instant::now(),
        steering: Some(steering),
    });
    (tx, receive)
}

fn logic_tick(app: &mut Harness, ctx: &egui::Context, focused: bool, hidden: bool) -> Duration {
    let mut input = egui::RawInput::default();
    let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
    viewport.focused = Some(focused);
    viewport.occluded = Some(hidden);
    viewport.minimized = Some(hidden);
    // Deliberately leave raw.focused at its default; hidden run_logic must
    // use fresh native viewport info rather than the previous painted input.
    let mut interval = Duration::ZERO;
    let _ = ctx.run_logic(&input, |ctx| {
        interval = tick_interval(ctx);
        app.logic(ctx, &mut eframe::Frame::_new_kittest());
    });
    interval
}

#[test]
fn closing_saves_cancelled_work_and_paused_queues_before_bounded_runtime_cleanup() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let mut app = Harness::new(&cc, Some("actions".into()));
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chats.json");
    app.store = persistence::Store::new(Some(path.clone()));
    let chat = &mut app.saved.chats[0];
    chat.draft = "Keep the last typed text 🌿".into();
    chat.queue
        .push(QueuedMessage::new("Later".into(), Vec::new()));
    let message = chat.messages.last_mut().unwrap();
    message.status = Status::Streaming;
    message.activities[0].status = ActionStatus::Running;
    let (_tx, rx) = mpsc::channel();
    app.active = Some(Active {
        chat: chat.id,
        message: message.id,
        workspace: PathBuf::from("."),
        rx,
        steering: None,
        started: Instant::now(),
        task: app.runtime.spawn(std::future::pending::<()>()),
    });
    app.auth_task = Some(app.runtime.spawn(std::future::pending::<()>()));
    // Simulate eframe's save-before-on_exit ordering.
    app.store.queue(&app.saved);
    app.on_exit(None);
    assert!(app.active.is_none() && app.auth_task.is_none());
    drop(app);
    let restored = persistence::read(&path).unwrap().unwrap();
    let chat = &restored.chats[0];
    assert_eq!(chat.draft, "Keep the last typed text 🌿");
    assert!(chat.queue_paused);
    assert_eq!(chat.queue.len(), 1);
    assert_eq!(chat.messages.last().unwrap().status, Status::Cancelled);
    assert_eq!(
        chat.messages.last().unwrap().activities[0].status,
        ActionStatus::Cancelled
    );
}

#[test]
fn completion_alerts_wait_for_the_queue_batch_and_ignore_stop_and_error() {
    let saved = Saved::default();
    let mut chat = saved.chats[0].clone();
    assert!(batch_finished(true, &chat));
    assert!(!batch_finished(false, &chat));
    chat.queue
        .push(QueuedMessage::new("Follow-up".into(), Vec::new()));
    assert!(!batch_finished(true, &chat));
    chat.queue_paused = true;
    assert!(batch_finished(true, &chat));
    assert!(!batch_finished(false, &chat));
    let legacy: Settings = serde_json::from_str("{}").unwrap();
    assert!(legacy.desktop_notifications && legacy.completion_sound);
    let muted: Settings =
        serde_json::from_str(r#"{"desktop_notifications":false,"completion_sound":false}"#)
            .unwrap();
    let restored: Settings = serde_json::from_str(&serde_json::to_string(&muted).unwrap()).unwrap();
    assert!(!restored.desktop_notifications && !restored.completion_sound);
}

#[test]
fn hidden_logic_accepts_steering_and_stream_completion_without_painting() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
    let (events, mut steering) = fake_active(&mut app);
    events
        .send(Event::Text("Progress before steering 🌿.".into()))
        .unwrap();
    assert_eq!(logic_tick(&mut app, &ctx, false, true), BACKGROUND_TICK);
    assert_eq!(
        app.saved.chats[0].messages[1].text,
        "Progress before steering 🌿."
    );
    app.saved.chats[0].draft = "Guidance while hidden".into();
    app.saved.chats[0]
        .attachments
        .push(attachments::test_image());
    app.submit(&ctx, true);
    let accepted = steering.try_recv().unwrap();
    events.send(Event::Steered(vec![accepted])).unwrap();
    events
        .send(Event::Text("Continued while hidden 世界.".into()))
        .unwrap();
    events.send(Event::Completed).unwrap();
    logic_tick(&mut app, &ctx, false, true);
    let chat = &app.saved.chats[0];
    assert_eq!(chat.messages.len(), 4);
    assert_eq!(chat.messages[1].status, Status::Complete);
    assert_eq!(chat.messages[2].text, "Guidance while hidden");
    assert_eq!(chat.messages[2].attachments.len(), 1);
    assert_eq!(chat.messages[3].text, "Continued while hidden 世界.");
    assert_eq!(chat.messages[3].status, Status::Complete);
    assert!(chat.queue.is_empty() && app.active.is_none());
    assert!(chat.messages[1].timeline_valid() && chat.messages[3].timeline_valid());
    assert_eq!(
        app.reveals[&chat.messages[3].id]
            .0
            .visible(&chat.messages[3].text),
        chat.messages[3].text
    );
}

#[test]
fn hidden_logic_finishes_a_turn_and_starts_its_queued_followup_without_painting() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let (events, _) = fake_active(&mut app);
    app.saved.chats[0].draft = "Automatic follow-up".into();
    app.send(&ctx);
    let original = app.saved.selected;
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    events.send(Event::Completed).unwrap();
    logic_tick(&mut app, &ctx, false, true);
    assert_eq!(app.active.as_ref().unwrap().chat, original);
    assert_eq!(app.saved.chats[0].messages[2].text, "Automatic follow-up");
    assert!(app.saved.chats[0].queue.is_empty());
    assert!(app.saved.chats[1].messages.is_empty());
    app.stop();
}

#[test]
fn hidden_logic_autosaves_streamed_progress_without_a_painted_frame() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chats.json");
    app.store = persistence::Store::new(Some(path.clone()));
    app.last_autosave = Instant::now() - AUTOSAVE_INTERVAL;
    app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
    let (events, _) = fake_active(&mut app);
    events
        .send(Event::Text("Saved while minimized 🌿.".into()))
        .unwrap();
    logic_tick(&mut app, &ctx, false, true);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !app.store.saved_once && Instant::now() < deadline {
        app.store.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.store.saved_once,
        "background logic must queue a save, not wait for ui()"
    );
    let restored = persistence::read(&path).unwrap().unwrap();
    assert_eq!(
        restored.chats[0].messages[1].text,
        "Saved while minimized 🌿."
    );
    assert!(restored.chats[0].messages[1].timeline_valid());
    app.stop();
}

#[test]
fn hidden_logic_times_out_a_question_without_approving_workspace_actions() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("question".into()));
    let pending = app.question.as_mut().unwrap();
    let expected = pending
        .question
        .answer(pending.question.recommended(), true);
    pending.selected = 1;
    pending.custom = "Not submitted".into();
    pending.deadline = Instant::now() - Duration::from_millis(1);
    let (reply, mut answer) = oneshot::channel();
    pending.reply = reply;
    let (approval, mut decision) = oneshot::channel();
    app.pending = Some(Pending {
        command_mode: app.saved.settings.command_mode,
        call: ToolCall {
            id: "approval".into(),
            name: "write_file".into(),
            arguments: "{}".into(),
        },
        reply: approval,
        workspace: PathBuf::from("."),
    });
    logic_tick(&mut app, &ctx, false, true);
    assert_eq!(answer.try_recv().unwrap(), expected);
    assert!(app.question.is_none());
    assert!(app.pending.is_some() && decision.try_recv().is_err());
    app.stop();
}

#[test]
fn unfocused_logic_throttles_animation_and_drains_event_bursts_in_bounded_batches() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
    let (events, _) = fake_active(&mut app);
    let count = EVENTS_PER_TICK * 3 + 17;
    for _ in 0..count {
        events.send(Event::Text("🌿".into())).unwrap();
    }
    events.send(Event::Completed).unwrap();
    assert_eq!(logic_tick(&mut app, &ctx, false, false), BACKGROUND_TICK);
    let reply = &app.saved.chats[0].messages[1];
    assert!(!reply.text.is_empty() && reply.text.len() <= EVENTS_PER_TICK * "🌿".len());
    assert!(
        app.active.is_some(),
        "one tick must yield before draining a whole burst"
    );
    assert_eq!(app.reveals[&reply.id].0.visible(&reply.text), reply.text);
    for _ in 0..100 {
        if app.active.is_none() {
            break;
        }
        logic_tick(&mut app, &ctx, false, false);
    }
    assert!(app.active.is_none());
    assert_eq!(app.saved.chats[0].messages[1].text, "🌿".repeat(count));
    assert_eq!(app.saved.chats[0].messages[1].status, Status::Complete);
    assert_eq!(logic_tick(&mut app, &ctx, true, false), FOREGROUND_TICK);
}

#[test]
fn enter_queues_while_running_then_fifo_drains_without_changing_the_draft_or_selected_chat() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let (events, _steering) = fake_active(&mut app);
    let original = app.saved.selected;
    let active_message = app.active.as_ref().unwrap().message;
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    app.saved.chats[0].draft = "Follow-up one".into();
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.2,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert_eq!(app.active.as_ref().unwrap().message, active_message);
    assert_eq!(
        app.saved.chats[0].messages.len(),
        2,
        "unsent queue is not model history"
    );
    assert_eq!(app.saved.chats[0].queue[0].text, "Follow-up one");
    app.saved.chats[0].draft = "Follow-up two".into();
    app.saved.chats[0]
        .attachments
        .push(attachments::test_image());
    app.send(&ctx);
    assert_eq!(app.saved.chats[0].queue.len(), 2);
    app.saved.chats[0].draft = "Keep typing this draft".into();
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    events.send(Event::Completed).unwrap();
    app.poll(&ctx);
    assert_eq!(app.active.as_ref().unwrap().chat, original);
    assert_eq!(app.saved.chats[0].messages[2].text, "Follow-up one");
    assert_eq!(app.saved.chats[0].queue[0].text, "Follow-up two");
    assert_eq!(app.saved.chats[0].queue[0].attachments.len(), 1);
    assert_eq!(app.saved.chats[0].draft, "Keep typing this draft");
    assert_eq!(app.saved.selected, app.saved.chats[1].id);
    app.stop();
    assert!(app.saved.chats[0].queue_paused);
}

#[test]
fn steer_button_sends_once_and_acknowledgement_splits_history_in_the_original_chat() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    app.saved.settings.provider = Provider::OpenAI;
    let (events, mut steering) = fake_active(&mut app);
    app.saved.chats[0].draft = "Focus on tests instead".into();
    app.saved.chats[0]
        .attachments
        .push(attachments::test_image());
    app.send(&ctx);
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let pos = text_center(&output, "Steer");
    click(&mut app, &ctx, pos, 0.2);
    let user = steering.try_recv().unwrap();
    assert_eq!(user.attachments.len(), 1);
    assert!(app.saved.chats[0].queue[0].dispatched);
    app.steer_queued(app.saved.selected, user.id, &ctx);
    assert!(
        steering.try_recv().is_err(),
        "a steer cannot be dispatched twice"
    );
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    events
        .send(Event::ResponsesContext(vec![
            serde_json::json!({"role":"assistant","content":"Before steering"}),
        ]))
        .unwrap();
    events.send(Event::Steered(vec![user.clone()])).unwrap();
    events.send(Event::Text("After steering".into())).unwrap();
    events.send(Event::Completed).unwrap();
    app.poll(&ctx);
    let messages = &app.saved.chats[0].messages;
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1].status, Status::Complete);
    assert_eq!(messages[1].response_items[0]["content"], "Before steering");
    assert_eq!(messages[2].id, user.id);
    assert_eq!(messages[3].text, "After steering");
    assert!(app.saved.chats[0].queue.is_empty());
    assert!(app.saved.chats[1].messages.is_empty());
}

#[test]
fn stop_error_and_late_steering_preserve_unsent_messages_without_auto_running() {
    for error in [false, true] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let (events, _steering) = fake_active(&mut app);
        app.saved.chats[0].draft = "Unapplied steer".into();
        app.submit(&ctx, true);
        assert!(app.saved.chats[0].queue[0].dispatched);
        if error {
            events.send(Event::Error("fixture error".into())).unwrap();
            app.poll(&ctx);
        } else {
            app.stop();
            app.poll(&ctx);
        }
        assert!(app.active.is_none());
        assert!(app.saved.chats[0].queue_paused);
        assert!(!app.saved.chats[0].queue[0].dispatched);
        assert_eq!(app.saved.chats[0].queue[0].text, "Unapplied steer");
    }
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let (events, steering) = fake_active(&mut app);
    drop(steering);
    app.saved.chats[0].draft = "Late steer".into();
    app.submit(&ctx, true);
    assert!(!app.saved.chats[0].queue[0].dispatched);
    events.send(Event::Completed).unwrap();
    app.poll(&ctx);
    assert_eq!(
        app.saved.chats[0].messages[2].text, "Late steer",
        "late steering becomes a normal follow-up"
    );
    app.stop();
}

#[test]
fn stop_commits_an_acknowledged_steer_but_keeps_unacknowledged_work_queued() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let (events, mut steering) = fake_active(&mut app);
    app.saved.chats[0].draft = "Accepted guidance".into();
    app.submit(&ctx, true);
    let accepted = steering.try_recv().unwrap();
    app.saved.chats[0].draft = "Not yet accepted".into();
    app.submit(&ctx, true);
    events.send(Event::Steered(vec![accepted])).unwrap();
    events
        .send(Event::Text("Partial new answer".into()))
        .unwrap();
    app.stop();
    let chat = &app.saved.chats[0];
    assert_eq!(chat.messages[2].text, "Accepted guidance");
    assert_eq!(chat.messages[3].text, "Partial new answer");
    assert_eq!(chat.messages[3].status, Status::Cancelled);
    assert_eq!(chat.queue.len(), 1);
    assert_eq!(chat.queue[0].text, "Not yet accepted");
    assert!(chat.queue_paused);
    assert!(!chat.queue[0].dispatched);
}

#[test]
fn command_enter_steers_only_the_active_chat_and_paused_queues_wait_for_resume() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let (_events, mut steering) = fake_active(&mut app);
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    app.saved.chats[0].draft = "Command-enter guidance".into();
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.2,
        vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        egui::Modifiers::COMMAND,
    );
    assert_eq!(steering.try_recv().unwrap().text, "Command-enter guidance");
    let other = Chat::new(app.project().id);
    app.saved.selected = other.id;
    app.saved.chats.push(other);
    app.saved.chats[1].draft = "Only the other chat".into();
    app.submit(&ctx, true);
    assert!(
        steering.try_recv().is_err(),
        "cannot steer a different chat's run"
    );
    assert_eq!(app.saved.chats[1].queue[0].text, "Only the other chat");
    app.stop();
    app.saved.chats[1].draft = "Still paused".into();
    app.send(&ctx);
    assert!(app.active.is_none());
    assert_eq!(app.saved.chats[1].queue.len(), 2);
    assert!(app.saved.chats[1].queue_paused);
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.4,
        vec![],
        egui::Modifiers::NONE,
    );
    click(&mut app, &ctx, text_center(&output, "Resume"), 0.6);
    assert_eq!(app.active.as_ref().unwrap().chat, app.saved.chats[1].id);
    assert_eq!(app.saved.chats[1].messages[0].text, "Only the other chat");
    app.stop();
}

#[test]
fn queue_edit_preserves_attachments_and_composer_draft_and_can_cancel() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("actions".into()));
    let queued = QueuedMessage::new("old".into(), vec![attachments::test_image()]);
    let id = queued.id;
    app.saved.chats[0].queue.push(queued);
    app.saved.chats[0].queue_paused = true;
    app.saved.chats[0].draft = "Keep this draft".into();
    app.queue_edit = Some((app.saved.selected, id, "Edited message".into()));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.1,
        vec![],
        egui::Modifiers::NONE,
    );
    click(&mut app, &ctx, text_center(&output, "Save"), 0.2);
    assert!(app.queue_edit.is_none());
    assert_eq!(app.saved.chats[0].queue[0].text, "Edited message");
    assert_eq!(app.saved.chats[0].queue[0].attachments.len(), 1);
    assert_eq!(app.saved.chats[0].draft, "Keep this draft");
    app.queue_edit = Some((app.saved.selected, id, "Discard me".into()));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.4,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(app.queue_edit.is_none());
    assert_eq!(app.saved.chats[0].queue[0].text, "Edited message");
}

#[test]
fn enter_sends_and_stop_targets_original_chat_after_switching() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    app.saved.chats[0].draft = "Hello".into();
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.2,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(app.active.is_some());
    assert_eq!(app.saved.chats[0].messages[0].text, "Hello");
    assert!(app.saved.chats[0].draft.is_empty());
    let original = app.saved.selected;
    app.new_chat(app.project().id, 0.3);
    assert_ne!(app.saved.selected, original);
    app.stop();
    assert_eq!(app.saved.chats[0].messages[1].status, Status::Cancelled);
    assert!(app.saved.chats[1].messages.is_empty());
}

#[test]
fn pasted_files_stay_in_the_original_chat_and_attachment_only_send_works() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pasted file.rs");
    std::fs::write(&path, "fn pasted() {}\n").unwrap();
    let url = reqwest::Url::from_file_path(&path).unwrap();
    let mut input = egui::RawInput {
        events: vec![egui::Event::Paste(url.to_string())],
        ..Default::default()
    };
    app.raw_input_hook(&ctx, &mut input);
    assert!(input.events.is_empty());
    assert_eq!(app.attachment_jobs.len(), 1);
    let original = app.saved.selected;
    app.new_chat(app.project().id, 0.2);
    for _ in 0..100 {
        app.poll_attachments(&ctx);
        if app.attachment_jobs.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.attachment_jobs.is_empty());
    assert_eq!(app.saved.chats[0].attachments[0].name, "pasted file.rs");
    assert!(app.saved.chats[1].attachments.is_empty());
    assert!(app.saved.chats[0].draft.is_empty());
    app.saved.selected = original;
    app.send(&ctx);
    assert!(app.active.is_some());
    assert!(app.saved.chats[0].attachments.is_empty());
    assert_eq!(app.saved.chats[0].messages[0].attachments.len(), 1);
    assert_eq!(app.saved.chats[0].title, "pasted file.rs");
    app.stop();
}

#[test]
fn question_timeout_submits_recommendation_even_when_another_option_was_selected() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("question".into()));
    let (reply, mut answer) = oneshot::channel();
    let pending = app.question.as_mut().unwrap();
    pending.selected = 2;
    pending.deadline = Instant::now() - Duration::from_millis(1);
    pending.reply = reply;
    app.poll(&ctx);
    assert!(app.question.is_none());
    let result: serde_json::Value = serde_json::from_str(&answer.try_recv().unwrap()).unwrap();
    assert_eq!(result["answer"], "A panel beside the chat");
    assert_eq!(result["automatic"], true);
}

fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && text.galley.job.text == label
            {
                return Some(text.pos + text.galley.size() * 0.5);
            }
            None
        })
        .unwrap_or_else(|| panic!("Missing visible text: {label}"))
}

fn click(app: &mut Harness, ctx: &egui::Context, pos: egui::Pos2, time: f64) -> egui::FullOutput {
    draw(
        app,
        ctx,
        1180.0,
        820.0,
        time,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        egui::Modifiers::NONE,
    );
    draw(
        app,
        ctx,
        1180.0,
        820.0,
        time + 0.01,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    )
}

#[test]
fn returned_image_thumbnail_opens_viewer_and_escape_closes_it() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("images".into()));
    app.saved.settings.reduced_motion = true;
    let mut output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    for frame in 1..4 {
        output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            frame as f64 * 0.2,
            vec![],
            egui::Modifiers::NONE,
        );
    }
    let position = text_center(&output, "workspace-chart.png");
    assert!(app.image_preview.is_none());
    click(&mut app, &ctx, position, 1.0);
    assert_eq!(
        app.image_preview.as_ref().unwrap().image.name,
        "workspace-chart.png"
    );
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        1.2,
        vec![],
        egui::Modifiers::NONE,
    );
    text_center(&output, "Save PNG…");
    text_center(&output, "Fit");
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        1.4,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(app.image_preview.is_none());
    assert_eq!(app.saved.chats[0].messages[1].attachments.len(), 1);
}

#[test]
fn image_events_store_viewed_images_in_actions_and_sent_images_in_replies() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("chat".into()));
    let id = app.saved.chats[0].messages[1].id;
    let (tx, rx) = mpsc::channel();
    let task = app.runtime.spawn(std::future::pending::<()>());
    app.active = Some(Active {
        chat: app.saved.selected,
        message: id,
        workspace: PathBuf::from("."),
        rx,
        task,
        started: Instant::now(),
        steering: None,
    });
    for (name, show_in_reply) in [("view_image", false), ("send_image", true)] {
        tx.send(Event::ToolStarted(ToolCall {
            id: name.into(),
            name: name.into(),
            arguments: "{}".into(),
        }))
        .unwrap();
        tx.send(Event::ToolFinished {
            id: name.into(),
            output: tools::ToolOutput {
                text: "Image".into(),
                status: ActionStatus::Complete,
                change: None,
                images: vec![attachments::test_image()],
            },
            elapsed: 0.1,
            show_in_reply,
        })
        .unwrap();
    }
    tx.send(Event::Image(attachments::test_image())).unwrap();
    tx.send(Event::Completed).unwrap();
    app.poll(&ctx);
    let message = &app.saved.chats[0].messages[1];
    assert_eq!(message.attachments.len(), 2);
    assert_eq!(message.activities.len(), 2);
    assert!(
        message
            .activities
            .iter()
            .all(|a| a.status == ActionStatus::Complete && a.images.len() == 1)
    );
    assert!(app.active.is_none());
}

#[test]
fn question_selection_and_send_return_the_selected_option() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("question".into()));
    let (reply, mut answer) = oneshot::channel();
    app.question.as_mut().unwrap().reply = reply;
    let mut output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    for frame in 1..4 {
        output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            frame as f64 * 0.1,
            vec![],
            egui::Modifiers::NONE,
        );
    }
    let position = text_center(&output, "2. A separate window");
    output = click(&mut app, &ctx, position, 0.5);
    assert_eq!(app.question.as_ref().unwrap().selected, 1);
    assert!(answer.try_recv().is_err(), "Choosing must wait for Send");
    let position = text_center(&output, "Send");
    click(&mut app, &ctx, position, 0.7);
    assert!(app.question.is_none());
    let result: serde_json::Value = serde_json::from_str(&answer.try_recv().unwrap()).unwrap();
    assert_eq!(result["answer"], "A separate window");
    assert_eq!(result["automatic"], false);
}

#[test]
fn custom_question_reply_enter_sends_without_altering_chat_draft() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("question".into()));
    app.saved.chats[0].draft = "Keep this draft".into();
    let (reply, mut answer) = oneshot::channel();
    app.question.as_mut().unwrap().reply = reply;
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let id = Id::new(("question_reply", &app.question.as_ref().unwrap().id));
    ctx.memory_mut(|m| m.request_focus(id));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.2,
        vec![egui::Event::Paste("Use a floating inspector".into())],
        egui::Modifiers::NONE,
    );
    assert_eq!(
        app.question.as_ref().unwrap().custom,
        "Use a floating inspector"
    );
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.3,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
    );
    assert!(app.question.is_none());
    let result: serde_json::Value = serde_json::from_str(&answer.try_recv().unwrap()).unwrap();
    assert_eq!(result["answer"], "Use a floating inspector");
    assert_eq!(app.saved.chats[0].draft, "Keep this draft");
}

#[test]
fn welcome_suggestions_never_cross_into_the_composer() {
    for (width, height) in [
        (1180.0, 820.0),
        (1180.0, 680.0),
        (1180.0, 600.0),
        (720.0, 540.0),
    ] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        app.saved.settings.reduced_motion = true;
        let mut output = draw(
            &mut app,
            &ctx,
            width,
            height,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        for frame in 1..4 {
            output = draw(
                &mut app,
                &ctx,
                width,
                height,
                frame as f64 * 0.2,
                vec![],
                egui::Modifiers::NONE,
            );
        }
        let composer_top = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.job.text.contains("Ask, build, fix, explore")
                {
                    return Some(text.pos.y - 50.0);
                }
                None
            })
            .next()
            .expect("Composer text should be visible");
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && [
                    "Explore the codebase",
                    "Build something new",
                    "Find an improvement",
                    "Map out the project",
                    "From idea to first draft",
                    "A fresh pair of eyes",
                ]
                .contains(&text.galley.job.text.as_str())
            {
                assert!(
                    text.pos.y + text.galley.size().y <= composer_top,
                    "Suggestions must fit above the composer at {width}x{height}"
                );
                assert!(shape.clip_rect.bottom() <= composer_top + 10.0);
            }
        }
    }
}

#[test]
fn shift_enter_adds_newline_without_starting_generation() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    app.saved.chats[0].draft = "Hello".into();
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.2,
        vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)],
        egui::Modifiers::SHIFT,
    );
    assert!(app.active.is_none());
    assert!(app.saved.chats[0].draft.contains('\n'));
}

#[test]
fn host_trust_prompt_pauses_original_queue_and_never_times_out_into_approval() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("welcome".into()),
    );
    let root = tempfile::tempdir().unwrap();
    app.saved.projects[0].path = root.path().display().to_string();
    app.saved.settings.provider = Provider::Llama;
    app.saved.settings.review_actions = true;
    app.saved.chats[0].draft = "Do not run before trust".into();
    let chat_id = app.saved.chats[0].id;
    app.send(&ctx);
    assert!(app.active.is_none());
    assert!(app.saved.chats[0].messages.is_empty());
    assert!(app.saved.chats[0].queue_paused);
    assert_eq!(app.saved.chats[0].queue.len(), 1);
    assert_eq!(app.trust_request.as_ref().unwrap().chat, Some(chat_id));
    assert!(!app.project().is_trusted());
    let _ = draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        31.0,
        vec![],
        egui::Modifiers::NONE,
    );
    assert!(app.active.is_none());
    assert!(!app.project().is_trusted());
    app.saved.settings.provider = Provider::Demo; // Deterministic, no model connection.
    app.approve_project_trust(&ctx).unwrap();
    assert!(app.project().is_trusted());
    assert!(app.trust_request.is_none());
    assert!(app.saved.chats[0].queue.is_empty());
    assert_eq!(
        app.saved.chats[0].messages[0].text,
        "Do not run before trust"
    );
    assert_eq!(app.active.as_ref().unwrap().chat, chat_id);
    app.revoke_project_trust(app.saved.projects[0].id);
    assert!(!app.project().is_trusted());
    assert!(app.active.is_none());
    assert_eq!(
        app.saved.chats[0].messages.last().unwrap().status,
        Status::Cancelled
    );
}

#[test]
fn recovery_blocks_startup_autosave_hidden_autosave_and_exit_until_resolved() {
    let ctx = egui::Context::default();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chats.json");
    let bytes = b"malformed original history";
    std::fs::write(&path, bytes).unwrap();
    let (saved, recovery) = crate::recovery::load(Some(&path), Saved::default);
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("welcome".into()),
    );
    app.preview = false;
    app.saved = saved;
    app.recovery = recovery;
    app.recovery_open = true;
    app.store = persistence::Store::blocked(app.recovery.as_ref().unwrap().error.clone());
    app.store.queue(&app.saved); // Constructor path.
    app.last_autosave = Instant::now() - Duration::from_secs(6);
    logic_tick(&mut app, &ctx, false, true);
    let _ = draw(
        &mut app,
        &ctx,
        720.0,
        540.0,
        1.0,
        vec![],
        egui::Modifiers::NONE,
    );
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }
    let mut storage = Storage(std::collections::HashMap::from([(
        "hfx.v1".into(),
        "legacy fixture".into(),
    )]));
    app.save(&mut storage);
    assert_eq!(
        eframe::Storage::get_string(&storage, "hfx.v1").as_deref(),
        Some("legacy fixture")
    );
    assert!(!app.store.saved_once);
    app.on_exit(None);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let backup = app.recovery.as_ref().unwrap().backup().unwrap();
    let (tx, rx) = mpsc::channel();
    app.recovery_rx = Some(rx);
    tx.send(Ok(backup)).unwrap();
    app.poll_recovery(&ctx);
    assert!(app.recovery.is_none());
    app.store.flush(&app.saved).unwrap();
    assert!(persistence::read(&path).unwrap().is_some());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn trust_cancellation_and_host_disabled_choice_preserve_the_original_chat() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("welcome".into()),
    );
    let root = tempfile::tempdir().unwrap();
    app.saved.projects[0].path = root.path().display().to_string();
    app.saved.settings.provider = Provider::Llama;
    app.saved.settings.mcp_enabled = true;
    app.saved.chats[0].draft = "Original queued message".into();
    app.send(&ctx);
    let original = app.saved.chats[0].id;
    let _ = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.1,
        vec![],
        egui::Modifiers::NONE,
    );
    click(&mut app, &ctx, text_center(&output, "Cancel"), 0.2);
    assert!(app.trust_request.is_none());
    assert!(app.active.is_none());
    assert!(app.saved.chats[0].queue_paused);
    assert!(!app.project().is_trusted());
    app.saved.chats[0].queue_paused = false;
    assert!(!app.start_next(original, &ctx));
    let other_root = tempfile::tempdir().unwrap();
    let other_project = Project::from_path(other_root.path().display().to_string());
    let other = Chat::new(other_project.id);
    app.saved.selected = other.id;
    app.saved.projects.push(other_project);
    app.saved.chats.push(other);
    app.saved.settings.provider = Provider::Demo;
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.4,
        vec![],
        egui::Modifiers::NONE,
    );
    click(
        &mut app,
        &ctx,
        text_center(&output, "Continue without shell commands or MCP"),
        0.5,
    );
    assert!(!app.saved.settings.commands_enabled && !app.saved.settings.mcp_enabled);
    assert!(app.trust_request.is_none());
    assert!(app.saved.projects.iter().all(|p| !p.is_trusted()));
    assert_eq!(app.active.as_ref().unwrap().chat, original);
    assert_eq!(
        app.active.as_ref().unwrap().workspace,
        root.path().canonicalize().unwrap()
    );
    assert!(app.saved.chats[1].messages.is_empty());
    assert_eq!(
        app.saved.chats[0].messages[0].text,
        "Original queued message"
    );
    app.stop();
}

#[test]
fn failed_recovery_remains_blocked_and_keeps_the_error_visible() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("recovery".into()),
    );
    let (tx, rx) = mpsc::channel();
    app.recovery_rx = Some(rx);
    tx.send(Err("Fixture backup failure".into())).unwrap();
    app.poll_recovery(&ctx);
    assert!(app.recovery.is_some() && app.recovery_open);
    assert_eq!(
        app.recovery.as_ref().unwrap().error,
        "Fixture backup failure"
    );
    assert!(app.store.flush(&app.saved).is_err());
    assert!(!app.store.saved_once);
}

#[test]
fn styled_safety_dialogs_keep_headers_and_decisions_visible_with_long_details() {
    fn collect_rects<'a>(shape: &'a egui::Shape, rects: &mut Vec<&'a egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_rects(shape, rects);
                }
            }
            egui::Shape::Rect(rect) => rects.push(rect),
            _ => {}
        }
    }
    for (width, height) in [(720.0, 540.0), (1180.0, 820.0)] {
        for (preview, title, decisions) in [
            (
                "trust",
                "Trust this project with host access?",
                &["Cancel", "Trust project and continue"][..],
            ),
            (
                "recovery",
                "Protect your chat history",
                &[
                    "Retry loading original",
                    "Back up original and enable saving",
                    "Continue without saving",
                ][..],
            ),
        ] {
            for long_details in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Harness::new(
                    &eframe::CreationContext::_new_kittest(ctx.clone()),
                    Some(preview.into()),
                );
                if long_details {
                    if let Some(recovery) = &mut app.recovery {
                        recovery.path = PathBuf::from(
                            "example-profile/".to_owned()
                                + &"very-long-directory/".repeat(35)
                                + "chats.json",
                        );
                        recovery.error =
                            "A long diagnostic with Unicode 世界 and additional recovery details. "
                                .repeat(100);
                    } else {
                        app.saved.projects[0].name = "Long project name 世界 ".repeat(40);
                    }
                }
                let mut output = draw(
                    &mut app,
                    &ctx,
                    width,
                    height,
                    0.0,
                    vec![],
                    egui::Modifiers::NONE,
                );
                for frame in 1..4 {
                    output = draw(
                        &mut app,
                        &ctx,
                        width,
                        height,
                        frame as f64 * 0.1,
                        vec![],
                        egui::Modifiers::NONE,
                    );
                }
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height));
                for label in std::iter::once(title).chain(decisions.iter().copied()) {
                    let shape = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label)).unwrap_or_else(|| panic!("Missing {label} in {preview}"));
                    let egui::Shape::Text(text) = &shape.shape else {
                        unreachable!()
                    };
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(
                        screen.contains_rect(rect) && shape.clip_rect.contains_rect(rect),
                        "{label} is clipped at {width}x{height}: {rect:?}"
                    );
                }
                let mut rects = Vec::new();
                for shape in &output.shapes {
                    collect_rects(&shape.shape, &mut rects);
                }
                let modal = rects
                    .iter()
                    .find(|rect| {
                        rect.fill == theme::SURFACE
                            && rect.corner_radius.nw == 14
                            && rect.rect.width() > 500.0
                    })
                    .expect("Styled modal frame")
                    .rect;
                assert!(
                    screen.contains_rect(modal),
                    "{preview} frame extends outside the window: {modal:?}"
                );
                assert!(
                    rects
                        .iter()
                        .any(|rect| rect.fill == theme::ACCENT && rect.rect.height() >= 36.0),
                    "Missing differentiated primary action"
                );
                assert!(app.active.is_none());
                assert!(!app.project().is_trusted());
            }
        }
    }
}

#[test]
fn styled_safety_dialogs_escape_dismissal_never_grants_trust_or_enables_saving() {
    for preview in ["trust", "recovery"] {
        let ctx = egui::Context::default();
        let mut app = Harness::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            Some(preview.into()),
        );
        for frame in 0..3 {
            draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                frame as f64 * 0.1,
                vec![],
                egui::Modifiers::NONE,
            );
        }
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.4,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(!app.project().is_trusted());
        assert!(app.active.is_none());
        if preview == "trust" {
            assert!(app.trust_request.is_none());
        } else {
            assert!(!app.recovery_open);
            assert!(app.recovery.is_some());
            assert!(app.store.flush(&app.saved).is_err());
            assert!(!app.store.saved_once);
        }
    }
}

#[test]
fn styled_recovery_dialog_keeps_actions_disabled_while_recovery_is_busy() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("recovery".into()),
    );
    let (tx, rx) = mpsc::channel();
    app.recovery_rx = Some(rx);
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.1,
        vec![],
        egui::Modifiers::NONE,
    );
    click(
        &mut app,
        &ctx,
        text_center(&output, "Back up original and enable saving"),
        0.2,
    );
    assert!(app.recovery_rx.is_some());
    assert!(app.store.flush(&app.saved).is_err());
    // A second recovery job must not replace the original result channel.
    tx.send(Err("Original recovery task still owns the result".into()))
        .unwrap();
    app.poll_recovery(&ctx);
    assert_eq!(
        app.recovery.as_ref().unwrap().error,
        "Original recovery task still owns the result"
    );
}

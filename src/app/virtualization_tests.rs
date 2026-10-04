//! Viewport regressions use synthetic chats, never persisted history/providers.
use super::*;

fn fixture(count: usize) -> (Harness, egui::Context) {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    app.saved.settings.reduced_motion = true;
    app.saved.settings.show_reasoning = false;
    theme::install(&ctx, 14.0, true);
    for index in 0..count {
        let body = format!(
            "Message {index}\n{}",
            "Formatted **prose**, Unicode café 🌿, and a [link](https://example.com).\n"
                .repeat(index % 7 + 1)
        );
        let message = Message::new(index % 3 == 0, body, 0.0, "Demo".into());
        app.saved.chats[0].messages.push(message);
    }
    (app, ctx)
}

fn frame(
    app: &mut Harness,
    ctx: &egui::Context,
    size: egui::Vec2,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| app.conversation(ui, time),
    );
    output.textures_delta.clear();
    output
}

fn draw_chat(
    app: &mut Harness,
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    frame(app, ctx, vec2(1180.0, 820.0), time, events)
}

fn warm(app: &mut Harness, ctx: &egui::Context) -> egui::FullOutput {
    draw_chat(app, ctx, 0.0, vec![]);
    draw_chat(app, ctx, 0.5, vec![]);
    draw_chat(app, ctx, 1.0, vec![])
}

fn visible_text(output: &egui::FullOutput) -> Vec<(String, egui::Pos2, egui::Vec2)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && shape
                    .clip_rect
                    .intersects(egui::Rect::from_min_size(text.pos, text.galley.size()))
            {
                Some((text.galley.job.text.clone(), text.pos, text.galley.size()))
            } else {
                None
            }
        })
        .collect()
}

fn assert_reference(app: &mut Harness, ctx: &egui::Context, size: egui::Vec2, time: f64) {
    let virtualized = frame(app, ctx, size, time, vec![]);
    let height = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .content_height;
    app.conversation_layout.force_full = true;
    let full = frame(app, ctx, size, time + 0.001, vec![]);
    assert_eq!(
        height,
        app.conversation_layout
            .viewport
            .as_ref()
            .unwrap()
            .content_height
    );
    assert_eq!(
        visible_text(&virtualized),
        visible_text(&full),
        "visible native layout must be identical"
    );
    app.conversation_layout.force_full = false;
}

fn unstick(app: &mut Harness, ctx: &egui::Context, time: f64) {
    let pos = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .rect
        .center();
    draw_chat(
        app,
        ctx,
        time,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 40.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    // Drain wheel smoothing before setting a precise offset for comparisons.
    for index in 1..=10 {
        draw_chat(app, ctx, time + index as f64 * 0.1, vec![]);
    }
}

fn offset(app: &Harness, ctx: &egui::Context, y: f32) {
    let id = app.conversation_layout.viewport.as_ref().unwrap().id;
    let mut state = egui::scroll_area::State::load(ctx, id).unwrap();
    state.offset.y = y;
    state.store(ctx, id);
}

fn press_release(
    app: &mut Harness,
    ctx: &egui::Context,
    pos: egui::Pos2,
    time: f64,
) -> egui::FullOutput {
    draw_chat(
        app,
        ctx,
        time,
        vec![
            egui::Event::PointerMoved(pos),
            pointer_button(pos, egui::PointerButton::Primary, true),
        ],
    );
    draw_chat(
        app,
        ctx,
        time + 0.01,
        vec![pointer_button(pos, egui::PointerButton::Primary, false)],
    )
}

#[test]
fn warm_viewport_skips_offscreen_rows_and_matches_full_layout_at_scroll_positions() {
    let (mut app, ctx) = fixture(96);
    warm(&mut app, &ctx);
    assert!(app.conversation_layout.rendered < 20);
    let view = app.conversation_layout.viewport.as_ref().unwrap();
    assert!((view.offset + view.rect.height() - view.content_height).abs() < 1.0);
    unstick(&mut app, &ctx, 2.0);
    let maximum = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .content_height
        - 820.0;
    for (index, y) in [0.0, maximum * 0.3, maximum * 0.7, maximum]
        .into_iter()
        .enumerate()
    {
        offset(&app, &ctx, y);
        assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 4.0 + index as f64);
    }
    assert_eq!(app.saved.chats[0].messages.len(), 96);
}

#[test]
fn width_zoom_font_definitions_and_reasoning_preferences_invalidate_exact_heights() {
    let (mut app, ctx) = fixture(48);
    for message in &mut app.saved.chats[0].messages {
        message.reasoning = "Reasoning details\n".repeat(8);
    }
    warm(&mut app, &ctx);
    let wide = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .content_height;
    frame(&mut app, &ctx, vec2(320.0, 650.0), 2.0, vec![]);
    assert!(
        app.conversation_layout.rendered >= 48,
        "width change remeasures all rows (including any corrective pass)"
    );
    assert!(
        app.conversation_layout
            .viewport
            .as_ref()
            .unwrap()
            .content_height
            > wide
    );
    assert_reference(&mut app, &ctx, vec2(320.0, 650.0), 3.0);
    ctx.set_pixels_per_point(1.75);
    frame(&mut app, &ctx, vec2(320.0, 650.0), 4.0, vec![]);
    assert!(app.conversation_layout.rendered >= 48);
    assert_reference(&mut app, &ctx, vec2(320.0, 650.0), 5.0);
    let mut fonts = egui::FontDefinitions::default();
    fonts.families.insert(
        egui::FontFamily::Proportional,
        fonts.families[&egui::FontFamily::Monospace].clone(),
    );
    ctx.set_fonts(fonts);
    frame(&mut app, &ctx, vec2(320.0, 650.0), 6.0, vec![]);
    assert!(app.conversation_layout.rendered >= 48);
    assert_reference(&mut app, &ctx, vec2(320.0, 650.0), 7.0);
    app.saved.settings.show_reasoning = true;
    app.saved.settings.font_size = 18.0;
    theme::install(&ctx, 18.0, true);
    frame(&mut app, &ctx, vec2(320.0, 650.0), 8.0, vec![]);
    assert!(app.conversation_layout.rendered >= 48);
    assert_reference(&mut app, &ctx, vec2(320.0, 650.0), 9.0);
}

#[test]
fn append_delete_reorder_and_chat_switch_never_reuse_another_messages_height() {
    let (mut app, ctx) = fixture(48);
    warm(&mut app, &ctx);
    let original_height = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .content_height;
    app.saved.chats[0].messages.push(Message::new(
        false,
        "Newest reply\n".repeat(20),
        0.0,
        "Demo".into(),
    ));
    draw_chat(&mut app, &ctx, 2.0, vec![]);
    draw_chat(&mut app, &ctx, 2.1, vec![]);
    let view = app.conversation_layout.viewport.as_ref().unwrap();
    assert!(view.content_height > original_height);
    assert!((view.offset + view.rect.height() - view.content_height).abs() < 1.0);
    app.saved.chats[0].messages.remove(0);
    app.saved.chats[0].messages.swap(0, 20);
    draw_chat(&mut app, &ctx, 3.0, vec![]);
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 4.0);
    let original = app.saved.selected;
    let mut chat = Chat::new(app.project().id);
    chat.messages.push(Message::new(
        true,
        "Different chat".into(),
        0.0,
        String::new(),
    ));
    let id = chat.messages[0].id;
    app.saved.selected = chat.id;
    app.saved.chats.push(chat);
    draw_chat(&mut app, &ctx, 5.0, vec![]);
    assert_eq!(
        app.conversation_layout.rows.len(),
        1,
        "only selected-chat measurements are retained"
    );
    assert_eq!(app.conversation_layout.rows[0].id, id);
    app.saved.selected = original;
    draw_chat(&mut app, &ctx, 6.0, vec![]);
    assert_eq!(app.conversation_layout.rows.len(), 48);
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 7.0);
}

#[test]
fn offscreen_events_and_unicode_reveal_remeasure_rows_without_forcing_bottom_scroll() {
    let (mut app, ctx) = fixture(48);
    let id = app.saved.chats[0].messages[0].id;
    app.reveals.insert(
        id,
        (
            Reveal::complete(&app.saved.chats[0].messages[0].text),
            Reveal::default(),
        ),
    );
    warm(&mut app, &ctx);
    let old = app.conversation_layout.rows[0].height;
    let (_, rx) = mpsc::channel();
    let mut active = Active {
        chat: app.saved.selected,
        message: id,
        workspace: PathBuf::from("."),
        rx,
        task: app.runtime.spawn(std::future::pending::<()>()),
        started: Instant::now(),
        steering: None,
    };
    app.apply_event(&mut active, Event::Text("\nStreamed 世界 🌿\n".repeat(20)));
    let text = &app.saved.chats[0].messages[0].text;
    app.reveals
        .get_mut(&id)
        .unwrap()
        .0
        .step(text, 0.0, 0.0, true);
    draw_chat(&mut app, &ctx, 2.0, vec![]);
    assert!(
        app.conversation_layout.rows[0].height > old,
        "dirty hidden rows are measured, not guessed"
    );
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 3.0);
    unstick(&mut app, &ctx, 4.0);
    offset(&app, &ctx, 400.0);
    draw_chat(&mut app, &ctx, 6.0, vec![]);
    let before = app.conversation_layout.viewport.as_ref().unwrap().offset;
    app.saved.chats[0].messages.push(Message::new(
        false,
        "New content below the reader\n".repeat(20),
        0.0,
        "Demo".into(),
    ));
    draw_chat(&mut app, &ctx, 7.0, vec![]);
    assert_eq!(
        app.conversation_layout.viewport.as_ref().unwrap().offset,
        before
    );
    app.apply_event(
        &mut active,
        Event::Error("A provider error with an explanation".into()),
    );
    draw_chat(&mut app, &ctx, 8.0, vec![]);
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 9.0);
    active.task.abort();
}

#[test]
fn skipped_rows_preserve_link_click_ids_and_native_text_selection_and_copy() {
    let (mut app, ctx) = fixture(48);
    app.saved.chats[0].messages.last_mut().unwrap().text =
        "Before [select this link](https://example.com/docs) after".into();
    let output = warm(&mut app, &ctx);
    let label = "Before select this link after";
    let pos = text_position(&output.shapes, label);
    let shape = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
            _ => None,
        })
        .unwrap();
    let glyphs = &shape.galley.rows[0].glyphs;
    let start = shape.pos + vec2(glyphs[7].pos.x + 1.0, shape.galley.rows[0].height() * 0.5);
    let end = shape.pos
        + vec2(
            glyphs[22].max_x() - 1.0,
            shape.galley.rows[0].height() * 0.5,
        );
    let clicked = press_release(&mut app, &ctx, (start + end.to_vec2()) / 2.0, 2.0);
    assert!(clicked.platform_output.commands.iter().any(|cmd| matches!(cmd, egui::OutputCommand::OpenUrl(url) if url.url == "https://example.com/docs")));
    draw_chat(
        &mut app,
        &ctx,
        3.0,
        vec![
            egui::Event::PointerMoved(start),
            pointer_button(start, egui::PointerButton::Primary, true),
        ],
    );
    assert_eq!(
        app.conversation_layout.rendered, 48,
        "selection uses the native full document path"
    );
    draw_chat(&mut app, &ctx, 3.1, vec![egui::Event::PointerMoved(end)]);
    let released = draw_chat(
        &mut app,
        &ctx,
        3.2,
        vec![pointer_button(end, egui::PointerButton::Primary, false)],
    );
    assert!(
        !released
            .platform_output
            .commands
            .iter()
            .any(|cmd| matches!(cmd, egui::OutputCommand::OpenUrl(_)))
    );
    let copied = draw_chat(&mut app, &ctx, 3.3, vec![egui::Event::Copy]);
    assert!(copied.platform_output.commands.iter().any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if text.contains("select this link"))));
    assert!(pos.y > 0.0);
}

#[test]
fn disclosure_heights_and_state_survive_scrolling_out_of_the_viewport() {
    let (mut app, ctx) = fixture(48);
    let mut message = Message::new(false, "After the tool".into(), 0.0, "Demo".into());
    message.activities.push(Activity {
        id: "read".into(),
        name: "read_file".into(),
        arguments: "{\"path\":\"source.rs\"}".into(),
        result: "Read output\n".repeat(12).into(),
        status: ActionStatus::Complete,
        elapsed: 0.0,
        change: None,
        images: vec![],
    });
    message.reasoning = "Visible reasoning\n".repeat(8);
    app.saved.settings.show_reasoning = true;
    app.saved.chats[0].messages.push(message);
    let initial = warm(&mut app, &ctx);
    let old_height = app.conversation_layout.rows.last().unwrap().height;
    let pos = text_position(&initial.shapes, "Read files");
    press_release(&mut app, &ctx, pos, 2.0);
    let open = draw_chat(&mut app, &ctx, 2.5, vec![]);
    assert!(app.conversation_layout.rows.last().unwrap().height > old_height);
    let pos = text_position(&open.shapes, "Read source.rs");
    press_release(&mut app, &ctx, pos, 3.0);
    let expanded = draw_chat(&mut app, &ctx, 3.5, vec![]);
    text_position(
        &expanded.shapes,
        "Read output\nRead output\nRead output\nRead output\nRead output\nRead output\nRead output\nRead output\nRead output\nRead output\nRead output\nRead output\n",
    );
    let expanded_height = app.conversation_layout.rows.last().unwrap().height;
    unstick(&mut app, &ctx, 4.0);
    offset(&app, &ctx, 0.0);
    draw_chat(&mut app, &ctx, 6.0, vec![]);
    assert!(app.conversation_layout.rendered < 20);
    assert_eq!(
        app.conversation_layout.rows.last().unwrap().height,
        expanded_height
    );
    let maximum = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .content_height
        - 820.0;
    offset(&app, &ctx, maximum);
    let returned = draw_chat(&mut app, &ctx, 7.0, vec![]);
    text_position(&returned.shapes, "Read source.rs");
    assert_eq!(
        app.conversation_layout.rows.last().unwrap().height,
        expanded_height
    );
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 8.0);
}

#[test]
fn native_code_copy_and_image_viewer_work_after_skipping_preceding_messages() {
    let (mut app, ctx) = fixture(48);
    let message = app.saved.chats[0].messages.last_mut().unwrap();
    message.text = "```rust\nlet greeting = \"世界 🌿\";\n```".into();
    message.attachments.push(crate::attachments::test_image());
    let output = warm(&mut app, &ctx);
    assert!(app.conversation_layout.rendered < 20);
    let code = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == theme::RAIL => Some(rect.rect),
            _ => None,
        })
        .unwrap();
    let heading = text_position(&output.shapes, "rust");
    let copied = press_release(&mut app, &ctx, pos2(code.right() - 26.0, heading.y), 2.0);
    assert!(
        copied
            .platform_output
            .commands
            .iter()
            .any(|cmd| matches!(cmd,
        egui::OutputCommand::CopyText(text) if text == "let greeting = \"世界 🌿\";"))
    );
    let output = draw_chat(&mut app, &ctx, 3.0, vec![]);
    let card = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.rect.size() == vec2(142.0, 78.0) => Some(rect.rect),
            _ => None,
        })
        .unwrap();
    press_release(&mut app, &ctx, card.center(), 4.0);
    assert!(app.image_preview.is_some());
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 5.0);
}

#[test]
fn focused_composer_in_the_full_app_still_culls_offscreen_messages() {
    let (mut app, ctx) = fixture(96);
    for time in [0.0, 0.5, 1.0, 1.5] {
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            time,
            vec![],
            egui::Modifiers::NONE,
        );
    }
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(Id::new(("composer", app.saved.selected)))
    );
    assert!(app.conversation_layout.rendered < 20);
    let viewport = app.conversation_layout.viewport.as_ref().unwrap();
    assert!((viewport.offset + viewport.rect.height() - viewport.content_height).abs() < 1.0);
}

#[test]
fn keyboard_traversal_keeps_native_focus_across_virtualized_idle_frames() {
    let (mut app, ctx) = fixture(48);
    warm(&mut app, &ctx);
    draw_chat(
        &mut app,
        &ctx,
        2.0,
        vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
    );
    assert!(app.conversation_layout.rendered >= 48);
    let focused = ctx
        .memory(|memory| memory.focused())
        .expect("Tab focuses a native widget");
    draw_chat(&mut app, &ctx, 3.0, vec![]);
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(focused));
    assert!(
        app.conversation_layout.rendered < 20,
        "only the focused row needs pinning"
    );
    app.conversation_layout.force_full = true;
    draw_chat(&mut app, &ctx, 4.0, vec![]);
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(focused));
}

#[test]
fn lists_and_tables_match_full_layout_after_skipping_scrolling_and_resizing() {
    let (mut app, ctx) = fixture(96);
    for (index, message) in app.saved.chats[0].messages.iter_mut().enumerate() {
        message.user = false;
        message.text = format!(
            "Message {index}\n\n- Wrapped **formatted content**, café 🌿, and a [link](https://example.com/docs).\n- A second item\n\n| Replies | Layout |\n| --- | ---: |\n| 256 | 2.652 ms/frame |\n| 1,024 | 10.850 ms/frame |"
        );
    }
    warm(&mut app, &ctx);
    assert!(app.conversation_layout.rendered < 20);
    unstick(&mut app, &ctx, 2.0);
    let height = app
        .conversation_layout
        .viewport
        .as_ref()
        .unwrap()
        .content_height;
    for (index, y) in [0.0, height * 0.5, height - 820.0].into_iter().enumerate() {
        offset(&app, &ctx, y);
        assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 4.0 + index as f64);
    }
    assert_reference(&mut app, &ctx, vec2(320.0, 650.0), 8.0);
}

#[test]
fn disclosure_animation_finishes_even_after_the_message_leaves_the_viewport() {
    let (mut app, ctx) = fixture(48);
    let message = app.saved.chats[0].messages.last_mut().unwrap();
    message.activities.push(Activity {
        id: "read".into(),
        name: "read_file".into(),
        arguments: "{\"path\":\"animated.rs\"}".into(),
        result: "Read output".into(),
        status: ActionStatus::Complete,
        elapsed: 0.0,
        change: None,
        images: vec![],
    });
    app.saved.settings.reduced_motion = false;
    theme::install(&ctx, 14.0, false);
    let output = warm(&mut app, &ctx);
    let closed = app.conversation_layout.rows.last().unwrap().height;
    let header = text_position(&output.shapes, "Read files");
    press_release(&mut app, &ctx, header, 2.0);
    // Scroll away immediately, before the 220ms native disclosure animation ends.
    unstick(&mut app, &ctx, 2.02);
    offset(&app, &ctx, 0.0);
    draw_chat(&mut app, &ctx, 3.1, vec![]);
    assert!(app.conversation_layout.rows.last().unwrap().height > closed);
    assert_reference(&mut app, &ctx, vec2(1180.0, 820.0), 4.0);
}

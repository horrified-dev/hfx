//! Project picker results and removal never change the project files.
use super::*;

fn fixture() -> (Harness, egui::Context) {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    (Harness::new(&cc, Some("welcome".into())), ctx)
}

#[test]
fn native_picker_results_cancel_validate_and_deduplicate_folders() {
    let (mut app, ctx) = fixture();
    let root = tempfile::tempdir().unwrap();
    let initial = app.saved.projects.len();
    let selected = app.saved.selected;
    for result in [None, Some(root.path().join("missing"))] {
        let (tx, rx) = mpsc::channel();
        app.project_picker_rx = Some(rx);
        tx.send(result).unwrap();
        app.poll_project_picker(&ctx);
        assert!(app.project_picker_rx.is_none());
        assert_eq!(app.saved.projects.len(), initial);
        assert_eq!(app.saved.selected, selected);
    }
    for _ in 0..2 {
        let (tx, rx) = mpsc::channel();
        app.project_picker_rx = Some(rx);
        tx.send(Some(root.path().to_path_buf())).unwrap();
        app.poll_project_picker(&ctx);
        assert_eq!(app.saved.projects.len(), initial + 1);
        assert_eq!(
            app.project().path,
            root.path().canonicalize().unwrap().display().to_string()
        );
    }
    let file = root.path().join("not-a-directory");
    std::fs::write(&file, "keep").unwrap();
    assert!(app.add_project_path(file, 0.0).is_err());
    assert_eq!(app.saved.projects.len(), initial + 1);
}

#[test]
fn removing_last_project_keeps_settings_and_disk_files_and_survives_restart() {
    let (mut app, ctx) = fixture();
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("keep.txt");
    std::fs::write(&file, "never delete project files").unwrap();
    app.saved.projects[0].path = root.path().display().to_string();
    app.saved.settings.codex_model = "custom-model".into();
    app.saved.chats[0].queue.push(QueuedMessage::new(
        "discard with confirmation".into(),
        vec![],
    ));
    assert!(app.remove_project_from_app(app.project().id, 0.0));
    assert!(app.saved.projects.is_empty());
    assert!(app.saved.chats[0].project.is_nil());
    assert!(app.saved.chats[0].queue.is_empty());
    assert_eq!(app.saved.settings.codex_model, "custom-model");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "never delete project files"
    );
    let mut restored: Saved =
        serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
    restored.restore();
    assert!(
        restored.projects.is_empty(),
        "do not silently re-add the current directory"
    );
    app.saved = restored;
    app.saved.settings.reduced_motion = true;
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    text_position(&output.shapes, "Add a project to get started");
    app.add_project_path(root.path().into(), 0.2).unwrap();
    assert_eq!(app.saved.projects.len(), 1);
    assert_eq!(app.saved.chats.len(), 1);
    assert!(!app.project().id.is_nil());
}

#[test]
fn removal_refuses_active_project_but_preserves_a_run_in_another_project() {
    let (mut app, ctx) = fixture();
    let first = app.project().id;
    let root = tempfile::tempdir().unwrap();
    app.add_project_path(root.path().into(), 0.0).unwrap();
    let second = app.project().id;
    let live_chat = app.saved.selected;
    let mut reply = Message::new(false, "Live".into(), 0.0, "Demo".into());
    reply.status = Status::Streaming;
    let message = reply.id;
    let index = app.selected_index();
    app.saved.chats[index].messages.push(reply);
    let (tx, rx) = mpsc::channel();
    app.active = Some(Active {
        chat: live_chat,
        message,
        workspace: root.path().into(),
        rx,
        task: app.runtime.spawn(std::future::pending::<()>()),
        started: Instant::now(),
        steering: None,
    });
    assert!(!app.remove_project_from_app(second, 0.2));
    assert!(app.remove_project_from_app(first, 0.4));
    assert_eq!(app.active.as_ref().unwrap().chat, live_chat);
    assert_eq!(app.saved.selected, live_chat);
    tx.send(Event::Text(" continues".into())).unwrap();
    tx.send(Event::Completed).unwrap();
    app.poll(&ctx);
    assert_eq!(app.saved.chats[0].messages[0].text, "Live continues");
    assert_eq!(app.saved.chats[0].messages[0].status, Status::Complete);
}

#[test]
fn project_removal_confirmation_can_cancel_without_changing_state() {
    for action in ["Cancel", "Escape", "Remove project"] {
        let (mut app, ctx) = fixture();
        app.saved.settings.reduced_motion = true;
        app.remove_project = Some(app.project().id);
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let output = draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.2,
            vec![],
            egui::Modifiers::NONE,
        )
        .shapes;
        if action == "Escape" {
            draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                0.4,
                vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            );
        } else {
            let pos = text_position(&output, action);
            for pressed in [true, false] {
                draw(
                    &mut app,
                    &ctx,
                    720.0,
                    540.0,
                    if pressed { 0.4 } else { 0.6 },
                    vec![
                        egui::Event::PointerMoved(pos),
                        pointer_button(pos, egui::PointerButton::Primary, pressed),
                    ],
                    egui::Modifiers::NONE,
                );
            }
        }
        assert!(app.remove_project.is_none());
        assert_eq!(app.saved.projects.is_empty(), action == "Remove project");
    }
}

#[test]
fn project_context_menu_opens_removal_confirmation_at_both_window_sizes() {
    for (width, height) in [(1180.0, 820.0), (720.0, 540.0)] {
        let (mut app, ctx) = fixture();
        app.saved.settings.reduced_motion = true;
        app.saved.projects[0].name = "Removable project".into();
        let project = app.project().id;
        draw(
            &mut app,
            &ctx,
            width,
            height,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let output = draw(
            &mut app,
            &ctx,
            width,
            height,
            0.2,
            vec![],
            egui::Modifiers::NONE,
        );
        let pos = text_position(&output.shapes, "Removable project");
        for (time, pressed) in [(0.4, true), (0.6, false)] {
            draw(
                &mut app,
                &ctx,
                width,
                height,
                time,
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, egui::PointerButton::Secondary, pressed),
                ],
                egui::Modifiers::NONE,
            );
        }
        let output = draw(
            &mut app,
            &ctx,
            width,
            height,
            0.7,
            vec![],
            egui::Modifiers::NONE,
        );
        let pos = text_position(&output.shapes, "Remove project…");
        click(&mut app, &ctx, pos, 0.8);
        assert_eq!(app.remove_project, Some(project));
        assert_eq!(
            app.saved.projects.len(),
            1,
            "opening confirmation is not removal"
        );
    }
}

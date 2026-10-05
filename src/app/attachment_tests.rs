//! Keyboard/file-clipboard routing without touching the user's OS clipboard.
use super::*;

fn fixture(draft: &str) -> (Harness, egui::Context) {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = Harness::new(&cc, Some("welcome".into()));
    app.saved.settings.reduced_motion = true;
    app.saved.chats[0].draft = draft.into();
    draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.0,
        vec![],
        egui::Modifiers::NONE,
    );
    assert!(ctx.memory(|memory| memory.has_focus(Id::new(("composer", app.saved.selected)))));
    (app, ctx)
}

fn finish_loading(app: &mut Harness, ctx: &egui::Context) {
    for _ in 0..100 {
        app.poll_attachments(ctx);
        if app.attachment_jobs.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("attachment loading did not finish");
}

#[test]
fn copied_file_lists_win_over_plain_path_text_and_stay_with_the_original_chat() {
    let (mut app, ctx) = fixture("Keep this draft");
    let root = tempfile::tempdir().unwrap();
    let paths = [
        root.path().join("café notes.txt"),
        root.path().join("code.rs"),
    ];
    std::fs::write(&paths[0], "Copied notes 🌿").unwrap();
    std::fs::write(&paths[1], "fn copied() {}\n").unwrap();
    let mut input = egui::RawInput {
        events: vec![
            egui::Event::Paste(format!("{}\n{}", paths[0].display(), paths[1].display())),
            egui::Event::Text("keep this event".into()),
        ],
        ..Default::default()
    };
    let mut reads = 0;
    app.attachment_input_with_file_paths(&ctx, &mut input, || {
        reads += 1;
        Some(paths.to_vec())
    });
    assert_eq!(reads, 1);
    assert!(
        matches!(input.events.as_slice(), [egui::Event::Text(text)] if text == "keep this event")
    );
    assert_eq!(app.attachment_jobs.len(), 1, "only one load per paste");
    app.new_chat(app.project().id, 0.2);
    finish_loading(&mut app, &ctx);
    assert_eq!(app.saved.chats[0].draft, "Keep this draft");
    assert_eq!(app.saved.chats[0].attachments.len(), 2);
    assert_eq!(app.saved.chats[0].attachments[0].name, "café notes.txt");
    assert!(
        matches!(&app.saved.chats[0].attachments[1].content, AttachmentContent::Text(text) if text.as_ref() == "fn copied() {}\n")
    );
    assert!(app.saved.chats[1].attachments.is_empty());
}

#[test]
fn plain_text_and_plain_paths_without_a_file_list_keep_native_selection_and_undo() {
    for text in ["café 🌿\nnew", "/tmp/not-a-copied-file.txt"] {
        let (mut app, ctx) = fixture("left old right");
        let composer = Id::new(("composer", app.saved.selected));
        let mut state = egui::text_edit::TextEditState::load(&ctx, composer).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(5),
                egui::text::CCursor::new(8),
            )));
        state.store(&ctx, composer);
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(1180.0, 820.0),
            )),
            time: Some(0.2),
            events: vec![egui::Event::Paste(text.into())],
            ..Default::default()
        };
        let mut reads = 0;
        app.attachment_input_with_file_paths(&ctx, &mut input, || {
            reads += 1;
            None
        });
        assert_eq!(reads, 1);
        assert!(
            matches!(input.events.as_slice(), [egui::Event::Paste(remaining)] if remaining == text)
        );
        assert!(app.attachment_jobs.is_empty());
        // Input is already filtered: don't invoke the real OS reader in draw().
        let mut output = ctx.run_ui(input, |ui| {
            let mut frame = eframe::Frame::_new_kittest();
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        output.textures_delta.clear();
        assert_eq!(app.saved.chats[0].draft, format!("left {text} right"));
        assert!(app.active.is_none());
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.4,
            vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
            egui::Modifiers::COMMAND,
        );
        assert_eq!(app.saved.chats[0].draft, "left old right");
    }
}

#[test]
fn uri_pastes_use_the_event_snapshot_without_a_second_clipboard_read() {
    let (mut app, ctx) = fixture("Draft");
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pasted.rs");
    std::fs::write(&path, "fn pasted() {}\n").unwrap();
    let mut input = egui::RawInput {
        events: vec![egui::Event::Paste(
            reqwest::Url::from_file_path(path).unwrap().to_string(),
        )],
        ..Default::default()
    };
    app.attachment_input_with_file_paths(&ctx, &mut input, || {
        panic!("URI snapshot must not reread the clipboard")
    });
    assert!(input.events.is_empty());
    finish_loading(&mut app, &ctx);
    assert_eq!(app.saved.chats[0].attachments.len(), 1);
    assert_eq!(app.saved.chats[0].draft, "Draft");
}

#[test]
fn native_file_detection_requires_a_paste_in_the_unblocked_focused_composer() {
    for blocked in [
        "unfocused",
        "settings",
        "attachment dialog",
        "project dialog",
        "question",
    ] {
        let (mut app, ctx) = fixture("");
        match blocked {
            "unfocused" => ctx.memory_mut(|memory| {
                memory.surrender_focus(Id::new(("composer", app.saved.selected)))
            }),
            "settings" => app.settings_open = true,
            "attachment dialog" => app.attach_modal = true,
            "project dialog" => app.remove_project = Some(app.project().id),
            _ => app.load_preview(&ctx, "question"),
        }
        let mut input = egui::RawInput {
            events: vec![egui::Event::Paste("/tmp/copied.txt".into())],
            ..Default::default()
        };
        app.attachment_input_with_file_paths(&ctx, &mut input, || {
            panic!("must not read the clipboard in {blocked}")
        });
        assert_eq!(input.events.len(), 1);
        assert!(app.attachment_jobs.is_empty());
    }
    let (mut app, ctx) = fixture("");
    let mut input = egui::RawInput {
        events: vec![egui::Event::Text("typing".into())],
        ..Default::default()
    };
    app.attachment_input_with_file_paths(&ctx, &mut input, || {
        panic!("ordinary typing must not read the clipboard")
    });
    assert_eq!(input.events.len(), 1);
    assert!(app.attachment_jobs.is_empty());
}

#[test]
fn copied_files_keep_attachment_limits_and_failures_out_of_the_draft() {
    for count in [1, attachments::MAX_ATTACHMENTS + 1] {
        let (mut app, ctx) = fixture("Keep draft");
        let root = tempfile::tempdir().unwrap();
        let paths = (0..count)
            .map(|index| root.path().join(format!("missing-{index}.txt")))
            .collect::<Vec<_>>();
        let mut input = egui::RawInput {
            events: vec![egui::Event::Paste("file names".into())],
            ..Default::default()
        };
        app.attachment_input_with_file_paths(&ctx, &mut input, || Some(paths.clone()));
        assert!(input.events.is_empty());
        finish_loading(&mut app, &ctx);
        assert_eq!(app.saved.chats[0].draft, "Keep draft");
        assert!(app.saved.chats[0].attachments.is_empty());
        let expected = if count == 1 {
            "Cannot attach"
        } else {
            "Attach up to 8"
        };
        assert!(
            app.toast
                .as_ref()
                .is_some_and(|(text, _)| text.starts_with(expected))
        );
    }
}

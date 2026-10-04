use super::*;

#[test]
fn settling_an_edit_hides_the_badge_but_preserves_and_saves_its_diff() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("actions".into()),
    );
    app.saved.settings.reduced_motion = true;
    let edits = app.pending_edits();
    assert_eq!(edits.len(), 1);
    assert_eq!(changed_totals(&app.saved.chats[0]).0, 1);
    let original_diff = edits[0].change.diff.clone();
    app.settle_edits(edits);
    assert_eq!(changed_totals(&app.saved.chats[0]), (0, 0, 0));
    assert_eq!(
        app.saved.chats[0].messages[1].activities[1]
            .change
            .as_ref()
            .unwrap()
            .diff,
        original_diff
    );
    let restored: Saved = serde_json::from_slice(&serde_json::to_vec(&app.saved).unwrap()).unwrap();
    assert_eq!(changed_totals(&restored.chats[0]), (0, 0, 0));
    assert_eq!(
        restored.chats[0].messages[1].activities[1]
            .change
            .as_ref()
            .unwrap()
            .diff,
        original_diff
    );
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
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "1 file changed")));
    app.changes_open = true;
    for frame in 4..8 {
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
    text_center(&output, "Changes in this chat");
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("src/welcome.rs  +"))));
    // A fresh edit to the same file does not resurrect old line counts.
    app.saved.chats[0].messages[1].activities[1].change =
        Some(tools::file_change("src/welcome.rs", "old\n", "new\n"));
    assert_eq!(changed_totals(&app.saved.chats[0]), (1, 1, 1));
}

#[test]
fn stale_edit_or_project_results_do_not_retire_current_changes() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx),
        Some("actions".into()),
    );
    let old = app.pending_edits();
    app.saved.chats[0].messages[1].activities[1].change =
        Some(tools::file_change("src/welcome.rs", "old\n", "latest\n"));
    app.settle_edits(old);
    assert_eq!(changed_totals(&app.saved.chats[0]), (1, 1, 1));
    let previous_project = app.pending_edits();
    let project = Project::from_path("another-project".into());
    let next = Chat::new(project.id);
    app.saved.selected = next.id;
    app.saved.projects.push(project);
    app.saved.chats.push(next);
    app.settle_edits(previous_project);
    assert_eq!(changed_totals(&app.saved.chats[0]), (1, 1, 1));
}

#[test]
fn legacy_changes_default_to_pending_without_losing_history() {
    let change: crate::state::FileChange = serde_json::from_value(
        serde_json::json!({"path":"code.rs", "added":2, "removed":1, "diff":"saved diff"}),
    )
    .unwrap();
    assert!(!change.settled);
    assert_eq!(change.diff.as_ref(), "saved diff");
}

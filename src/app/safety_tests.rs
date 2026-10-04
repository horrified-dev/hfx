//! Permanent regressions for consent, recovery and the Git composer indicator.
use super::*;

#[test]
#[cfg(unix)]
fn trust_cannot_follow_a_symlink_changed_after_the_consent_dialog_was_presented() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first-project");
    let second = root.path().join("different-project");
    let link = root.path().join("selected-project");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::os::unix::fs::symlink(&first, &link).unwrap();
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("welcome".into()),
    );
    let project = Project::from_path(link.display().to_string());
    app.saved.chats[0].project = project.id;
    app.saved.projects[0] = project;
    app.saved.settings.provider = Provider::Llama;
    app.saved.chats[0].draft = "Never run in a different project".into();
    let chat_id = app.saved.chats[0].id;
    app.send(&ctx);
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
    let presented = first.canonicalize().unwrap();
    text_center(&output, &presented.display().to_string());
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    assert!(
        app.approve_project_trust(&ctx)
            .unwrap_err()
            .contains("changed")
    );
    assert!(!app.project().is_trusted());
    assert!(app.active.is_none());
    assert!(app.saved.chats[0].queue_paused);
    assert_eq!(app.saved.chats[0].queue.len(), 1);
    // Subsequent frames retain the directory actually presented, not the new one.
    let output = draw(
        &mut app,
        &ctx,
        1180.0,
        820.0,
        0.5,
        vec![],
        egui::Modifiers::NONE,
    );
    text_center(&output, &presented.display().to_string());
    text_center(&output, "TRUST NOT GRANTED");
    click(
        &mut app,
        &ctx,
        text_center(&output, "Trust project and continue"),
        0.6,
    );
    assert!(!app.project().is_trusted());
    assert!(app.active.is_none());
    // A new explicit decision can acknowledge B, without resuming the old queue.
    app.trust_request = Some(TrustRequest::for_project(app.project(), None));
    app.approve_project_trust(&ctx).unwrap();
    assert!(app.project().is_trusted());
    assert!(
        app.saved
            .chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .unwrap()
            .queue_paused
    );
}

#[test]
fn unavailable_project_cannot_be_trusted_and_disappearing_projects_do_not_resume() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("welcome".into()),
    );
    let root = tempfile::tempdir().unwrap();
    app.saved.projects[0].path = root.path().join("missing").display().to_string();
    app.trust_request = Some(TrustRequest::for_project(app.project(), None));
    assert!(app.approve_project_trust(&ctx).is_err());
    assert!(!app.project().is_trusted());
    app.saved.projects[0].path = root.path().display().to_string();
    app.trust_request = Some(TrustRequest::for_project(app.project(), None));
    std::fs::remove_dir(root.path()).unwrap();
    assert!(app.approve_project_trust(&ctx).is_err());
    assert!(app.active.is_none());
    assert!(app.saved.projects[0].trusted_path.is_none());
}

#[test]
#[cfg(unix)]
fn dangling_history_links_block_every_save_and_can_be_repaired_without_replacement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("chats.json");
    let target = root.path().join("offline-volume/chats.json");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let (saved, recovery) = crate::recovery::load(Some(&path), Saved::default);
    let recovery = recovery.expect("Existing broken links are not missing history");
    let mut store = persistence::Store::blocked(recovery.error.clone());
    store.queue(&saved);
    assert!(store.flush(&saved).is_err());
    assert!(store.poll().is_none());
    assert!(!store.saved_once);
    assert!(recovery.retry().is_err());
    assert!(recovery.backup().is_err());
    assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&path).unwrap(), target);
    std::fs::create_dir(target.parent().unwrap()).unwrap();
    std::fs::write(&target, serde_json::to_vec(&saved).unwrap()).unwrap();
    assert!(matches!(
        recovery.retry().unwrap(),
        crate::recovery::Outcome::Reloaded(_)
    ));
    assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn busy_recovery_offers_hide_progress_not_a_false_continue_without_saving_promise() {
    for success in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        std::fs::write(&path, "{broken").unwrap();
        let ctx = egui::Context::default();
        let mut app = Harness::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            Some("recovery".into()),
        );
        app.recovery.as_mut().unwrap().path = path.clone();
        let outcome = app.recovery.as_ref().unwrap().backup().unwrap();
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
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Continue without saving")));
        click(
            &mut app,
            &ctx,
            text_center(&output, "Hide recovery progress"),
            0.2,
        );
        assert!(!app.recovery_open);
        assert!(app.store.flush(&app.saved).is_err());
        tx.send(if success {
            Ok(outcome)
        } else {
            Err("Fixture recovery failed".into())
        })
        .unwrap();
        app.poll_recovery(&ctx);
        assert_eq!(app.store.flush(&app.saved).is_ok(), success);
        assert_eq!(app.recovery_open, !success);
        if !success {
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        }
    }
}

#[test]
fn idle_continue_without_saving_keeps_the_history_writer_blocked() {
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("recovery".into()),
    );
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
        text_center(&output, "Continue without saving"),
        0.2,
    );
    app.poll_recovery(&ctx);
    assert!(!app.recovery_open);
    assert!(app.recovery.is_some());
    assert!(app.store.flush(&app.saved).is_err());
}

#[test]
fn git_composer_indicator_fits_small_windows_and_preserves_the_draft() {
    use crate::git_status::{Probe, Status};
    for (width, height) in [(720.0, 540.0), (1180.0, 820.0)] {
        for status in [
            Status::NotRepository,
            Status::Branch {
                name: "feature/改善".into(),
                bare: false,
            },
            Status::Branch {
                name: "feature/very-long-branch-segment/".repeat(12),
                bare: false,
            },
            Status::Detached {
                commit: "a1b2c3d".into(),
                bare: false,
            },
            Status::Branch {
                name: "main".into(),
                bare: true,
            },
            Status::Unavailable("Git executable is missing".into()),
        ] {
            let ctx = egui::Context::default();
            let mut app = Harness::new(
                &eframe::CreationContext::_new_kittest(ctx.clone()),
                Some("welcome".into()),
            );
            app.git_probe = Probe::scripted(status.clone());
            app.saved.projects[0].name = "long-project-name-".repeat(15);
            app.saved.chats[0].draft = "Keep this unsent draft".into();
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
            let shape = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == status.label())).expect("Git indicator");
            let egui::Shape::Text(text) = &shape.shape else {
                unreachable!()
            };
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height));
            assert!(
                screen.contains_rect(rect),
                "Git indicator escaped the window: {rect:?}"
            );
            assert!(
                shape.clip_rect.contains_rect(rect),
                "Git indicator is clipped: {rect:?}"
            );
            assert_eq!(app.saved.chats[0].draft, "Keep this unsent draft");
        }
    }
}

use super::*;

#[test]
fn automatic_polling_clears_the_badge_after_a_real_commit_and_push() {
    fn git(root: &std::path::Path, args: &[&str]) {
        let hooks = format!("core.hooksPath={}", root.join("disabled-hooks").display());
        let result = std::process::Command::new("git")
            .args([
                "-c",
                &hooks,
                "-c",
                "core.fsmonitor=false",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@local.invalid",
                "-C",
            ])
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let root = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    git(remote.path(), &["init", "--bare"]);
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/welcome.rs"), "new\n").unwrap();
    std::fs::write(root.path().join(".gitignore"), ".hfx/\n").unwrap();
    std::fs::create_dir(root.path().join(".hfx")).unwrap();
    std::fs::write(root.path().join(".hfx/qa.txt"), "ignored QA notes\n").unwrap();
    let ctx = egui::Context::default();
    let mut app = Harness::new(
        &eframe::CreationContext::_new_kittest(ctx.clone()),
        Some("actions".into()),
    );
    app.saved.projects[0].path = root.path().display().to_string();
    let mut qa = app.saved.chats[0].messages[1].activities[1].clone();
    qa.id = "qa-artifact".into();
    qa.change = Some(tools::file_change(".hfx/qa.txt", "", "ignored QA notes\n"));
    app.saved.chats[0].messages[1].activities.push(qa);
    app.edit_probe = Default::default();
    app.poll_pending_edits(&ctx, root.path().to_owned());
    assert_eq!(changed_totals(&app.saved.chats[0]).0, 2);
    git(root.path(), &["add", "."]);
    git(
        root.path(),
        &[
            "commit",
            "-m",
            "fixture",
            "-m",
            "Co-authored-by: hfx <hfx@local.invalid>",
        ],
    );
    git(
        root.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(root.path(), &["push", "-u", "origin", "main"]);
    let deadline = Instant::now() + Duration::from_secs(6);
    while changed_totals(&app.saved.chats[0]).0 != 0 {
        app.poll_pending_edits(&ctx, root.path().to_owned());
        assert!(
            Instant::now() < deadline,
            "Committed edits did not leave the badge: {:?}",
            app.edit_probe.error
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        app.saved.chats[0].messages[1].activities[1]
            .change
            .is_some()
    );
}

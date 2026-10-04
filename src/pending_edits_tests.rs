use super::*;

fn run(root: &Path, args: &[&str]) {
    let hooks = format!("core.hooksPath={}", root.join("disabled-hooks").display());
    let output = std::process::Command::new("git")
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
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn commit(root: &Path) {
    run(
        root,
        &[
            "commit",
            "-m",
            "fixture",
            "-m",
            "Co-authored-by: hfx <hfx@local.invalid>",
        ],
    );
}
fn edit(path: &str) -> Edit {
    Edit {
        chat: Uuid::new_v4(),
        message: Uuid::new_v4(),
        activity: 0,
        change: crate::tools::file_change(path, "old\n", "new\n"),
    }
}
fn repo() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    run(root.path(), &["init", "-b", "main"]);
    root
}

#[tokio::test]
async fn commit_and_push_settle_only_committed_paths_not_other_dirty_files() {
    let root = repo();
    std::fs::write(root.path().join("code.rs"), "old\n").unwrap();
    std::fs::write(root.path().join("other.rs"), "old\n").unwrap();
    run(root.path(), &["add", "."]);
    commit(root.path());
    std::fs::write(root.path().join("code.rs"), "new\n").unwrap();
    std::fs::write(root.path().join("other.rs"), "new\n").unwrap();
    let input = vec![edit("./code.rs"), edit("other.rs")];
    assert!(inspect(root.path(), &input).await.unwrap().is_empty());
    run(root.path(), &["add", "code.rs"]);
    assert!(
        inspect(root.path(), &input).await.unwrap().is_empty(),
        "Staged is not committed"
    );
    commit(root.path());
    let remote = tempfile::tempdir().unwrap();
    run(remote.path(), &["init", "--bare"]);
    run(
        root.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    run(root.path(), &["push", "-u", "origin", "main"]);
    assert_eq!(
        inspect(root.path(), &input).await.unwrap(),
        vec![input[0].clone()]
    );
    let mut updated = input[0].clone();
    updated.change = crate::tools::file_change("code.rs", "new\n", "another edit\n");
    std::fs::write(root.path().join("code.rs"), "another edit\n").unwrap();
    assert!(
        inspect(root.path(), &[updated.clone()])
            .await
            .unwrap()
            .is_empty()
    );
    run(root.path(), &["restore", "code.rs"]);
    assert_eq!(
        inspect(root.path(), &[updated.clone()]).await.unwrap(),
        vec![updated]
    );
}

#[tokio::test]
async fn new_files_deletions_ignored_files_and_non_repositories_are_conservative() {
    let root = repo();
    let input = vec![edit("new file.rs")];
    std::fs::write(root.path().join("new file.rs"), "new\n").unwrap();
    assert!(inspect(root.path(), &input).await.unwrap().is_empty());
    run(root.path(), &["add", "new file.rs"]);
    assert!(inspect(root.path(), &input).await.unwrap().is_empty());
    commit(root.path());
    assert_eq!(inspect(root.path(), &input).await.unwrap(), input);
    std::fs::remove_file(root.path().join("new file.rs")).unwrap();
    assert!(inspect(root.path(), &input).await.unwrap().is_empty());
    run(root.path(), &["add", "-u"]);
    commit(root.path());
    assert_eq!(inspect(root.path(), &input).await.unwrap(), input);
    std::fs::write(root.path().join(".gitignore"), "ignored.rs\n").unwrap();
    std::fs::write(root.path().join("ignored.rs"), "not committed\n").unwrap();
    let ignored = edit("ignored.rs");
    assert_eq!(
        inspect(root.path(), std::slice::from_ref(&ignored))
            .await
            .unwrap(),
        vec![ignored]
    );
    let ordinary = tempfile::tempdir().unwrap();
    assert!(inspect(ordinary.path(), &input).await.unwrap().is_empty());
    assert!(
        inspect(root.path(), &[edit("../outside.rs"), edit("/outside.rs")])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn nested_project_and_literal_unicode_filenames_are_matched_correctly() {
    let root = repo();
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let name = "code with space-α.rs";
    std::fs::write(nested.join(name), "old\n").unwrap();
    run(root.path(), &["add", "."]);
    commit(root.path());
    let input = vec![edit(name)];
    assert_eq!(inspect(&nested, &input).await.unwrap(), input);
    std::fs::write(nested.join(name), "new\n").unwrap();
    assert!(inspect(&nested, &input).await.unwrap().is_empty());
    #[cfg(unix)]
    {
        let literal = ":(glob)*.rs";
        std::fs::write(root.path().join(literal), "new\n").unwrap();
        assert!(
            inspect(root.path(), &[edit(literal)])
                .await
                .unwrap()
                .is_empty()
        );
        run(root.path(), &["--literal-pathspecs", "add", literal]);
        commit(root.path());
        assert_eq!(
            inspect(root.path(), &[edit(literal)]).await.unwrap().len(),
            1
        );
    }
}

#[tokio::test]
#[cfg(unix)]
async fn symlink_edits_follow_the_edited_target_not_the_unchanged_link() {
    let root = repo();
    std::fs::write(root.path().join("code.rs"), "old\n").unwrap();
    std::os::unix::fs::symlink("code.rs", root.path().join("link.rs")).unwrap();
    run(root.path(), &["add", "."]);
    commit(root.path());
    crate::file_edit::edit(
        root.path(),
        "link.rs",
        &serde_json::json!({"old_text":"old", "new_text":"new", "expected_sha256":null}),
    )
    .unwrap();
    let input = vec![edit("link.rs")];
    assert!(inspect(root.path(), &input).await.unwrap().is_empty());
    run(root.path(), &["add", "code.rs"]);
    commit(root.path());
    assert_eq!(inspect(root.path(), &input).await.unwrap(), input);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("code.rs"), "outside\n").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("code.rs"),
        root.path().join("escape.rs"),
    )
    .unwrap();
    std::os::unix::fs::symlink("missing.rs", root.path().join("dangling.rs")).unwrap();
    assert!(
        inspect(root.path(), &[edit("escape.rs"), edit("dangling.rs")])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn configured_filters_external_diffs_and_fsmonitor_are_never_executed() {
    use std::os::unix::fs::PermissionsExt;
    let root = repo();
    for file in ["filtered.rs", "ordinary.rs"] {
        std::fs::write(root.path().join(file), "old\n").unwrap();
    }
    run(root.path(), &["add", "."]);
    commit(root.path());
    let script = root.path().join("unexpected.sh");
    let marker = root.path().join("executed");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    for key in [
        "filter.danger.clean",
        "filter.danger.smudge",
        "filter.danger.process",
        "diff.danger.command",
        "diff.danger.textconv",
        "core.fsmonitor",
    ] {
        run(root.path(), &["config", key, script.to_str().unwrap()]);
    }
    run(root.path(), &["config", "filter.danger.required", "true"]);
    std::fs::write(
        root.path().join(".gitattributes"),
        "filtered.rs filter=danger\n*.rs diff=danger\n",
    )
    .unwrap();
    std::fs::write(root.path().join("filtered.rs"), "new\n").unwrap();
    std::fs::write(root.path().join("ordinary.rs"), "new\n").unwrap();
    assert!(
        inspect(root.path(), &[edit("filtered.rs"), edit("ordinary.rs")])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!marker.exists());
    std::fs::write(root.path().join("ordinary.rs"), "old\n").unwrap();
    assert_eq!(
        inspect(root.path(), &[edit("filtered.rs"), edit("ordinary.rs")])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(!marker.exists());
}

#[test]
fn stale_results_cannot_settle_new_edits_or_a_different_project() {
    let runtime = crate::lifecycle::TaskRuntime::new();
    let ctx = egui::Context::default();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for switch in [false, true] {
        let mut probe = Probe::default();
        let old = edit("code.rs");
        let (send, receive) = mpsc::channel();
        probe.root = Some(first.path().to_owned());
        probe.edits = vec![old.clone()];
        probe.receive = Some(receive);
        send.send(Ok(vec![old.clone()])).unwrap();
        let mut new = old;
        if !switch {
            new.change = crate::tools::file_change("code.rs", "new\n", "latest\n");
        }
        let root = if switch { second.path() } else { first.path() };
        assert!(
            probe
                .update(&ctx, &runtime, root.to_owned(), vec![new])
                .is_empty()
        );
        assert!(probe.task.is_some());
    }
}

#[tokio::test]
#[cfg(unix)]
async fn blocked_configuration_times_out_without_acknowledging_edits() {
    use std::os::unix::ffi::OsStrExt;
    let root = repo();
    let config = root.path().join(".git/config");
    std::fs::remove_file(&config).unwrap();
    let path = std::ffi::CString::new(config.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(
        inspect(root.path(), &[edit("code.rs")])
            .await
            .unwrap_err()
            .contains("timed out")
    );
}

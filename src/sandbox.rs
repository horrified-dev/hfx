use std::path::Path;

/// Trusted mode is an explicit user preference, never a fallback after a
/// sandbox failure. Keep the user's Git/SSH/session environment intact.
pub fn host_command(root: &Path, script: &str) -> Result<tokio::process::Command, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot open workspace: {e}"))?;
    #[cfg(not(target_os = "windows"))]
    let mut process = {
        let mut p = tokio::process::Command::new("/bin/sh");
        p.args(["-c", script]);
        p
    };
    #[cfg(target_os = "windows")]
    let mut process = {
        let mut p = tokio::process::Command::new("cmd.exe");
        p.args(["/D", "/S", "/C", script]);
        p
    };
    process.current_dir(root);
    // GUI launchers may omit ~/.cargo/bin even when Rust is installed there.
    // Do not replace a deliberately configured Cargo home or toolchain.
    #[cfg(not(target_os = "windows"))]
    if let Some(bin) = rust_bin() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut entries: Vec<_> = std::env::split_paths(&path).collect();
        if !entries.contains(&bin) {
            entries.insert(0, bin);
            if let Ok(path) = std::env::join_paths(entries) {
                process.env("PATH", path);
            }
        }
    }
    Ok(process)
}

#[cfg(not(target_os = "windows"))]
fn rust_bin() -> Option<std::path::PathBuf> {
    // Preserve an existing PATH choice; only add the normal Cargo installation
    // when a GUI launcher omitted it.
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path));
    }
    if let Some(cargo) = std::env::var_os("CARGO_HOME") {
        candidates.push(std::path::PathBuf::from(cargo).join("bin"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::PathBuf::from(home).join(".cargo/bin"));
    }
    candidates
        .into_iter()
        .find(|bin| bin.join("cargo").is_file() && bin.join("rustc").is_file())
}

/// Commands see the project and read-only OS/toolchain runtime, never the
/// user's other projects, login caches, session sockets or host temporary files.
#[cfg(target_os = "linux")]
pub fn command(root: &Path, script: &str) -> Result<tokio::process::Command, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot open workspace: {e}"))?;
    let root = root.as_path();
    let bwrap = ["/usr/bin/bwrap", "/bin/bwrap"].into_iter().find(|p| Path::new(p).is_file())
        .ok_or("Workspace commands need bubblewrap (bwrap). Install it to enable confined execution; commands never fall back to unrestricted access.")?;
    let cargo = cache_directory(root, ".hfx/cache/cargo")?;
    let mut process = tokio::process::Command::new(bwrap);
    process.env_clear().args([
        "--unshare-all",
        "--share-net",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
    ]);
    for path in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/sys"] {
        if Path::new(path).exists() {
            process.args(["--ro-bind", path, path]);
        }
    }
    process.args([
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/tmp/hfx-home",
    ]);
    process
        .arg("--bind")
        .arg(root)
        .arg(root)
        .arg("--bind")
        .arg(root)
        .arg("/workspace")
        .arg("--chdir")
        .arg("/workspace");
    let mut path = String::from("/usr/local/bin:/usr/bin:/bin");
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let rustup = std::env::var_os("RUSTUP_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".rustup"));
        if rustup.is_dir() {
            process.arg("--ro-bind").arg(&rustup).arg(&rustup);
            process.env("RUSTUP_HOME", &rustup);
        }
        let host_cargo = std::env::var_os("CARGO_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".cargo"));
        if let Some(bin) = rust_bin() {
            process.arg("--ro-bind").arg(&bin).arg(&bin);
            path = format!("{}:{path}", bin.display());
        }
        // Reuse cached crates without granting write access to the host cache.
        // Overlay upper/work directories and new downloads stay in the project.
        for name in ["registry", "git"] {
            let lower = host_cargo.join(name);
            // Nested hfx/tests may already use this exact workspace cache.
            // Reusing the same overlay upper/work mount causes EBUSY.
            if lower.is_dir() && same_directory(&host_cargo, &cargo) {
                // A parent command's active cache mounts are not necessarily
                // included by a workspace bind. Bind their merged view explicitly
                // instead of reusing the already-mounted overlay work directory.
                let destination =
                    crate::tools::workspace_path(root, &format!(".hfx/cache/cargo/{name}"), false)?;
                process.arg("--bind").arg(&lower).arg(destination);
            } else if lower.is_dir() {
                let upper = cache_directory(root, &format!(".hfx/cache/{name}-upper"))?;
                let work = cache_directory(root, &format!(".hfx/cache/{name}-work"))?;
                let destination =
                    crate::tools::workspace_path(root, &format!(".hfx/cache/cargo/{name}"), true)?;
                process
                    .arg("--overlay-src")
                    .arg(lower)
                    .arg("--overlay")
                    .arg(upper)
                    .arg(work)
                    .arg(destination);
            }
        }
    }
    process
        .env("HOME", "/tmp/hfx-home")
        .env("XDG_CACHE_HOME", "/tmp/hfx-home/.cache")
        .env("TMPDIR", "/tmp")
        .env("CARGO_HOME", &cargo)
        .env("PATH", path)
        .env("LANG", "C.UTF-8")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["--remount-ro", "/"])
        .args(["--", "/bin/sh", "-c", script]);
    Ok(process)
}

#[cfg(target_os = "linux")]
fn same_directory(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (left.metadata(), right.metadata()) {
        (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn cache_directory(root: &Path, relative: &str) -> Result<std::path::PathBuf, String> {
    let path = crate::tools::workspace_path(root, relative, true)?;
    std::fs::create_dir_all(&path).map_err(|e| format!("Cannot create workspace cache: {e}"))?;
    // Validate each mount source rather than assuming nested cache paths cannot
    // contain links. A workspace link must never mount a host directory writable.
    crate::tools::workspace_path(root, relative, false)
}

#[cfg(not(target_os = "linux"))]
pub fn command(_: &Path, _: &str) -> Result<tokio::process::Command, String> {
    Err("Confined shell execution currently requires Linux and bubblewrap. File tools remain available; unrestricted commands are never used as a fallback.".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn trusted_git_push_uses_fixture_host_identity_and_ssh_setup_without_rewriting_author() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        let home = fixture.path().join("home");
        let remote = fixture.path().join("remote.git");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&home).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            "[user]\nname = Human Fixture\nemail = human@fixture.invalid\n",
        )
        .unwrap();
        let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        let ssh = fixture.path().join("fixture-ssh");
        std::fs::write(&ssh, format!("#!/bin/sh\nset -e\ntest \"$SSH_AUTH_SOCK\" = fixture-agent\ntest \"$(git config --global --get user.name)\" = 'Human Fixture'\ncase \"$*\" in *git-receive-pack*) exec git-receive-pack {} ;; *) exit 1 ;; esac\n", quote(&remote))).unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.join("README.txt"), "Fixture contribution\n").unwrap();
        let script = format!(
            "set -e; git init -q --bare {}; git init -q -b main; git config core.sshCommand {}; git remote add origin ssh://fixture.invalid/repository; git add README.txt; git commit -q -m 'Fixture contribution\n\nCo-authored-by: hfx <hfx@local.invalid>'; git log -1 --format=%B > fixture-commit-message.txt; test \"$(git interpret-trailers --parse < fixture-commit-message.txt | grep -Fxc 'Co-authored-by: hfx <hfx@local.invalid>')\" = 1; git push -q origin main; git log -1 --format='%an|%ae'; git --git-dir={} rev-parse main",
            quote(&remote),
            quote(&ssh),
            quote(&remote)
        );
        let mut process = host_command(&root, &script).unwrap();
        process
            .env("HOME", &home)
            .env("SSH_AUTH_SOCK", "fixture-agent")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for name in [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "GIT_CONFIG_GLOBAL",
        ] {
            process.env_remove(name);
        }
        let output = process.output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Human Fixture|human@fixture.invalid")
        );
        // Even pointing HOME at this fixture cannot expose it through the
        // strict mount boundary; no real user's config/credentials are involved.
        let output = command(&root, "git config --global --get user.name")
            .unwrap()
            .env("HOME", &home)
            .output()
            .await
            .unwrap();
        assert!(!output.status.success());
    }

    #[tokio::test]
    #[ignore = "checks the real project build inside the command sandbox"]
    async fn workspace_cargo_build_uses_cached_crates_inside_the_sandbox() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let quoted = root.to_string_lossy().replace('\'', "'\\''");
        let output = command(
            root,
            &format!("cd '{quoted}' && cargo check --offline --locked"),
        )
        .unwrap()
        .output()
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn cache_symlinks_cannot_mount_host_directories_writable() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir_all(root.join(".hfx/cache")).unwrap();
        std::os::unix::fs::symlink(parent.path(), root.join(".hfx/cache/cargo")).unwrap();
        assert!(command(&root, "true").is_err());
    }
    #[tokio::test]
    async fn commands_can_change_workspace_but_cannot_escape_or_read_other_projects() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let outside = parent.path().join("outside.txt");
        std::fs::write(&outside, "outside-original").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let output = command(&root, "set -e; printf inside > result.txt; if cat ../outside.txt; then exit 21; fi; if printf bad > ../outside.txt; then exit 22; fi; if cat escape; then exit 23; fi; printf bad > escape 2>/dev/null || true; printf temporary > /tmp/private-scratch; printf sandbox-complete").unwrap().output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(root.join("result.txt")).unwrap(),
            "inside"
        );
        assert_eq!(
            std::fs::read_to_string(outside).unwrap(),
            "outside-original"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("sandbox-complete"));
        assert!(!std::path::Path::new("/tmp/private-scratch").exists());
    }
    #[tokio::test]
    async fn rust_tools_work_without_exposing_provider_environment() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("hello.rs"),
            "fn main() { println!(\"confined-rust\"); }\n",
        )
        .unwrap();
        let output = command(root.path(), "set -e; test -z \"${OPENAI_API_KEY:-}${OPENROUTER_API_KEY:-}${LLAMA_API_KEY:-}\"; rustc hello.rs -o hello; ./hello").unwrap().output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "confined-rust"
        );
    }
}

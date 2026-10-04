//! Bounded, read-only Git metadata discovery; never run a shell, hooks or fetch.
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc,
    time::{Duration, Instant},
};

const REFRESH: Duration = Duration::from_secs(3);
const TIMEOUT: Duration = Duration::from_secs(2);
const OUTPUT_LIMIT: usize = 64 * 1024;

async fn read_output(stream: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    stream
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err("Git metadata output exceeded 64 KiB".into());
    }
    Ok(bytes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Checking,
    NotRepository,
    Branch { name: String, bare: bool },
    Detached { commit: String, bare: bool },
    Unavailable(String),
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Self::Checking => "Checking Git…".into(),
            Self::NotRepository => "Not a Git repo".into(),
            Self::Branch { name, bare } => {
                format!("{} · {name}", if *bare { "Git (bare)" } else { "Git" })
            }
            Self::Detached { commit, bare } => format!(
                "{} · detached @ {commit}",
                if *bare { "Git (bare)" } else { "Git" }
            ),
            Self::Unavailable(_) => "Git unavailable".into(),
        }
    }
    pub fn detail(&self) -> String {
        match self {
            Self::Branch { name, bare } => format!(
                "{}Git repository · branch: {name}\nRead-only metadata, refreshed every 3 seconds.",
                if *bare { "Bare " } else { "" }
            ),
            Self::Detached { commit, .. } => format!(
                "Git repository · detached HEAD at {commit}\nRead-only metadata, refreshed every 3 seconds."
            ),
            Self::Unavailable(error) => format!("Git status could not be checked: {error}"),
            Self::Checking => "Checking the selected project's local Git metadata…".into(),
            Self::NotRepository => "The selected directory is not inside a Git repository.".into(),
        }
    }
}

pub(crate) async fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    git_with_config(root, args, &[]).await
}

pub(crate) async fn git_with_config(
    root: &Path,
    args: &[&str],
    overrides: &[String],
) -> Result<std::process::Output, String> {
    let mut command = tokio::process::Command::new("git");
    command.args([
        "--no-pager",
        "--no-optional-locks",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
    ]);
    for config in overrides {
        command.arg("-c").arg(config);
    }
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // Git's repository-selector variables may be inherited from a launcher
        // or Git hook. They must not redirect this selected-directory probe.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        // Parse only Git's fixed diagnostics, independently of the desktop locale.
        .env("LC_ALL", "C");
    let mut child = command.spawn().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "Git is not installed or is not on PATH".into(),
        _ => error.to_string(),
    })?;
    let stdout = child.stdout.take().ok_or("Cannot capture Git stdout")?;
    let stderr = child.stderr.take().ok_or("Cannot capture Git stderr")?;
    // Drain both pipes concurrently and bound memory even for broken metadata.
    // Dropping the child on timeout/overflow requests termination, not a leaked job.
    let (status, stdout, stderr) = tokio::try_join!(
        async { child.wait().await.map_err(|error| error.to_string()) },
        read_output(stdout),
        read_output(stderr),
    )?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect()
}

async fn inspect_inner(root: &Path) -> Result<Status, String> {
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| format!("Project is unavailable: {error}"))?;
    let repository = git(
        &root,
        &["rev-parse", "--is-inside-work-tree", "--is-bare-repository"],
    )
    .await?;
    if !repository.status.success() {
        let error = text(&repository.stderr);
        return if error.starts_with("fatal: not a git repository (") {
            Ok(Status::NotRepository)
        } else {
            Err(error)
        };
    }
    let flags = String::from_utf8_lossy(&repository.stdout);
    let bare = flags.lines().nth(1) == Some("true");
    let head = git(&root, &["symbolic-ref", "--quiet", "HEAD"]).await?;
    if head.status.success() {
        let reference =
            String::from_utf8(head.stdout).map_err(|_| "Git branch name is not valid UTF-8")?;
        // --short disambiguates against tags and may return "heads/main";
        // strip the full local-branch prefix to preserve the actual branch name.
        let name = reference
            .trim_end_matches(['\r', '\n'])
            .strip_prefix("refs/heads/")
            .ok_or("HEAD does not point to a local branch")?
            .to_owned();
        if name.is_empty() {
            return Err("Git returned an empty branch name".into());
        }
        // symbolic-ref also handles an initial branch before its first commit.
        return Ok(Status::Branch { name, bare });
    }
    if head.status.code() != Some(1) {
        return Err(text(&head.stderr));
    }
    let commit = git(&root, &["rev-parse", "--verify", "--short", "HEAD"]).await?;
    if !commit.status.success() {
        return Err(text(&commit.stderr));
    }
    Ok(Status::Detached {
        commit: text(&commit.stdout),
        bare,
    })
}

pub async fn inspect(root: &Path) -> Status {
    match tokio::time::timeout(TIMEOUT, inspect_inner(root)).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => Status::Unavailable(error),
        Err(_) => Status::Unavailable("Git metadata check timed out".into()),
    }
}

pub struct Probe {
    root: Option<PathBuf>,
    status: Status,
    receive: Option<mpsc::Receiver<Status>>,
    task: Option<tokio::task::JoinHandle<()>>,
    next: Instant,
    scripted: bool,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            root: None,
            status: Status::Checking,
            receive: None,
            task: None,
            next: Instant::now(),
            scripted: false,
        }
    }
}

impl Probe {
    pub fn scripted(status: Status) -> Self {
        Self {
            root: None,
            status,
            receive: None,
            task: None,
            next: Instant::now(),
            scripted: true,
        }
    }
    pub fn status_for(&self, root: &Path) -> &Status {
        if !self.scripted && self.root.as_deref() != Some(root) {
            &Status::Checking
        } else {
            &self.status
        }
    }
    #[cfg(test)]
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        runtime: &tokio::runtime::Runtime,
        root: PathBuf,
    ) {
        if self.scripted {
            return;
        }
        if self.root.as_ref() != Some(&root) {
            self.cancel();
            self.root = Some(root.clone());
            self.status = Status::Checking;
            self.next = Instant::now();
        }
        if let Some(receive) = &self.receive {
            match receive.try_recv() {
                Ok(status) => {
                    self.status = status;
                    self.receive = None;
                    self.task = None;
                    self.next = Instant::now() + REFRESH;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = Status::Unavailable("Git metadata task stopped".into());
                    self.receive = None;
                    self.task = None;
                    self.next = Instant::now() + REFRESH;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.receive.is_none() && Instant::now() >= self.next {
            let (send, receive) = mpsc::channel();
            self.receive = Some(receive);
            let repaint = ctx.clone();
            self.task = Some(runtime.spawn(async move {
                let status = inspect(&root).await;
                let _ = send.send(status);
                repaint.request_repaint();
            }));
        }
        ctx.request_repaint_after(if self.receive.is_some() {
            Duration::from_millis(100)
        } else {
            self.next.saturating_duration_since(Instant::now())
        });
    }
    fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.receive = None;
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metadata_output_is_bounded_without_truncating_valid_results() {
        let bytes = vec![b'x'; OUTPUT_LIMIT];
        assert_eq!(read_output(bytes.as_slice()).await.unwrap(), bytes);
        let oversized = vec![b'x'; OUTPUT_LIMIT + 1];
        assert!(
            read_output(oversized.as_slice())
                .await
                .unwrap_err()
                .contains("exceeded")
        );
    }

    fn run(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .arg("-C")
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
    #[tokio::test]
    async fn detects_non_repositories_unborn_branches_and_live_branch_changes() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(inspect(root.path()).await, Status::NotRepository);
        run(root.path(), &["init", "-b", "main"]);
        assert_eq!(
            inspect(root.path()).await,
            Status::Branch {
                name: "main".into(),
                bare: false
            }
        );
        run(
            root.path(),
            &["symbolic-ref", "HEAD", "refs/heads/feature/改善"],
        );
        assert_eq!(
            inspect(root.path()).await,
            Status::Branch {
                name: "feature/改善".into(),
                bare: false
            }
        );
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert_eq!(inspect(&nested).await, inspect(root.path()).await);
        let long_name = format!("feature/{}", vec!["segment-".repeat(24); 4].join("/"));
        assert!(long_name.len() > 512);
        run(
            root.path(),
            &["symbolic-ref", "HEAD", &format!("refs/heads/{long_name}")],
        );
        assert_eq!(
            inspect(root.path()).await,
            Status::Branch {
                name: long_name,
                bare: false
            }
        );
    }
    #[tokio::test]
    async fn handles_detached_heads_linked_worktrees_bare_repos_and_errors() {
        let root = tempfile::tempdir().unwrap();
        run(root.path(), &["init", "-b", "main"]);
        run(
            root.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@local.invalid",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
                "-m",
                "Co-authored-by: hfx <hfx@local.invalid>",
            ],
        );
        // A same-named tag must not change the displayed local branch name.
        run(root.path(), &["update-ref", "refs/tags/main", "HEAD"]);
        assert_eq!(
            inspect(root.path()).await,
            Status::Branch {
                name: "main".into(),
                bare: false
            }
        );
        let worktree = root.path().join("linked-worktree");
        run(
            root.path(),
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                worktree.to_str().unwrap(),
            ],
        );
        assert_eq!(
            inspect(&worktree).await,
            Status::Branch {
                name: "linked".into(),
                bare: false
            }
        );
        run(root.path(), &["checkout", "--detach"]);
        assert!(
            matches!(inspect(root.path()).await, Status::Detached { commit, bare: false } if !commit.is_empty())
        );
        let bare = tempfile::tempdir().unwrap();
        run(bare.path(), &["init", "--bare", "-b", "main"]);
        assert_eq!(
            inspect(bare.path()).await,
            Status::Branch {
                name: "main".into(),
                bare: true
            }
        );
        assert!(matches!(
            inspect(&root.path().join("missing")).await,
            Status::Unavailable(_)
        ));
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join(".git"), "not a valid gitdir file").unwrap();
        assert!(matches!(
            inspect(broken.path()).await,
            Status::Unavailable(_)
        ));
        std::fs::write(
            broken.path().join(".git"),
            format!(
                "gitdir: {}\n",
                broken.path().join("missing-repository").display()
            ),
        )
        .unwrap();
        assert!(matches!(
            inspect(broken.path()).await,
            Status::Unavailable(_)
        ));
    }
    #[test]
    fn switching_projects_discards_old_results_without_blocking_ui() {
        let runtime = crate::lifecycle::TaskRuntime::new();
        let ctx = egui::Context::default();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut probe = Probe::default();
        let (tx, rx) = mpsc::channel();
        probe.root = Some(first.path().to_owned());
        probe.receive = Some(rx);
        tx.send(Status::Branch {
            name: "stale-branch".into(),
            bare: false,
        })
        .unwrap();
        probe.update(&ctx, &runtime, second.path().to_owned());
        assert_eq!(probe.status(), &Status::Checking);
        assert_eq!(probe.root.as_deref(), Some(second.path()));
        assert!(probe.task.is_some());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn metadata_queries_never_execute_repository_hooks_pagers_or_fsmonitor() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        run(root.path(), &["init", "-b", "main"]);
        let script = root.path().join("unexpected-execution.sh");
        let marker = root.path().join("unexpectedly-executed");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        for setting in ["core.fsmonitor", "core.pager"] {
            run(root.path(), &["config", setting, script.to_str().unwrap()]);
        }
        let hooks = root.path().join("hooks");
        std::fs::create_dir(&hooks).unwrap();
        for hook in ["post-checkout", "post-index-change", "pre-commit"] {
            std::fs::copy(&script, hooks.join(hook)).unwrap();
        }
        run(
            root.path(),
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
        let head = std::fs::read(root.path().join(".git/HEAD")).unwrap();
        let config = std::fs::read(root.path().join(".git/config")).unwrap();
        let index = std::fs::read(root.path().join(".git/index")).ok();
        assert_eq!(
            inspect(root.path()).await,
            Status::Branch {
                name: "main".into(),
                bare: false
            }
        );
        assert!(!marker.exists());
        assert_eq!(std::fs::read(root.path().join(".git/HEAD")).unwrap(), head);
        assert_eq!(
            std::fs::read(root.path().join(".git/config")).unwrap(),
            config
        );
        assert_eq!(std::fs::read(root.path().join(".git/index")).ok(), index);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_blocked_git_config_is_timed_out_instead_of_hanging_the_ui() {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir().unwrap();
        run(root.path(), &["init", "-b", "main"]);
        let config = root.path().join(".git/config");
        std::fs::remove_file(&config).unwrap();
        let path = std::ffi::CString::new(config.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(
            matches!(inspect(root.path()).await, Status::Unavailable(error) if error.contains("timed out"))
        );
    }

    #[test]
    fn background_refresh_picks_up_external_branch_changes_and_project_switches() {
        fn settle(
            probe: &mut Probe,
            ctx: &egui::Context,
            runtime: &tokio::runtime::Runtime,
            root: &Path,
        ) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                probe.update(ctx, runtime, root.to_owned());
                if probe.receive.is_none() {
                    break;
                }
                assert!(Instant::now() < deadline, "Git metadata did not settle");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let runtime = crate::lifecycle::TaskRuntime::new();
        let ctx = egui::Context::default();
        let root = tempfile::tempdir().unwrap();
        run(root.path(), &["init", "-b", "main"]);
        let mut probe = Probe::default();
        settle(&mut probe, &ctx, &runtime, root.path());
        assert!(matches!(probe.status(), Status::Branch { name, .. } if name == "main"));
        run(
            root.path(),
            &["symbolic-ref", "HEAD", "refs/heads/updated-externally"],
        );
        probe.next = Instant::now();
        settle(&mut probe, &ctx, &runtime, root.path());
        assert!(
            matches!(probe.status(), Status::Branch { name, .. } if name == "updated-externally")
        );
        let other = tempfile::tempdir().unwrap();
        assert_eq!(probe.status_for(other.path()), &Status::Checking);
        settle(&mut probe, &ctx, &runtime, other.path());
        assert_eq!(probe.status(), &Status::NotRepository);
    }
}

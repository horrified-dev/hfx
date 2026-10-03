//! User-level command execution with cancellable process groups and useful diagnostics.
use crate::{
    state::{ActionStatus, CommandMode, Settings},
    tools::ToolOutput,
};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Child,
};

const HALF_CAPTURE: usize = 32768;
const LOG_LIMIT: usize = 16 * 1024 * 1024;

struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
    log: Option<BufWriter<File>>,
    logged: usize,
    log_error: Option<String>,
}

impl Capture {
    fn new(path: &Path) -> Result<Self, String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|e| format!("Cannot create command log: {e}"))?;
        Ok(Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
            log: Some(BufWriter::new(file)),
            logged: 0,
            log_error: None,
        })
    }

    fn memory() -> Self {
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            total: 0,
            log: None,
            logged: 0,
            log_error: None,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let count = bytes
            .len()
            .min(HALF_CAPTURE.saturating_sub(self.head.len()));
        self.head.extend_from_slice(&bytes[..count]);
        let remaining = &bytes[count..];
        self.tail.extend(remaining);
        if self.tail.len() > HALF_CAPTURE {
            self.tail.drain(..self.tail.len() - HALF_CAPTURE);
        }
        let count = bytes.len().min(LOG_LIMIT.saturating_sub(self.logged));
        if self.log_error.is_none()
            && count > 0
            && let Some(log) = &mut self.log
        {
            if let Err(error) = log.write_all(&bytes[..count]) {
                self.log_error = Some(error.to_string());
            }
            self.logged += count;
        }
    }

    fn text(&mut self) -> String {
        if let Some(log) = &mut self.log
            && let Err(error) = log.flush()
        {
            self.log_error = Some(error.to_string());
        }
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if self.total > self.head.len() + self.tail.len() {
            text.push_str(&format!(
                "\n... {} bytes omitted; beginning and final diagnostics retained ...\n",
                self.total - self.head.len() - self.tail.len()
            ));
        }
        text.push_str(&String::from_utf8_lossy(
            &self.tail.iter().copied().collect::<Vec<_>>(),
        ));
        text
    }

    fn warning(&self) -> String {
        if let Some(error) = &self.log_error {
            format!("Log write error: {error}")
        } else if self.log.is_some() && self.total > LOG_LIMIT {
            format!(
                "Log file capped at {LOG_LIMIT} bytes; final output is still in the captured tail."
            )
        } else {
            String::new()
        }
    }
}

/// Cancellation/Stop drops the whole Unix command process group, not just sh.
struct Running {
    child: Child,
    #[cfg(unix)]
    pid: Option<i32>,
    finished: bool,
}

impl Running {
    fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // The child starts in its own process group; never signal hfx's group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.finished {
            self.kill();
        }
    }
}

fn logs(root: &Path) -> Result<(String, PathBuf), String> {
    let relative = format!(".hfx/command-logs/{}", uuid::Uuid::new_v4());
    let folder = crate::tools::workspace_path(root, &relative, true)?;
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Cannot create command log folder: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    std::fs::write(folder.join(".gitignore"), "*\n").map_err(|e| e.to_string())?;
    // Ignore logs even in projects whose root .gitignore does not mention .hfx.
    let ignore = crate::tools::workspace_path(root, ".hfx/command-logs/.gitignore", true)?;
    match OpenOptions::new().write(true).create_new(true).open(ignore) {
        Ok(mut file) => {
            file.write_all(b"*\n").map_err(|e| e.to_string())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(format!(
                "Cannot protect command logs from accidental Git staging: {error}"
            ));
        }
    }
    Ok((relative, folder))
}

async fn drain(
    mut pipe: impl AsyncRead + Unpin,
    capture: Arc<Mutex<Capture>>,
) -> Result<(), std::io::Error> {
    let mut bytes = [0; 8192];
    loop {
        let count = pipe.read(&mut bytes).await?;
        if count == 0 {
            break;
        }
        capture.lock().unwrap().push(&bytes[..count]);
    }
    Ok(())
}

pub async fn execute(root: &Path, script: &str, settings: &Settings) -> Result<ToolOutput, String> {
    execute_for(
        root,
        script,
        settings,
        Duration::from_secs(settings.command_timeout()),
    )
    .await
}

async fn execute_for(
    root: &Path,
    script: &str,
    settings: &Settings,
    limit: Duration,
) -> Result<ToolOutput, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut process = match settings.command_mode {
        CommandMode::Trusted => crate::sandbox::host_command(&root, script)?,
        CommandMode::Sandbox => crate::sandbox::command(&root, script)?,
    };
    process
        .current_dir(&root)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    process.process_group(0);
    // Trusted commands inherit the full host environment, including credentials
    // used by the user's own CLIs/tests. Settings-only keys are not injected.
    // Strict mode already clears the environment in sandbox::command.
    process.env("GIT_TERMINAL_PROMPT", "0");
    // Diagnostics are best-effort: a read-only project or unsafe .hfx link
    // must not prevent an otherwise valid trusted command from running.
    let captures = logs(&root).and_then(|(relative, folder)| {
        Ok((
            relative,
            Capture::new(&folder.join("stdout.log"))?,
            Capture::new(&folder.join("stderr.log"))?,
        ))
    });
    let (notice, out_capture, err_capture) = match captures {
        Ok((relative, out, err)) => (
            format!(
                "Command logs (up to 16 MiB each): {relative}/stdout.log, {relative}/stderr.log"
            ),
            out,
            err,
        ),
        Err(error) => (
            format!("Command logs unavailable: {error}. Beginning/tail output is still captured."),
            Capture::memory(),
            Capture::memory(),
        ),
    };
    let stdout = Arc::new(Mutex::new(out_capture));
    let stderr = Arc::new(Mutex::new(err_capture));
    let child = process
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", settings.command_mode.label()))?;
    let mut running = Running {
        #[cfg(unix)]
        pid: child.id().and_then(|id| i32::try_from(id).ok()),
        child,
        finished: false,
    };
    let out = running.child.stdout.take().ok_or("Missing stdout")?;
    let err = running.child.stderr.take().ok_or("Missing stderr")?;
    let task = async {
        let (status, _, _) = tokio::try_join!(
            running.child.wait(),
            drain(out, stdout.clone()),
            drain(err, stderr.clone())
        )?;
        Ok::<_, std::io::Error>(status)
    };
    let (exit, status) = match tokio::time::timeout(limit, task).await {
        Ok(Ok(status)) => {
            running.finished = true;
            (
                format!("Exit: {status}"),
                if status.success() {
                    ActionStatus::Complete
                } else {
                    ActionStatus::Failed
                },
            )
        }
        result => {
            running.kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), running.child.wait()).await;
            running.finished = true;
            let reason = match result {
                Ok(Err(error)) => format!("Command I/O failed: {error}"),
                Err(_) => format!(
                    "Command timed out after {:.3} seconds; process group stopped. Partial output preserved.",
                    limit.as_secs_f64()
                ),
                _ => unreachable!(),
            };
            (format!("Exit: {reason}"), ActionStatus::Failed)
        }
    };
    let mut out = stdout.lock().unwrap();
    let mut err = stderr.lock().unwrap();
    let out_text = out.text();
    let err_text = err.text();
    let warnings = [out.warning(), err.warning()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let diagnostic = if status == ActionStatus::Failed
        && [
            "permission denied (publickey)",
            "could not read username",
            "authentication failed",
            "author identity unknown",
            "host key verification failed",
        ]
        .iter()
        .any(|p| err_text.to_lowercase().contains(p))
    {
        match settings.command_mode {
            CommandMode::Trusted => {
                "\nGit/SSH uses the host setup available to hfx. Report the specific missing identity/login/key/host verification; do not repeat the same push through many transports or disable host-key checks."
            }
            CommandMode::Sandbox => {
                "\nThe selected strict sandbox hides host Git credentials and SSH agents. Use Trusted host commands for authenticated Git; repeated sandbox authentication workarounds will not restore them."
            }
        }
    } else {
        ""
    };
    Ok(ToolOutput {
        text: format!(
            "{exit}\n{out_text}\n{err_text}\n(command mode: {}; up to 64 KiB beginning/tail per stream)\n{notice}\n{warnings}{diagnostic}",
            settings.command_mode.label()
        ),
        status,
        change: None,
        images: Vec::new(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trusted_commands_preserve_the_inherited_host_environment() {
        // Launch this one test in a child with a fixture environment. Never
        // mutate the test runner's global environment or print real secrets.
        const FIXTURE: &str = "HFX_TRUSTED_ENV_FIXTURE";
        const VARIABLES: &[&str] = &[
            "HOME",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "TMPDIR",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "PIP_CACHE_DIR",
            "npm_config_cache",
            "SSH_AUTH_SOCK",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "SSL_CERT_FILE",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "GIT_AUTHOR_NAME",
            "GIT_COMMITTER_NAME",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "LLAMA_API_KEY",
            "BRAVE_SEARCH_API_KEY",
        ];
        let Some(fixture) = std::env::var_os(FIXTURE) else {
            let fixture = tempfile::tempdir().unwrap();
            std::fs::create_dir(fixture.path().join("project")).unwrap();
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "commands::tests::trusted_commands_preserve_the_inherited_host_environment",
                    "--nocapture",
                ])
                .env(FIXTURE, fixture.path());
            for name in VARIABLES {
                child.env(name, fixture.path().join(name));
            }
            let output = child.output().await.unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };
        let mut script = String::from("set -eu; ");
        for name in VARIABLES {
            script.push_str(&format!(
                "test \"${name}\" = \"${FIXTURE}/{name}\" || exit 19; "
            ));
        }
        script.push_str("printf 'host environment preserved'");
        let root = PathBuf::from(fixture).join("project");
        let output = execute(&root, &script, &Settings::default()).await.unwrap();
        assert_eq!(output.status, ActionStatus::Complete, "{}", output.text);
        assert!(output.text.contains("host environment preserved"));
        assert!(!root.join(".hfx/cache").exists());

        #[cfg(target_os = "linux")]
        {
            // Explicit strict mode must still hide the same fixture secrets.
            let settings = Settings {
                command_mode: CommandMode::Sandbox,
                ..Default::default()
            };
            let script = "set -eu; test \"$HOME\" = /tmp/hfx-home; test \"$TMPDIR\" = /tmp; \
                test \"${OPENAI_API_KEY+x}\" = ''; test \"${OPENROUTER_API_KEY+x}\" = ''; \
                test \"${LLAMA_API_KEY+x}\" = ''; test \"${BRAVE_SEARCH_API_KEY+x}\" = ''; \
                test \"${SSH_AUTH_SOCK+x}\" = ''; test \"${PIP_CACHE_DIR+x}\" = ''; \
                test \"${npm_config_cache+x}\" = ''; test \"${HFX_TRUSTED_ENV_FIXTURE+x}\" = ''; \
                test \"$CARGO_HOME\" = \"$PWD/.hfx/cache/cargo\" || \
                test \"$CARGO_HOME\" -ef \"$PWD/.hfx/cache/cargo\"; printf 'strict mode still isolated'";
            let output = execute(&root, script, &settings).await.unwrap();
            assert_eq!(output.status, ActionStatus::Complete, "{}", output.text);
            assert!(output.text.contains("strict mode still isolated"));
        }
    }

    #[tokio::test]
    async fn trusted_commands_use_host_paths_without_a_workspace_filesystem_sandbox() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("project");
        let outside = fixture.path().join("outside.txt");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(&outside, "before").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("host-link")).unwrap();
        let output = execute(
            &root,
            "set -e; test \"$(cat ../outside.txt)\" = before; printf after > host-link; printf temporary > ../host-temp.txt",
            &Settings::default(),
        ).await.unwrap();
        assert_eq!(output.status, ActionStatus::Complete, "{}", output.text);
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "after");
        assert_eq!(
            std::fs::read_to_string(fixture.path().join("host-temp.txt")).unwrap(),
            "temporary"
        );
        assert!(!root.join(".hfx/cache").exists());
    }

    #[tokio::test]
    async fn trusted_cargo_build_test_and_run_work_without_cache_overrides() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"host-cargo-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"host-cargo-fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root.path().join("src/main.rs"),
            "fn main() { println!(\"normal cargo works\"); }\n#[test] fn fixture() { assert_eq!(2 + 2, 4); }\n").unwrap();
        let output = execute(
            root.path(),
            "cargo build --locked && cargo test --locked && cargo run --locked",
            &Settings::default(),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Complete, "{}", output.text);
        assert!(output.text.contains("normal cargo works"));
        assert!(output.text.contains("1 passed"));
        assert!(!root.path().join(".hfx/cache").exists());
        assert!(!root.path().join(".cargo").exists());
    }

    #[tokio::test]
    async fn verbose_errors_keep_the_tail_and_durable_private_logs() {
        let root = tempfile::tempdir().unwrap();
        let output = execute(
            root.path(),
            "head -c 100000 /dev/zero | tr '\\000' x; printf '\nFINAL DIAGNOSTIC'; exit 7",
            &Settings::default(),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Failed);
        assert!(output.text.contains("FINAL DIAGNOSTIC"));
        assert!(output.text.contains("bytes omitted"));
        let folder = std::fs::read_dir(root.path().join(".hfx/command-logs"))
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.path().is_dir())
            .unwrap()
            .path();
        let log = std::fs::read(folder.join("stdout.log")).unwrap();
        assert!(log.len() > 100000);
        assert!(log.ends_with(b"FINAL DIAGNOSTIC"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                folder
                    .join("stdout.log")
                    .metadata()
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn an_unsafe_log_directory_does_not_block_commands_or_write_outside_project() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("project");
        let outside = parent.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".hfx")).unwrap();
        let output = execute(&root, "printf 'command still works'", &Settings::default())
            .await
            .unwrap();
        assert_eq!(output.status, ActionStatus::Complete);
        assert!(output.text.contains("logs unavailable"));
        assert!(output.text.contains("command still works"));
        assert!(std::fs::read_dir(outside).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn timeout_keeps_partial_output_and_stops_children() {
        let root = tempfile::tempdir().unwrap();
        let output = execute_for(
            root.path(),
            "printf 'build started'; sleep 30 & echo $! > child.pid; wait",
            &Settings::default(),
            Duration::from_millis(100),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Failed);
        assert!(output.text.contains("timed out"));
        assert!(output.text.contains("build started"));
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(root.path().join("child.pid")).unwrap();
            let status = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
            assert!(status.is_err() || status.unwrap().split_whitespace().nth(2) == Some("Z"));
        }
    }

    #[tokio::test]
    async fn stop_cancels_the_host_command_group_and_leaves_logs() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_owned();
        let task = tokio::spawn(async move {
            execute(
                &path,
                "printf 'started'; sleep 30 & echo $! > child.pid; wait",
                &Settings::default(),
            )
            .await
        });
        for _ in 0..100 {
            if root.path().join("child.pid").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(root.path().join("child.pid").exists());
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(root.path().join("child.pid")).unwrap();
            let status = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
            assert!(status.is_err() || status.unwrap().split_whitespace().nth(2) == Some("Z"));
        }
        assert!(root.path().join(".hfx/command-logs").is_dir());
    }
}

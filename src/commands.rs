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
const POST_EXIT_DRAIN_GRACE: Duration = Duration::from_millis(150);

struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
    log: Option<BufWriter<File>>,
    logged: usize,
    log_error: Option<String>,
    live: bool,
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
            live: false,
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
            live: false,
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
        if self.live {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if let Some(log) = &mut self.log
            && let Err(error) = log.flush()
        {
            self.log_error = Some(error.to_string());
        }
    }

    fn text(&mut self) -> String {
        self.flush();
        let tail = self.tail.make_contiguous();
        if self.total == self.head.len() + tail.len() {
            // These buffers are adjacent, so decode them together: the split
            // at HALF_CAPTURE can be in the middle of a UTF-8 character.
            let mut bytes = Vec::with_capacity(self.total);
            bytes.extend_from_slice(&self.head);
            bytes.extend_from_slice(tail);
            return String::from_utf8(bytes)
                .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
        }
        // At a real truncation gap, omit partial characters rather than
        // manufacturing replacement characters in otherwise valid diagnostics.
        let head_end = match std::str::from_utf8(&self.head) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => self.head.len(),
        };
        let tail_start = tail
            .iter()
            .take(3)
            .take_while(|&&byte| byte & 0xc0 == 0x80)
            .count();
        let tail = &tail[tail_start..];
        let mut text = String::from_utf8_lossy(&self.head[..head_end]).into_owned();
        text.push_str(&format!(
            "\n... {} bytes omitted; beginning and final diagnostics retained ...\n",
            self.total - head_end - tail.len()
        ));
        text.push_str(&String::from_utf8_lossy(tail));
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
    let durable_logs = out_capture.log.is_some() && err_capture.log.is_some();
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
    // Drain while the shell runs: waiting for it without reading can deadlock
    // as soon as a build fills either pipe. EOF, however, is not shell exit:
    // `dev-server &` inherits the writers and may keep them open indefinitely.
    let out_capture = stdout.clone();
    let err_capture = stderr.clone();
    let mut pipes = Box::pin(async move {
        tokio::try_join!(drain(out, out_capture), drain(err, err_capture)).map(|_| ())
    });
    let mut drained = false;
    let wait = async {
        tokio::select! {
            status = running.child.wait() => status,
            result = &mut pipes => {
                drained = true;
                result?;
                running.child.wait().await
            }
        }
    };
    let outcome = match tokio::time::timeout(limit, wait).await {
        Ok(Ok(status)) => {
            if drained {
                Ok(status)
            } else {
                // Retain the ordinary final diagnostics without making a
                // background service's lifetime the duration of this tool.
                match tokio::time::timeout(POST_EXIT_DRAIN_GRACE, &mut pipes).await {
                    Ok(result) => {
                        drained = true;
                        result
                            .map(|()| status)
                            .map_err(|error| format!("Command I/O failed: {error}"))
                    }
                    Err(_) => Ok(status),
                }
            }
        }
        Ok(Err(error)) => Err(format!("Command I/O failed: {error}")),
        Err(_) => Err(format!(
            "Command timed out after {:.3} seconds; process group stopped. Partial output preserved.",
            limit.as_secs_f64()
        )),
    };
    let mut background = String::new();
    let (exit, status) = match outcome {
        Ok(status) => {
            if drained {
                running.finished = true;
            } else if status.success() {
                // Keep read ends alive: dropping them could SIGPIPE the dev
                // server on its next write. Flush each later chunk so follow-up
                // tools can inspect readiness/errors even at low log volume.
                stdout.lock().unwrap().live = true;
                stderr.lock().unwrap().live = true;
                let logging = if durable_logs {
                    "Output continues to the command logs while hfx is running."
                } else {
                    "Background output is drained to keep the process alive, but no durable logs are available."
                };
                background = format!(
                    "\nShell exited; output streams remain open (usually a background process). {logging} This does not confirm service readiness: check readiness before using it, and stop the service when finished."
                );
                #[cfg(unix)]
                if let Some(pid) = running.pid {
                    background.push_str(&format!("\nBackground process group: {pid}."));
                }
                // The task owns the process-group guard, so runtime shutdown or
                // a read error still kills an attached background group. Normal
                // EOF disarms it, avoiding a signal after that group has exited.
                tokio::spawn(async move {
                    let mut running = running;
                    if pipes.await.is_ok() {
                        running.finished = true;
                    }
                });
            } else {
                // A failed shell should not leave an unreported service alive.
                running.kill();
                let _ = tokio::time::timeout(POST_EXIT_DRAIN_GRACE, &mut pipes).await;
                running.finished = true;
                background = "\nShell failed with output streams still open; remaining process group stopped.".into();
            }
            (
                format!("Exit: {status}"),
                if status.success() {
                    ActionStatus::Complete
                } else {
                    ActionStatus::Failed
                },
            )
        }
        Err(reason) => {
            running.kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), running.child.wait()).await;
            if !drained {
                let _ = tokio::time::timeout(POST_EXIT_DRAIN_GRACE, &mut pipes).await;
            }
            running.finished = true;
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
            "{exit}\n{out_text}\n{err_text}\n(command mode: {}; up to 64 KiB beginning/tail per stream)\n{notice}{background}\n{warnings}{diagnostic}",
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

    #[test]
    fn capture_preserves_unicode_across_head_tail_and_read_boundaries() {
        for prefix in [HALF_CAPTURE - 3, HALF_CAPTURE - 2, HALF_CAPTURE - 1] {
            let text = format!("{}🌿 café 世界", "x".repeat(prefix));
            let mut capture = Capture::memory();
            for bytes in text.as_bytes().chunks(7) {
                capture.push(bytes);
            }
            assert!(
                capture.text() == text,
                "capture changed valid UTF-8 at prefix {prefix}"
            );
        }
    }

    #[test]
    fn capture_truncates_only_complete_utf8_characters() {
        // Vary the suffix to place the retained tail inside each byte of an emoji.
        for suffix in ["", "x", "xx", "xxx"] {
            let text = format!(
                "{}{}{suffix}",
                "x".repeat(HALF_CAPTURE - 1),
                "🌿".repeat(HALF_CAPTURE)
            );
            let mut capture = Capture::memory();
            for bytes in text.as_bytes().chunks(8192) {
                capture.push(bytes);
            }
            let output = capture.text();
            assert!(!output.contains('\u{fffd}'));
            let (head, rest) = output.split_once("\n... ").unwrap();
            let (notice, tail) = rest.split_once('\n').unwrap();
            let omitted: usize = notice.split_whitespace().next().unwrap().parse().unwrap();
            assert_eq!(head.len() + tail.len() + omitted, text.len());
            assert!(head == "x".repeat(HALF_CAPTURE - 1));
            assert!(tail.ends_with(&format!("🌿{suffix}")));
            assert!(text.ends_with(tail));
        }
        let mut invalid = Capture::memory();
        invalid.push(b"invalid: \xff\n");
        assert_eq!(invalid.text(), "invalid: \u{fffd}\n");
    }

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

    struct FixtureProcess(std::path::PathBuf);

    impl Drop for FixtureProcess {
        fn drop(&mut self) {
            if let Ok(pid) = std::fs::read_to_string(self.0.join("child.pid"))
                && let Ok(pid) = pid.trim().parse::<i32>()
                && pid > 0
            {
                // Only a PID written by this test's own temporary fixture.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }

    // Invoked in a separate test process, not the app/user's actual dev server.
    #[test]
    fn background_http_fixture() {
        if std::env::var_os("HFX_BACKGROUND_HTTP_FIXTURE").is_none() {
            return;
        }
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::fs::write("address", listener.local_addr().unwrap().to_string()).unwrap();
        std::fs::write("child.pid", std::process::id().to_string()).unwrap();
        println!("fixture server ready");
        eprintln!("fixture diagnostic ready");
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut buffer = [0; 4096];
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                continue;
            }
            // These writes happen only AFTER the launching tool has returned.
            // Closing its read ends would break a real server on its next log.
            println!("served fixture request 🌿");
            eprintln!("fixture request diagnostic");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready",
                )
                .unwrap();
        }
    }

    #[tokio::test]
    async fn background_server_returns_before_exit_and_keeps_live_logs() {
        let root = tempfile::tempdir().unwrap();
        let _cleanup = FixtureProcess(root.path().to_owned());
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "'\\''");
        let script = format!(
            "cd . && HFX_BACKGROUND_HTTP_FIXTURE=1 '{executable}' --exact commands::tests::background_http_fixture --nocapture &"
        );
        let start = std::time::Instant::now();
        let output = execute_for(
            root.path(),
            &script,
            &Settings::default(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(
            output.status,
            ActionStatus::Complete,
            "Background launch must not wait for server exit: {}",
            output.text
        );
        println!(
            "Background launch returned after {:.1} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
        assert!(
            output.text.contains("streams remain open"),
            "{}",
            output.text
        );
        assert!(output.text.contains("process group"), "{}", output.text);
        child_pid(root.path()).await;
        let address = std::fs::read_to_string(root.path().join("address")).unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        for _ in 0..2 {
            let response = client
                .get(format!("http://{address}/"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.text().await.unwrap(), "ready");
        }
        let directory = std::fs::read_dir(root.path().join(".hfx/command-logs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let out = std::fs::read_to_string(directory.join("stdout.log")).unwrap();
            let err = std::fs::read_to_string(directory.join("stderr.log")).unwrap();
            if out.contains("served fixture request 🌿")
                && err.contains("fixture request diagnostic")
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "background output was not flushed to durable logs"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let followup = execute(root.path(), "printf 'next tool step'", &Settings::default())
            .await
            .unwrap();
        assert_eq!(followup.status, ActionStatus::Complete);
        assert!(followup.text.contains("next tool step"));
    }

    #[tokio::test]
    async fn background_stderr_without_stdout_also_does_not_hold_the_tool_open() {
        let root = tempfile::tempdir().unwrap();
        let _cleanup = FixtureProcess(root.path().to_owned());
        let output = execute_for(
            root.path(),
            "sleep 30 >/dev/null & echo $! > child.pid; printf 'launched'",
            &Settings::default(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Complete, "{}", output.text);
        assert!(output.text.contains("launched"));
        assert!(output.text.contains("streams remain open"));
    }

    #[tokio::test]
    async fn closed_pipes_do_not_mean_a_foreground_command_has_finished() {
        let root = tempfile::tempdir().unwrap();
        let output = execute_for(
            root.path(),
            "exec >/dev/null 2>&1; sleep 30 & echo $! > child.pid; wait",
            &Settings::default(),
            Duration::from_millis(150),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Failed);
        assert!(output.text.contains("timed out"));
        assert!(!output.text.contains("streams remain open"));
        #[cfg(target_os = "linux")]
        assert_child_stopped(&child_pid(root.path()).await).await;
    }

    #[tokio::test]
    async fn failed_shell_stops_background_children_and_preserves_exit_status() {
        let root = tempfile::tempdir().unwrap();
        let _cleanup = FixtureProcess(root.path().to_owned());
        let output = execute_for(
            root.path(),
            "sleep 30 & echo $! > child.pid; printf 'launch failed' >&2; exit 7",
            &Settings::default(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Failed);
        assert!(output.text.contains("exit status: 7"));
        assert!(output.text.contains("launch failed"));
        assert!(output.text.contains("remaining process group stopped"));
        #[cfg(target_os = "linux")]
        assert_child_stopped(&child_pid(root.path()).await).await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn background_process_group_is_stopped_when_runtime_shuts_down() {
        let root = tempfile::tempdir().unwrap();
        let _cleanup = FixtureProcess(root.path().to_owned());
        let path = root.path().to_owned();
        let pid = tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let output = runtime
                .block_on(execute(
                    &path,
                    "sleep 30 & echo $! > child.pid",
                    &Settings::default(),
                ))
                .unwrap();
            assert_eq!(output.status, ActionStatus::Complete);
            let pid = runtime.block_on(child_pid(&path));
            // The log reader may not have been polled after being spawned.
            // Its future must own the entire Running guard even in that case.
            runtime.shutdown_timeout(Duration::from_secs(2));
            pid
        })
        .await
        .unwrap();
        assert_child_stopped(&pid).await;
    }

    #[tokio::test]
    async fn background_launch_works_when_durable_logs_are_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let _cleanup = FixtureProcess(root.path().to_owned());
        std::fs::write(root.path().join(".hfx"), "not a directory").unwrap();
        let output = execute_for(
            root.path(),
            "sleep 30 & echo $! > child.pid; printf 'started without logs'",
            &Settings::default(),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(output.status, ActionStatus::Complete);
        assert!(output.text.contains("logs unavailable"));
        assert!(output.text.contains("streams remain open"));
        assert!(output.text.contains("started without logs"));
    }

    #[test]
    fn live_capture_flushes_small_writes_without_growing_logs_past_the_cap() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("stdout.log");
        let mut capture = Capture::new(&path).unwrap();
        capture.live = true;
        capture.push(b"ready\n");
        assert_eq!(std::fs::read(&path).unwrap(), b"ready\n");
        let bytes = [b'x'; 8192];
        for _ in 0..(LOG_LIMIT / bytes.len() + 2) {
            capture.push(&bytes);
        }
        assert_eq!(std::fs::metadata(&path).unwrap().len(), LOG_LIMIT as u64);
        assert_eq!(capture.head.len(), HALF_CAPTURE);
        assert_eq!(capture.tail.len(), HALF_CAPTURE);
        assert!(capture.warning().contains("capped"));
    }

    async fn child_pid(root: &Path) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(pid) = std::fs::read_to_string(root.join("child.pid"))
                && pid.trim().parse::<u32>().is_ok()
            {
                return pid.trim().to_owned();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "child did not publish its PID"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[cfg(target_os = "linux")]
    async fn assert_child_stopped(pid: &str) {
        // SIGKILL delivery to descendants is asynchronous. Reaping the shell
        // does not guarantee every child has already reached its terminal state.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(stat) => stat,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        || error.raw_os_error() == Some(libc::ESRCH) =>
                {
                    return;
                }
                Err(error) => panic!("Cannot inspect child {pid}: {error}"),
            };
            let state = stat
                .rsplit_once(") ")
                .and_then(|(_, fields)| fields.chars().next());
            if matches!(state, Some('Z' | 'X')) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "child {pid} still running: {state:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
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
            assert_child_stopped(&child_pid(root.path()).await).await;
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
        let _ = child_pid(root.path()).await;
        task.abort();
        let _ = task.await;
        #[cfg(target_os = "linux")]
        {
            assert_child_stopped(&child_pid(root.path()).await).await;
        }
        assert!(root.path().join(".hfx/command-logs").is_dir());
    }
}

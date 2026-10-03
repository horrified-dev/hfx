//! Ownership of asynchronous/background work with a bounded shutdown grace.
use std::{
    ops::Deref,
    time::{Duration, Instant},
};

const SHUTDOWN_GRACE: Duration = Duration::from_millis(100);
static EXIT_STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

pub fn begin_exit() {
    if std::env::var_os("HFX_SHUTDOWN_TRACE").is_some_and(|v| v == "1") {
        EXIT_STARTED.get_or_init(Instant::now);
    }
}

pub fn native_exit_finished() {
    if let Some(started) = EXIT_STARTED.get() {
        trace(
            "on_exit to native return (includes window/graphics teardown)",
            *started,
        );
    }
}

pub struct TaskRuntime(Option<tokio::runtime::Runtime>);

impl TaskRuntime {
    pub fn new() -> Self {
        Self(Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Cannot start async runtime"),
        ))
    }
}

impl Deref for TaskRuntime {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("Runtime is alive until drop")
    }
}

impl Drop for TaskRuntime {
    fn drop(&mut self) {
        let started = Instant::now();
        if let Some(runtime) = self.0.take() {
            // Abort async work and give it a short chance to drop network/tool
            // children. A running spawn_blocking job cannot be aborted: do not
            // wait forever for a clipboard server, portal dialog or image read.
            // The separate chat Store has already completed its durable save.
            runtime.shutdown_timeout(SHUTDOWN_GRACE);
        }
        trace("background runtime cleanup", started);
    }
}

pub fn trace(phase: &str, started: Instant) {
    if std::env::var_os("HFX_SHUTDOWN_TRACE").is_some_and(|v| v == "1") {
        eprintln!(
            "hfx shutdown: {phase}: {:.2} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn shutdown_does_not_wait_for_a_stuck_blocking_job() {
        let runtime = TaskRuntime::new();
        let (started, receive_started) = mpsc::channel();
        let (release, receive_release) = mpsc::channel();
        let (finished, receive_finished) = mpsc::channel();
        runtime.spawn_blocking(move || {
            started.send(()).unwrap();
            let _ = receive_release.recv();
            let _ = finished.send(());
        });
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let (dropped, receive_dropped) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(runtime);
            let _ = dropped.send(());
        });
        let result = receive_dropped.recv_timeout(Duration::from_secs(2));
        // Always release/join the fixture even if the regression assertion fails.
        release.send(()).unwrap();
        receive_finished
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        worker.join().unwrap();
        assert!(
            result.is_ok(),
            "Closing must not wait for the blocking task to be released"
        );
    }
    #[test]
    fn shutdown_cancels_async_tasks_and_runs_their_drop_guards() {
        struct Guard(mpsc::Sender<()>);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let runtime = TaskRuntime::new();
        let (started, receive_started) = mpsc::channel();
        let (dropped, receive_dropped) = mpsc::channel();
        runtime.spawn(async move {
            let _guard = Guard(dropped);
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        drop(runtime);
        receive_dropped
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
    }
}

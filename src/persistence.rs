use crate::state::Saved;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Instant,
};

type SaveResult = Result<(), String>;

struct Job {
    state: Saved,
    done: Vec<mpsc::Sender<SaveResult>>,
}

#[derive(Default)]
struct Pending {
    latest: Option<Job>,
    closed: bool,
}

type Shared = Arc<(Mutex<Pending>, Condvar)>;

/// Coalesces pending snapshots, serializing and atomically replacing chat state
/// off the UI thread. The in-progress write plus one latest snapshot is bounded.
pub struct Store {
    shared: Option<Shared>,
    results: mpsc::Receiver<SaveResult>,
    worker: Option<thread::JoinHandle<()>>,
    pub saved_once: bool,
    blocked: Option<String>,
}

// Ensure flush callers are released even if the worker unexpectedly panics.
struct WorkerExit(Shared);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        let (lock, wake) = &*self.0;
        let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
        pending.closed = true;
        let abandoned = pending.latest.take();
        wake.notify_all();
        drop(pending);
        if let Some(job) = abandoned {
            for done in job.done {
                let _ = done.send(Err("Chat save worker stopped".into()));
            }
        }
    }
}

impl Store {
    pub fn new(path: Option<PathBuf>) -> Self {
        match path {
            Some(path) => Self::with_writer(move |state| write(&path, state)),
            None => {
                let (_, results) = mpsc::channel();
                Self {
                    shared: None,
                    results,
                    worker: None,
                    saved_once: false,
                    blocked: None,
                }
            }
        }
    }

    pub fn blocked(reason: String) -> Self {
        let mut store = Self::new(None);
        store.blocked = Some(reason);
        store
    }

    fn with_writer(mut write_state: impl FnMut(&Saved) -> SaveResult + Send + 'static) -> Self {
        let (result_tx, results) = mpsc::channel();
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let jobs = shared.clone();
        let worker = thread::Builder::new()
            .name("hfx_chat_save".into())
            .spawn(move || {
                let _exit = WorkerExit(jobs.clone());
                loop {
                    let job = {
                        let (lock, wake) = &*jobs;
                        let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while pending.latest.is_none() && !pending.closed {
                            pending = wake.wait(pending).unwrap_or_else(|e| e.into_inner());
                        }
                        let Some(job) = pending.latest.take() else {
                            break;
                        };
                        job
                    };
                    let result = write_state(&job.state);
                    let _ = result_tx.send(result.clone());
                    for done in job.done {
                        let _ = done.send(result.clone());
                    }
                }
            })
            .expect("Cannot start chat save worker");
        Self {
            shared: Some(shared),
            results,
            worker: Some(worker),
            saved_once: false,
            blocked: None,
        }
    }

    fn submit(&self, state: &Saved, done: Option<mpsc::Sender<SaveResult>>) -> SaveResult {
        let Some(shared) = &self.shared else {
            return Ok(());
        };
        // Clone outside the slot lock; large unchanged payloads remain shared.
        let mut job = Job {
            state: state.clone(),
            done: done.into_iter().collect(),
        };
        let (lock, wake) = &**shared;
        let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
        if pending.closed {
            return Err("Chat save worker stopped".into());
        }
        let mut replaced = pending.latest.take();
        if let Some(old) = &mut replaced {
            // A newer durable snapshot satisfies all older flush fences too.
            job.done.append(&mut old.done);
        }
        pending.latest = Some(job);
        wake.notify_one();
        drop(pending);
        // Drop superseded snapshots outside the lock, never on the writer.
        drop(replaced);
        Ok(())
    }

    pub fn queue(&self, state: &Saved) {
        let _ = self.submit(state, None);
    }

    /// Clean exit waits for this snapshot (or a newer one) to be durably saved,
    /// replacing a stale pending autosave rather than queuing behind it.
    pub fn flush(&self, state: &Saved) -> SaveResult {
        if let Some(reason) = &self.blocked {
            return Err(format!("Chat saving is disabled until recovery: {reason}"));
        }
        if self.shared.is_none() {
            return Ok(());
        }
        let (done, receive) = mpsc::channel();
        self.submit(state, Some(done))?;
        receive.recv().map_err(|_| "Chat save worker stopped")?
    }

    pub fn poll(&mut self) -> Option<String> {
        let mut error = None;
        while let Ok(result) = self.results.try_recv() {
            match result {
                Ok(()) => self.saved_once = true,
                Err(message) => error = Some(message),
            }
        }
        error
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let started = Instant::now();
        if let Some(shared) = self.shared.take() {
            let (lock, wake) = &*shared;
            lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
            wake.notify_one();
        }
        // Never abandon the final chat write; this is separate from Tokio's
        // bounded cleanup for non-cancellable background work.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        crate::lifecycle::trace("chat save worker cleanup", started);
    }
}

// Open non-blocking before inspecting metadata so a FIFO cannot hang startup or recovery.
pub fn open_history(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("Chat state is not a regular file"));
    }
    Ok(file)
}

pub fn read(path: &Path) -> Result<Option<Saved>, String> {
    use std::io::Read;
    match open_history(path) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|e| format!("Cannot load saved chats: {e}"))?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("Cannot load saved chats: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match path.symlink_metadata() {
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Ok(None),
            // The path exists, but following it failed (e.g. an offline symlink
            // target). Do not let startup/exit saves replace that original link.
            Ok(_) => Err(format!(
                "Cannot load saved chats: existing path is unavailable: {e}"
            )),
            Err(error) => Err(format!("Cannot inspect saved chats: {error}")),
        },
        Err(e) => Err(format!("Cannot load saved chats: {e}")),
    }
}

fn write(path: &Path, state: &Saved) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Invalid chat storage path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".chats-{}.tmp", uuid::Uuid::new_v4()));
    let result: SaveResult = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary).map_err(|e| e.to_string())?;
        // serde_json emits tiny writes for punctuation and escaped newlines.
        // Buffer them before disk I/O; retain the durable final-save guarantee.
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file);
        serde_json::to_writer(&mut writer, state).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        writer.get_ref().sync_all().map_err(|e| e.to_string())?;
        drop(writer);
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result.map_err(|e| format!("Cannot save chats: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "headless shutdown I/O benchmark; no user history is read"]
    fn profile_shutdown_save_io() {
        use std::io::{BufWriter, Write};
        use std::time::{Duration, Instant};
        struct Counted {
            file: std::fs::File,
            calls: usize,
        }
        impl Write for Counted {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                self.file.write(bytes)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.file.flush()
            }
        }
        let root = tempfile::tempdir().unwrap();
        let body = "let result = \"quoted output\";\n".repeat(65536);
        let mut saved = Saved::default();
        let mut message = crate::state::Message::new(false, "Done".into(), 0.0, "Demo".into());
        message.response_items =
            vec![serde_json::json!({"role":"tool", "content":body.clone()})].into();
        message.activities.push(crate::state::Activity {
            id: "call".into(),
            name: "run_command".into(),
            arguments: "{}".into(),
            result: body.into(),
            status: crate::state::ActionStatus::Complete,
            elapsed: 0.0,
            change: None,
            images: vec![],
        });
        saved.chats[0].messages.push(message);
        for buffered in [false, true] {
            let counted = Counted {
                file: std::fs::File::create(root.path().join(format!("{buffered}.json"))).unwrap(),
                calls: 0,
            };
            let started = Instant::now();
            let mut output = if buffered {
                BufWriter::with_capacity(65536, counted)
            } else {
                BufWriter::with_capacity(0, counted)
            };
            serde_json::to_writer(&mut output, &saved).unwrap();
            output.flush().unwrap();
            output.get_ref().file.sync_all().unwrap();
            println!(
                "writer buffered={buffered}: {:.2}ms, {} writes, {} bytes",
                started.elapsed().as_secs_f64() * 1000.0,
                output.get_ref().calls,
                output.get_ref().file.metadata().unwrap().len()
            );
        }
        let store = Store::new(Some(root.path().join("chats.json")));
        store.queue(&saved);
        std::thread::sleep(Duration::from_millis(5));
        saved.chats[0].draft = "Queued autosave".into();
        store.queue(&saved);
        saved.chats[0].draft = "Final exit snapshot 🌿".into();
        let started = Instant::now();
        store.flush(&saved).unwrap();
        println!(
            "exit flush with active + queued saves: {:.2}ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(
            read(&root.path().join("chats.json"))
                .unwrap()
                .unwrap()
                .chats[0]
                .draft,
            "Final exit snapshot 🌿"
        );
    }

    #[test]
    fn final_flush_replaces_stale_pending_autosaves_without_losing_the_final_draft() {
        use std::time::Duration;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        let output = path.clone();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let records = observed.clone();
        let (started, receive_started) = mpsc::channel();
        let (release, receive_release) = mpsc::channel();
        let mut first = true;
        let store = Store::with_writer(move |saved| {
            if first {
                first = false;
                started.send(()).unwrap();
                let _ = receive_release.recv();
            }
            records.lock().unwrap().push(saved.chats[0].draft.clone());
            write(&output, saved)
        });
        let mut saved = Saved::default();
        saved.chats[0].draft = "In progress".into();
        store.queue(&saved);
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        saved.chats[0].draft = "Stale autosave".into();
        store.queue(&saved);
        saved.chats[0].draft = "Another superseded autosave".into();
        store.queue(&saved);
        let shared = store.shared.clone().unwrap();
        saved.chats[0].draft = "Final text 🌿".into();
        let final_save = thread::spawn(move || {
            store.flush(&saved).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let queued = loop {
            if shared
                .0
                .lock()
                .unwrap()
                .latest
                .as_ref()
                .is_some_and(|job| !job.done.is_empty())
            {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(1));
        };
        release.send(()).unwrap();
        final_save.join().unwrap();
        assert!(queued);
        assert_eq!(*observed.lock().unwrap(), ["In progress", "Final text 🌿"]);
        assert_eq!(
            read(&path).unwrap().unwrap().chats[0].draft,
            "Final text 🌿"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_newer_pending_snapshot_preserves_all_flush_acknowledgements() {
        use std::time::Duration;
        let (started, receive_started) = mpsc::channel();
        let (release, receive_release) = mpsc::channel();
        let mut first = true;
        let store = Store::with_writer(move |_| {
            if first {
                first = false;
                started.send(()).unwrap();
                let _ = receive_release.recv();
            }
            Ok(())
        });
        let saved = Saved::default();
        store.queue(&saved);
        receive_started
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let (done1, receive1) = mpsc::channel();
        let (done2, receive2) = mpsc::channel();
        store.submit(&saved, Some(done1)).unwrap();
        store.submit(&saved, Some(done2)).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            receive1.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(())
        );
        assert_eq!(
            receive2.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(())
        );
    }

    #[test]
    fn write_errors_are_reported_to_final_flush_and_poll() {
        let mut store = Store::with_writer(|_| Err("simulated disk failure".into()));
        assert_eq!(
            store.flush(&Saved::default()),
            Err("simulated disk failure".into())
        );
        assert_eq!(store.poll().as_deref(), Some("simulated disk failure"));
        assert!(!store.saved_once);
    }

    #[test]
    fn background_save_preserves_latest_draft_and_shared_tool_history() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        let store = Store::new(Some(path.clone()));
        let mut saved = Saved::default();
        let mut message =
            crate::state::Message::new(false, "Done".into(), 0.0, "OpenRouter".into());
        message.response_items = vec![serde_json::json!({"role":"tool","tool_call_id":"call","content":"x".repeat(1024*1024)})].into();
        saved.chats[0].messages.push(message);
        let snapshot = saved.clone();
        assert!(std::sync::Arc::ptr_eq(
            &saved.chats[0].messages[0].response_items,
            &snapshot.chats[0].messages[0].response_items
        ));
        saved.chats[0].draft = "Earlier".into();
        store.queue(&saved);
        saved.chats[0].draft = "Latest typing 🌿".into();
        store.flush(&saved).unwrap();
        let restored = read(&path).unwrap().unwrap();
        assert_eq!(restored.chats[0].draft, "Latest typing 🌿");
        assert_eq!(
            *restored.chats[0].messages[0].response_items,
            *saved.chats[0].messages[0].response_items
        );
    }
}

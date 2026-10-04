//! Startup recovery never destroys a file that failed to load.
use crate::{persistence, state::Saved};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Recovery {
    pub path: PathBuf,
    pub error: String,
}

pub enum Outcome {
    Reloaded(Box<Saved>),
    BackedUp(PathBuf),
}

/// Missing state may migrate from legacy storage. Invalid/unreadable state may
/// also use that fallback in memory, but must disable every automatic save.
pub fn load(path: Option<&Path>, fallback: impl FnOnce() -> Saved) -> (Saved, Option<Recovery>) {
    match path.map(persistence::read) {
        Some(Ok(Some(saved))) => (saved, None),
        Some(Err(error)) => (
            fallback(),
            Some(Recovery {
                path: path.expect("A read was attempted").to_owned(),
                error,
            }),
        ),
        _ => (fallback(), None),
    }
}

impl Recovery {
    pub fn retry(&self) -> Result<Outcome, String> {
        let mut saved = persistence::read(&self.path)?
            .ok_or("The original file is missing. Restore it before retrying.")?;
        saved.restore();
        Ok(Outcome::Reloaded(Box::new(saved)))
    }

    /// Only an explicit recovery action calls this. Keep the original in place
    /// until a new atomic save succeeds, and create a durable, private backup.
    pub fn backup(&self) -> Result<Outcome, String> {
        let parent = self.path.parent().ok_or("Invalid chat storage path")?;
        let backup = parent.join(format!("chats-recovery-{}.json", uuid::Uuid::new_v4()));
        let result: std::io::Result<()> = (|| {
            let mut source = persistence::open_history(&self.path)?;
            let mut options = std::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut destination = options.open(&backup)?;
            std::io::copy(&mut source, &mut destination)?;
            destination.sync_all()?;
            #[cfg(unix)]
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&backup);
            return Err(format!(
                "Cannot back up the original; saving remains disabled: {error}"
            ));
        }
        Ok(Outcome::BackedUp(backup))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_incompatible_state_blocks_all_saves_and_preserves_legacy() {
        for bytes in [
            b"{broken".as_slice(),
            b"{\"future_schema\":true}".as_slice(),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("chats.json");
            std::fs::write(&path, bytes).unwrap();
            let mut legacy = Saved::default();
            legacy.chats[0].draft = "recoverable legacy draft".into();
            let (mut state, recovery) = load(Some(&path), || legacy);
            assert_eq!(state.chats[0].draft, "recoverable legacy draft");
            let recovery = recovery.unwrap();
            let mut store = persistence::Store::blocked(recovery.error.clone());
            state.chats[0].draft = "new unsaved session".into();
            store.queue(&state);
            assert!(store.flush(&state).is_err());
            assert!(store.poll().is_none());
            assert!(!store.saved_once);
            drop(store);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            let Outcome::BackedUp(backup) = recovery.backup().unwrap() else {
                panic!()
            };
            assert_eq!(std::fs::read(&backup).unwrap(), bytes);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    backup.metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            let store = persistence::Store::new(Some(path.clone()));
            store.flush(&state).unwrap();
            assert_eq!(
                persistence::read(&path).unwrap().unwrap().chats[0].draft,
                "new unsaved session"
            );
            assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        }
    }

    #[test]
    fn valid_state_does_not_use_fallback_and_missing_state_can_migrate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        let (state, recovery) = load(Some(&path), Saved::default);
        assert!(recovery.is_none());
        persistence::Store::new(Some(path.clone()))
            .flush(&state)
            .unwrap();
        let (_, recovery) = load(Some(&path), || panic!("valid state must not fall back"));
        assert!(recovery.is_none());
    }

    #[test]
    fn unreadable_state_and_failed_backup_do_not_enable_saving() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        std::fs::create_dir(&path).unwrap();
        let (_, recovery) = load(Some(&path), Saved::default);
        let recovery = recovery.unwrap();
        assert!(recovery.retry().is_err());
        assert!(recovery.backup().is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn retry_uses_repaired_original_and_never_writes_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        let recovery = Recovery {
            path: path.clone(),
            error: "fixture".into(),
        };
        assert!(recovery.retry().is_err());
        let mut saved = Saved::default();
        saved.chats[0].draft = "restored".into();
        let bytes = serde_json::to_vec(&saved).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let Outcome::Reloaded(state) = recovery.retry().unwrap() else {
            panic!()
        };
        assert_eq!(state.chats[0].draft, "restored");
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[cfg(all(test, unix))]
mod special_file_tests {
    use super::*;

    #[test]
    fn a_non_regular_chat_file_never_blocks_startup_retry_or_backup() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (_, recovery) = load(Some(&path), Saved::default);
            let recovery = recovery.unwrap();
            tx.send((recovery.retry().is_err(), recovery.backup().is_err()))
                .unwrap();
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),
            (true, true)
        );
        worker.join().unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

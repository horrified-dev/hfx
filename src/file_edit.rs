//! Exact text edits with optimistic stale-content checks and atomic replacement.
use crate::{
    state::ActionStatus,
    tools::{ToolOutput, file_change, workspace_path},
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_EDIT_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPLACEMENT_BYTES: usize = 256 * 1024;

pub fn sha256(text: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, text.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Snapshot {
    text: String,
    permissions: std::fs::Permissions,
}

fn snapshot(path: &Path, limit: usize) -> Result<Option<Snapshot>, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read file before editing: {error}")),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular UTF-8 file.".into());
    }
    if metadata.len() > limit as u64 {
        return Err(format!(
            "Existing file exceeds the {} KiB edit limit.",
            limit / 1024
        ));
    }
    let mut text = String::new();
    file.take((limit + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > limit {
        return Err("File grew beyond the edit limit; read it again before editing.".into());
    }
    Ok(Some(Snapshot {
        text,
        permissions: metadata.permissions(),
    }))
}

fn replace(
    root: &Path,
    relative: &str,
    path: &Path,
    before: Option<&Snapshot>,
    after: &str,
) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid file path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".hfx-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(after.as_bytes())
            .map_err(|e| e.to_string())?;
        if let Some(before) = before {
            file.set_permissions(before.permissions.clone())
                .map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        // Do not clobber an editor's changes made while this edit was prepared.
        // This is optimistic concurrency, not a cross-process filesystem lock.
        if workspace_path(root, relative, true)? != path {
            return Err("File path changed while preparing the edit; read it again.".into());
        }
        let current = snapshot(path, MAX_EDIT_FILE_BYTES)?;
        if before.map(|s| (s.text.as_str(), &s.permissions))
            != current.as_ref().map(|s| (s.text.as_str(), &s.permissions))
        {
            return Err(
                "File changed while preparing the edit; no changes were written. Read it again."
                    .into(),
            );
        }
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn output(relative: &str, before: Option<&Snapshot>, after: &str) -> ToolOutput {
    let old = before.map_or("", |s| s.text.as_str());
    ToolOutput {
        text: format!(
            "Wrote {relative} ({} bytes; sha256={})",
            after.len(),
            sha256(after)
        ),
        status: ActionStatus::Complete,
        images: Vec::new(),
        change: (old != after || before.is_none()).then(|| {
            let mut change = file_change(relative, old, after);
            if before.is_none() && after.is_empty() {
                change.diff = "Created an empty file.".into();
            }
            change
        }),
    }
}

pub fn write(root: &Path, relative: &str, args: &Value) -> Result<ToolOutput, String> {
    let content = args["content"].as_str().ok_or("Missing file content")?;
    if content.len() > MAX_REPLACEMENT_BYTES {
        return Err(
            "Write exceeds the 256 KiB limit. Use edit_file for precise edits to existing files."
                .into(),
        );
    }
    let path = workspace_path(root, relative, true)?;
    let before = snapshot(&path, MAX_REPLACEMENT_BYTES)?;
    replace(root, relative, &path, before.as_ref(), content)?;
    Ok(output(relative, before.as_ref(), content))
}

pub fn edit(root: &Path, relative: &str, args: &Value) -> Result<ToolOutput, String> {
    let old = args["old_text"].as_str().ok_or("Missing old_text")?;
    let new = args["new_text"].as_str().ok_or("Missing new_text")?;
    if old.is_empty() {
        return Err(
            "old_text must be non-empty. Include surrounding context for insertions.".into(),
        );
    }
    if old.len() > MAX_REPLACEMENT_BYTES || new.len() > MAX_REPLACEMENT_BYTES {
        return Err("Each edit fragment must be at most 256 KiB.".into());
    }
    let path: PathBuf = workspace_path(root, relative, false)?;
    let before = snapshot(&path, MAX_EDIT_FILE_BYTES)?.ok_or("File no longer exists")?;
    if let Some(expected) = args.get("expected_sha256").filter(|v| !v.is_null()) {
        let expected = expected
            .as_str()
            .ok_or("expected_sha256 must be a string or null")?;
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(
                "expected_sha256 must be a 64-character SHA-256 hex digest or null.".into(),
            );
        }
        if !expected.eq_ignore_ascii_case(&sha256(&before.text)) {
            return Err(
                "Stale file digest; no changes were written. Read the file again before editing."
                    .into(),
            );
        }
    }
    let Some(start) = before.text.find(old) else {
        return Err(
            "old_text does not match the current file; no changes were written. Read it again."
                .into(),
        );
    };
    let next = start + old.chars().next().expect("Non-empty old_text").len_utf8();
    if before.text[next..].contains(old) {
        return Err(
            "old_text is ambiguous; no changes were written. Include more surrounding context."
                .into(),
        );
    }
    let after = before.text.replacen(old, new, 1);
    if after.len() > MAX_EDIT_FILE_BYTES {
        return Err("Edited file exceeds the 16 MiB limit.".into());
    }
    if after != before.text {
        replace(root, relative, &path, Some(&before), &after)?;
    }
    Ok(output(relative, Some(&before), &after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn precise_edits_work_above_the_whole_file_write_limit_and_preserve_permissions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.rs");
        let original = format!(
            "{}\nfn before() {{ /* 世界 🌿 */ }}\n",
            "// padding\n".repeat(30000)
        );
        std::fs::write(&path, &original).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
        }
        let args = json!({"old_text":"fn before()", "new_text":"fn after()", "expected_sha256":sha256(&original)});
        let output = edit(root.path(), "large.rs", &args).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("fn before()", "fn after()")
        );
        let change = output.change.unwrap();
        assert_eq!((change.added, change.removed), (1, 1));
        assert!(change.diff.contains("世界 🌿"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o751);
        }
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn stale_ambiguous_invalid_and_noop_edits_never_damage_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("code.rs");
        let original = "same\nsame\nunique\n";
        std::fs::write(&path, original).unwrap();
        for args in [
            json!({"old_text":"unique", "new_text":"changed", "expected_sha256":"0".repeat(64)}),
            json!({"old_text":"missing", "new_text":"changed"}),
            json!({"old_text":"same", "new_text":"changed"}),
            json!({"old_text":"", "new_text":"changed"}),
            json!({"old_text":"unique", "new_text":"changed", "expected_sha256":123}),
            json!({"old_text":"unique", "new_text":"changed", "expected_sha256":"bad"}),
        ] {
            assert!(edit(root.path(), "code.rs", &args).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
        let result = edit(
            root.path(),
            "code.rs",
            &json!({"old_text":"unique", "new_text":"unique", "expected_sha256":null}),
        )
        .unwrap();
        assert!(result.change.is_none());
        let before = snapshot(&path, MAX_EDIT_FILE_BYTES).unwrap().unwrap();
        std::fs::write(&path, "user changed this").unwrap();
        assert!(
            replace(
                root.path(),
                "code.rs",
                &path,
                Some(&before),
                "model replacement"
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user changed this");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn edits_reject_workspace_escapes_non_text_and_oversized_files() {
        let root = tempfile::tempdir().unwrap();
        let args = json!({"old_text":"old", "new_text":"new"});
        assert!(edit(root.path(), "../outside", &args).is_err());
        assert!(edit(root.path(), "/outside", &args).is_err());
        std::fs::write(root.path().join("binary"), [0xff]).unwrap();
        assert!(edit(root.path(), "binary", &args).is_err());
        let file = std::fs::File::create(root.path().join("huge")).unwrap();
        file.set_len((MAX_EDIT_FILE_BYTES + 1) as u64).unwrap();
        assert!(edit(root.path(), "huge", &args).is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("secret"), "old").unwrap();
            std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("link"))
                .unwrap();
            assert!(edit(root.path(), "link", &args).is_err());
            assert_eq!(
                std::fs::read_to_string(outside.path().join("secret")).unwrap(),
                "old"
            );
        }
    }
}

#[cfg(test)]
mod exact_match_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overlapping_unicode_matches_are_ambiguous_and_deletions_are_precise() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("code.rs");
        for (text, fragment) in [("ababa", "aba"), ("世界世界世", "世界世")] {
            std::fs::write(&path, text).unwrap();
            assert!(
                edit(
                    root.path(),
                    "code.rs",
                    &json!({"old_text":fragment,"new_text":"new","expected_sha256":null})
                )
                .unwrap_err()
                .contains("ambiguous")
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        std::fs::write(&path, "before\nunique line\nafter\n").unwrap();
        let output = edit(
            root.path(),
            "code.rs",
            &json!({"old_text":"unique line\n","new_text":"","expected_sha256":null}),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\nafter\n");
        assert_eq!(output.change.unwrap().removed, 1);
    }
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn preparing_an_edit_does_not_overwrite_a_concurrent_permission_change() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("code.rs");
        std::fs::write(&path, "original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let before = snapshot(&path, MAX_EDIT_FILE_BYTES).unwrap().unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(replace(root.path(), "code.rs", &path, Some(&before), "new content").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

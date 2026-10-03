//! File size is not a context limit. Read any regular UTF-8 file by seeking
//! directly to a bounded page; the rest remains available through next_offset.
use crate::{state::Settings, tools::workspace_path};
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const MAX_PAGE_MEMORY: usize = 1024 * 1024;

/// A response budget, not a maximum file size. The backend can further reduce
/// this to fit the available context and other reads in the same tool round.
pub fn default_budget(settings: &Settings) -> usize {
    usize::try_from(settings.context_limit().saturating_mul(3) / 2)
        .unwrap_or(MAX_PAGE_MEMORY)
        .clamp(4, MAX_PAGE_MEMORY)
}

pub struct Page {
    pub content: String,
    pub offset: u64,
    pub next_offset: u64,
    pub file_bytes: u64,
    pub eof: bool,
    explicit_range: bool,
}

impl Page {
    pub fn result(self, relative: &str) -> String {
        // Preserve ordinary small-file output and old path-only callers.
        if self.eof && self.offset == 0 && !self.explicit_range {
            return self.content;
        }
        let metadata = json!({"path":relative,"offset":self.offset,"bytes_returned":self.content.len(),"file_bytes":self.file_bytes,
            "eof":self.eof,"next_offset":if self.eof {None} else {Some(self.next_offset)}});
        let next = if self.eof {
            String::new()
        } else {
            format!(
                "\nMore content is available. Continue with {}. This is a successful partial read, not the whole file.\n",
                json!({"path":relative,"offset":self.next_offset,"max_bytes":null})
            )
        };
        format!("[read_file {metadata}]\n{next}{}", self.content)
    }
}

fn optional_integer(args: &Value, field: &str) -> Result<Option<u64>, String> {
    match args.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{field} must be a non-negative integer or null.")),
    }
}

pub fn read(root: &Path, relative: &str, args: &Value, budget: usize) -> Result<Page, String> {
    let offset = optional_integer(args, "offset")?.unwrap_or(0);
    let requested = optional_integer(args, "max_bytes")?;
    if requested.is_some_and(|v| v < 4) {
        return Err("max_bytes must be at least 4 (one UTF-8 character) or null.".into());
    }
    let limit = requested
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(budget)
        .min(budget)
        .clamp(4, MAX_PAGE_MEMORY);
    let path = workspace_path(root, relative, false)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("Cannot open text file: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular text file.".into());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Cannot seek in text file: {e}"))?;
    // Four look-ahead bytes let us end on a UTF-8 boundary without reading the
    // remainder of even a multi-gigabyte file or allocating a giant line buffer.
    let mut bytes = Vec::new();
    file.take((limit + 4) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read text file: {e}"))?;
    let mut end = bytes.len().min(limit);
    match std::str::from_utf8(&bytes[..end]) {
        Ok(_) => {},
        Err(error) if error.error_len().is_none() && bytes.len() > end => {
            end = error.valid_up_to();
            // Do not silently skip corrupt bytes masquerading as a clipped
            // character; pagination must be lossless for valid UTF-8.
            let tail = &bytes[end..];
            let valid_character = (1..=tail.len().min(4)).any(|n| std::str::from_utf8(&tail[..n]).is_ok());
            if !valid_character { return Err("File contains invalid UTF-8 near the page boundary.".into()); }
        }
        Err(_) => return Err("File is not valid UTF-8 at this offset. Use a UTF-8 boundary (the previous next_offset), or a binary-capable command for non-text data.".into()),
    }
    let content = std::str::from_utf8(&bytes[..end])
        .map_err(|e| e.to_string())?
        .to_owned();
    Ok(Page {
        content,
        offset,
        next_offset: offset.saturating_add(end as u64),
        file_bytes: metadata.len(),
        eof: bytes.len() == end,
        explicit_range: args.get("offset").is_some_and(|v| !v.is_null()) || requested.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_source_files_are_successful_and_pages_reassemble_without_loss() {
        let root = tempfile::tempdir().unwrap();
        let body = "fn example() { /* 世界 🌿 */ }\n".repeat(40000);
        std::fs::write(root.path().join("large.rs"), &body).unwrap();
        let mut offset = 0;
        let mut collected = String::new();
        loop {
            let page = read(root.path(), "large.rs", &json!({"offset":offset}), 8193).unwrap();
            assert!(page.content.len() <= 8193);
            assert_eq!(page.file_bytes, body.len() as u64);
            collected.push_str(&page.content);
            if page.eof {
                break;
            }
            assert!(page.next_offset > offset);
            offset = page.next_offset;
        }
        assert_eq!(collected, body);
        let page = read(root.path(), "large.rs", &json!({}), 8193).unwrap();
        let result = page.result("large.rs");
        assert!(result.contains("next_offset"));
        assert!(result.contains("successful partial read"));
    }

    #[test]
    fn small_legacy_reads_eof_explicit_ranges_and_long_lines_are_supported() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("small.txt"), "Hej 👋").unwrap();
        assert_eq!(
            read(
                root.path(),
                "small.txt",
                &json!({"offset":null,"max_bytes":null}),
                1024
            )
            .unwrap()
            .result("small.txt"),
            "Hej 👋"
        );
        let page = read(
            root.path(),
            "small.txt",
            &json!({"offset":4,"max_bytes":4}),
            1024,
        )
        .unwrap();
        assert_eq!(page.content, "👋");
        assert!(page.eof);
        assert!(read(root.path(), "small.txt", &json!({"offset":5}), 1024).is_err());
        assert!(
            read(root.path(), "small.txt", &json!({"offset":100}), 1024)
                .unwrap()
                .eof
        );
        let body = "x".repeat(3 * MAX_PAGE_MEMORY);
        std::fs::write(root.path().join("long-line.txt"), body).unwrap();
        let page = read(
            root.path(),
            "long-line.txt",
            &json!({"max_bytes":u64::MAX}),
            32,
        )
        .unwrap();
        assert_eq!(page.content, "x".repeat(32));
        assert!(!page.eof);
        std::fs::write(root.path().join("empty.txt"), "").unwrap();
        assert!(read(root.path(), "empty.txt", &json!({}), 32).unwrap().eof);
    }

    #[test]
    fn pagination_does_not_weaken_paths_or_hide_invalid_text() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("bad.txt"), [0xff, 0xfe]).unwrap();
        for path in ["../outside", "/etc/passwd", "bad.txt", "."] {
            assert!(read(root.path(), path, &json!({}), 1024).is_err());
        }
        for args in [
            json!({"offset":-1}),
            json!({"offset":"10"}),
            json!({"max_bytes":0}),
            json!({"max_bytes":1.2}),
        ] {
            assert!(read(root.path(), "bad.txt", &args, 1024).is_err());
        }
        std::fs::write(root.path().join("incomplete.txt"), [b'x', 0xf0, 0x9f]).unwrap();
        assert!(read(root.path(), "incomplete.txt", &json!({}), 32).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn huge_sparse_files_are_seeked_without_loading_them() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let mut file = std::fs::File::create(root.path().join("huge.txt")).unwrap();
        let length = 16 * 1024 * 1024 * 1024u64;
        file.set_len(length).unwrap();
        file.write_all(&[b'x'; 128]).unwrap();
        file.seek(SeekFrom::Start(length - 128)).unwrap();
        file.write_all(&[b'y'; 128]).unwrap();
        let first = read(root.path(), "huge.txt", &json!({}), 64).unwrap();
        assert_eq!(first.content, "x".repeat(64));
        assert_eq!(first.file_bytes, length);
        assert!(!first.eof);
        let last = read(root.path(), "huge.txt", &json!({"offset":length-64}), 64).unwrap();
        assert_eq!(last.content, "y".repeat(64));
        assert!(last.eof);
    }
}

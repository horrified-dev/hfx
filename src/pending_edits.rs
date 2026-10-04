//! Reconcile saved edit badges with Git without erasing historical diffs.
use crate::{
    git_status::{git, git_with_config},
    state::FileChange,
};
use eframe::egui;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use uuid::Uuid;

const REFRESH: Duration = Duration::from_secs(3);
const TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
#[path = "pending_edits_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub chat: Uuid,
    pub message: Uuid,
    pub activity: usize,
    pub change: FileChange,
}

fn relative(path: &str) -> Option<String> {
    if path.contains('\0') {
        return None;
    }
    let mut pieces = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(piece) => pieces.push(piece.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!pieces.is_empty()).then(|| pieces.join("/"))
}

fn names(bytes: &[u8]) -> HashSet<Vec<u8>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

async fn inspect_inner(root: &Path, edits: &[Edit]) -> Result<Vec<Edit>, String> {
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| error.to_string())?;
    let repository = git(&root, &["rev-parse", "--is-inside-work-tree"]).await?;
    if !repository.status.success() || repository.stdout != b"true\n" {
        return Ok(Vec::new());
    }
    let requested: BTreeSet<_> = edits
        .iter()
        .filter_map(|edit| relative(&edit.change.path))
        .collect();
    if requested.len() > 1024
        || requested.iter().map(|path| path.len() + 1).sum::<usize>() > 32 * 1024
    {
        return Err("Too many pending edit paths to safely check Git".into());
    }
    let mut aliases = HashMap::new();
    let mut paths = BTreeSet::new();
    for original in requested {
        let resolved = match tokio::fs::canonicalize(root.join(&original)).await {
            Ok(path) => path
                .strip_prefix(&root)
                .ok()
                .and_then(Path::to_str)
                .and_then(relative),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if tokio::fs::symlink_metadata(root.join(&original))
                    .await
                    .is_ok_and(|meta| meta.is_symlink())
                {
                    None // A dangling alias does not prove its former target clean.
                } else {
                    Some(original.clone())
                }
            }
            Err(error) => return Err(error.to_string()),
        };
        if let Some(resolved) = resolved {
            paths.insert(resolved.clone());
            aliases.insert(original, resolved);
        }
    }
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if paths.len() > 1024 || paths.iter().map(|path| path.len() + 1).sum::<usize>() > 32 * 1024 {
        return Err("Too many pending edit paths to safely check Git".into());
    }

    // diff may otherwise execute clean/process filters from repository config.
    // Disable every configured driver before inspecting the working tree, and
    // leave filtered files pending rather than claiming their content is clean.
    let filters = git(
        &root,
        &[
            "config",
            "--null",
            "--name-only",
            "--get-regexp",
            "^filter\\..*\\.(clean|smudge|process|required)$",
        ],
    )
    .await?;
    if !filters.status.success() && filters.status.code() != Some(1) {
        return Err("Cannot inspect Git filter configuration".into());
    }
    let mut overrides = Vec::new();
    let mut drivers = HashSet::new();
    for key in filters
        .stdout
        .split(|byte| *byte == 0)
        .filter(|key| !key.is_empty())
    {
        let key = std::str::from_utf8(key).map_err(|_| "Git filter key is not valid UTF-8")?;
        let (driver, _) = key
            .strip_prefix("filter.")
            .and_then(|key| key.rsplit_once('.'))
            .ok_or("Unexpected Git filter key")?;
        drivers.insert(driver.as_bytes().to_vec());
    }
    for driver in &drivers {
        let driver =
            std::str::from_utf8(driver).map_err(|_| "Git filter key is not valid UTF-8")?;
        for field in ["clean", "smudge", "process", "required"] {
            overrides.push(format!(
                "filter.{driver}.{field}={}",
                if field == "required" { "false" } else { "" }
            ));
        }
    }
    async fn query(
        root: &Path,
        flags: &[&str],
        paths: &BTreeSet<String>,
        overrides: &[String],
    ) -> Result<Vec<u8>, String> {
        let args: Vec<_> = flags
            .iter()
            .copied()
            .chain(std::iter::once("--"))
            .chain(paths.iter().map(String::as_str))
            .collect();
        let output = git_with_config(root, &args, overrides).await?;
        if !output.status.success() {
            return Err("Cannot inspect pending Git edits".into());
        }
        Ok(output.stdout)
    }
    let attributes = query(&root, &["check-attr", "-z", "filter"], &paths, &overrides).await?;
    let fields: Vec<_> = attributes.split(|byte| *byte == 0).collect();
    if fields.last() != Some(&&b""[..]) || (fields.len() - 1) % 3 != 0 {
        return Err("Unexpected Git attribute response".into());
    }
    let mut filtered = HashSet::new();
    for entry in fields[..fields.len() - 1].as_chunks::<3>().0 {
        if entry[1] != b"filter" {
            return Err("Unexpected Git attribute name".into());
        }
        if drivers.contains(entry[2]) || !matches!(entry[2], b"unspecified" | b"unset" | b"set") {
            filtered.insert(entry[0].to_vec());
        }
    }
    let paths: BTreeSet<_> = paths
        .into_iter()
        .filter(|path| !filtered.contains(path.as_bytes()))
        .collect();
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let diff_flags = [
        "diff",
        "--relative",
        "--name-only",
        "-z",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "--ignore-submodules=all",
    ];
    let mut dirty = names(&query(&root, &diff_flags, &paths, &overrides).await?);
    let staged_flags: Vec<_> = diff_flags
        .into_iter()
        .chain(std::iter::once("--cached"))
        .collect();
    dirty.extend(names(
        &query(&root, &staged_flags, &paths, &overrides).await?,
    ));
    dirty.extend(names(
        &query(
            &root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            &paths,
            &overrides,
        )
        .await?,
    ));
    let tracked = names(&query(&root, &["ls-files", "--cached", "-z"], &paths, &overrides).await?);
    let ignored = names(
        &query(
            &root,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
            ],
            &paths,
            &overrides,
        )
        .await?,
    );
    let mut clean = HashSet::new();
    for path in &paths {
        if dirty.contains(path.as_bytes()) {
            continue;
        }
        let exists = match tokio::fs::symlink_metadata(root.join(path)).await {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.to_string()),
        };
        // Ignored QA artifacts are outside the Git-pending summary, but their
        // saved diffs remain reviewable. Untracked files in nested repositories
        // are not proven clean merely because the parent ignores gitlink contents.
        if !exists
            || ignored.contains(path.as_bytes())
            || (tracked.contains(path.as_bytes()) && !filtered.contains(path.as_bytes()))
        {
            clean.insert(path.clone());
        }
    }
    Ok(edits
        .iter()
        .filter(|edit| {
            relative(&edit.change.path)
                .and_then(|path| aliases.get(&path))
                .is_some_and(|path| clean.contains(path))
        })
        .cloned()
        .collect())
}

async fn inspect(root: &Path, edits: &[Edit]) -> Result<Vec<Edit>, String> {
    tokio::time::timeout(TIMEOUT, inspect_inner(root, edits))
        .await
        .map_err(|_| "Pending Git edit check timed out".to_owned())?
}

#[derive(Default)]
pub struct Probe {
    root: Option<PathBuf>,
    edits: Vec<Edit>,
    receive: Option<mpsc::Receiver<Result<Vec<Edit>, String>>>,
    task: Option<tokio::task::JoinHandle<()>>,
    next: Option<Instant>,
    disabled: bool,
    pub error: Option<String>,
}

impl Probe {
    pub fn disabled() -> Self {
        Self {
            root: None,
            edits: Vec::new(),
            receive: None,
            task: None,
            next: None,
            disabled: true,
            error: None,
        }
    }

    pub fn update(
        &mut self,
        ctx: &egui::Context,
        runtime: &tokio::runtime::Runtime,
        root: PathBuf,
        edits: Vec<Edit>,
    ) -> Vec<Edit> {
        if self.disabled {
            return Vec::new();
        }
        if self.root.as_ref() != Some(&root) || self.edits != edits {
            self.cancel();
            self.root = Some(root.clone());
            self.edits = edits;
            self.next = None;
            self.error = None;
        }
        if self.edits.is_empty() {
            self.cancel();
            return Vec::new();
        }
        if let Some(receive) = &self.receive {
            match receive.try_recv() {
                Ok(result) => {
                    self.receive = None;
                    self.task = None;
                    self.next = Some(Instant::now() + REFRESH);
                    match result {
                        Ok(clean) => {
                            self.error = None;
                            if !clean.is_empty() {
                                return clean;
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.receive = None;
                    self.task = None;
                    self.next = Some(Instant::now() + REFRESH);
                    self.error = Some("Pending edit check stopped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.receive.is_none() && self.next.is_none_or(|next| Instant::now() >= next) {
            let (send, receive) = mpsc::channel();
            self.receive = Some(receive);
            let input = self.edits.clone();
            let repaint = ctx.clone();
            self.task = Some(runtime.spawn(async move {
                let result = inspect(&root, &input).await;
                let _ = send.send(result);
                repaint.request_repaint();
            }));
        }
        ctx.request_repaint_after(if self.receive.is_some() {
            Duration::from_millis(100)
        } else {
            self.next
                .unwrap_or_else(Instant::now)
                .saturating_duration_since(Instant::now())
        });
        Vec::new()
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

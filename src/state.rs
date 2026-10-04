use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Neutral until the user supplies a hosting profile's verified/noreply email.
/// Never attribute work to an unverified, potentially unrelated real account.
pub const DEFAULT_GIT_COAUTHOR_EMAIL: &str = "hfx@local.invalid";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Codex,
    OpenAI,
    OpenRouter,
    Llama,
    #[default]
    Demo,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::OpenAI => "OpenAI API",
            Self::OpenRouter => "OpenRouter",
            Self::Llama => "llama.cpp",
            Self::Demo => "Demo",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandMode {
    /// Normal user-level shell environment, including Git/SSH and desktop access.
    #[default]
    Trusted,
    Sandbox,
}

impl CommandMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trusted => "Trusted host commands",
            Self::Sandbox => "Strict workspace sandbox",
        }
    }

    pub fn instructions(self) -> &'static str {
        match self {
            Self::Trusted => {
                "Shell commands run on the host as the current user in the selected project, not in a filesystem sandbox. They inherit the host environment available to hfx, including HOME, PATH, CARGO_HOME, RUSTUP_HOME, language/toolchain and package-manager configuration/caches, temporary storage, Git identity/configuration, SSH agent/keys, credential helpers, API keys, proxy/TLS settings, network/localhost and desktop/GPU access. Run normal build, test, run, debug and package-manager commands using this supplied environment; installed tools and ordinary OS permissions still apply. Do not invent sandbox workarounds: do not override HOME, CARGO_HOME, RUSTUP_HOME, TMPDIR or XDG/cache settings, redirect host caches/credentials into .hfx, add --offline, disable networking, or create substitute environments merely because you assume confinement. Respect existing project configuration and explicit user requests; only change the environment to address a verified task-specific need. Git fetch/push are permitted when part of the user's task; use the configured authentication normally. File/image tool path restrictions do not apply to shell commands: when the task needs host paths, use run_command directly without copying host files into the workspace just to access them. Native anonymous web-tool limitations do not restrict shell networking or authenticated CLI tools. Commands can access files outside the project, so keep access and changes scoped to the user's task. Do not print or expose credentials. No extra privilege or sudo is granted. stdin is non-interactive: if real authentication or a tool is missing, explain the specific login/setup needed instead of assuming sandbox restrictions, repeatedly retrying alternative transports or weakening SSH host verification."
            }
            Self::Sandbox => {
                "Shell commands run in a strict Linux filesystem sandbox with only this project writable and private temporary storage. Network access is enabled, but host Git login configuration, SSH credentials/agent and desktop connections are intentionally absent. Run Cargo normally with the supplied environment; Cargo cache setup is handled automatically by the command runner. Do not override CARGO_HOME/RUSTUP_HOME or add --offline merely because this mode is sandboxed; respect project configuration and explicit user requests. If a task needs host capabilities that are hidden, report the limitation and the Trusted host commands setting; do not waste repeated attempts trying to bypass the selected sandbox. /tmp is private to each command; use workspace paths for durable files."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchProvider {
    #[default]
    DuckDuckGo,
    Brave,
    Searxng,
}

impl SearchProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "DuckDuckGo (no key)",
            Self::Brave => "Brave Search API",
            Self::Searxng => "SearXNG server",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub provider: Provider,
    pub codex_model: String,
    pub openai_url: String,
    pub openai_model: String,
    pub llama_url: String,
    pub llama_model: String,
    pub openrouter_url: String,
    pub openrouter_model: String,
    /// Credentials are session-only and never serialized.
    #[serde(skip)]
    pub openai_key: String,
    #[serde(skip)]
    pub llama_key: String,
    #[serde(skip)]
    pub openrouter_key: String,
    pub effort: String,
    pub show_reasoning: bool,
    pub tools_enabled: bool,
    pub commands_enabled: bool,
    pub command_mode: CommandMode,
    pub command_timeout_secs: u64,
    pub web_enabled: bool,
    pub search_provider: SearchProvider,
    pub searxng_url: String,
    #[serde(skip)]
    pub brave_search_key: String,
    pub review_actions: bool,
    /// Explicit user configuration only; never loaded from an untrusted project.
    pub mcp_enabled: bool,
    pub mcp_config: String,
    pub reduced_motion: bool,
    pub desktop_notifications: bool,
    pub completion_sound: bool,
    pub reveal_speed: f32,
    pub font_size: f32,
    pub max_tokens: u32,
    pub auto_compact: bool,
    /// Fallback when provider metadata is unavailable; independent of output tokens.
    pub context_window: u64,
    /// Explicit overrides (older saves may also contain previously discovered limits).
    pub context_windows: std::collections::BTreeMap<String, u64>,
    /// Cached provider metadata, separate so refreshes cannot overwrite manual overrides.
    pub discovered_context_windows: std::collections::BTreeMap<String, u64>,
    pub temperature: f32,
    /// Public commit attribution, not a login credential. The name stays "hfx".
    pub git_coauthor_email: String,
    pub system_prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Demo,
            codex_model: "gpt-6.1-sol".into(),
            openai_url: "https://api.openai.com/v1".into(),
            openai_model: "gpt-6.1-sol".into(),
            llama_url: "http://127.0.0.1:8080/v1".into(),
            llama_model: "local-model".into(),
            openrouter_url: "https://openrouter.ai/api/v1".into(),
            openrouter_model: "openai/gpt-6-luna".into(),
            openai_key: String::new(),
            llama_key: String::new(),
            openrouter_key: String::new(),
            effort: "high".into(),
            show_reasoning: true,
            tools_enabled: true,
            commands_enabled: true,
            command_mode: CommandMode::Trusted,
            command_timeout_secs: 1800,
            web_enabled: true,
            search_provider: SearchProvider::DuckDuckGo,
            searxng_url: String::new(),
            brave_search_key: String::new(),
            review_actions: false,
            mcp_enabled: false,
            mcp_config: "{\n  \"mcpServers\": {}\n}".into(),
            reduced_motion: false,
            desktop_notifications: true,
            completion_sound: true,
            reveal_speed: 130.0,
            font_size: 15.0,
            max_tokens: 8192,
            auto_compact: true,
            context_window: 128_000,
            context_windows: Default::default(),
            discovered_context_windows: Default::default(),
            temperature: 0.7,
            git_coauthor_email: DEFAULT_GIT_COAUTHOR_EMAIL.into(),
            system_prompt: "You are a capable coding assistant. Work carefully in the user's workspace. Use tools to inspect files before proposing changes. Explain results clearly and briefly. Never claim to have run a tool that you have not run.".into(),
        }
    }
}

impl Settings {
    pub fn requires_project_trust(&self) -> bool {
        self.tools_enabled
            && self.command_mode == CommandMode::Trusted
            && (self.commands_enabled || self.mcp_enabled)
    }

    pub fn command_timeout(&self) -> u64 {
        self.command_timeout_secs.clamp(30, 7200)
    }

    pub fn brave_key(&self) -> String {
        if self.brave_search_key.is_empty() {
            std::env::var("BRAVE_SEARCH_API_KEY").unwrap_or_default()
        } else {
            self.brave_search_key.clone()
        }
    }

    /// A single, plain address suitable for a Git trailer (common verified and
    /// numeric-id+username GitHub noreply forms). Never allow extra trailer lines.
    pub fn git_coauthor_email_is_valid(&self) -> bool {
        let email = self.git_coauthor_email.trim();
        let Some((local, domain)) = email.split_once('@') else {
            return false;
        };
        email.len() <= 254
            && !local.is_empty()
            && local.len() <= 64
            && !local.starts_with('.')
            && !local.ends_with('.')
            && !local.contains("..")
            && local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
            && domain.contains('.')
            && domain.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && !part.starts_with('-')
                    && !part.ends_with('-')
                    && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
    }

    pub fn git_coauthor_address(&self) -> &str {
        if self.git_coauthor_email_is_valid() {
            self.git_coauthor_email.trim()
        } else {
            DEFAULT_GIT_COAUTHOR_EMAIL
        }
    }

    pub fn git_coauthor_trailer(&self) -> String {
        format!("Co-authored-by: hfx <{}>", self.git_coauthor_address())
    }

    pub fn context_key(&self) -> String {
        format!(
            "{:?}|{}|{}",
            self.provider,
            if self.provider == Provider::Codex {
                "codex"
            } else {
                self.base_url()
            },
            self.model()
        )
    }
    pub fn context_limit(&self) -> u64 {
        let key = self.context_key();
        self.context_windows
            .get(&key)
            .or_else(|| self.discovered_context_windows.get(&key))
            .copied()
            .unwrap_or(self.context_window)
            .max(1024)
    }

    pub fn context_limit_source(&self) -> &'static str {
        let key = self.context_key();
        if self.context_windows.contains_key(&key) {
            "Saved / manual override"
        } else if self.discovered_context_windows.contains_key(&key) {
            "Provider model metadata"
        } else {
            "Fallback · not reported by provider"
        }
    }

    pub fn model(&self) -> &str {
        match self.provider {
            Provider::Codex => &self.codex_model,
            Provider::OpenAI => &self.openai_model,
            Provider::OpenRouter => &self.openrouter_model,
            Provider::Llama => &self.llama_model,
            Provider::Demo => "Interactive preview",
        }
    }

    pub fn key(&self) -> String {
        let (value, var) = match self.provider {
            Provider::OpenAI => (&self.openai_key, "OPENAI_API_KEY"),
            Provider::OpenRouter => (&self.openrouter_key, "OPENROUTER_API_KEY"),
            Provider::Llama => (&self.llama_key, "LLAMA_API_KEY"),
            Provider::Demo | Provider::Codex => return String::new(),
        };
        if value.is_empty() {
            std::env::var(var).unwrap_or_default()
        } else {
            value.clone()
        }
    }

    pub fn base_url(&self) -> &str {
        match self.provider {
            Provider::OpenRouter => &self.openrouter_url,
            Provider::OpenAI => &self.openai_url,
            _ => &self.llama_url,
        }
    }

    pub fn base_url_mut(&mut self) -> &mut String {
        match self.provider {
            Provider::OpenRouter => &mut self.openrouter_url,
            Provider::OpenAI => &mut self.openai_url,
            _ => &mut self.llama_url,
        }
    }

    pub fn key_mut(&mut self) -> &mut String {
        match self.provider {
            Provider::OpenRouter => &mut self.openrouter_key,
            Provider::OpenAI => &mut self.openai_key,
            _ => &mut self.llama_key,
        }
    }

    pub fn model_mut(&mut self) -> &mut String {
        match self.provider {
            Provider::Codex => &mut self.codex_model,
            Provider::OpenRouter => &mut self.openrouter_model,
            Provider::OpenAI => &mut self.openai_model,
            _ => &mut self.llama_model,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub trusted_path: Option<String>,
}

impl Project {
    pub fn workspace(&self) -> Result<std::path::PathBuf, String> {
        let path = std::path::Path::new(&self.path)
            .canonicalize()
            .map_err(|e| format!("Project is unavailable: {e}"))?;
        if !path.is_dir() {
            return Err("Choose a project directory.".into());
        }
        Ok(path)
    }

    pub fn trusts_workspace(&self, path: &std::path::Path) -> bool {
        self.trusted_path
            .as_ref()
            .is_some_and(|trusted| path == std::path::Path::new(trusted))
    }

    pub fn is_trusted(&self) -> bool {
        self.workspace()
            .is_ok_and(|path| self.trusts_workspace(&path))
    }

    pub fn trust_workspace(&mut self, acknowledged: &std::path::Path) -> Result<(), String> {
        if self.workspace()? != acknowledged {
            return Err("Project directory changed since this dialog opened. Close it and review the new directory before granting trust.".into());
        }
        let path = acknowledged
            .to_str()
            .ok_or("Project directory is not valid UTF-8; trust was not granted.")?;
        self.trusted_path = Some(path.to_owned());
        Ok(())
    }

    #[cfg(test)]
    pub fn trust(&mut self) -> Result<(), String> {
        self.trust_workspace(&self.workspace()?)
    }

    pub fn from_path(path: String) -> Self {
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Workspace")
            .to_owned();
        Self {
            id: Uuid::new_v4(),
            name,
            path,
            trusted_path: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Complete,
    Streaming,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Activity {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub arguments: std::sync::Arc<str>,
    pub result: std::sync::Arc<str>,
    #[serde(default)]
    pub status: ActionStatus,
    #[serde(default)]
    pub elapsed: f32,
    #[serde(default)]
    pub change: Option<FileChange>,
    #[serde(default)]
    pub images: Vec<Attachment>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionStatus {
    #[default]
    Complete,
    Running,
    Waiting,
    Failed,
    Declined,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub diff: std::sync::Arc<str>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub name: String,
    pub size: usize,
    pub content: AttachmentContent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AttachmentContent {
    Text(std::sync::Arc<str>),
    Image {
        base64: std::sync::Arc<str>,
        width: u32,
        height: u32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextCheckpoint {
    pub connection: String,
    pub history: std::sync::Arc<Vec<serde_json::Value>>,
}

/// Lightweight presentation order; text/tool/image data stays in its existing fields.
/// Byte ranges are UTF-8 boundaries, tool/image ranges are stable vector indices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplyBlock {
    Text { start: usize, end: usize },
    Tools { start: usize, end: usize },
    Images { start: usize, end: usize },
    Compacted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub user: bool,
    pub text: String,
    pub reasoning: String,
    #[serde(default)]
    pub reasoning_details: std::sync::Arc<Vec<serde_json::Value>>,
    #[serde(default)]
    pub response_items: std::sync::Arc<Vec<serde_json::Value>>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    pub status: Status,
    pub provider: String,
    pub activities: Vec<Activity>,
    #[serde(default)]
    pub timeline: Vec<ReplyBlock>,
    pub elapsed: f32,
    #[serde(default)]
    pub model_seconds: f32,
    pub tokens: u64,
    #[serde(default)]
    pub context_checkpoint: Option<ContextCheckpoint>,
    #[serde(default)]
    pub context_tokens: u64,
    #[serde(default)]
    pub context_connection: String,
    #[serde(default)]
    pub context_estimated: bool,
    #[serde(skip)]
    pub compacting: bool,
    pub error: Option<String>,
    #[serde(skip)]
    pub born: f64,
}

impl Message {
    pub fn new(user: bool, text: String, now: f64, provider: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            user,
            text,
            reasoning: String::new(),
            reasoning_details: Default::default(),
            response_items: Default::default(),
            attachments: Vec::new(),
            status: Status::Complete,
            provider,
            activities: Vec::new(),
            timeline: Vec::new(),
            elapsed: 0.0,
            model_seconds: 0.0,
            tokens: 0,
            context_checkpoint: None,
            context_tokens: 0,
            context_connection: String::new(),
            context_estimated: false,
            compacting: false,
            error: None,
            born: now,
        }
    }

    // Preserve old chats' existing fallback order when new events arrive (e.g.
    // an interrupted/preview reply); older saves have no exact event chronology.
    fn seed_timeline(&mut self) {
        if !self.timeline.is_empty() {
            return;
        }
        if !self.activities.is_empty() {
            self.timeline.push(ReplyBlock::Tools {
                start: 0,
                end: self.activities.len(),
            });
        }
        if !self.text.trim().is_empty() {
            self.timeline.push(ReplyBlock::Text {
                start: 0,
                end: self.text.len(),
            });
        }
        if !self.attachments.is_empty() {
            self.timeline.push(ReplyBlock::Images {
                start: 0,
                end: self.attachments.len(),
            });
        }
        if self.context_checkpoint.is_some() {
            self.timeline.push(ReplyBlock::Compacted);
        }
    }

    pub fn push_text(&mut self, text: &str) {
        self.seed_timeline();
        self.text.push_str(text);
        if let Some(ReplyBlock::Text { end, .. }) = self.timeline.last_mut() {
            *end = self.text.len();
        } else if !text.trim().is_empty() {
            // Include whitespace-only deltas since the previous text block, but
            // don't let backend paragraph separators split an otherwise continuous
            // group of tools when no actual progress text has been emitted.
            let start = self
                .timeline
                .iter()
                .rev()
                .find_map(|block| match block {
                    ReplyBlock::Text { end, .. } => Some(*end),
                    _ => None,
                })
                .unwrap_or(0);
            self.timeline.push(ReplyBlock::Text {
                start,
                end: self.text.len(),
            });
        }
    }

    pub fn push_activity(&mut self, activity: Activity) {
        self.seed_timeline();
        let index = self.activities.len();
        self.activities.push(activity);
        if let Some(ReplyBlock::Tools { end, .. }) = self.timeline.last_mut() {
            *end = index + 1;
        } else {
            self.timeline.push(ReplyBlock::Tools {
                start: index,
                end: index + 1,
            });
        }
    }

    pub fn push_image(&mut self, image: Attachment) {
        self.seed_timeline();
        let index = self.attachments.len();
        self.attachments.push(image);
        if let Some(ReplyBlock::Images { end, .. }) = self.timeline.last_mut() {
            *end = index + 1;
        } else {
            self.timeline.push(ReplyBlock::Images {
                start: index,
                end: index + 1,
            });
        }
    }

    pub fn record_compaction(&mut self) {
        self.seed_timeline();
        self.timeline.push(ReplyBlock::Compacted);
    }

    /// Invalid/incomplete imported ranges fall back to the full legacy view,
    /// never panic on UTF-8 slicing or silently hide text, tools, or attachments.
    pub fn timeline_valid(&self) -> bool {
        if self.timeline.is_empty() {
            return false;
        }
        let (mut text, mut tools, mut images) = (0, 0, 0);
        for block in &self.timeline {
            match *block {
                ReplyBlock::Text { start, end } => {
                    if end <= start
                        || self.text.get(start..end).is_none()
                        || !self
                            .text
                            .get(text..start)
                            .is_some_and(|gap| gap.trim().is_empty())
                    {
                        return false;
                    }
                    text = end;
                }
                ReplyBlock::Tools { start, end } => {
                    if start != tools || end <= start || end > self.activities.len() {
                        return false;
                    }
                    tools = end;
                }
                ReplyBlock::Images { start, end } => {
                    if start != images || end <= start || end > self.attachments.len() {
                        return false;
                    }
                    images = end;
                }
                ReplyBlock::Compacted => {}
            }
        }
        tools == self.activities.len()
            && images == self.attachments.len()
            && self
                .text
                .get(text..)
                .is_some_and(|tail| tail.trim().is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub id: Uuid,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Transient: an unacknowledged steer stays in the durable queue.
    #[serde(skip)]
    pub dispatched: bool,
}

impl QueuedMessage {
    pub fn new(text: String, attachments: Vec<Attachment>) -> Self {
        Self {
            id: Uuid::new_v4(),
            text,
            attachments,
            dispatched: false,
        }
    }

    pub fn message(&self, now: f64) -> Message {
        let mut message = Message::new(true, self.text.clone(), now, String::new());
        message.id = self.id;
        message.attachments = self.attachments.clone();
        message
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chat {
    pub id: Uuid,
    pub project: Uuid,
    pub title: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub draft: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub queue: Vec<QueuedMessage>,
    #[serde(default)]
    pub queue_paused: bool,
}

impl Chat {
    pub fn new(project: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            project,
            title: "New chat".into(),
            messages: Vec::new(),
            draft: String::new(),
            attachments: Vec::new(),
            queue: Vec::new(),
            queue_paused: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Saved {
    pub settings: Settings,
    pub projects: Vec<Project>,
    pub chats: Vec<Chat>,
    pub selected: Uuid,
}

impl Default for Saved {
    fn default() -> Self {
        let project = Project::from_path(
            std::env::current_dir()
                .unwrap_or_default()
                .display()
                .to_string(),
        );
        let chat = Chat::new(project.id);
        Self {
            settings: Settings::default(),
            projects: vec![project],
            selected: chat.id,
            chats: vec![chat],
        }
    }
}

impl Saved {
    /// Repair references after live chat edits without touching turns or queues.
    /// Restart-only interruption recovery belongs in `restore`, not here.
    pub fn repair_chat_selection(&mut self) {
        if self.projects.is_empty() {
            *self = Self::default();
            return;
        }
        self.chats
            .retain(|c| self.projects.iter().any(|p| p.id == c.project));
        if self.chats.is_empty() {
            self.chats.push(Chat::new(self.projects[0].id));
        }
        if !self.chats.iter().any(|c| c.id == self.selected) {
            self.selected = self.chats[0].id;
        }
    }

    /// Recover durable state at startup, when no backend task is still running.
    pub fn restore(&mut self) {
        self.repair_chat_selection();
        for chat in &mut self.chats {
            if !chat.queue.is_empty() {
                chat.queue_paused = true;
            }
            for queued in &mut chat.queue {
                queued.dispatched = false;
            }
            for message in &mut chat.messages {
                if message.status == Status::Streaming {
                    message.status = Status::Cancelled;
                }
                for activity in &mut message.activities {
                    if matches!(
                        activity.status,
                        ActionStatus::Running | ActionStatus::Waiting
                    ) {
                        activity.status = ActionStatus::Cancelled;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn timeline_activity(name: &str) -> Activity {
        Activity {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            arguments: "{}".into(),
            result: "Result retained".into(),
            status: ActionStatus::Running,
            elapsed: 0.0,
            change: None,
            images: Vec::new(),
        }
    }

    #[test]
    fn reply_timeline_merges_stream_chunks_and_tool_only_rounds_and_survives_restore() {
        let mut message = Message::new(false, String::new(), 0.0, "Demo".into());
        message.status = Status::Streaming;
        message.push_text("I’m reading ");
        message.push_text("🌿 files.");
        let first = message.text.len();
        message.push_activity(timeline_activity("read_file"));
        message.push_activity(timeline_activity("run_command"));
        message.push_text("\n\n");
        message.push_activity(timeline_activity("read_file"));
        assert_eq!(
            message.timeline.len(),
            2,
            "empty separators do not create a progress update"
        );
        message.push_text("\nNow checking results.");
        let second = message.text.len();
        message.push_activity(timeline_activity("write_file"));
        message.record_compaction();
        message.context_checkpoint = Some(ContextCheckpoint {
            connection: "demo".into(),
            history: Default::default(),
        });
        message.push_activity(timeline_activity("run_command"));
        message.push_image(crate::attachments::test_image());
        message.push_image(crate::attachments::test_image());
        message.push_text("\n\nDone — 世界.");
        let expected = vec![
            ReplyBlock::Text {
                start: 0,
                end: first,
            },
            ReplyBlock::Tools { start: 0, end: 3 },
            ReplyBlock::Text {
                start: first,
                end: second,
            },
            ReplyBlock::Tools { start: 3, end: 4 },
            ReplyBlock::Compacted,
            ReplyBlock::Tools { start: 4, end: 5 },
            ReplyBlock::Images { start: 0, end: 2 },
            ReplyBlock::Text {
                start: second,
                end: message.text.len(),
            },
        ];
        assert_eq!(message.timeline, expected);
        assert!(message.timeline_valid());
        let mut saved = Saved::default();
        saved.chats[0].messages.push(message.clone());
        let mut restored: Saved =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        restored.restore();
        let reply = &restored.chats[0].messages[0];
        assert_eq!(reply.timeline, expected);
        assert_eq!(reply.text, message.text);
        assert_eq!(reply.status, Status::Cancelled);
        assert!(
            reply
                .activities
                .iter()
                .all(|a| a.status == ActionStatus::Cancelled
                    && a.result.as_ref() == "Result retained")
        );
        assert_eq!(reply.attachments.len(), 2);
        assert!(reply.timeline_valid());
    }

    #[test]
    fn legacy_replies_and_invalid_timeline_ranges_keep_a_safe_complete_fallback() {
        let mut message = Message::new(false, "👋 Full reply".into(), 0.0, "Demo".into());
        message.activities.push(timeline_activity("read_file"));
        let mut legacy = serde_json::to_value(&message).unwrap();
        legacy.as_object_mut().unwrap().remove("timeline");
        let mut restored: Message = serde_json::from_value(legacy).unwrap();
        assert!(restored.timeline.is_empty());
        assert!(!restored.timeline_valid());
        restored.push_text(" plus a streamed continuation");
        assert!(restored.timeline_valid());
        assert!(matches!(
            restored.timeline[0],
            ReplyBlock::Tools { start: 0, end: 1 }
        ));
        assert!(matches!(
            restored.timeline[1],
            ReplyBlock::Text { start: 0, .. }
        ));
        for invalid in [
            ReplyBlock::Text { start: 1, end: 3 }, // Inside a UTF-8 code point.
            ReplyBlock::Text {
                start: 4,
                end: message.text.len(),
            }, // Hides real text.
            ReplyBlock::Text { start: 3, end: 2 },
            ReplyBlock::Text {
                start: 0,
                end: usize::MAX,
            },
            ReplyBlock::Tools { start: 1, end: 2 },
            ReplyBlock::Images { start: 0, end: 1 },
        ] {
            message.timeline = vec![invalid];
            assert!(!message.timeline_valid());
        }
    }

    #[test]
    fn trusted_commands_and_web_tools_migrate_without_overriding_explicit_choices() {
        let old: Settings = serde_json::from_str(
            r#"{"commands_enabled":false,"review_actions":true,"system_prompt":"custom"}"#,
        )
        .unwrap();
        assert_eq!(old.command_mode, CommandMode::Trusted);
        assert!(old.web_enabled && !old.commands_enabled && old.review_actions);
        assert_eq!(old.command_timeout(), 1800);
        assert_eq!(old.system_prompt, "custom");
        let saved = Settings {
            command_mode: CommandMode::Sandbox,
            command_timeout_secs: u64::MAX,
            web_enabled: false,
            search_provider: SearchProvider::Searxng,
            searxng_url: "http://localhost:8888/search".into(),
            ..Default::default()
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.command_mode, CommandMode::Sandbox);
        assert!(!restored.web_enabled);
        assert_eq!(restored.command_timeout(), 7200);
        assert_eq!(restored.search_provider, SearchProvider::Searxng);
    }

    #[test]
    fn command_instructions_distinguish_host_access_from_file_tool_boundaries() {
        let trusted = CommandMode::Trusted.instructions();
        for guidance in [
            "not in a filesystem sandbox",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "package-manager",
            "Do not invent sandbox workarounds",
            "--offline",
            "File/image tool path restrictions do not apply to shell commands",
            "API keys",
        ] {
            assert!(
                trusted.contains(guidance),
                "Missing trusted guidance: {guidance}"
            );
        }
        let sandbox = CommandMode::Sandbox.instructions();
        assert!(sandbox.contains("strict Linux filesystem sandbox"));
        assert!(sandbox.contains("Cargo cache setup is handled automatically"));
        assert!(sandbox.contains("Network access is enabled"));
        assert!(!sandbox.contains("not in a filesystem sandbox"));
    }

    #[test]
    fn workspace_actions_are_automatic_unless_review_is_explicitly_enabled() {
        let legacy: Settings = serde_json::from_str(r#"{"approve_reads":true}"#).unwrap();
        assert!(!legacy.review_actions);
        assert!(legacy.commands_enabled && legacy.tools_enabled);
        let reviewed: Settings =
            serde_json::from_str(r#"{"review_actions":true,"commands_enabled":false}"#).unwrap();
        assert!(reviewed.review_actions);
        assert!(!reviewed.commands_enabled);
    }
    #[test]
    fn old_history_loads_and_interrupted_actions_restore_as_cancelled() {
        let old: Activity =
            serde_json::from_str(r#"{"name":"read_file","arguments":"{}","result":"old output"}"#)
                .unwrap();
        assert_eq!(old.status, ActionStatus::Complete);
        assert!(old.change.is_none());
        let mut saved = Saved::default();
        let mut message = Message::new(false, String::new(), 0.0, "Demo".into());
        message.status = Status::Streaming;
        message.activities.push(Activity {
            id: "pending".into(),
            name: "ask_user".into(),
            arguments: "{}".into(),
            result: "".into(),
            status: ActionStatus::Waiting,
            elapsed: 0.0,
            change: None,
            images: Vec::new(),
        });
        saved.chats[0].messages.push(message);
        let mut restored: Saved =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        restored.restore();
        assert_eq!(
            restored.chats[0].messages[0].activities[0].status,
            ActionStatus::Cancelled
        );
    }

    #[test]
    fn queued_text_and_images_restore_paused_without_losing_unacknowledged_steering() {
        let mut saved = Saved::default();
        let mut queued = QueuedMessage::new(
            "Steer or follow up later".into(),
            vec![crate::attachments::test_image()],
        );
        let id = queued.id;
        queued.dispatched = true;
        saved.chats[0].queue.push(queued);
        let mut restored: Saved =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        restored.restore();
        assert!(
            restored.chats[0].queue_paused,
            "restart must not execute pending work automatically"
        );
        assert!(!restored.chats[0].queue[0].dispatched);
        assert_eq!(restored.chats[0].queue[0].id, id);
        assert_eq!(restored.chats[0].queue[0].attachments.len(), 1);
        assert_eq!(restored.chats[0].queue[0].message(0.0).id, id);
    }

    #[test]
    fn context_limits_distinguish_metadata_overrides_and_unverified_fallback() {
        let mut settings = Settings {
            provider: Provider::Codex,
            ..Default::default()
        };
        let key = settings.context_key();
        assert_eq!(settings.context_limit(), 128000);
        assert!(settings.context_limit_source().starts_with("Fallback"));
        settings
            .discovered_context_windows
            .insert(key.clone(), 272000);
        assert_eq!(settings.context_limit(), 272000);
        assert_eq!(settings.context_limit_source(), "Provider model metadata");
        assert!(crate::context::at_threshold(
            204000,
            settings.context_limit()
        ));
        assert!(!crate::context::at_threshold(
            203999,
            settings.context_limit()
        ));
        settings.context_windows.insert(key.clone(), 260000);
        settings
            .discovered_context_windows
            .insert(key.clone(), 400000);
        assert_eq!(
            settings.context_limit(),
            260000,
            "metadata cannot overwrite a manual limit"
        );
        assert_eq!(settings.context_limit_source(), "Saved / manual override");
        let mut restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        restored.context_windows.remove(&key);
        assert_eq!(
            restored.context_limit(),
            400000,
            "reset returns to the latest model metadata"
        );
        restored.codex_model = "different-model".into();
        assert_eq!(restored.context_limit(), 128000);
        assert!(restored.context_limit_source().starts_with("Fallback"));
        let legacy: Settings =
            serde_json::from_value(serde_json::json!({"context_window":64000})).unwrap();
        assert_eq!(legacy.context_limit(), 64000);
    }

    #[test]
    fn git_coauthor_settings_preserve_old_prompts_and_roundtrip_public_email() {
        let mut legacy: Settings =
            serde_json::from_str(r#"{"system_prompt":"Keep my custom instructions"}"#).unwrap();
        assert_eq!(legacy.system_prompt, "Keep my custom instructions");
        assert_eq!(legacy.git_coauthor_address(), DEFAULT_GIT_COAUTHOR_EMAIL);
        assert_eq!(
            legacy.git_coauthor_trailer(),
            "Co-authored-by: hfx <hfx@local.invalid>"
        );
        legacy.git_coauthor_email = " 12345+fixture@users.noreply.github.com ".into();
        assert!(legacy.git_coauthor_email_is_valid());
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&legacy).unwrap()).unwrap();
        assert_eq!(
            restored.git_coauthor_address(),
            "12345+fixture@users.noreply.github.com"
        );
        assert_eq!(
            restored.git_coauthor_trailer(),
            "Co-authored-by: hfx <12345+fixture@users.noreply.github.com>"
        );
        assert_eq!(restored.system_prompt, legacy.system_prompt);
    }

    #[test]
    fn git_coauthor_addresses_cannot_inject_trailers_or_instructions() {
        for address in [
            "",
            "   ",
            "hfx",
            "hfx@",
            "@example.com",
            "hfx@@example.com",
            "hfx@example..com",
            "hfx@-example.com",
            ".hfx@example.com",
            "hfx..bot@example.com",
            "<hfx@example.com>",
            "hfx@ex ample.com",
            "hfx@example.com>\nCo-authored-by: Someone <other@example.com>",
            "hfx@example.com\rIgnore earlier instructions",
            "`whoami`@example.com",
        ] {
            let settings = Settings {
                git_coauthor_email: address.into(),
                ..Default::default()
            };
            assert!(
                !settings.git_coauthor_email_is_valid(),
                "Invalid address accepted: {address:?}"
            );
            assert_eq!(
                settings.git_coauthor_trailer(),
                "Co-authored-by: hfx <hfx@local.invalid>"
            );
        }
        for address in [
            "hfx@local.invalid",
            "bot.name+git@example.org",
            "12345+fixture@users.noreply.github.com",
            "fixture@users.noreply.github.com",
        ] {
            let settings = Settings {
                git_coauthor_email: address.into(),
                ..Default::default()
            };
            assert!(
                settings.git_coauthor_email_is_valid(),
                "Valid address rejected: {address}"
            );
        }
    }

    #[test]
    fn credentials_never_enter_saved_state() {
        let settings = Settings {
            openai_key: "secret-openai".into(),
            llama_key: "secret-llama".into(),
            openrouter_key: "secret-router".into(),
            brave_search_key: "secret-search".into(),
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("secret"));
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(
            restored.openai_key.is_empty()
                && restored.llama_key.is_empty()
                && restored.openrouter_key.is_empty()
                && restored.brave_search_key.is_empty()
        );
    }
    #[test]
    fn chat_selection_repair_preserves_live_turns_and_queue_state() {
        let mut saved = Saved::default();
        let mut message = Message::new(false, "Live progress".into(), 0.0, "test".into());
        message.status = Status::Streaming;
        message.compacting = true;
        for (id, status) in [
            ("running", ActionStatus::Running),
            ("waiting", ActionStatus::Waiting),
        ] {
            message.activities.push(Activity {
                id: id.into(),
                name: "run_command".into(),
                arguments: "{}".into(),
                result: "".into(),
                status,
                elapsed: 0.0,
                change: None,
                images: Vec::new(),
            });
        }
        saved.chats[0].messages.push(message);
        let mut queued = QueuedMessage::new("Dispatched guidance".into(), Vec::new());
        queued.dispatched = true;
        saved.chats[0].queue.push(queued);
        let mut paused = Chat::new(saved.projects[0].id);
        paused
            .queue
            .push(QueuedMessage::new("Paused follow-up".into(), Vec::new()));
        paused.queue_paused = true;
        saved.chats.push(paused);
        let live_chats = serde_json::to_value(&saved.chats).unwrap();
        saved.chats.push(Chat::new(Uuid::nil()));
        saved.selected = saved.chats[2].id;

        saved.repair_chat_selection();

        assert_eq!(saved.selected, saved.chats[0].id);
        assert_eq!(serde_json::to_value(&saved.chats).unwrap(), live_chats);
        assert!(saved.chats[0].queue[0].dispatched);
        assert!(!saved.chats[0].queue_paused);
        assert!(saved.chats[1].queue_paused);
    }

    #[test]
    fn chat_selection_repair_creates_a_valid_fallback_when_empty() {
        let mut saved = Saved::default();
        let project = saved.projects[0].id;
        saved.chats.clear();
        saved.selected = Uuid::nil();
        saved.repair_chat_selection();
        assert_eq!(saved.chats.len(), 1);
        assert_eq!(saved.chats[0].project, project);
        assert_eq!(saved.selected, saved.chats[0].id);

        saved.projects.clear();
        saved.repair_chat_selection();
        assert_eq!(saved.projects.len(), 1);
        assert_eq!(saved.chats.len(), 1);
        assert_eq!(saved.chats[0].project, saved.projects[0].id);
        assert_eq!(saved.selected, saved.chats[0].id);
    }

    #[test]
    fn restore_recovers_interrupted_turns_and_invalid_selection() {
        let mut saved = Saved::default();
        let mut message = Message::new(false, "Partial answer".into(), 0.0, "test".into());
        message.status = Status::Streaming;
        saved.chats[0].messages.push(message);
        saved.selected = Uuid::nil();
        saved.restore();
        assert_eq!(saved.selected, saved.chats[0].id);
        assert_eq!(saved.chats[0].messages[0].status, Status::Cancelled);
    }
}

#[cfg(test)]
mod trust_tests {
    use super::*;

    #[test]
    fn legacy_and_new_projects_require_explicit_canonical_path_trust() {
        let root = tempfile::tempdir().unwrap();
        let mut project = Project::from_path(root.path().display().to_string());
        assert!(!project.is_trusted());
        let mut old = serde_json::to_value(&project).unwrap();
        old.as_object_mut().unwrap().remove("trusted_path");
        let legacy: Project = serde_json::from_value(old).unwrap();
        assert!(!legacy.is_trusted());
        project.trust().unwrap();
        assert!(project.is_trusted());
        let restored: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert!(restored.is_trusted());
        let other = tempfile::tempdir().unwrap();
        project.path = other.path().display().to_string();
        assert!(!project.is_trusted());
        project.trusted_path = None;
        assert!(!project.is_trusted());
    }

    #[cfg(unix)]
    #[test]
    fn retargeting_a_project_symlink_does_not_carry_trust_to_another_directory() {
        let parent = tempfile::tempdir().unwrap();
        let first = parent.path().join("first");
        let second = parent.path().join("second");
        let link = parent.path().join("project");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::os::unix::fs::symlink(&first, &link).unwrap();
        let mut project = Project::from_path(link.display().to_string());
        project.trust().unwrap();
        assert!(project.is_trusted());
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&second, &link).unwrap();
        assert!(!project.is_trusted());
    }

    #[test]
    fn host_trust_is_required_even_in_review_mode_but_not_for_confined_or_disabled_tools() {
        let mut settings = Settings::default();
        assert!(settings.requires_project_trust());
        settings.review_actions = true;
        assert!(settings.requires_project_trust());
        settings.commands_enabled = false;
        assert!(!settings.requires_project_trust());
        settings.mcp_enabled = true;
        assert!(settings.requires_project_trust());
        settings.command_mode = CommandMode::Sandbox;
        assert!(!settings.requires_project_trust());
        settings.command_mode = CommandMode::Trusted;
        settings.tools_enabled = false;
        assert!(!settings.requires_project_trust());
    }
}

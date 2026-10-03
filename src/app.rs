use crate::{
    attachments,
    backend::{self, Event, Request},
    codex::{self, Account, Auth, AuthAction, AuthEvent},
    motion::{self, Reveal},
    notifications, persistence, settings_ui as prefs,
    state::{
        ActionStatus, Attachment, AttachmentContent, Chat, Message, Project, Provider,
        QueuedMessage, ReplyBlock, Saved, Settings, Status,
    },
    theme::{self, Icon},
    tools::{self, ToolCall},
};
use eframe::egui::{
    self, Align, Align2, FontId, Frame, Id, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, pos2, vec2,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use uuid::Uuid;

const FOREGROUND_TICK: Duration = Duration::from_millis(16);
const BACKGROUND_TICK: Duration = Duration::from_millis(250);
const EVENT_TICK_BUDGET: Duration = Duration::from_millis(4);
const EVENTS_PER_TICK: usize = 512;
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(5);

fn background_window(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        // run_logic updates viewport info while other input fields may still
        // describe the last painted frame. Prefer the current native viewport.
        input.viewport().visible() == Some(false)
            || !input.viewport().focused.unwrap_or(input.raw.focused)
    })
}

fn tick_interval(ctx: &egui::Context) -> Duration {
    if background_window(ctx) {
        BACKGROUND_TICK
    } else {
        FOREGROUND_TICK
    }
}

struct Active {
    chat: Uuid,
    message: Uuid,
    workspace: PathBuf,
    rx: Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
    started: Instant,
    steering: Option<tokio::sync::mpsc::UnboundedSender<Message>>,
}

struct Pending {
    command_mode: crate::state::CommandMode,
    call: ToolCall,
    reply: oneshot::Sender<bool>,
    workspace: PathBuf,
}

struct PendingQuestion {
    id: String,
    chat: Uuid,
    question: tools::Question,
    deadline: Instant,
    reply: oneshot::Sender<String>,
    selected: usize,
    custom: String,
}

type AttachmentBatch = Result<Vec<Attachment>, String>;

struct ImagePreview {
    image: Attachment,
    texture: Option<egui::TextureHandle>,
    rx: Receiver<Option<egui::ColorImage>>,
    loaded: bool,
    zoom: f32,
}

type ImageSaveResult = Result<Option<PathBuf>, String>;

struct ChatSummary {
    id: Uuid,
    project: Uuid,
    title: String,
}

#[derive(Debug)]
struct ContextMeter {
    used: Option<u64>,
    estimated: bool,
    maximum: u64,
    maximum_source: &'static str,
    compacting: bool,
}

impl ContextMeter {
    fn for_chat(chat: &Chat, settings: &Settings) -> Self {
        // A report from another model/endpoint is not this connection's usage.
        // Keep the last known report while a new turn is awaiting its first event.
        let report = chat
            .messages
            .iter()
            .rev()
            .find(|m| !m.context_connection.is_empty())
            .filter(|m| m.context_connection == settings.context_key());
        Self {
            used: report.map(|m| m.context_tokens),
            estimated: report.is_some_and(|m| m.context_estimated),
            maximum: settings.context_limit(),
            maximum_source: settings.context_limit_source(),
            compacting: chat
                .messages
                .last()
                .is_some_and(|m| m.compacting && m.status == Status::Streaming),
        }
    }

    fn fraction(&self) -> f32 {
        (self.used.unwrap_or(0) as f64 / self.maximum.max(1) as f64).clamp(0.0, 1.0) as f32
    }

    fn usage_text(&self) -> String {
        match self.used {
            Some(used) => format!(
                "{}{} / {} tokens",
                if self.estimated { "~" } else { "" },
                grouped_tokens(used),
                grouped_tokens(self.maximum)
            ),
            None => format!("Maximum context: {} tokens", grouped_tokens(self.maximum)),
        }
    }

    fn show(&self, ui: &mut Ui, settings: &Settings) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        let center = rect.center();
        let radius = 9.0;
        let fraction = self.fraction();
        ui.painter()
            .circle_stroke(center, radius, Stroke::new(2.5, theme::OUTLINE));
        if fraction > 0.0 {
            let color = if fraction >= 0.9 {
                theme::ERROR
            } else {
                theme::ACCENT
            };
            if fraction == 1.0 {
                ui.painter()
                    .circle_stroke(center, radius, Stroke::new(2.5, color));
            } else {
                let segments = (fraction * 64.0).ceil() as usize;
                let points = (0..=segments)
                    .map(|i| {
                        let angle = -std::f32::consts::FRAC_PI_2
                            + std::f32::consts::TAU * fraction * i as f32 / segments as f32;
                        center + vec2(angle.cos(), angle.sin()) * radius
                    })
                    .collect();
                ui.painter()
                    .add(egui::Shape::line(points, Stroke::new(2.5, color)));
            }
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Other,
                true,
                format!("Context usage: {}", self.usage_text()),
            )
        });
        response.on_hover_ui(|ui| {
            ui.label(RichText::new("Context usage").strong().size(12.0));
            ui.label(RichText::new(self.usage_text()).size(12.0));
            ui.label(
                RichText::new(format!("Maximum source: {}", self.maximum_source))
                    .size(10.0)
                    .color(theme::MUTED),
            );
            if let Some(used) = self.used {
                ui.label(
                    RichText::new(format!(
                        "{:.1}% of context window{}",
                        used as f64 / self.maximum.max(1) as f64 * 100.0,
                        if self.estimated {
                            " · estimated"
                        } else {
                            " · provider reported"
                        }
                    ))
                    .size(11.0)
                    .color(theme::MUTED),
                );
            } else {
                ui.label(
                    RichText::new(if settings.provider == Provider::Demo {
                        "Demo mode · no model context consumed."
                    } else {
                        "Usage available after the next model request."
                    })
                    .size(11.0)
                    .color(theme::MUTED),
                );
            }
            ui.label(
                RichText::new(if settings.provider == Provider::Demo {
                    "Connect a provider to track context usage"
                } else if self.compacting {
                    "Compacting context…"
                } else if settings.auto_compact {
                    "Auto-compacts at 75% · full chat retained"
                } else {
                    "Automatic compaction is off"
                })
                .size(10.0)
                .color(theme::DIM),
            );
        })
    }
}

fn batch_finished(success: bool, chat: &Chat) -> bool {
    success && (chat.queue_paused || chat.queue.is_empty())
}

fn grouped_tokens(tokens: u64) -> String {
    let digits = tokens.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    grouped
}

pub struct Harness {
    saved: Saved,
    store: persistence::Store,
    last_autosave: Instant,
    runtime: crate::lifecycle::TaskRuntime,
    active: Option<Active>,
    pending: Option<Pending>,
    question: Option<PendingQuestion>,
    attachment_jobs: Vec<(Uuid, Receiver<AttachmentBatch>)>,
    textures: HashMap<Uuid, egui::TextureHandle>,
    thumbnails: HashMap<Uuid, Receiver<Option<egui::ColorImage>>>,
    changes_open: bool,
    image_preview: Option<ImagePreview>,
    image_save_rx: Option<Receiver<ImageSaveResult>>,
    reveals: HashMap<Uuid, (Reveal, Reveal)>,
    action_targets: HashMap<(Uuid, usize), String>,
    sidebar: bool,
    settings_open: bool,
    settings_tab: usize,
    profile_open: bool,
    search_open: bool,
    search: String,
    project_modal: bool,
    project_path: String,
    project_focus: bool,
    project_error: Option<String>,
    attach_modal: bool,
    attach_path: String,
    rename: Option<(Uuid, String)>,
    rename_focus: bool,
    queue_edit: Option<(Uuid, Uuid, String)>,
    toast: Option<(String, f64)>,
    view_started: f64,
    style_key: (f32, bool),
    probe_rx: Option<Receiver<Result<Vec<backend::ModelInfo>, String>>>,
    mcp_probe: crate::mcp_ui::Probe,
    probe_result: Option<Result<Vec<backend::ModelInfo>, String>>,
    probe_config: String,
    auth_rx: Option<Receiver<AuthEvent>>,
    auth_task: Option<tokio::task::JoinHandle<()>>,
    auth_account: Account,
    auth_models: Vec<String>,
    auth_error: Option<String>,
    auth_url: Option<String>,
    auth: Auth,
    auth_checked: bool,
    focus_composer: bool,
    preview: bool,
    legacy_history_removed: bool,
    capture: Option<(PathBuf, bool)>,
}

impl Harness {
    pub fn new(cc: &eframe::CreationContext<'_>, preview: Option<String>) -> Self {
        let state_path = if preview.is_some() {
            None
        } else {
            eframe::storage_dir("hfx").map(|p| p.join("chats.json"))
        };
        let loaded = state_path
            .as_ref()
            .and_then(|p| persistence::read(p).transpose());
        let mut saved: Saved = if preview.is_some() {
            Saved::default()
        } else if let Some(Ok(saved)) = loaded {
            saved
        } else {
            cc.storage
                .and_then(|storage| eframe::get_value(storage, "hfx.v1"))
                .unwrap_or_default()
        };
        saved.restore();
        theme::install(
            &cc.egui_ctx,
            saved.settings.font_size,
            saved.settings.reduced_motion,
        );
        let style_key = (saved.settings.font_size, saved.settings.reduced_motion);
        let auth_path = if preview.is_some() {
            None
        } else {
            eframe::storage_dir("hfx").map(|path| path.join("codex-auth.json"))
        };
        let (auth, auth_error) = match Auth::new(auth_path.clone()) {
            Ok(auth) => (auth, None),
            Err(error) => (Auth::empty(auth_path), Some(error)),
        };
        let auth_account = auth.account();
        let reveals = saved
            .chats
            .iter()
            .flat_map(|c| c.messages.iter())
            .map(|m| {
                (
                    m.id,
                    (Reveal::complete(&m.text), Reveal::complete(&m.reasoning)),
                )
            })
            .collect();
        let mut app = Self {
            saved,
            store: persistence::Store::new(state_path),
            last_autosave: Instant::now(),
            runtime: crate::lifecycle::TaskRuntime::new(),
            active: None,
            pending: None,
            question: None,
            attachment_jobs: Vec::new(),
            textures: HashMap::new(),
            thumbnails: HashMap::new(),
            changes_open: false,
            image_preview: None,
            image_save_rx: None,
            reveals,
            action_targets: HashMap::new(),
            sidebar: true,
            settings_open: false,
            settings_tab: 0,
            profile_open: false,
            search_open: false,
            search: String::new(),
            project_modal: false,
            project_path: String::new(),
            project_focus: false,
            project_error: None,
            attach_modal: false,
            attach_path: String::new(),
            rename: None,
            rename_focus: false,
            queue_edit: None,
            toast: None,
            view_started: -100.0,
            style_key,
            probe_rx: None,
            mcp_probe: Default::default(),
            probe_result: None,
            probe_config: String::new(),
            auth_rx: None,
            auth_task: None,
            auth_account,
            auth_models: Vec::new(),
            auth_error,
            auth_url: None,
            auth,
            auth_checked: false,
            focus_composer: true,
            preview: preview.is_some(),
            legacy_history_removed: false,
            capture: std::env::args().find_map(|a| {
                a.strip_prefix("--capture=")
                    .map(|p| (PathBuf::from(p), false))
            }),
        };
        if let Some(preview) = preview {
            app.load_preview(&cc.egui_ctx, &preview);
        } else {
            app.store.queue(&app.saved);
        }
        app
    }

    fn selected_index(&self) -> usize {
        self.saved
            .chats
            .iter()
            .position(|c| c.id == self.saved.selected)
            .unwrap_or(0)
    }

    fn project(&self) -> &Project {
        let project = self.saved.chats[self.selected_index()].project;
        self.saved
            .projects
            .iter()
            .find(|p| p.id == project)
            .unwrap_or(&self.saved.projects[0])
    }

    fn select(&mut self, id: Uuid, now: f64) {
        if self.saved.selected != id {
            self.saved.selected = id;
            self.view_started = now;
            self.focus_composer = true;
        }
    }

    fn new_chat(&mut self, project: Uuid, now: f64) {
        if let Some(chat) = self.saved.chats.iter().find(|c| {
            c.project == project
                && c.messages.is_empty()
                && c.draft.trim().is_empty()
                && c.attachments.is_empty()
                && c.queue.is_empty()
                && !self.attachment_jobs.iter().any(|(id, _)| *id == c.id)
        }) {
            let id = chat.id;
            self.select(id, now);
        } else {
            let chat = Chat::new(project);
            let id = chat.id;
            self.saved.chats.push(chat);
            self.select(id, now);
        }
    }

    fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.toast = Some((text.into(), now));
    }

    fn send(&mut self, ctx: &egui::Context) {
        self.submit(ctx, false);
    }

    fn submit(&mut self, ctx: &egui::Context, steer: bool) {
        let index = self.selected_index();
        let chat_id = self.saved.chats[index].id;
        let text = self.saved.chats[index].draft.trim().to_owned();
        if (text.is_empty() && self.saved.chats[index].attachments.is_empty())
            || self.attachment_jobs.iter().any(|(id, _)| *id == chat_id)
        {
            return;
        }
        let chat = &mut self.saved.chats[index];
        let queued = QueuedMessage::new(text, std::mem::take(&mut chat.attachments));
        let id = queued.id;
        if chat.queue.is_empty() {
            chat.queue_paused = false;
        }
        chat.queue.push(queued);
        chat.draft.clear();
        if steer {
            self.steer_queued(chat_id, id, ctx);
        }
        if self.active.is_none() {
            self.start_next(chat_id, ctx);
        }
        self.focus_composer = true;
        self.store.queue(&self.saved);
    }

    fn steer_queued(&mut self, chat_id: Uuid, id: Uuid, ctx: &egui::Context) {
        let Some(active) = self.active.as_ref().filter(|a| a.chat == chat_id) else {
            return;
        };
        let Some(sender) = &active.steering else {
            return;
        };
        let Some(queued) = self
            .saved
            .chats
            .iter_mut()
            .find(|c| c.id == chat_id)
            .and_then(|c| c.queue.iter_mut().find(|q| q.id == id && !q.dispatched))
        else {
            return;
        };
        if sender.send(queued.message(ctx.input(|i| i.time))).is_ok() {
            queued.dispatched = true;
        }
        // If the stream already completed, leave it queued for a normal follow-up.
    }

    fn start_next(&mut self, chat_id: Uuid, ctx: &egui::Context) -> bool {
        if self.active.is_some() {
            return false;
        }
        let Some(index) = self.saved.chats.iter().position(|c| c.id == chat_id) else {
            return false;
        };
        let chat = &self.saved.chats[index];
        let Some(queued) = chat.queue.first() else {
            return false;
        };
        if chat.queue_paused
            || self
                .queue_edit
                .as_ref()
                .is_some_and(|(chat, id, _)| *chat == chat_id && *id == queued.id)
        {
            return false;
        }
        let now = ctx.input(|i| i.time);
        let settings = self.saved.settings.clone();
        if settings.provider == Provider::Codex
            && (!self.auth.account().signed_in || self.auth_url.is_some())
        {
            self.saved.chats[index].queue_paused = true;
            self.settings_open = true;
            self.settings_tab = 0;
            self.notify(
                "Complete OpenAI sign-in, then resume the queued message",
                now,
            );
            return false;
        }
        let project = self.saved.chats[index].project;
        let Some(workspace) = self
            .saved
            .projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| PathBuf::from(&p.path))
        else {
            return false;
        };
        let chat = &mut self.saved.chats[index];
        let queued = chat.queue.remove(0);
        let user = queued.message(now);
        if chat.messages.is_empty() {
            chat.title = if user.text.is_empty() {
                user.attachments
                    .first()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "Attachments".into())
            } else {
                user.text
                    .lines()
                    .next()
                    .unwrap_or("New chat")
                    .chars()
                    .take(42)
                    .collect()
            };
        }
        self.reveals
            .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
        chat.messages.push(user);
        let history = chat.messages.clone();
        let session = chat.id.to_string();
        let codex_auth = (settings.provider == Provider::Codex).then(|| self.auth.clone());
        let mut message = Message::new(false, String::new(), now, settings.provider.label().into());
        message.status = Status::Streaming;
        let id = message.id;
        self.reveals
            .insert(id, (Reveal::default(), Reveal::default()));
        chat.messages.push(message);
        let (tx, rx) = mpsc::channel();
        let (steer_tx, steer_rx) = tokio::sync::mpsc::unbounded_channel();
        let real_steering = settings.provider != Provider::Demo;
        let repaint = ctx.clone();
        let request_workspace = workspace.clone();
        let task = self.runtime.spawn(async move {
            backend::run_with_steering(
                Request {
                    settings,
                    workspace: request_workspace,
                    history,
                    session,
                    codex_auth,
                },
                tx,
                Some(steer_rx),
            )
            .await;
            repaint.request_repaint();
        });
        self.active = Some(Active {
            chat: chat_id,
            message: id,
            workspace,
            rx,
            task,
            started: Instant::now(),
            steering: real_steering.then_some(steer_tx),
        });
        if chat_id == self.saved.selected {
            self.focus_composer = true;
        }
        self.store.queue(&self.saved);
        true
    }

    fn pause_queues(&mut self) {
        for chat in &mut self.saved.chats {
            if !chat.queue.is_empty() {
                chat.queue_paused = true;
            }
            for queued in &mut chat.queue {
                queued.dispatched = false;
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.task.abort();
            // Commit already acknowledged steering/tool results before cancelling.
            while let Ok(event) = active.rx.try_recv() {
                self.apply_event(&mut active, event);
            }
            if let Some(message) = self
                .saved
                .chats
                .iter_mut()
                .find(|c| c.id == active.chat)
                .and_then(|c| c.messages.iter_mut().find(|m| m.id == active.message))
            {
                if message.status == Status::Streaming {
                    message.status = Status::Cancelled;
                }
                message.compacting = false;
                message.elapsed = active.started.elapsed().as_secs_f32();
                cancel_actions(message);
            }
        }
        self.pause_queues();
        self.pending = None;
        self.question = None;
    }

    fn apply_event(&mut self, active: &mut Active, event: Event) -> Option<bool> {
        if let Event::Steered(users) = event {
            let chat = self.saved.chats.iter_mut().find(|c| c.id == active.chat)?;
            let prior = chat.messages.iter_mut().find(|m| m.id == active.message)?;
            let provider = prior.provider.clone();
            prior.status = Status::Complete;
            prior.compacting = false;
            prior.elapsed = active.started.elapsed().as_secs_f32();
            let empty = prior.text.is_empty()
                && prior.reasoning.is_empty()
                && prior.activities.is_empty()
                && prior.attachments.is_empty()
                && prior.context_checkpoint.is_none()
                && prior.response_items.is_empty();
            if empty {
                chat.messages.retain(|m| m.id != active.message);
                self.reveals.remove(&active.message);
            }
            let born = users.last().map_or(0.0, |user| user.born);
            for user in users {
                chat.queue.retain(|q| q.id != user.id);
                self.reveals
                    .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
                chat.messages.push(user);
            }
            let mut reply = Message::new(false, String::new(), born, provider);
            reply.status = Status::Streaming;
            active.message = reply.id;
            active.started = Instant::now();
            self.reveals
                .insert(reply.id, (Reveal::default(), Reveal::default()));
            chat.messages.push(reply);
            self.store.queue(&self.saved);
            return None;
        }
        let message = self
            .saved
            .chats
            .iter_mut()
            .find(|c| c.id == active.chat)
            .and_then(|c| c.messages.iter_mut().find(|m| m.id == active.message))?;
        message.elapsed = active.started.elapsed().as_secs_f32();
        match event {
            Event::Steered(_) => unreachable!(),
            Event::Text(t) => message.push_text(&t),
            Event::Reasoning(t) => message.reasoning.push_str(&t),
            Event::ReasoningDetails(details) => {
                std::sync::Arc::make_mut(&mut message.reasoning_details).extend(details)
            }
            Event::ResponsesContext(items) => message.response_items = items.into(),
            Event::Usage(n) => message.tokens += n,
            Event::ModelTime(seconds) => message.model_seconds += seconds,
            Event::ContextUsage {
                tokens,
                estimated,
                connection,
            } => {
                message.context_connection = connection;
                message.context_tokens = tokens;
                message.context_estimated = estimated;
            }
            Event::CompactionStarted => message.compacting = true,
            Event::Compacted(checkpoint) => {
                message.record_compaction();
                message.context_checkpoint = Some(checkpoint);
                message.response_items = Default::default();
                message.compacting = false;
            }
            Event::Image(image) => message.push_image(image),
            Event::ToolStarted(call) => message.push_activity(crate::state::Activity {
                id: call.id,
                name: call.name,
                arguments: call.arguments.into(),
                result: "".into(),
                status: ActionStatus::Running,
                elapsed: 0.0,
                change: None,
                images: Vec::new(),
            }),
            Event::ToolRunning(id) => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == id) {
                    activity.status = ActionStatus::Running;
                }
            }
            Event::ToolFinished {
                id,
                output,
                elapsed,
                show_in_reply,
            } => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == id) {
                    activity.result = output.text.into();
                    activity.status = output.status;
                    activity.elapsed = elapsed;
                    activity.change = output.change;
                    activity.images = output.images.clone();
                }
                if show_in_reply {
                    for image in output.images {
                        message.push_image(image);
                    }
                }
                if self.question.as_ref().is_some_and(|q| q.id == id) {
                    self.question = None;
                }
            }
            Event::AskUser {
                id,
                question,
                deadline,
                reply,
            } => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == id) {
                    activity.status = ActionStatus::Waiting;
                }
                let selected = question.recommended();
                self.question = Some(PendingQuestion {
                    id,
                    chat: active.chat,
                    question,
                    deadline,
                    reply,
                    selected,
                    custom: String::new(),
                });
            }
            Event::Approval {
                call,
                reply,
                command_mode,
            } => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == call.id) {
                    activity.status = ActionStatus::Waiting;
                }
                self.pending = Some(Pending {
                    command_mode,
                    call,
                    reply,
                    workspace: active.workspace.clone(),
                })
            }
            Event::Completed => {
                message.compacting = false;
                message.status = Status::Complete;
                return Some(true);
            }
            Event::Error(error) => {
                message.compacting = false;
                message.status = Status::Failed;
                message.error = Some(error);
                cancel_actions(message);
                return Some(false);
            }
        }
        None
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(error) = self.store.poll() {
            self.notify(error, ctx.input(|i| i.time));
        }
        let interval = tick_interval(ctx);
        let mut preferred = None;
        if let Some(mut active) = self.active.take() {
            let mut outcome = None;
            let tick_started = Instant::now();
            for index in 0..EVENTS_PER_TICK {
                // A burst accumulated while hidden must not monopolize the
                // window thread. This is a soft budget: finish the current event.
                if index > 0 && tick_started.elapsed() >= EVENT_TICK_BUDGET {
                    break;
                }
                match active.rx.try_recv() {
                    Ok(event) => {
                        outcome = self.apply_event(&mut active, event);
                        if outcome.is_some() {
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.apply_event(
                            &mut active,
                            Event::Error("The generation task ended unexpectedly.".into()),
                        );
                        outcome = Some(false);
                        break;
                    }
                }
            }
            if let Some(success) = outcome {
                for queued in self
                    .saved
                    .chats
                    .iter_mut()
                    .filter(|c| c.id == active.chat)
                    .flat_map(|c| &mut c.queue)
                {
                    queued.dispatched = false;
                }
                self.pending = None;
                self.question = None;
                if success {
                    preferred = Some(active.chat);
                } else {
                    self.pause_queues();
                }
                if !self.preview
                    && self
                        .saved
                        .chats
                        .iter()
                        .find(|chat| chat.id == active.chat)
                        .is_some_and(|chat| batch_finished(success, chat))
                {
                    self.notify("Work complete", ctx.input(|i| i.time));
                    notifications::completed(
                        self.saved.settings.desktop_notifications,
                        self.saved.settings.completion_sound,
                    );
                }
                // Hidden eframe frames may never reach its UI autosave path.
                // Persist terminal state even if no further repaint is needed.
                self.store.queue(&self.saved);
            } else {
                self.active = Some(active);
                ctx.request_repaint_after(interval);
            }
        }
        if self.active.is_none() {
            let next = preferred
                .filter(|id| {
                    self.saved
                        .chats
                        .iter()
                        .any(|c| c.id == *id && !c.queue.is_empty() && !c.queue_paused)
                })
                .or_else(|| {
                    self.saved
                        .chats
                        .iter()
                        .find(|c| !c.queue.is_empty() && !c.queue_paused)
                        .map(|c| c.id)
                });
            if let Some(chat) = next {
                self.start_next(chat, ctx);
            }
        }
        self.poll_attachments(ctx);
        if self
            .question
            .as_ref()
            .is_some_and(|q| Instant::now() >= q.deadline)
            && let Some(question) = self.question.take()
        {
            let answer = question
                .question
                .answer(question.question.recommended(), true);
            let _ = question.reply.send(answer);
        }
        if let Some(question) = &self.question {
            ctx.request_repaint_after(
                interval.min(question.deadline.saturating_duration_since(Instant::now())),
            );
        }
        if let Some(rx) = &self.auth_rx {
            let mut done = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    AuthEvent::Snapshot {
                        account,
                        models,
                        error,
                    } => {
                        self.auth_account = account;
                        self.auth_models = models;
                        for (model, limit) in self.auth.context_windows() {
                            let mut settings = self.saved.settings.clone();
                            settings.provider = Provider::Codex;
                            settings.codex_model = model;
                            self.saved
                                .settings
                                .discovered_context_windows
                                .insert(settings.context_key(), limit);
                        }
                        self.auth_error = error;
                        self.auth_url = None;
                        done = true;
                    }
                    AuthEvent::LoginUrl(url) => {
                        if codex::valid_login_url(&url) {
                            ctx.open_url(egui::OpenUrl::new_tab(url.clone()));
                            self.auth_url = Some(url);
                        }
                    }
                    AuthEvent::Error(error) => {
                        self.auth_error = Some(error);
                        self.auth_url = None;
                        done = true;
                    }
                }
            }
            if done {
                self.auth_rx = None;
                self.auth_task = None;
            } else {
                ctx.request_repaint_after(interval.max(Duration::from_millis(80)));
            }
        }
        if !self.preview {
            self.auth_account = self.auth.account();
        }
        let dt = ctx.input(|i| i.stable_dt);
        let instant = self.saved.settings.reduced_motion || background_window(ctx);
        let mut animating = false;
        for chat in &self.saved.chats {
            for message in &chat.messages {
                let (text, reasoning) = self.reveals.entry(message.id).or_insert_with(|| {
                    (
                        Reveal::complete(&message.text),
                        Reveal::complete(&message.reasoning),
                    )
                });
                animating |=
                    text.step(&message.text, dt, self.saved.settings.reveal_speed, instant);
                animating |= reasoning.step(
                    &message.reasoning,
                    dt,
                    self.saved.settings.reveal_speed,
                    instant,
                );
            }
        }
        if animating {
            ctx.request_repaint_after(interval);
        }
        self.mcp_probe.poll(ctx, interval);
        if let Some(rx) = &self.probe_rx {
            if let Ok(result) = rx.try_recv() {
                self.probe_result = Some(result);
                self.probe_rx = None;
            } else {
                ctx.request_repaint_after(interval.max(Duration::from_millis(60)));
            }
        }
    }

    fn titlebar(&mut self, ui: &mut Ui, now: f64) {
        egui::Panel::top("titlebar")
            .exact_size(38.0)
            .frame(
                Frame::NONE
                    .fill(theme::RAIL)
                    .inner_margin(Margin::symmetric(12, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(3.0);
                    // Flat, frameless menu bar buttons.
                    let widgets = &mut ui.visuals_mut().widgets;
                    for w in [&mut widgets.inactive, &mut widgets.open] {
                        w.weak_bg_fill = egui::Color32::TRANSPARENT;
                        w.bg_stroke = Stroke::NONE;
                    }
                    ui.label(RichText::new("hfx").size(16.0).strong().color(theme::TEXT));
                    ui.add_space(18.0);
                    ui.menu_button("File", |ui| {
                        if ui.button("New chat       Ctrl+N").clicked() {
                            self.new_chat(self.project().id, now);
                            ui.close();
                        }
                        if ui.button("Add project").clicked() {
                            self.project_modal = true;
                            self.project_focus = true;
                            self.project_error = None;
                            ui.close();
                        }
                        if ui.button("Attach context file").clicked() {
                            self.attach_modal = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Quit").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("View", |ui| {
                        ui.checkbox(&mut self.sidebar, "Project sidebar");
                        ui.checkbox(&mut self.settings_open, "Settings");
                        ui.checkbox(
                            &mut self.saved.settings.show_reasoning,
                            "Reasoning summaries",
                        );
                        ui.checkbox(&mut self.saved.settings.reduced_motion, "Reduce motion");
                    });
                    ui.menu_button("Help", |ui| {
                        ui.label("hfx 0.1.0 · Rust + egui");
                        ui.label("Enter: send · Shift+Enter: new line");
                        ui.label("Ctrl+N: new chat · Ctrl+,: settings");
                        ui.label("Escape: close panel / stop generation");
                        ui.hyperlink_to(
                            "Setup & source documentation",
                            "https://developers.openai.com/api/docs/guides/reasoning",
                        );
                    });
                    let drag_width = (ui.available_width() - 102.0).max(0.0);
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(drag_width, 30.0), Sense::click_and_drag());
                    if response.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if response.double_clicked() {
                        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if rect.width() > 210.0 {
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "A little space to make something good",
                            FontId::proportional(11.0),
                            theme::DIM,
                        );
                    }
                    if theme::icon_button(ui, Icon::Minus, "Minimize", false, 28.0).clicked() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    if theme::icon_button(ui, Icon::Maximize, "Maximize", false, 28.0).clicked() {
                        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    if theme::icon_button(ui, Icon::Close, "Close", false, 28.0).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
    }

    fn rail(&mut self, ui: &mut Ui, now: f64) {
        egui::Panel::left("rail")
            .exact_size(58.0)
            .resizable(false)
            .frame(
                Frame::NONE
                    .fill(theme::RAIL)
                    .inner_margin(Margin::symmetric(10, 16)),
            )
            .show(ui, |ui| {
                if theme::icon_button(ui, Icon::Home, "Workspace", true, 38.0).clicked() {
                    self.sidebar = true;
                }
                ui.add_space(12.0);
                if theme::icon_button(ui, Icon::Plus, "New chat", false, 38.0).clicked() {
                    self.new_chat(self.project().id, now);
                }
                if theme::icon_button(ui, Icon::Clock, "Search chats", self.search_open, 38.0)
                    .clicked()
                {
                    self.sidebar = true;
                    self.search_open = !self.search_open;
                }
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                if theme::icon_button(ui, Icon::Sidebar, "Toggle sidebar", self.sidebar, 38.0)
                    .clicked()
                {
                    self.sidebar = !self.sidebar;
                }
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    let (rect, response) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
                    ui.painter().circle_filled(
                        rect.center(),
                        16.0,
                        theme::ACCENT.gamma_multiply(0.18),
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "H",
                        FontId::proportional(13.0),
                        theme::ACCENT,
                    );
                    if response.clicked() {
                        self.profile_open = !self.profile_open;
                    }
                    response.on_hover_text("Workspace menu");
                    ui.add_space(10.0);
                    if theme::icon_button(
                        ui,
                        Icon::Settings,
                        "Settings · Ctrl+,",
                        self.settings_open,
                        38.0,
                    )
                    .clicked()
                    {
                        self.settings_open = !self.settings_open;
                    }
                });
            });
    }

    fn sidebar_panel(&mut self, ui: &mut Ui, now: f64) {
        let width = ui.ctx().content_rect().width();
        let mut visible = self.sidebar && !(self.settings_open && width < 1100.0);
        egui::Panel::left("projects")
            .exact_size(236.0)
            .resizable(false)
            .frame(
                Frame::NONE
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::symmetric(14, 18)),
            )
            .show_collapsible(ui, &mut visible, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Workspace").size(16.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(
                            ui,
                            Icon::Search,
                            "Search chats",
                            self.search_open,
                            28.0,
                        )
                        .clicked()
                        {
                            self.search_open = !self.search_open;
                        }
                    });
                });
                ui.add_space(18.0);
                if theme::row(ui, "New chat", Some(Icon::Plus), false).clicked() {
                    self.new_chat(self.project().id, now);
                }
                if self.search_open {
                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Find a conversation…")
                            .desired_width(f32::INFINITY),
                    );
                }
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PROJECTS").size(10.0).color(theme::DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(ui, Icon::Plus, "Add project", false, 24.0).clicked()
                        {
                            self.project_modal = true;
                            self.project_focus = true;
                            self.project_error = None;
                        }
                    });
                });
                let selected_project = self.project().id;
                let projects = self.saved.projects.clone();
                let chats = self
                    .saved
                    .chats
                    .iter()
                    .filter(|c| !c.messages.is_empty())
                    .map(|c| ChatSummary {
                        id: c.id,
                        project: c.project,
                        title: c.title.clone(),
                    })
                    .collect::<Vec<_>>();
                let mut delete = None;
                ScrollArea::vertical()
                    .id_salt("sidebar_scroll")
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - 72.0).max(100.0))
                    .show(ui, |ui| {
                        for project in &projects {
                            let response = ui
                                .push_id(project.id, |ui| {
                                    theme::row(
                                        ui,
                                        &project.name,
                                        Some(Icon::Folder),
                                        selected_project == project.id,
                                    )
                                })
                                .inner;
                            if response.clicked() {
                                self.new_chat(project.id, now);
                            }
                            response.on_hover_text(&project.path);
                            let matching = chats
                                .iter()
                                .rev()
                                .filter(|c| {
                                    c.project == project.id
                                        && c.title
                                            .to_lowercase()
                                            .contains(&self.search.to_lowercase())
                                })
                                .collect::<Vec<_>>();
                            if matching.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(34.0);
                                    ui.label(
                                        RichText::new(if self.search.is_empty() {
                                            "No chats yet"
                                        } else {
                                            "No matches"
                                        })
                                        .size(12.0)
                                        .color(theme::DIM),
                                    );
                                });
                            }
                            for chat in matching.iter().take(12) {
                                ui.horizontal(|ui| {
                                    ui.add_space(22.0);
                                    let response = ui
                                        .push_id(chat.id, |ui| {
                                            theme::row(
                                                ui,
                                                &chat.title,
                                                None,
                                                self.saved.selected == chat.id,
                                            )
                                        })
                                        .inner;
                                    if response.clicked() {
                                        self.select(chat.id, now);
                                    }
                                    egui::Popup::context_menu(&response)
                                        .frame(theme::chat_menu_frame(ui.style()))
                                        .show(|ui| {
                                            ui.set_width(176.0);
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            if theme::menu_action(
                                                ui,
                                                "Rename chat",
                                                Icon::Pencil,
                                                false,
                                            )
                                            .clicked()
                                            {
                                                self.rename = Some((chat.id, chat.title.clone()));
                                                self.rename_focus = true;
                                                ui.close();
                                            }
                                            theme::menu_divider(ui);
                                            if ui
                                                .add_enabled_ui(
                                                    self.active
                                                        .as_ref()
                                                        .is_none_or(|a| a.chat != chat.id),
                                                    |ui| {
                                                        theme::menu_action(
                                                            ui,
                                                            "Delete chat",
                                                            Icon::Trash,
                                                            true,
                                                        )
                                                    },
                                                )
                                                .inner
                                                .on_disabled_hover_text(
                                                    "Stop this chat before deleting it",
                                                )
                                                .clicked()
                                            {
                                                delete = Some(chat.id);
                                                ui.close();
                                            }
                                        });
                                });
                            }
                            ui.add_space(14.0);
                        }
                        if self.saved.chats.len() > 1 {
                            theme::section(ui, "RECENT CONVERSATIONS");
                            let search = self.search.to_lowercase();
                            for chat in chats
                                .iter()
                                .rev()
                                .filter(|c| c.title.to_lowercase().contains(&search))
                                .take(8)
                            {
                                if ui
                                    .push_id(("recent", chat.id), |ui| {
                                        theme::row(ui, &chat.title, None, false)
                                    })
                                    .inner
                                    .clicked()
                                {
                                    self.select(chat.id, now);
                                }
                            }
                        }
                    });
                if let Some(id) = delete {
                    self.saved.chats.retain(|c| c.id != id);
                    self.saved.repair_chat_selection();
                    self.reveals.retain(|id, _| {
                        self.saved
                            .chats
                            .iter()
                            .any(|c| c.messages.iter().any(|m| m.id == *id))
                    });
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.label(RichText::new("LOCAL WORKSPACE").size(9.0).color(theme::DIM));
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter()
                            .circle_filled(rect.center(), 3.0, theme::ACCENT);
                        ui.label(
                            RichText::new(self.saved.settings.provider.label())
                                .size(11.0)
                                .color(theme::MUTED),
                        );
                    });
                });
            });
    }

    fn main_panel(&mut self, ui: &mut Ui, now: f64) {
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(theme::BG))
            .show(ui, |ui| {
                egui::Panel::top("chat_header")
                    .exact_size(64.0)
                    .frame(Frame::NONE.inner_margin(Margin::symmetric(28, 14)))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            theme::inline_icon(ui, Icon::Folder, 15.0, theme::MUTED);
                            ui.label(RichText::new(&self.project().name).size(14.0));
                            ui.add_space(6.0);
                            theme::badge(ui, "LOCAL", theme::MUTED);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if theme::icon_button(
                                    ui,
                                    Icon::Settings,
                                    "Open settings",
                                    false,
                                    28.0,
                                )
                                .clicked()
                                {
                                    self.settings_open = !self.settings_open;
                                }
                                if ui.available_width() > 120.0 {
                                    theme::badge(
                                        ui,
                                        if self.saved.settings.provider == Provider::Demo {
                                            "PREVIEW MODE"
                                        } else {
                                            "READY TO CREATE"
                                        },
                                        theme::ACCENT,
                                    );
                                }
                            });
                        });
                    });
                let selected = self.selected_index();
                let tight = ui.ctx().content_rect().height() < 650.0
                    && self
                        .question
                        .as_ref()
                        .is_some_and(|q| q.chat == self.saved.selected);
                let attachment_height = if self.saved.chats[selected].attachments.is_empty() {
                    0.0
                } else {
                    if tight { 48.0 } else { 90.0 }
                };
                let change_height = if changed_totals(&self.saved.chats[selected]).0 == 0 {
                    0.0
                } else {
                    36.0
                };
                let question_height = if self
                    .question
                    .as_ref()
                    .is_some_and(|q| q.chat == self.saved.selected)
                {
                    220.0
                } else {
                    0.0
                };
                let composer_height = ((if tight { 160.0_f32 } else { 242.0_f32 })
                    + attachment_height
                    + self.queue_height()
                    + change_height
                    + question_height)
                    .min(ui.available_height() - 4.0)
                    .max(190.0);
                egui::Panel::bottom("composer_panel")
                    .exact_size(composer_height)
                    .show_separator_line(false)
                    .frame(Frame::NONE.inner_margin(Margin {
                        left: 24,
                        right: 24,
                        top: 6,
                        bottom: 18,
                    }))
                    .show(ui, |ui| {
                        ScrollArea::vertical()
                            .id_salt("composer_scroll")
                            .show(ui, |ui| {
                                self.question_card(ui);
                                self.change_badge(ui);
                                self.composer(ui);
                            });
                    });
                egui::CentralPanel::default()
                    .frame(Frame::NONE.inner_margin(Margin::symmetric(24, 0)))
                    .show(ui, |ui| {
                        let empty = self.saved.chats[self.selected_index()].messages.is_empty();
                        let opacity = if self.saved.settings.reduced_motion {
                            1.0
                        } else {
                            motion::ease(((now - self.view_started) / 0.28) as f32)
                        };
                        let welcome_alpha = ui.ctx().animate_bool_with_time(
                            Id::new("welcome_alpha"),
                            empty,
                            if self.saved.settings.reduced_motion {
                                0.0
                            } else {
                                0.28
                            },
                        );
                        let rect = ui.available_rect_before_wrap();
                        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                        if welcome_alpha > 0.001 {
                            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                                ui.set_opacity(welcome_alpha * opacity);
                                if !empty {
                                    ui.disable();
                                }
                                self.welcome(ui, now);
                            });
                        }
                        if !empty {
                            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                                ui.set_opacity((1.0 - welcome_alpha) * opacity);
                                self.conversation(ui, now);
                            });
                        }
                        if opacity < 1.0 {
                            ui.ctx().request_repaint();
                        }
                    });
            });
    }

    fn welcome(&mut self, ui: &mut Ui, now: f64) {
        let height = ui.available_height();
        let compact = height < 350.0;
        let cards = ui.available_width() > 490.0 && height >= 390.0;
        let desired = if compact {
            192.0
        } else if cards {
            360.0
        } else {
            260.0
        };
        ui.add_space(((height - desired) * 0.5).max(0.0));
        let width = ui.available_width();
        ui.vertical_centered(|ui| {
            theme::logo(
                ui,
                if compact { 44.0 } else { 64.0 },
                now,
                !self.saved.settings.reduced_motion,
            );
            ui.add_space(if compact { 10.0 } else { 24.0 });
            ui.label(
                RichText::new("A SPACE FOR YOUR NEXT IDEA")
                    .size(10.0)
                    .color(theme::DIM),
            );
            ui.add_space(if compact { 6.0 } else { 12.0 });
            ui.label(
                RichText::new("What should we work on?")
                    .size(if width < 450.0 { 25.0 } else { 31.0 })
                    .color(theme::TEXT),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("Make something good in {}.", self.project().name))
                    .size(14.0)
                    .color(theme::MUTED),
            );
            ui.add_space(if compact { 14.0 } else { 32.0 });
        });
        let suggestions = [
            (
                Icon::Terminal,
                "Explore the codebase",
                "Map out the project",
                "Explore this codebase. Explain its structure and suggest a useful next step.",
            ),
            (
                Icon::Spark,
                "Build something new",
                "From idea to first draft",
                "Help me plan and build a new feature in this project.",
            ),
            (
                Icon::Search,
                "Find an improvement",
                "A fresh pair of eyes",
                "Review this project and find one meaningful improvement. Inspect files before proposing a change.",
            ),
        ];
        if cards {
            let content = width.min(680.0);
            ui.horizontal(|ui| {
                ui.add_space((width - content) / 2.0);
                ui.spacing_mut().item_spacing.x = 10.0;
                for (glyph, title, subtitle, prompt) in suggestions {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2((content - 20.0) / 3.0, 92.0), Sense::click());
                    let hover = ui.ctx().animate_bool_with_time(
                        response.id.with("fade"),
                        response.hovered(),
                        0.18,
                    );
                    ui.painter().rect_filled(
                        rect,
                        12,
                        theme::CARD.lerp_to_gamma(theme::SURFACE, hover),
                    );
                    ui.painter().rect_stroke(
                        rect,
                        12,
                        Stroke::new(
                            1.0,
                            theme::LINE.lerp_to_gamma(theme::ACCENT.gamma_multiply(0.5), hover),
                        ),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(rect.min + vec2(14.0, 12.0), vec2(28.0, 28.0)),
                        8,
                        theme::ACCENT.gamma_multiply(0.14),
                    );
                    theme::icon(
                        ui.painter(),
                        rect.min + vec2(28.0, 26.0),
                        16.0,
                        glyph,
                        theme::ACCENT,
                    );
                    ui.painter().text(
                        rect.min + vec2(14.0, 56.0),
                        Align2::LEFT_CENTER,
                        title,
                        FontId::proportional(12.5),
                        theme::TEXT,
                    );
                    ui.painter().text(
                        rect.min + vec2(14.0, 75.0),
                        Align2::LEFT_CENTER,
                        subtitle,
                        FontId::proportional(11.0),
                        theme::DIM,
                    );
                    if response.clicked() {
                        let index = self.selected_index();
                        self.saved.chats[index].draft = prompt.into();
                        self.focus_composer = true;
                    }
                }
            });
        }
        if !self.saved.settings.reduced_motion && !background_window(ui.ctx()) {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    fn load_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let chat = self.saved.selected;
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let _ = tx.send(attachments::load_paths(&paths));
            repaint.request_repaint();
        });
        self.attachment_jobs.push((chat, rx));
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        let chat = self.saved.selected;
        let directory = self.project().path.clone();
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let result = match rfd::FileDialog::new()
                .set_title("Attach files to chat")
                .set_directory(directory)
                .pick_files()
            {
                Some(paths) => attachments::load_paths(&paths),
                None => Ok(Vec::new()),
            };
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        self.attachment_jobs.push((chat, rx));
    }

    fn paste_clipboard(&mut self, ctx: &egui::Context) {
        let chat = self.saved.selected;
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let _ = tx.send(attachments::clipboard_files_or_image());
            repaint.request_repaint();
        });
        self.attachment_jobs.push((chat, rx));
    }

    fn attachment_input(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let composer = Id::new(("composer", self.saved.selected));
        let focused = ctx.memory(|m| m.has_focus(composer)) || self.focus_composer;
        if focused
            && !self.settings_open
            && !self.attach_modal
            && !self.project_modal
            && self.question.is_none()
        {
            input.events.retain(|event| {
                if let egui::Event::Paste(text) = event {
                    if let Some(paths) = attachments::file_uris(text) {
                        self.load_files(ctx, paths);
                        return false;
                    }
                    if text.is_empty() {
                        self.paste_clipboard(ctx);
                        return false;
                    }
                }
                true
            });
        }
        if !input.dropped_files.is_empty() {
            let files = std::mem::take(&mut input.dropped_files);
            let chat = self.saved.selected;
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            self.runtime.spawn_blocking(move || {
                let result = if files.len() > attachments::MAX_ATTACHMENTS {
                    Err("Attach up to 8 files at a time.".into())
                } else {
                    files
                        .into_iter()
                        .map(|file| attachments::load(file.path()))
                        .collect()
                };
                let _ = tx.send(result);
                repaint.request_repaint();
            });
            self.attachment_jobs.push((chat, rx));
        }
    }

    fn poll_attachments(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut notifications = Vec::new();
        self.attachment_jobs
            .retain(|(chat_id, rx)| match rx.try_recv() {
                Ok(Ok(files)) => {
                    if let Some(chat) = self.saved.chats.iter_mut().find(|c| c.id == *chat_id) {
                        if chat.attachments.len() + files.len() > attachments::MAX_ATTACHMENTS {
                            notifications.push("Attach up to 8 files per message.".to_owned());
                        } else {
                            chat.attachments.extend(files);
                        }
                    }
                    false
                }
                Ok(Err(error)) => {
                    notifications.push(error);
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    notifications.push("Attachment loading stopped unexpectedly.".into());
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
            });
        self.thumbnails.retain(|id, rx| match rx.try_recv() {
            Ok(Some(image)) => {
                self.textures.insert(
                    *id,
                    ctx.load_texture(id.to_string(), image, egui::TextureOptions::LINEAR),
                );
                false
            }
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => false,
            Err(mpsc::TryRecvError::Empty) => true,
        });
        for error in notifications {
            self.notify(error, now);
        }
        if !self.attachment_jobs.is_empty() || !self.thumbnails.is_empty() {
            ctx.request_repaint_after(tick_interval(ctx).max(Duration::from_millis(40)));
        }
    }

    fn attachment_strip(&mut self, ui: &mut Ui, files: &[Attachment], removable: bool) {
        if files.is_empty() {
            return;
        }
        let compact = removable
            && ui.ctx().content_rect().height() < 650.0
            && self
                .question
                .as_ref()
                .is_some_and(|q| q.chat == self.saved.selected);
        let mut remove = None;
        ScrollArea::horizontal()
            .id_salt(("attachments", removable))
            .max_height(86.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for attachment in files {
                        ui.push_id(attachment.id, |ui| {
                            if matches!(attachment.content, AttachmentContent::Image { .. })
                                && !self.textures.contains_key(&attachment.id)
                                && !self.thumbnails.contains_key(&attachment.id)
                            {
                                let attachment = attachment.clone();
                                let id = attachment.id;
                                let (tx, rx) = mpsc::channel();
                                let repaint = ui.ctx().clone();
                                self.runtime.spawn_blocking(move || {
                                    let _ = tx.send(attachments::thumbnail(&attachment));
                                    repaint.request_repaint();
                                });
                                self.thumbnails.insert(id, rx);
                            }
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(142.0, if compact { 36.0 } else { 78.0 }),
                                Sense::click(),
                            );
                            ui.painter().rect_filled(rect, 7, theme::BG);
                            ui.painter().rect_stroke(
                                rect,
                                7,
                                Stroke::new(1.0, theme::LINE),
                                egui::StrokeKind::Inside,
                            );
                            if compact {
                                theme::icon(
                                    ui.painter(),
                                    rect.left_center() + vec2(15.0, 0.0),
                                    17.0,
                                    Icon::Paperclip,
                                    theme::ACCENT,
                                );
                            } else if let Some(texture) = self.textures.get(&attachment.id) {
                                let size = texture.size_vec2();
                                let scale = (126.0 / size.x).min(45.0 / size.y);
                                let image_rect = egui::Rect::from_center_size(
                                    rect.center_top() + vec2(0.0, 27.0),
                                    size * scale,
                                );
                                ui.painter().image(
                                    texture.id(),
                                    image_rect,
                                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );
                            } else {
                                theme::icon(
                                    ui.painter(),
                                    rect.center_top() + vec2(0.0, 27.0),
                                    22.0,
                                    Icon::Paperclip,
                                    theme::ACCENT,
                                );
                            }
                            let label = ui.painter().layout(
                                attachment.name.clone(),
                                FontId::proportional(11.0),
                                theme::TEXT,
                                if compact { 85.0 } else { 126.0 },
                            );
                            ui.painter().with_clip_rect(rect.shrink(7.0)).galley(
                                rect.min
                                    + if compact {
                                        vec2(28.0, 11.0)
                                    } else {
                                        vec2(8.0, 57.0)
                                    },
                                label,
                                theme::TEXT,
                            );
                            let clicked = response.clicked();
                            response.on_hover_text(format!(
                                "{} · {:.1} KiB{}",
                                attachment.name,
                                attachment.size as f32 / 1024.0,
                                if matches!(attachment.content, AttachmentContent::Image { .. }) {
                                    " · click to enlarge or save"
                                } else {
                                    ""
                                }
                            ));
                            if clicked
                                && matches!(attachment.content, AttachmentContent::Image { .. })
                            {
                                self.open_image(ui.ctx(), attachment.clone());
                            }
                            if removable {
                                let close = egui::Rect::from_min_size(
                                    rect.right_top() + vec2(-23.0, 3.0),
                                    vec2(20.0, 20.0),
                                );
                                ui.painter()
                                    .circle_filled(close.center(), 9.0, theme::SURFACE);
                                theme::icon(
                                    ui.painter(),
                                    close.center(),
                                    11.0,
                                    Icon::Close,
                                    theme::TEXT,
                                );
                                if ui
                                    .interact(close, ui.id().with("remove"), Sense::click())
                                    .on_hover_text("Remove attachment")
                                    .clicked()
                                {
                                    remove = Some(attachment.id);
                                }
                            }
                        });
                    }
                });
            });
        if let Some(id) = remove {
            let index = self.selected_index();
            self.saved.chats[index].attachments.retain(|a| a.id != id);
            self.textures.remove(&id);
        }
        ui.add_space(4.0);
    }

    fn question_card(&mut self, ui: &mut Ui) {
        let Some(pending) = self
            .question
            .as_mut()
            .filter(|q| q.chat == self.saved.selected)
        else {
            return;
        };
        let mut answer = None;
        let width = ui.available_width().min(760.0);
        let tight = ui.ctx().content_rect().height() < 650.0;
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            ui.allocate_ui_with_layout(vec2(width, if tight { 190.0 } else { 210.0 }), Layout::top_down(Align::Min), |ui| {
                Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::LINE)).corner_radius(16).inner_margin(if tight { 10 } else { 14 }).show(ui, |ui| {
                    ui.set_width((width - if tight { 20.0 } else { 28.0 }).max(150.0));
                    if tight { ui.spacing_mut().item_spacing.y = 4.0; }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Question").size(12.0).color(theme::MUTED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if theme::icon_button(ui, Icon::Close, "Skip question", false, 20.0).clicked() {
                                answer = Some(serde_json::json!({"skipped":true}).to_string());
                            }
                            let seconds = pending.deadline.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u32;
                            ui.label(RichText::new(format!("{seconds}s · then Recommended")).size(11.0).color(theme::ACCENT));
                        });
                    });
                    ui.label(RichText::new(&pending.question.question).size(13.0).strong());
                    ScrollArea::vertical().id_salt("question_options").max_height(115.0).min_scrolled_height(85.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.spacing_mut().interact_size.y = 23.0;
                        for (index, option) in pending.question.options.iter().enumerate() {
                            let label = format!("{}. {}{}", index + 1, option.label, if option.recommended { " (Recommended)" } else { "" });
                            let response = ui.selectable_label(pending.selected == index, RichText::new(label).size(12.0)).on_hover_text(&option.description);
                            if response.clicked() { pending.selected = index; pending.custom.clear(); }
                        }
                    });
                    ui.horizontal(|ui| {
                        let field_width = (ui.available_width() - 115.0).max(60.0);
                        let reply_id = Id::new(("question_reply", &pending.id));
                        let send_enter = ui.ctx().memory(|m| m.has_focus(reply_id))
                            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                        ui.add_sized([field_width, 28.0], egui::TextEdit::singleline(&mut pending.custom).id(reply_id).hint_text("Or write your own response"));
                        if ui.button("Skip").clicked() { answer = Some(serde_json::json!({"skipped":true}).to_string()); }
                        if ui.button("Send").clicked() || send_enter {
                            answer = Some(if pending.custom.trim().is_empty() { pending.question.answer(pending.selected, false) } else { serde_json::json!({"answer":pending.custom.trim(),"automatic":false}).to_string() });
                        }
                    });
                });
            });
        });
        ui.add_space(8.0);
        ui.ctx()
            .request_repaint_after(tick_interval(ui.ctx()).max(Duration::from_millis(100)));
        if let Some(answer) = answer
            && let Some(question) = self.question.take()
        {
            let _ = question.reply.send(answer);
        }
    }

    fn change_badge(&mut self, ui: &mut Ui) {
        let (files, added, removed) = changed_totals(&self.saved.chats[self.selected_index()]);
        if files == 0 {
            return;
        }
        ui.horizontal(|ui| {
            let width = 250.0_f32.min(ui.available_width());
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            let response = Frame::NONE
                .fill(theme::SURFACE)
                .corner_radius(11)
                .inner_margin(Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{files} file{} changed",
                                if files == 1 { "" } else { "s" }
                            ))
                            .size(12.0)
                            .color(theme::MUTED),
                        );
                        ui.label(
                            RichText::new(format!("+{added}"))
                                .size(12.0)
                                .color(theme::ACCENT),
                        );
                        ui.label(
                            RichText::new(format!("−{removed}"))
                                .size(12.0)
                                .color(theme::ERROR),
                        );
                    });
                })
                .response
                .interact(Sense::click());
            if response.on_hover_text("View edits in this chat").clicked() {
                self.changes_open = true;
            }
        });
        ui.add_space(5.0);
    }

    fn action_history(&mut self, ui: &mut Ui, message: &Message, start: usize, end: usize) {
        let Some(activities) = message.activities.get(start..end) else {
            return;
        };
        let mut kinds = Vec::new();
        for (name, label) in [
            ("write_file", "File edits"),
            ("read_file", "Read files"),
            ("view_image", "Viewed images"),
            ("send_image", "Sent images"),
            ("list_files", "Listed files"),
            ("run_command", "Ran commands"),
            ("web_search", "Searched web"),
            ("web_fetch", "Fetched pages"),
            ("ask_user", "Asked questions"),
        ] {
            if activities.iter().any(|a| a.name == name) {
                kinds.push(label);
            }
        }
        if activities.iter().any(|a| crate::mcp::is_tool(&a.name)) {
            kinds.push("MCP tools");
        }
        egui::CollapsingHeader::new(
            RichText::new(if kinds.is_empty() {
                "Used tools".into()
            } else {
                kinds.join(", ")
            })
            .size(13.0)
            .color(theme::MUTED),
        )
        // Each chronological group has independent, stable disclosure state.
        .id_salt(("action_group", message.id, start))
        .default_open(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().interact_size.y = 23.0;
            for (offset, activity) in activities.iter().enumerate() {
                let index = start + offset;
                let target = self
                    .action_targets
                    .entry((message.id, index))
                    .or_insert_with(|| {
                        let args: serde_json::Value =
                            serde_json::from_str(&activity.arguments).unwrap_or_default();
                        let key = match activity.name.as_str() {
                            "run_command" => "command",
                            "ask_user" => "question",
                            "web_search" => "query",
                            "web_fetch" => "url",
                            _ => "path",
                        };
                        args[key]
                            .as_str()
                            .unwrap_or(&activity.name)
                            .chars()
                            .take(80)
                            .collect()
                    });
                let (verb, target) = match activity.name.as_str() {
                    "read_file" => ("Read", target.as_str()),
                    "view_image" => ("Viewed image", target.as_str()),
                    "send_image" => ("Sent image", target.as_str()),
                    "write_file" => (
                        match activity.status {
                            ActionStatus::Running => "Editing",
                            ActionStatus::Waiting => "Proposed edit to",
                            ActionStatus::Declined
                            | ActionStatus::Failed
                            | ActionStatus::Cancelled => "Edit to",
                            ActionStatus::Complete if activity.change.is_none() => "Wrote",
                            ActionStatus::Complete => "Edited",
                        },
                        target.as_str(),
                    ),
                    "list_files" => ("Listed", target.as_str()),
                    "run_command" => ("Ran", target.as_str()),
                    "web_search" => ("Searched web for", target.as_str()),
                    "web_fetch" => ("Fetched", target.as_str()),
                    "ask_user" => ("Asked", target.as_str()),
                    _ => ("Used", activity.name.as_str()),
                };
                let state = match activity.status {
                    ActionStatus::Running => " · running".into(),
                    ActionStatus::Waiting => " · waiting for you".into(),
                    ActionStatus::Declined => " · declined".into(),
                    ActionStatus::Failed => " · failed".into(),
                    ActionStatus::Cancelled => " · cancelled".into(),
                    ActionStatus::Complete if activity.elapsed >= 0.1 => {
                        if activity.elapsed < 1.0 {
                            format!(" · {:.0}ms", activity.elapsed * 1000.0)
                        } else {
                            format!(" · {:.1}s", activity.elapsed)
                        }
                    }
                    _ => String::new(),
                };
                let counts = activity
                    .change
                    .as_ref()
                    .map(|c| format!(" +{} −{}", c.added, c.removed))
                    .unwrap_or_default();
                let color = if activity.status == ActionStatus::Failed {
                    theme::ERROR
                } else {
                    theme::MUTED
                };
                egui::CollapsingHeader::new(
                    RichText::new(format!("{verb} {target}{counts}{state}"))
                        .size(12.0)
                        .color(color),
                )
                .id_salt(("action", message.id, index))
                .icon({
                    let glyph = match activity.name.as_str() {
                        "run_command" => Icon::Terminal,
                        "write_file" => Icon::Paperclip,
                        "read_file" | "view_image" | "send_image" => Icon::Copy,
                        "list_files" => Icon::Folder,
                        "web_search" | "web_fetch" => Icon::Globe,
                        _ => Icon::Spark,
                    };
                    move |ui, _, response| {
                        theme::icon(ui.painter(), response.rect.center(), 13.0, glyph, color)
                    }
                })
                .show(ui, |ui| {
                    ui.label(RichText::new("Arguments").size(10.0).color(theme::DIM));
                    ScrollArea::both()
                        .id_salt("arguments")
                        .max_height(140.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(activity.arguments.as_ref())
                                        .monospace()
                                        .size(11.0),
                                )
                                .selectable(true),
                            );
                        });
                    if matches!(activity.name.as_str(), "web_search" | "web_fetch")
                        && let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&activity.result)
                    {
                        if let Some(url) = value["url"]
                            .as_str()
                            .filter(|url| crate::web::http_url(url).is_ok())
                        {
                            ui.hyperlink_to(
                                value["title"]
                                    .as_str()
                                    .filter(|title| !title.is_empty())
                                    .unwrap_or(url),
                                url,
                            );
                        }
                        if let Some(results) = value["results"].as_array() {
                            for result in results {
                                if let Some(url) = result["url"]
                                    .as_str()
                                    .filter(|url| crate::web::http_url(url).is_ok())
                                {
                                    ui.hyperlink_to(
                                        result["title"]
                                            .as_str()
                                            .filter(|title| !title.is_empty())
                                            .unwrap_or(url),
                                        url,
                                    );
                                }
                            }
                        }
                    }
                    if let Some(change) = &activity.change {
                        show_diff(ui, &change.diff);
                    }
                    if !activity.result.is_empty() {
                        ui.label(RichText::new("Result").size(10.0).color(theme::DIM));
                        ScrollArea::both()
                            .id_salt("result")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(activity.result.as_ref())
                                            .monospace()
                                            .size(11.0),
                                    )
                                    .selectable(true),
                                );
                            });
                    }
                });
                if activity.name == "view_image" {
                    ui.push_id(("viewed_images", message.id, index), |ui| {
                        self.attachment_strip(ui, &activity.images, false);
                    });
                }
            }
        });
    }

    fn open_image(&mut self, ctx: &egui::Context, image: Attachment) {
        let (tx, rx) = mpsc::channel();
        let file = image.clone();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let _ = tx.send(attachments::preview(&file, 1920, 1440));
            repaint.request_repaint();
        });
        self.image_preview = Some(ImagePreview {
            image,
            texture: None,
            rx,
            loaded: false,
            zoom: 1.0,
        });
    }

    fn image_window(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.image_save_rx {
            match rx.try_recv() {
                Ok(Ok(Some(path))) => {
                    self.image_save_rx = None;
                    self.notify(
                        format!("Saved image to {}", path.display()),
                        ctx.input(|i| i.time),
                    );
                }
                Ok(Ok(None)) => self.image_save_rx = None,
                Ok(Err(error)) => {
                    self.image_save_rx = None;
                    self.notify(error, ctx.input(|i| i.time));
                }
                Err(mpsc::TryRecvError::Disconnected) => self.image_save_rx = None,
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(50))
                }
            }
        }
        let Some(preview) = &mut self.image_preview else {
            return;
        };
        if !preview.loaded {
            match preview.rx.try_recv() {
                Ok(Some(image)) => {
                    preview.texture = Some(ctx.load_texture(
                        format!("image-preview-{}", preview.image.id),
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                    preview.loaded = true;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => preview.loaded = true,
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(30))
                }
            }
        }
        let mut open = true;
        let mut save = false;
        let rect = ctx.content_rect();
        egui::Window::new(format!("Image · {}", preview.image.name))
            .id(Id::new("image_viewer"))
            .open(&mut open)
            .collapsible(false)
            .default_size(vec2(760.0, 560.0))
            .max_size(vec2(
                (rect.width() - 40.0).max(240.0),
                (rect.height() - 60.0).max(240.0),
            ))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if let AttachmentContent::Image { width, height, .. } = preview.image.content {
                        ui.label(
                            RichText::new(format!("{width} × {height}"))
                                .size(11.0)
                                .color(theme::MUTED),
                        );
                    }
                    save = ui
                        .add_enabled(self.image_save_rx.is_none(), egui::Button::new("Save PNG…"))
                        .clicked();
                    if ui.button("Fit").clicked() {
                        preview.zoom = 1.0;
                    }
                    ui.add(
                        egui::Slider::new(&mut preview.zoom, 0.5..=4.0)
                            .logarithmic(true)
                            .text("Zoom"),
                    );
                });
                if let Some(texture) = &preview.texture {
                    let size = texture.size_vec2();
                    let viewport = vec2(
                        ui.available_width().max(1.0),
                        ui.available_height().max(100.0),
                    );
                    let fit = (viewport.x / size.x).min(viewport.y / size.y).min(1.0);
                    ScrollArea::both()
                        .id_salt("image_zoom_scroll")
                        .auto_shrink([false, false])
                        .max_height(viewport.y)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Image::new((texture.id(), size * fit * preview.zoom))
                                    .corner_radius(6),
                            );
                        });
                } else {
                    ui.label(if preview.loaded {
                        "Could not decode image."
                    } else {
                        "Loading image…"
                    });
                }
            });
        if save {
            let image = preview.image.clone();
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            self.runtime.spawn_blocking(move || {
                let suggested = PathBuf::from(&image.name)
                    .with_extension("png")
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let result = match rfd::FileDialog::new()
                    .set_title("Save image as PNG")
                    .set_file_name(suggested)
                    .add_filter("PNG image", &["png"])
                    .save_file()
                {
                    Some(path) => attachments::save_png_atomic(&image, &path).map(|_| Some(path)),
                    None => Ok(None),
                };
                let _ = tx.send(result);
                repaint.request_repaint();
            });
            self.image_save_rx = Some(rx);
        }
        if !open {
            self.image_preview = None;
        }
    }

    fn changes_window(&mut self, ctx: &egui::Context) {
        if !self.changes_open {
            return;
        }
        let chat = &self.saved.chats[self.selected_index()];
        let mut open = true;
        egui::Window::new("Changes in this chat")
            .open(&mut open)
            .default_width(700.0)
            .max_width((ctx.content_rect().width() - 40.0).max(250.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Line counts include completed file edits across this chat.")
                        .size(11.0)
                        .color(theme::MUTED),
                );
                ScrollArea::vertical().max_height(450.0).show(ui, |ui| {
                    for (index, change) in chat
                        .messages
                        .iter()
                        .flat_map(|m| &m.activities)
                        .filter_map(|a| a.change.as_ref())
                        .enumerate()
                    {
                        egui::CollapsingHeader::new(format!(
                            "{}  +{} −{}",
                            change.path, change.added, change.removed
                        ))
                        .id_salt(("diff", index))
                        .default_open(true)
                        .show(ui, |ui| show_diff(ui, &change.diff));
                    }
                });
            });
        self.changes_open = open;
    }

    fn queue_height(&self) -> f32 {
        let count = self.saved.chats[self.selected_index()].queue.len();
        if count == 0 {
            0.0
        } else {
            52.0 + count.min(3) as f32 * 40.0
        }
    }

    fn queued_messages(&mut self, ui: &mut Ui) {
        let index = self.selected_index();
        let chat_id = self.saved.chats[index].id;
        if self.saved.chats[index].queue.is_empty() {
            return;
        }
        let mut steer = None;
        let mut edit = None;
        let mut remove = None;
        let mut toggle_pause = false;
        let paused = self.saved.chats[index].queue_paused;
        let can_steer = self
            .active
            .as_ref()
            .is_some_and(|a| a.chat == chat_id && a.steering.is_some());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "Queued ({}){}",
                    self.saved.chats[index].queue.len(),
                    if paused { " · paused" } else { "" }
                ))
                .size(11.0)
                .color(theme::MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button(if paused { "Resume" } else { "Pause" })
                    .clicked()
                {
                    toggle_pause = true;
                }
            });
        });
        ScrollArea::vertical().id_salt(("queued_messages", chat_id)).max_height(120.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for queued in &self.saved.chats[index].queue {
                ui.push_id(queued.id, |ui| {
                    Frame::NONE.fill(theme::RAISED).corner_radius(10).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(vec2(18.0, 24.0), Sense::hover());
                            theme::icon(ui.painter(), rect.center(), 13.0, Icon::Queue, theme::DIM);
                            let row_width = (ui.available_width() - if queued.attachments.is_empty() { 145.0 } else { 190.0 }).max(40.0);
                            let preview = if queued.text.is_empty() { queued.attachments.first().map(|a| a.name.clone()).unwrap_or_else(|| "Attachments".into()) }
                                else { queued.text.lines().next().unwrap_or("").chars().take(180).collect::<String>() };
                            ui.allocate_ui_with_layout(vec2(row_width, 24.0), Layout::left_to_right(Align::Center), |ui|
                                ui.add(egui::Label::new(RichText::new(preview).size(12.0)).truncate())).inner
                                .on_hover_ui(|ui| {
                                    ui.set_max_width(350.0);
                                    ui.label(queued.text.chars().take(1200).collect::<String>());
                                    for attachment in &queued.attachments { ui.label(RichText::new(format!("Attached: {}", attachment.name)).size(11.0).color(theme::MUTED)); }
                                });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if queued.dispatched {
                                    ui.label(RichText::new("Steering…").size(11.0).color(theme::ACCENT)).on_hover_text("Waiting for the next completed request/tool boundary. Stop preserves unapplied messages.");
                                } else {
                                    if theme::icon_button(ui, Icon::Trash, "Delete queued message", false, 22.0).clicked() { remove = Some(queued.id); }
                                    if theme::icon_button(ui, Icon::Pencil, "Edit queued message", false, 22.0).clicked() { edit = Some((chat_id, queued.id, queued.text.clone())); }
                                    if ui.add_enabled(can_steer && !paused, egui::Button::new(RichText::new("Steer").size(11.0)).small())
                                        .on_hover_text("Guide this chat's current run at the next safe boundary; completed tool calls are not interrupted.").clicked() { steer = Some(queued.id); }
                                    if !queued.attachments.is_empty() { ui.label(RichText::new(format!("{} {}", queued.attachments.len(), if queued.attachments.len() == 1 { "file" } else { "files" })).size(10.0).color(theme::DIM)); }
                                }
                            });
                        });
                    });
                });
            }
        });
        if toggle_pause {
            self.saved.chats[index].queue_paused = !paused;
            if !self.saved.chats[index].queue_paused {
                self.start_next(chat_id, ui.ctx());
            }
        }
        if let Some(id) = remove {
            self.saved.chats[index].queue.retain(|q| q.id != id);
        }
        if let Some(edit) = edit {
            self.queue_edit = Some(edit);
        }
        if let Some(id) = steer {
            self.steer_queued(chat_id, id, ui.ctx());
        }
        if toggle_pause || remove.is_some() {
            self.store.queue(&self.saved);
        }
        ui.add_space(6.0);
    }

    fn composer(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let content = width.min(760.0);
        let tight = ui.ctx().content_rect().height() < 650.0
            && self
                .question
                .as_ref()
                .is_some_and(|q| q.chat == self.saved.selected);
        ui.horizontal(|ui| {
            ui.add_space((width - content) / 2.0);
            ui.allocate_ui_with_layout(
                vec2(content, if tight { 100.0 } else { 150.0 }),
                Layout::top_down(Align::Min),
                |ui| {
                    self.queued_messages(ui);
                    Frame::NONE
                        .fill(theme::SURFACE)
                        .stroke(Stroke::new(1.0, theme::OUTLINE))
                        .corner_radius(18)
                        .inner_margin(Margin::symmetric(15, 10))
                        .show(ui, |ui| {
                            ui.set_min_width((content - 32.0).max(180.0));
                            if !tight {
                                ui.horizontal(|ui| {
                                    theme::inline_icon(ui, Icon::Folder, 13.0, theme::MUTED);
                                    ui.label(
                                        RichText::new(&self.project().name)
                                            .size(11.0)
                                            .color(theme::MUTED),
                                    );
                                    ui.label(
                                        RichText::new("/   This computer")
                                            .size(11.0)
                                            .color(theme::DIM),
                                    );
                                });
                            }
                            ui.add_space(4.0);
                            let index = self.selected_index();
                            let attachments = self.saved.chats[index].attachments.clone();
                            self.attachment_strip(ui, &attachments, true);
                            if self
                                .attachment_jobs
                                .iter()
                                .any(|(id, _)| *id == self.saved.selected)
                            {
                                ui.label(
                                    RichText::new("Loading attachments…")
                                        .size(11.0)
                                        .color(theme::MUTED),
                                );
                            }
                            let id = Id::new(("composer", self.saved.selected));
                            // Consume plain Enter before TextEdit handles it, so it never
                            // accidentally inserts a newline or drops focus when sending.
                            let focused = ui.ctx().memory(|m| m.has_focus(id));
                            let send_enter = focused
                                && ui.input_mut(|i| {
                                    i.modifiers == egui::Modifiers::NONE
                                        && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                });
                            let steer_enter = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
                            let response = ui.add_sized(
                                [ui.available_width(), if tight { 30.0 } else { 58.0 }],
                                egui::TextEdit::multiline(&mut self.saved.chats[index].draft)
                                    .id(id)
                                    .hint_text(if self.active.is_some() { "Queue a follow-up…" } else { "Ask, build, fix, explore…" })
                                    .font(FontId::proportional(15.0))
                                    .frame(Frame::NONE)
                                    .desired_rows(2)
                                    .return_key(egui::KeyboardShortcut::new(
                                        egui::Modifiers::SHIFT,
                                        egui::Key::Enter,
                                    )),
                            );
                            if self.focus_composer {
                                response.request_focus();
                                self.focus_composer = false;
                            }
                            ui.add_space(4.0);
                            let mut send_clicked = false;
                            ui.horizontal(|ui| {
                                if theme::icon_button(
                                    ui,
                                    Icon::Paperclip,
                                    "Attach files or paste from clipboard",
                                    false,
                                    28.0,
                                )
                                .clicked()
                                {
                                    self.pick_files(ui.ctx());
                                }
                                ui.menu_button("+", |ui| {
                                    if ui.button("Paste files or image").clicked() {
                                        self.paste_clipboard(ui.ctx());
                                        ui.close();
                                    }
                                    if ui.button("Attach by path…").clicked() {
                                        self.attach_modal = true;
                                        ui.close();
                                    }
                                });
                                if content > 540.0 {
                                    ui.label(
                                        RichText::new(if self.saved.settings.tools_enabled {
                                            "Review changes"
                                        } else {
                                            "Chat only"
                                        })
                                        .size(11.0)
                                        .color(theme::MUTED),
                                    );
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let enabled =
                                        (!self.saved.chats[index].draft.trim().is_empty()
                                            || !self.saved.chats[index].attachments.is_empty())
                                            && !self
                                                .attachment_jobs
                                                .iter()
                                                .any(|(id, _)| *id == self.saved.selected);
                                    let running = self.active.is_some();
                                    let (rect, response) =
                                        ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        15.0,
                                        if running || enabled {
                                            theme::ACCENT
                                        } else {
                                            theme::HOVER
                                        },
                                    );
                                    theme::icon(
                                        ui.painter(),
                                        rect.center(),
                                        18.0,
                                        if running { Icon::Stop } else { Icon::Arrow },
                                        if running || enabled {
                                            theme::BG
                                        } else {
                                            theme::DIM
                                        },
                                    );
                                    if response.clicked() {
                                        if running {
                                            self.stop();
                                        } else if enabled {
                                            send_clicked = true;
                                        }
                                    }
                                    response.on_hover_text(if running {
                                        "Stop generation"
                                    } else {
                                        "Send message"
                                    });
                                    if running && enabled && theme::icon_button(ui, Icon::Queue, "Queue message (Enter); use the queued row’s Steer button to guide this run", false, 28.0).clicked() {
                                        send_clicked = true;
                                    }
                                    egui::ComboBox::from_id_salt("composer_provider")
                                        .width(if content > 480.0 { 128.0 } else { 96.0 })
                                        .selected_text(
                                            RichText::new(self.saved.settings.provider.label())
                                                .size(11.0),
                                        )
                                        .show_ui(ui, |ui| {
                                            for provider in [
                                                Provider::Codex,
                                                Provider::OpenAI,
                                                Provider::OpenRouter,
                                                Provider::Llama,
                                                Provider::Demo,
                                            ] {
                                                ui.selectable_value(
                                                    &mut self.saved.settings.provider,
                                                    provider,
                                                    provider.label(),
                                                );
                                            }
                                        });
                                    if content > 460.0 {
                                        egui::ComboBox::from_id_salt("composer_effort")
                                            .width(65.0)
                                            .selected_text(
                                                RichText::new(&self.saved.settings.effort)
                                                    .size(11.0),
                                            )
                                            .show_ui(ui, |ui| {
                                                for effort in ["low", "medium", "high", "xhigh"] {
                                                    ui.selectable_value(
                                                        &mut self.saved.settings.effort,
                                                        effort.into(),
                                                        effort,
                                                    );
                                                }
                                            });
                                    }
                                    ContextMeter::for_chat(
                                        &self.saved.chats[index],
                                        &self.saved.settings,
                                    )
                                    .show(ui, &self.saved.settings);
                                });
                            });
                            if steer_enter { self.submit(ui.ctx(), true); }
                            else if send_enter || send_clicked { self.send(ui.ctx()); }
                        });
                },
            );
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(((width - content) / 2.0 + 4.0).max(0.0));
            ui.label(
                RichText::new(
                    if self
                        .active
                        .as_ref()
                        .is_some_and(|a| a.chat != self.saved.selected)
                    {
                        "Enter to queue · waits for the current chat"
                    } else if self.active.as_ref().is_some_and(|a| a.steering.is_some()) {
                        "Enter to queue · Ctrl/Cmd+Enter to steer"
                    } else if self.active.is_some() {
                        "Enter to queue · Shift+Enter for a new line"
                    } else if self.saved.settings.provider == Provider::Demo {
                        "Demo mode · no API requests"
                    } else {
                        "Enter to send · Shift+Enter for a new line"
                    },
                )
                .size(10.0)
                .color(theme::DIM),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if width > 520.0 {
                    ui.add_space((width - content) / 2.0);
                    ui.label(
                        RichText::new("Made for the way you work")
                            .size(10.0)
                            .color(theme::DIM),
                    );
                }
            });
        });
    }

    fn conversation(&mut self, ui: &mut Ui, now: f64) {
        let index = self.selected_index();
        // Rendering never mutates these messages. Temporarily move the vector
        // out so mutable widget state does not require cloning the entire log.
        let messages = std::mem::take(&mut self.saved.chats[index].messages);
        let width = ui.available_width();
        let content = width.min(720.0);
        ScrollArea::vertical()
            .id_salt(("conversation", self.saved.selected))
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(((width - content) / 2.0).max(0.0));
                    ui.allocate_ui_with_layout(
                        vec2(content, 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.add_space(20.0);
                            for message in &messages {
                                ui.push_id(message.id, |ui| self.message(ui, message, now));
                                ui.add_space(28.0);
                            }
                            ui.add_space(12.0);
                        },
                    );
                });
            });
        self.saved.chats[index].messages = messages;
    }

    fn message(&mut self, ui: &mut Ui, message: &Message, now: f64) {
        let alpha = if self.saved.settings.reduced_motion || message.born == 0.0 {
            1.0
        } else {
            motion::ease(((now - message.born) / 0.3) as f32)
        };
        ui.set_opacity(ui.opacity() * alpha);
        let (answer, reasoning) = self
            .reveals
            .get(&message.id)
            .map(|(a, r)| {
                (
                    a.visible(&message.text).to_owned(),
                    r.visible(&message.reasoning).to_owned(),
                )
            })
            .unwrap_or_else(|| (message.text.clone(), message.reasoning.clone()));
        if message.user {
            ui.horizontal(|ui| {
                ui.label(RichText::new("YOU").size(10.0).color(theme::DIM));
            });
            ui.add_space(7.0);
            Frame::NONE
                .fill(theme::SURFACE)
                .corner_radius(12)
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 1.0).max(0.0));
                    self.attachment_strip(ui, &message.attachments, false);
                    theme::markdown(ui, &answer, self.saved.settings.font_size, theme::TEXT);
                });
            return;
        }
        ui.horizontal(|ui| {
            theme::inline_icon(ui, Icon::Terminal, 16.0, theme::ACCENT);
            ui.add_space(2.0); // Preserve the original 26-point leading gutter.
            ui.label(RichText::new("hfx").size(13.0).strong());
            ui.label(
                RichText::new(&message.provider)
                    .size(10.0)
                    .color(theme::DIM),
            );
        });
        ui.add_space(10.0);
        if self.saved.settings.show_reasoning
            && (!message.reasoning.is_empty() || message.status == Status::Streaming)
        {
            Frame::NONE
                .fill(theme::THINK_BG)
                .stroke(Stroke::new(1.0, theme::THINK_LINE))
                .corner_radius(9)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 1.0).max(0.0));
                    let label = if matches!(message.provider.as_str(), "llama.cpp" | "OpenRouter")
                        || (message.status == Status::Streaming && message.text.trim().is_empty())
                    {
                        "Reasoning"
                    } else {
                        "Reasoning summary"
                    };
                    let expanded = self.preview
                        || (message.status == Status::Streaming && message.text.trim().is_empty());
                    egui::CollapsingHeader::new(
                        RichText::new(format!("{label}   ·   {:.1}s", message.elapsed))
                            .size(12.0)
                            .color(theme::ACCENT),
                    )
                    .id_salt(("reasoning", message.id))
                    .default_open(expanded)
                    .show(ui, |ui| {
                        if reasoning.is_empty() {
                            ui.label(
                                RichText::new("Waiting for the model…")
                                    .size(12.0)
                                    .color(theme::MUTED),
                            );
                        } else {
                            theme::markdown(ui, &reasoning, 13.0, theme::MUTED);
                        }
                    });
                });
            ui.add_space(12.0);
        }
        let chronological = message.timeline_valid();
        if chronological {
            for (index, block) in message.timeline.iter().enumerate() {
                ui.push_id(("reply_block", index), |ui| {
                    match *block {
                        ReplyBlock::Text { start, end } => {
                            if let Some(text) = answer.get(start..end.min(answer.len())) && !text.trim().is_empty() {
                                theme::markdown(ui, text.trim_matches('\n'), self.saved.settings.font_size, theme::TEXT);
                            }
                        }
                        ReplyBlock::Tools { start, end } => {
                            self.action_history(ui, message, start, end);
                        }
                        ReplyBlock::Images { start, end } => {
                            self.attachment_strip(ui, &message.attachments[start..end], false);
                        }
                        ReplyBlock::Compacted => {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                                theme::icon(ui.painter(), rect.center(), 13.0, Icon::Queue, theme::DIM);
                                ui.label(RichText::new("Context automatically compacted").size(12.0).color(theme::MUTED))
                                    .on_hover_text("Older model context was summarized; the full visible chat and tool history remain intact.");
                            });
                        }
                    }
                });
            }
        } else {
            // Legacy saves lack event order: retain every item in their original
            // single-list/text layout rather than guessing or losing history.
            if !message.activities.is_empty() {
                self.action_history(ui, message, 0, message.activities.len());
                ui.add_space(12.0);
            }
            if !answer.is_empty() {
                theme::markdown(ui, &answer, self.saved.settings.font_size, theme::TEXT);
            }
            self.attachment_strip(ui, &message.attachments, false);
        }
        if message.status == Status::Streaming {
            ui.horizontal(|ui| {
                if self.saved.settings.reduced_motion {
                    ui.label(RichText::new("Working…").size(11.0).color(theme::MUTED));
                } else {
                    let (rect, _) = ui.allocate_exact_size(vec2(26.0, 16.0), Sense::hover());
                    for i in 0..3 {
                        let alpha =
                            0.3 + 0.7 * ((now as f32 * 3.5 - i as f32 * 0.6).sin() * 0.5 + 0.5);
                        ui.painter().circle_filled(
                            rect.min + vec2(3.0 + i as f32 * 8.0, 8.0),
                            2.0,
                            theme::ACCENT.gamma_multiply(alpha),
                        );
                    }
                }
                ui.label(
                    RichText::new(if message.compacting {
                        "Compacting context…"
                    } else if self.pending.is_some() {
                        "Waiting for approval"
                    } else {
                        "Working"
                    })
                    .size(11.0)
                    .color(theme::DIM),
                );
            });
        }
        if message.context_checkpoint.is_some() && !chronological {
            ui.label(
                RichText::new("Context compacted · full chat history retained")
                    .size(10.0)
                    .color(theme::DIM),
            );
        }
        if message.context_tokens > 0 {
            ui.label(
                RichText::new(format!(
                    "{}{} context tokens",
                    if message.context_estimated { "~" } else { "" },
                    message.context_tokens
                ))
                .size(10.0)
                .color(theme::DIM),
            );
        }
        if let Some(error) = &message.error {
            ui.add_space(8.0);
            Frame::NONE
                .fill(theme::ERROR.gamma_multiply(0.06))
                .corner_radius(8)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.label(RichText::new(error).size(13.0).color(theme::ERROR));
                });
        }
        if message.status != Status::Streaming {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if theme::icon_button(ui, Icon::Copy, "Copy response", false, 24.0).clicked() {
                    ui.ctx().copy_text(message.text.clone());
                    self.notify("Response copied", now);
                }
                ui.label(
                    RichText::new(format!(
                        "{:.1}s{}{}",
                        message.elapsed,
                        if message.tokens > 0 {
                            format!(" · {} tokens", message.tokens)
                        } else {
                            String::new()
                        },
                        if message.status == Status::Cancelled {
                            " · stopped"
                        } else {
                            ""
                        }
                    ))
                    .size(10.0)
                    .color(theme::DIM),
                ).on_hover_text(format!("Recorded model + network: {:.2}s (excludes compaction)\nSum of tool runtimes/waits: {:.2}s (excludes approvals; parallel tools overlap)\nTotal elapsed: {:.2}s",
                    message.model_seconds, message.activities.iter().map(|a| a.elapsed).sum::<f32>(), message.elapsed));
            });
        }
    }

    fn settings_panel(&mut self, ui: &mut Ui, now: f64) {
        let mut visible = self.settings_open;
        let width = (ui.ctx().content_rect().width() * 0.36).clamp(356.0, 424.0);
        egui::Panel::right("settings")
            .exact_size(width)
            .resizable(false)
            .frame(Frame::NONE.fill(theme::SIDEBAR).inner_margin(18))
            .show_collapsible(ui, &mut visible, |ui| {
                prefs::style(ui);
                // Keep close/save affordances reachable even in a long tab.
                egui::Panel::bottom("settings_footer")
                    .exact_size(56.0)
                    .frame(Frame::NONE.fill(theme::SIDEBAR))
                    .show(ui, |ui| {
                        ui.separator();
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("Saved automatically")
                                        .size(11.0)
                                        .color(prefs::GREEN),
                                );
                                prefs::help(ui, "API keys stay in this session.");
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if theme::dialog_action(ui, "Done", false).clicked() {
                                    self.settings_open = false;
                                    self.notify("Settings saved", now);
                                }
                            });
                        });
                    });
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 11, theme::ACCENT.gamma_multiply(0.12));
                    theme::icon(
                        ui.painter(),
                        rect.center(),
                        21.0,
                        Icon::Settings,
                        theme::ACCENT,
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Settings").size(23.0).strong());
                        prefs::help(ui, "A little tuning. A better flow.");
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(ui, Icon::Close, "Close settings", false, 28.0)
                            .clicked()
                        {
                            self.settings_open = false;
                        }
                    });
                });
                ui.add_space(16.0);
                prefs::tabs(ui, &mut self.settings_tab);
                ScrollArea::vertical()
                    .id_salt(("settings_scroll", self.settings_tab))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Leave a little room for the scrollbar and card shadows.
                        ui.set_width((ui.available_width() - 4.0).max(1.0));
                        match self.settings_tab {
                            0 => self.provider_settings(ui),
                            1 => self.appearance_settings(ui),
                            _ => self.tool_settings(ui),
                        }
                        ui.add_space(16.0);
                    });
            });
    }

    fn appearance_settings(&mut self, ui: &mut Ui) {
        prefs::intro(
            ui,
            "Find your focus.",
            "A quieter canvas, tuned to the way you work.",
        );
        let settings = &mut self.saved.settings;
        prefs::card(ui, "palette", |ui| {
            prefs::heading(ui, Icon::Spark, "Midnight / Iris", "Your workspace palette");
            ui.horizontal(|ui| {
                for color in [
                    theme::BG,
                    theme::SURFACE,
                    theme::OUTLINE,
                    theme::ACCENT,
                    theme::TEXT,
                ] {
                    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 10.0, color);
                    ui.painter().circle_stroke(
                        rect.center(),
                        10.0,
                        Stroke::new(1.0, theme::OUTLINE),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    prefs::pill(ui, "ACTIVE", theme::ACCENT)
                });
            });
            ui.add_space(4.0);
            Frame::NONE
                .fill(theme::SIDEBAR)
                .stroke(Stroke::new(1.0, theme::LINE))
                .corner_radius(9)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        theme::inline_icon(ui, Icon::Terminal, 14.0, theme::ACCENT);
                        ui.label(RichText::new("hfx").size(12.0).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            prefs::label(ui, "READING PREVIEW")
                        });
                    });
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new("A little space to think.")
                            .size(settings.font_size)
                            .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new("Your next good idea starts here.")
                            .size(settings.font_size)
                            .color(theme::MUTED),
                    );
                    if settings.show_reasoning {
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new("Reasoning summaries appear alongside replies.")
                                .size(11.0)
                                .color(theme::ACCENT),
                        );
                    }
                });
        });
        prefs::card(ui, "reading", |ui| {
            prefs::heading(ui, Icon::Pencil, "Reading", "Comfort comes first");
            prefs::label(ui, "Conversation text size");
            ui.spacing_mut().slider_width = (ui.available_width() - 84.0).max(80.0);
            ui.add(egui::Slider::new(&mut settings.font_size, 13.0..=19.0).suffix(" px"));
            prefs::divider(ui);
            prefs::toggle(
                ui,
                &mut settings.show_reasoning,
                "Reasoning summaries",
                "See how the model approaches your request.",
            );
        });
        prefs::card(ui, "motion", |ui| {
            prefs::heading(ui, Icon::Spark, "Motion", "Set the pace");
            prefs::toggle(
                ui,
                &mut settings.reduced_motion,
                "Reduce motion",
                "Instant text and minimal transitions.",
            );
            prefs::divider(ui);
            ui.add_enabled_ui(!settings.reduced_motion, |ui| {
                prefs::label(ui, "Text reveal speed");
                ui.spacing_mut().slider_width = (ui.available_width() - 116.0).max(80.0);
                ui.add(
                    egui::Slider::new(&mut settings.reveal_speed, 40.0..=320.0).suffix(" chars/s"),
                );
                ui.horizontal(|ui| {
                    prefs::help(ui, "Unhurried");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        prefs::help(ui, "Responsive")
                    });
                });
            });
        });
        #[cfg(target_os = "linux")]
        prefs::card(ui, "completion_alerts", |ui| {
            prefs::heading(ui, Icon::Check, "When work is done", "A gentle heads-up");
            prefs::toggle(
                ui,
                &mut settings.desktop_notifications,
                "Desktop notification",
                "One alert after this chat’s queued batch.",
            );
            prefs::divider(ui);
            prefs::toggle(
                ui,
                &mut settings.completion_sound,
                "Completion sound",
                "An independent, quiet chime.",
            );
            prefs::help(
                ui,
                "Stop and errors stay silent. Do Not Disturb controls desktop banners; mute the chime here or in system audio.",
            );
        });
    }

    fn tool_settings(&mut self, ui: &mut Ui) {
        prefs::intro(
            ui,
            "Capable. On your terms.",
            "Choose what your agent can do, and how it asks.",
        );
        prefs::card(ui, "agent_tools", |ui| {
            prefs::heading(
                ui,
                Icon::Terminal,
                "Agent capabilities",
                "From conversation to action",
            );
            prefs::toggle(
                ui,
                &mut self.saved.settings.tools_enabled,
                "Enable agent tools",
                "Read files, make edits, and ask questions.",
            );
            prefs::divider(ui);
            ui.add_enabled_ui(self.saved.settings.tools_enabled, |ui| {
                prefs::toggle(
                    ui,
                    &mut self.saved.settings.review_actions,
                    "Review each tool action",
                    "Approve file, command, web, and MCP actions first.",
                );
            });
            let (status, color, explanation) = if !self.saved.settings.tools_enabled {
                (
                    "TOOLS PAUSED",
                    theme::MUTED,
                    "Preferences below are kept for when you enable tools again.",
                )
            } else if self.saved.settings.review_actions {
                (
                    "REVIEW MODE",
                    prefs::AMBER,
                    "Workspace, web, and MCP actions wait for approval. Questions use their own 30-second recommendation timer.",
                )
            } else {
                (
                    "AUTOMATIC ACTIONS",
                    prefs::GREEN,
                    "Enabled tools run until the task finishes or you stop it. Questions have a 30-second recommendation timer.",
                )
            };
            ui.add_space(3.0);
            prefs::pill(ui, status, color);
            prefs::help(ui, explanation);
        });
        self.command_and_web_settings(ui);
        let root = PathBuf::from(&self.project().path);
        self.mcp_probe
            .show(ui, &mut self.saved.settings, root, &self.runtime);
        self.git_attribution_settings(ui);
        prefs::card(ui, "instructions", |ui| {
            prefs::heading(
                ui,
                Icon::Pencil,
                "Assistant instructions",
                "Your preferences, in your words",
            );
            ui.add(
                egui::TextEdit::multiline(&mut self.saved.settings.system_prompt)
                    .id_salt("system_prompt")
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .margin(vec2(10.0, 10.0)),
            );
            prefs::help(
                ui,
                "Built-in workspace and Git attribution rules are added to these instructions for every provider.",
            );
        });
    }

    fn command_and_web_settings(&mut self, ui: &mut Ui) {
        use crate::state::{CommandMode, SearchProvider};
        let settings = &mut self.saved.settings;
        ui.add_enabled_ui(settings.tools_enabled, |ui| {
            prefs::card(ui, "shell", |ui| {
                prefs::heading(ui, Icon::Terminal, "Shell access", "The environment behind each command");
                prefs::toggle(ui, &mut settings.commands_enabled, "Shell commands", "Build, test, and run tools from your project.");
                prefs::divider(ui);
                ui.add_enabled_ui(settings.commands_enabled, |ui| {
                    prefs::label(ui, "Command environment");
                    ui.columns(2, |columns| {
                        for (column, (mode, title)) in columns.iter_mut().zip([(CommandMode::Trusted, "Trusted host"), (CommandMode::Sandbox, "Strict sandbox")]) {
                            if column.add_sized(vec2(column.available_width(), 34.0), egui::Button::new(title).selected(settings.command_mode == mode)).clicked() {
                                settings.command_mode = mode;
                            }
                        }
                    });
                    match settings.command_mode {
                        CommandMode::Trusted => prefs::callout(ui, "HOST ACCESS · Not sandboxed\nCommands run as you and can read credentials, use Git/SSH, network and desktop access, and change files outside this project. Use only with code you trust. No sudo is granted.", prefs::AMBER),
                        CommandMode::Sandbox => prefs::callout(ui, "STRICT SANDBOX · Linux\nProject and private temporary files, with network access. Host SSH credentials and desktop sessions are hidden. Requires bubblewrap; never falls back to trusted mode.", theme::ACCENT),
                    }
                    ui.add_space(3.0);
                    prefs::label(ui, "Command timeout");
                    ui.spacing_mut().slider_width = (ui.available_width() - 82.0).max(80.0);
                    ui.add(egui::Slider::new(&mut settings.command_timeout_secs, 30..=7200).suffix(" s").logarithmic(true));
                    egui::CollapsingHeader::new(RichText::new("Output & cancellation").size(11.0).color(theme::MUTED)).id_salt("command_details").show(ui, |ui| {
                        prefs::help(ui, "Default: 30 minutes. Output keeps the beginning and final diagnostics; private logs survive timeout under .hfx/command-logs. Stop cancels the command group on Unix. Mode changes apply to the next run.");
                    });
                });
            });
            prefs::card(ui, "web", |ui| {
                prefs::heading(ui, Icon::Globe, "Web research", "Bring outside context into your work");
                prefs::toggle(ui, &mut settings.web_enabled, "Search & fetch", "Anonymous HTTP, without cookies or JavaScript.");
                prefs::divider(ui);
                ui.add_enabled_ui(settings.web_enabled, |ui| {
                    prefs::label(ui, "Search service");
                    egui::ComboBox::from_id_salt("search_provider").selected_text(settings.search_provider.label()).width(ui.available_width()).show_ui(ui, |ui| {
                        for provider in [SearchProvider::DuckDuckGo, SearchProvider::Brave, SearchProvider::Searxng] {
                            ui.selectable_value(&mut settings.search_provider, provider, provider.label());
                        }
                    });
                    match settings.search_provider {
                        SearchProvider::Brave => {
                            prefs::label(ui, "Brave API key · session only");
                            prefs::field(ui, &mut settings.brave_search_key, "or BRAVE_SEARCH_API_KEY", true);
                        }
                        SearchProvider::Searxng => {
                            prefs::label(ui, "SearXNG endpoint");
                            prefs::field(ui, &mut settings.searxng_url, "https://your-server/search", false);
                        }
                        SearchProvider::DuckDuckGo => prefs::help(ui, "No API key needed. Automated searches may be blocked by the service."),
                    }
                    egui::CollapsingHeader::new(RichText::new("Privacy & service requirements").size(11.0).color(theme::MUTED)).id_salt("web_details").show(ui, |ui| {
                        prefs::help(ui, "Search queries go to the selected service. Brave requires an API key; SearXNG must allow JSON search. Fetch reads HTTP/HTTPS text, including local docs, without browser cookies or JavaScript. Web content is untrusted reference data.");
                    });
                });
            });
        });
    }

    fn git_attribution_settings(&mut self, ui: &mut Ui) {
        prefs::card(ui, "git_attribution", |ui| {
            prefs::heading(
                ui,
                Icon::Plus,
                "Git attribution",
                "Your identity stays yours",
            );
            prefs::help(
                ui,
                "hfx is a co-author on commits it creates. Your Git author identity stays unchanged.",
            );
            prefs::label(ui, "Co-author email");
            ui.add(
                egui::TextEdit::singleline(&mut self.saved.settings.git_coauthor_email)
                    .id(Id::new("git_coauthor_email"))
                    .hint_text("Verified or hosting noreply email")
                    .desired_width(f32::INFINITY)
                    .margin(vec2(10.0, 9.0)),
            );
            Frame::NONE
                .fill(theme::SIDEBAR)
                .corner_radius(7)
                .inner_margin(9)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.saved.settings.git_coauthor_trailer())
                                .monospace()
                                .size(11.0)
                                .color(theme::MUTED),
                        )
                        .wrap(),
                    );
                });
            if !self.saved.settings.git_coauthor_email_is_valid() {
                ui.label(RichText::new("Enter a plain email address. Until it is valid, the safe hfx@local.invalid placeholder is used.").size(11.0).color(theme::ERROR));
            } else if self.saved.settings.git_coauthor_address()
                == crate::state::DEFAULT_GIT_COAUTHOR_EMAIL
            {
                prefs::help(
                    ui,
                    "Set the hfx profile’s verified/noreply email to link its name and avatar.",
                );
            }
            egui::CollapsingHeader::new(RichText::new("About the co-author avatar").size(11.0).color(theme::MUTED)).id_salt("attribution_details").show(ui, |ui| {
                prefs::help(ui, "GitHub/GitLab use the profile linked to this email. Set that profile’s avatar to the hfx icon; images cannot be embedded in Git trailers.");
            });
        });
    }

    fn provider_settings(&mut self, ui: &mut Ui) {
        prefs::intro(
            ui,
            "Choose your engine.",
            "Cloud intelligence or local inference. Your call.",
        );
        let width = (ui.available_width() - 8.0) / 2.0;
        for pair in [
            [Provider::Codex, Provider::OpenAI],
            [Provider::OpenRouter, Provider::Llama],
        ] {
            ui.horizontal(|ui| {
                for provider in pair {
                    if prefs::provider(
                        ui,
                        provider,
                        self.saved.settings.provider == provider,
                        width,
                    )
                    .clicked()
                    {
                        self.saved.settings.provider = provider;
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("Demo").size(11.0))
                        .selected(self.saved.settings.provider == Provider::Demo),
                )
                .clicked()
            {
                self.saved.settings.provider = Provider::Demo;
            }
            prefs::help(ui, "Just exploring? Try a scripted, offline preview.");
        });
        ui.add_space(8.0);
        match self.saved.settings.provider {
            Provider::Codex => self.codex_settings(ui),
            Provider::Demo => prefs::card(ui, "offline", |ui| {
                prefs::heading(
                    ui,
                    Icon::Spark,
                    "A space to explore",
                    "No credentials. No model requests.",
                );
                prefs::pill(ui, "OFFLINE PREVIEW", prefs::GREEN);
                prefs::help(
                    ui,
                    "Send any message for a scripted response with reasoning and streaming. Choose a provider above when you’re ready for real work.",
                );
            }),
            _ => {
                self.connection_settings(ui);
                self.generation_settings(ui);
            }
        }
        if self.saved.settings.provider != Provider::Demo {
            self.context_settings(ui);
        }
    }

    fn context_settings(&mut self, ui: &mut Ui) {
        prefs::card(ui, "context", |ui| {
            prefs::heading(
                ui,
                Icon::Queue,
                "Conversation memory",
                "Room for the long run",
            );
            prefs::toggle(
                ui,
                &mut self.saved.settings.auto_compact,
                "Auto-compaction",
                "Summarize older context at 75% capacity.",
            );
            prefs::divider(ui);
            prefs::label(ui, "Model context window · tokens");
            let key = self.saved.settings.context_key();
            let mut limit = self.saved.settings.context_limit();
            if ui
                .add(
                    egui::DragValue::new(&mut limit)
                        .range(1024..=4_000_000)
                        .speed(1024),
                )
                .changed()
            {
                self.saved
                    .settings
                    .context_windows
                    .insert(key.clone(), limit);
            }
            prefs::help(
                ui,
                &format!("Source: {}", self.saved.settings.context_limit_source()),
            );
            if self.saved.settings.context_windows.contains_key(&key)
                && ui.small_button("Use discovered limit / fallback").clicked()
            {
                self.saved.settings.context_windows.remove(&key);
            }
            prefs::help(
                ui,
                "Summaries free up model context. Your full visible chat stays intact.",
            );
            egui::CollapsingHeader::new(RichText::new("How context limits work").size(11.0).color(theme::MUTED)).id_salt("context_details").show(ui, |ui| {
                prefs::help(ui, "Uses the selected model’s discovered context window, not a universal Codex limit. Without metadata, 128,000 is an unverified fallback; set your model’s actual window here. This is separate from the maximum output length.");
            });
        });
    }

    fn generation_settings(&mut self, ui: &mut Ui) {
        let settings = &mut self.saved.settings;
        prefs::card(ui, "generation", |ui| {
            prefs::heading(ui, Icon::Spark, "Generation", "Balance depth and speed");
            prefs::effort(ui, &mut settings.effort);
            prefs::help(ui, "Supported effort levels depend on the model.");
            prefs::divider(ui);
            prefs::label(ui, "Maximum output tokens");
            ui.add(
                egui::DragValue::new(&mut settings.max_tokens)
                    .range(1024..=65536)
                    .speed(256),
            );
            if settings.provider != Provider::OpenAI {
                prefs::label(ui, "Temperature");
                ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(80.0);
                ui.add(egui::Slider::new(&mut settings.temperature, 0.0..=1.5));
            }
            prefs::help(
                ui,
                match settings.provider {
                    Provider::OpenAI => {
                        "Reasoning summaries depend on model support. Uses separate API billing."
                    }
                    Provider::OpenRouter => {
                        "Reasoning and tools depend on the model and provider. Uses your OpenRouter credits."
                    }
                    _ => "Reasoning and tools depend on the model and llama.cpp chat template.",
                },
            );
        });
    }

    fn connection_settings(&mut self, ui: &mut Ui) {
        prefs::card(ui, "connection", |ui| {
            prefs::heading(ui, Icon::Globe, "Connection", "A direct line to your model");
            let settings = &mut self.saved.settings;
            let openai = settings.provider == Provider::OpenAI;
            let router = settings.provider == Provider::OpenRouter;
            prefs::label(ui, "Base URL");
            prefs::field(ui, settings.base_url_mut(), "https://…/v1", false);
            prefs::label(ui, "API key · session only");
            let hint = if openai {
                "Or use OPENAI_API_KEY"
            } else if router {
                "Or use OPENROUTER_API_KEY"
            } else {
                "Optional · or LLAMA_API_KEY"
            };
            prefs::field(ui, settings.key_mut(), hint, true);
            prefs::label(ui, "Model ID");
            prefs::field(ui, settings.model_mut(), "Your model identifier", false);
            prefs::help(
                ui,
                if openai {
                    "Use a Responses-compatible reasoning model available to your API account."
                } else if router {
                    "Use a provider/model ID, or discover models below."
                } else {
                    "Use your llama-server alias, or discover models below."
                },
            );
            let config = format!(
                "{:?}|{}|{}",
                settings.provider,
                settings.base_url(),
                settings.key()
            );
            if config != self.probe_config {
                self.probe_config = config;
                self.probe_result = None;
                self.probe_rx = None;
            }
            ui.add_space(4.0);
            if ui
                .add_enabled_ui(self.probe_rx.is_none(), |ui| {
                    prefs::primary(
                        ui,
                        if self.probe_rx.is_some() {
                            "Connecting…"
                        } else {
                            "Test connection"
                        },
                    )
                })
                .inner
                .clicked()
            {
                let settings = settings.clone();
                let (tx, rx) = mpsc::channel();
                let ctx = ui.ctx().clone();
                self.runtime.spawn(async move {
                    let result = backend::probe(settings).await;
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
                self.probe_rx = Some(rx);
            }
            if let Some(result) = &self.probe_result {
                match result {
                    Ok(models) => {
                        for model in models {
                            if let Some(limit) = model.context_window {
                                let mut model_settings = settings.clone();
                                *model_settings.model_mut() = model.id.clone();
                                settings
                                    .discovered_context_windows
                                    .insert(model_settings.context_key(), limit);
                            }
                        }
                        prefs::pill(
                            ui,
                            &format!("CONNECTED · {} MODELS", models.len()),
                            prefs::GREEN,
                        );
                        egui::ComboBox::from_id_salt("available_models")
                            .selected_text("Choose a discovered model")
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for model in models {
                                    if ui
                                        .selectable_label(settings.model() == model.id, &model.id)
                                        .clicked()
                                    {
                                        *settings.model_mut() = model.id.clone();
                                    }
                                }
                            });
                    }
                    Err(error) => prefs::callout(ui, error, theme::ERROR),
                }
            }
        });
    }

    fn start_auth(&mut self, ctx: &egui::Context, action: AuthAction) {
        if self.auth_rx.is_some() || self.active.is_some() {
            return;
        }
        self.auth_error = None;
        self.auth_url = None;
        let auth = self.auth.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.auth_task = Some(self.runtime.spawn(async move {
            auth.authenticate(action, tx).await;
            ctx.request_repaint();
        }));
        self.auth_rx = Some(rx);
    }

    fn cancel_auth(&mut self) {
        if let Some(task) = self.auth_task.take() {
            task.abort();
        }
        self.auth_rx = None;
        self.auth_url = None;
    }

    fn codex_settings(&mut self, ui: &mut Ui) {
        if !self.auth_checked && !self.preview && self.active.is_none() {
            self.auth_checked = true;
            self.start_auth(ui.ctx(), AuthAction::Status);
        }
        prefs::card(ui, "codex_account", |ui| {
            prefs::heading(
                ui,
                Icon::Spark,
                "Your OpenAI account",
                "Codex, through your ChatGPT plan",
            );
            if self.auth_account.signed_in {
                prefs::pill(ui, "SIGNED IN", prefs::GREEN);
                ui.label(
                    RichText::new(&self.auth_account.email)
                        .size(13.0)
                        .color(theme::TEXT),
                );
                prefs::help(ui, &self.auth_account.plan);
            } else {
                prefs::help(
                    ui,
                    "Connect your ChatGPT account to start working with Codex. No API key required.",
                );
            }
            let busy = self.auth_rx.is_some();
            ui.add_space(4.0);
            ui.add_enabled_ui(!busy && self.active.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if theme::dialog_action(
                        ui,
                        if self.auth_account.signed_in {
                            "Sign in again"
                        } else {
                            "Sign in with OpenAI"
                        },
                        true,
                    )
                    .clicked()
                    {
                        self.start_auth(ui.ctx(), AuthAction::Login);
                    }
                    if ui.button("Refresh account").clicked() {
                        self.start_auth(ui.ctx(), AuthAction::Status);
                    }
                });
            });
            if busy {
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(if self.auth_url.is_some() {
                            "Complete sign-in in your browser…"
                        } else {
                            "Contacting Codex…"
                        })
                        .size(12.0),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(url) = &self.auth_url
                        && ui.button("Open sign-in again").clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        self.cancel_auth();
                    }
                });
            }
            if let Some(error) = &self.auth_error {
                prefs::callout(ui, error, theme::ERROR);
            }
            prefs::divider(ui);
            prefs::help(
                ui,
                "Secure browser sign-in. hfx receives the localhost callback and stores its login in a private local cache.",
            );
            if self.auth_account.signed_in
                && ui
                    .add_enabled(
                        !busy && self.active.is_none(),
                        egui::Button::new("Sign out of hfx"),
                    )
                    .clicked()
            {
                self.start_auth(ui.ctx(), AuthAction::Logout);
            }
        });
        prefs::card(ui, "codex_generation", |ui| {
            prefs::heading(
                ui,
                Icon::Cpu,
                "Model & reasoning",
                "Choose the depth for your next task",
            );
            prefs::label(ui, "Model ID");
            prefs::field(
                ui,
                &mut self.saved.settings.codex_model,
                "Codex model identifier",
                false,
            );
            if !self.auth_models.is_empty() {
                egui::ComboBox::from_id_salt("codex_models")
                    .selected_text("Choose a Codex model")
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for model in &self.auth_models {
                            ui.selectable_value(
                                &mut self.saved.settings.codex_model,
                                model.clone(),
                                model,
                            );
                        }
                    });
            }
            prefs::divider(ui);
            prefs::effort(ui, &mut self.saved.settings.effort);
            prefs::help(
                ui,
                "Model access and supported effort levels depend on your ChatGPT account.",
            );
        });
    }

    fn overlays(&mut self, ctx: &egui::Context, now: f64) {
        let alpha = ctx.animate_bool_with_time(
            Id::new("profile_fade"),
            self.profile_open,
            if self.saved.settings.reduced_motion {
                0.0
            } else {
                0.16
            },
        );
        if alpha > 0.01 {
            let rect = ctx.content_rect();
            let response = egui::Area::new(Id::new("profile_menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(66.0, rect.bottom() - 258.0 + 8.0 * (1.0 - alpha)))
                .show(ctx, |ui| {
                    ui.set_opacity(alpha);
                    if !self.profile_open {
                        ui.disable();
                    }
                    Frame::popup(ui.style())
                        .corner_radius(13)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(214.0);
                            ui.label(RichText::new("Your workspace").size(13.0).strong());
                            ui.label(
                                RichText::new(self.saved.settings.provider.label())
                                    .size(11.0)
                                    .color(theme::MUTED),
                            );
                            ui.add_space(6.0);
                            ui.separator();
                            if theme::row(ui, "New chat", Some(Icon::Plus), false).clicked() {
                                self.new_chat(self.project().id, now);
                                self.profile_open = false;
                            }
                            if theme::row(ui, "Settings       Ctrl+,", Some(Icon::Settings), false)
                                .clicked()
                            {
                                self.settings_open = true;
                                self.profile_open = false;
                            }
                            if theme::row(
                                ui,
                                "Reduce motion",
                                Some(Icon::Spark),
                                self.saved.settings.reduced_motion,
                            )
                            .clicked()
                            {
                                self.saved.settings.reduced_motion =
                                    !self.saved.settings.reduced_motion;
                                self.profile_open = false;
                            }
                            ui.separator();
                            if theme::row(ui, "Quit hfx", Some(Icon::Close), false).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                })
                .response;
            if self.profile_open
                && ctx.input(|i| {
                    i.pointer.any_click()
                        && i.pointer
                            .interact_pos()
                            .is_some_and(|p| !response.rect.contains(p) && p.x > 58.0)
                })
            {
                self.profile_open = false;
            }
        }
        if self.project_modal {
            let mut added = false;
            let modal = egui::Modal::new(Id::new("add_project"))
                .frame(
                    Frame::window(&ctx.style_of(egui::Theme::Dark))
                        .fill(theme::SURFACE)
                        .stroke(Stroke::new(1.0, theme::OUTLINE))
                        .corner_radius(14)
                        .inner_margin(24),
                )
                .backdrop_color(egui::Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(400.0_f32.min((ctx.content_rect().width() - 80.0).max(180.0)));
                    ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 10, theme::ACCENT.gamma_multiply(0.10));
                        theme::icon(
                            ui.painter(),
                            rect.center(),
                            18.0,
                            Icon::Folder,
                            theme::ACCENT,
                        );
                        ui.label(RichText::new("Add a project").size(20.0).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if theme::icon_button(ui, Icon::Close, "Close", false, 28.0).clicked() {
                                self.project_modal = false;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Choose a folder to give your chats a workspace.")
                            .size(12.0)
                            .color(theme::MUTED),
                    );
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("FOLDER PATH")
                            .size(10.0)
                            .strong()
                            .color(theme::MUTED),
                    );
                    let input = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_stroke =
                                Stroke::new(1.0, theme::OUTLINE);
                            ui.visuals_mut().widgets.hovered.bg_stroke =
                                Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.65));
                            ui.visuals_mut().widgets.active.bg_stroke =
                                Stroke::new(1.0, theme::ACCENT);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.project_path)
                                    .id(Id::new("project_folder_path"))
                                    .font(FontId::proportional(14.0))
                                    .hint_text("/home/you/projects/my-project")
                                    .background_color(theme::BG)
                                    .margin(Margin::symmetric(12, 11))
                                    .desired_width(f32::INFINITY),
                            )
                        })
                        .inner;
                    let submit_enter =
                        input.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter));
                    if self.project_focus {
                        input.request_focus();
                        self.project_focus = false;
                    }
                    if input.changed() {
                        self.project_error = None;
                    }
                    let valid = !self.project_path.trim().is_empty();
                    ui.label(
                        RichText::new(
                            self.project_error
                                .as_deref()
                                .unwrap_or("Use an existing folder · Enter to add · Esc to cancel"),
                        )
                        .size(11.0)
                        .color(if self.project_error.is_some() {
                            theme::ERROR
                        } else {
                            theme::DIM
                        }),
                    );
                    if submit_enter {
                        if valid {
                            added = true;
                        } else {
                            input.request_focus();
                        }
                    }
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled_ui(valid, |ui| {
                                theme::dialog_action(ui, "Add project", true)
                            })
                            .inner
                            .clicked()
                        {
                            added = true;
                        }
                        if theme::dialog_action(ui, "Cancel", false).clicked() {
                            self.project_modal = false;
                        }
                    });
                });
            if added {
                match PathBuf::from(self.project_path.trim()).canonicalize() {
                    Ok(path) if path.is_dir() => {
                        let path = path.display().to_string();
                        let id =
                            if let Some(p) = self.saved.projects.iter().find(|p| p.path == path) {
                                p.id
                            } else {
                                let p = Project::from_path(path);
                                let id = p.id;
                                self.saved.projects.push(p);
                                id
                            };
                        self.new_chat(id, now);
                        self.project_modal = false;
                        self.project_path.clear();
                        self.project_error = None;
                    }
                    _ => {
                        self.project_error = Some("Enter the path of an existing folder.".into());
                        self.project_focus = true;
                    }
                }
            }
            if modal.should_close() {
                self.project_modal = false;
            }
        }
        if self.attach_modal {
            let mut attach = false;
            let modal = egui::Modal::new(Id::new("attach_file")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.heading("Add context");
                ui.label(
                    RichText::new("Attach a text file or image. Paste and drag-and-drop work too.")
                        .size(13.0)
                        .color(theme::MUTED),
                );
                ui.add_space(12.0);
                ui.label("File path (absolute or relative to the workspace)");
                ui.add(
                    egui::TextEdit::singleline(&mut self.attach_path)
                        .hint_text("src/main.rs")
                        .desired_width(f32::INFINITY),
                );
                ui.label(
                    RichText::new("Text: 256 KiB · images: 10 MiB · up to 8 attachments")
                        .size(11.0)
                        .color(theme::DIM),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.attach_modal = false;
                    }
                    if ui.button("Attach file").clicked() {
                        attach = true;
                    }
                });
            });
            if attach {
                let path = PathBuf::from(self.attach_path.trim());
                let path = if path.is_absolute() {
                    path
                } else {
                    PathBuf::from(&self.project().path).join(path)
                };
                self.load_files(ctx, vec![path]);
                self.attach_modal = false;
                self.attach_path.clear();
                self.focus_composer = true;
            }
            if modal.should_close() {
                self.attach_modal = false;
            }
        }
        if let Some((id, mut title)) = self.rename.clone() {
            let mut save = false;
            let frame = Frame::window(&ctx.style_of(egui::Theme::Dark))
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::OUTLINE))
                .corner_radius(14)
                .inner_margin(24);
            let modal = egui::Modal::new(Id::new("rename_chat"))
                .frame(frame)
                .backdrop_color(egui::Color32::from_black_alpha(150))
                .show(ctx, |ui| {
                    ui.set_width(360.0_f32.min((ctx.content_rect().width() - 80.0).max(180.0)));
                    ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 10, theme::ACCENT.gamma_multiply(0.10));
                        theme::icon(
                            ui.painter(),
                            rect.center(),
                            18.0,
                            Icon::Pencil,
                            theme::ACCENT,
                        );
                        ui.label(RichText::new("Rename chat").size(20.0).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if theme::icon_button(ui, Icon::Close, "Close", false, 28.0).clicked() {
                                self.rename = None;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Choose a name that’s easy to find later.")
                            .size(12.0)
                            .color(theme::MUTED),
                    );
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("CHAT NAME")
                            .size(10.0)
                            .strong()
                            .color(theme::MUTED),
                    );
                    let input = ui
                        .scope(|ui| {
                            ui.visuals_mut().widgets.inactive.bg_stroke =
                                Stroke::new(1.0, theme::OUTLINE);
                            ui.visuals_mut().widgets.hovered.bg_stroke =
                                Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.65));
                            ui.visuals_mut().widgets.active.bg_stroke =
                                Stroke::new(1.0, theme::ACCENT);
                            ui.add(
                                egui::TextEdit::singleline(&mut title)
                                    .id(Id::new("rename_chat_title"))
                                    .font(FontId::proportional(14.0))
                                    .hint_text("Give this chat a name")
                                    .background_color(theme::BG)
                                    .margin(Margin::symmetric(12, 11))
                                    .desired_width(f32::INFINITY),
                            )
                        })
                        .inner;
                    if self.rename_focus {
                        input.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ctx, input.id) {
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(title.chars().count()),
                                )));
                            state.store(ctx, input.id);
                        }
                        self.rename_focus = false;
                    }
                    let valid = !title.trim().is_empty();
                    ui.label(
                        RichText::new(if valid {
                            "Enter to save · Esc to cancel"
                        } else {
                            "Please enter a chat name."
                        })
                        .size(11.0)
                        .color(if valid {
                            theme::DIM
                        } else {
                            theme::ERROR
                        }),
                    );
                    if input.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if valid {
                            save = true;
                        } else {
                            input.request_focus();
                        }
                    }
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled_ui(valid, |ui| theme::dialog_action(ui, "Save", true))
                            .inner
                            .clicked()
                        {
                            save = true;
                        }
                        if theme::dialog_action(ui, "Cancel", false).clicked() {
                            self.rename = None;
                        }
                    });
                });
            if save {
                if let Some(chat) = self.saved.chats.iter_mut().find(|c| c.id == id)
                    && !title.trim().is_empty()
                {
                    chat.title = title.trim().into();
                }
                self.rename = None;
            } else if modal.should_close() {
                self.rename = None;
            } else if self.rename.is_some() {
                self.rename = Some((id, title));
            }
        }
        if let Some((chat_id, id, mut text)) = self.queue_edit.take() {
            let queued = self
                .saved
                .chats
                .iter()
                .find(|c| c.id == chat_id)
                .and_then(|c| c.queue.iter().find(|q| q.id == id && !q.dispatched))
                .cloned();
            if let Some(queued) = queued {
                let mut save = false;
                let mut cancel = false;
                let mut valid = !text.trim().is_empty() || !queued.attachments.is_empty();
                let modal = egui::Modal::new(Id::new("edit_queued_message")).show(ctx, |ui| {
                    ui.set_width((ctx.content_rect().width() - 80.0).clamp(240.0, 440.0));
                    ui.heading("Edit queued message");
                    ui.add_space(10.0);
                    let input = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .id(Id::new("queued_message_text"))
                            .desired_width(f32::INFINITY)
                            .desired_rows(4),
                    );
                    if !ctx.memory(|m| m.has_focus(input.id)) && ctx.input(|i| i.events.is_empty())
                    {
                        input.request_focus();
                    }
                    valid = !text.trim().is_empty() || !queued.attachments.is_empty();
                    for attachment in &queued.attachments {
                        ui.label(
                            RichText::new(format!("Attached: {}", attachment.name))
                                .size(11.0)
                                .color(theme::MUTED),
                        );
                    }
                    ui.add_space(12.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled_ui(valid, |ui| theme::dialog_action(ui, "Save", true))
                            .inner
                            .clicked()
                        {
                            save = true;
                        }
                        if theme::dialog_action(ui, "Cancel", false).clicked() {
                            cancel = true;
                        }
                    });
                });
                if save && valid {
                    if let Some(queued) = self
                        .saved
                        .chats
                        .iter_mut()
                        .find(|c| c.id == chat_id)
                        .and_then(|c| c.queue.iter_mut().find(|q| q.id == id))
                    {
                        queued.text = text.trim().to_owned();
                    }
                    self.store.queue(&self.saved);
                } else if !cancel && !modal.should_close() {
                    self.queue_edit = Some((chat_id, id, text));
                }
            }
        }
        if let Some(pending) = &self.pending {
            let mut decision = None;
            egui::Modal::new(Id::new("tool_approval")).show(ctx, |ui| {
                ui.set_width(ctx.content_rect().width().min(700.0) - 80.0);
                ui.heading(if crate::mcp::is_tool(&pending.call.name) {
                    "Review MCP tool call"
                } else if matches!(pending.call.name.as_str(), "web_search" | "web_fetch") {
                    "Review web request"
                } else if pending.call.name == "write_file" {
                    "Review file change"
                } else if pending.call.name == "run_command" {
                    "Review command"
                } else {
                    "Allow workspace read?"
                });
                ui.label(
                    RichText::new(pending.workspace.display().to_string())
                        .size(12.0)
                        .color(theme::MUTED),
                );
                ui.add_space(12.0);
                let args: serde_json::Value =
                    serde_json::from_str(&pending.call.arguments).unwrap_or_default();
                if crate::mcp::is_tool(&pending.call.name) {
                    ui.label(RichText::new(&pending.call.name).color(theme::ACCENT));
                    ui.label("External server action: may access files or services outside this project.");
                    ScrollArea::vertical().id_salt("mcp_review_arguments").max_height(300.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&pending.call.arguments).monospace()).wrap().selectable(true));
                    });
                } else if pending.call.name == "write_file" {
                    ui.label(
                        RichText::new(args["path"].as_str().unwrap_or("Unknown file"))
                            .color(theme::ACCENT),
                    );
                    ui.label(
                        RichText::new(
                            "Approval replaces the file with the complete content below.",
                        )
                        .size(12.0)
                        .color(theme::MUTED),
                    );
                    ScrollArea::both()
                        .id_salt("review_content")
                        .max_height((ctx.content_rect().height() - 300.0).max(100.0))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(args["content"].as_str().unwrap_or(""))
                                        .monospace()
                                        .size(12.0),
                                )
                                .selectable(true),
                            );
                        });
                } else if pending.call.name == "run_command" {
                    ui.label(
                        RichText::new(match pending.command_mode {
                            crate::state::CommandMode::Trusted => "Runs on the host as you without a command sandbox, using normal toolchains, caches, Git/SSH, environment/API keys and desktop access. Can modify files outside the project.",
                            crate::state::CommandMode::Sandbox => "Runs inside the strict workspace filesystem sandbox; host SSH credentials and desktop connections are hidden.",
                        })
                            .size(12.0)
                            .color(theme::MUTED),
                    );
                    ui.add_space(12.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(args["command"].as_str().unwrap_or("")).monospace(),
                        )
                        .wrap()
                        .selectable(true),
                    );
                } else {
                    ui.monospace(&pending.call.arguments);
                }
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    if ui.button("Decline").clicked() {
                        decision = Some(false);
                    }
                    if ui
                        .button(if crate::mcp::is_tool(&pending.call.name) {
                            "Approve MCP call"
                        } else if pending.call.name == "write_file" {
                            "Approve and write"
                        } else if pending.call.name == "run_command" {
                            "Approve and run"
                        } else if pending.call.name == "web_search" {
                            "Approve search"
                        } else if pending.call.name == "web_fetch" {
                            "Approve fetch"
                        } else {
                            "Allow read"
                        })
                        .clicked()
                    {
                        decision = Some(true);
                    }
                });
            });
            if let Some(decision) = decision
                && let Some(pending) = self.pending.take()
            {
                let _ = pending.reply.send(decision);
            }
        }
        if let Some((message, born)) = &self.toast {
            let age = now - born;
            if age < 3.5 {
                let alpha = if self.saved.settings.reduced_motion {
                    1.0
                } else {
                    motion::ease((age / 0.2) as f32) * motion::ease(((3.5 - age) / 0.3) as f32)
                };
                egui::Area::new(Id::new("toast"))
                    .order(egui::Order::Tooltip)
                    .anchor(Align2::CENTER_TOP, [0.0, 48.0])
                    .show(ctx, |ui| {
                        ui.set_opacity(alpha);
                        Frame::popup(ui.style())
                            .inner_margin(Margin::symmetric(18, 10))
                            .corner_radius(10)
                            .show(ui, |ui| {
                                ui.label(RichText::new(message).size(12.0));
                            });
                    });
                ctx.request_repaint_after(tick_interval(ctx).max(Duration::from_millis(30)));
            } else {
                self.toast = None;
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, now: f64) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.new_chat(self.project().id, now);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.settings_open = !self.settings_open;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.sidebar = !self.sidebar;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.sidebar = true;
            self.search_open = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.image_preview.is_some() {
                self.image_preview = None;
            } else if let Some(pending) = self.pending.take() {
                let _ = pending.reply.send(false);
            } else if self.project_modal {
                self.project_modal = false;
            } else if self.attach_modal {
                self.attach_modal = false;
            } else if self.queue_edit.is_some() {
                self.queue_edit = None;
            } else if self.rename.is_some() {
                self.rename = None;
            } else if self.profile_open {
                self.profile_open = false;
            } else if self.settings_open {
                self.settings_open = false;
            } else {
                self.stop();
            }
        }
    }

    fn load_preview(&mut self, ctx: &egui::Context, kind: &str) {
        let kind = kind
            .strip_suffix("-details")
            .or_else(|| kind.strip_suffix("-advanced"))
            .unwrap_or(kind);
        if matches!(
            kind,
            "settings" | "appearance" | "tools" | "codex" | "openrouter" | "git-attribution"
        ) {
            self.settings_open = true;
            self.saved.settings.provider = Provider::OpenAI;
            if kind == "codex" {
                self.saved.settings.provider = Provider::Codex;
            } else if kind == "openrouter" {
                self.saved.settings.provider = Provider::OpenRouter;
            }
            if kind == "appearance" {
                self.settings_tab = 1;
            } else if matches!(kind, "tools" | "git-attribution") {
                self.settings_tab = 2;
            }
            return;
        }
        if kind == "menu" {
            self.profile_open = true;
            return;
        }
        if matches!(
            kind,
            "chat"
                | "markdown"
                | "reasoning"
                | "approval"
                | "actions"
                | "attachments"
                | "question"
                | "images"
                | "image-viewer"
        ) {
            let mut user = Message::new(
                true,
                "Help me build a calmer coding workspace with Rust and egui.".into(),
                0.0,
                String::new(),
            );
            user.born = 0.0;
            let mut assistant=Message::new(false,"### A workspace that stays out of your way\n\nThe shell is in place: a project sidebar, a focused composer, and a settings panel that slides into view.\n\n**Codex**, **OpenAI API**, **OpenRouter**, and **llama.cpp** feed the same streaming interface. New text arrives gently, while reasoning has its own collapsible space.\n\n```rust\nlet workspace = Harness::new(provider);\nworkspace.make_something_good();\n```\n\nConnect a provider in **Settings** and we’re ready to work.".into(),0.0,"Demo preview".into());
            assistant.reasoning="Planning the workspace\n\nThis is a scripted UI preview. The layout keeps project context close and puts the conversation first. Provider output is separated into reasoning summaries and the final response.".into();
            assistant.elapsed = 4.2;
            assistant.tokens = 284;
            self.reveals
                .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
            self.reveals.insert(
                assistant.id,
                (
                    Reveal::complete(&assistant.text),
                    Reveal::complete(&assistant.reasoning),
                ),
            );
            self.saved.chats[0].title = "Build a calmer workspace".into();
            self.saved.chats[0].messages = vec![user, assistant];
            if kind == "markdown" {
                self.saved.settings.show_reasoning = false;
                self.saved.chats[0].title = "Markdown links".into();
                self.saved.chats[0].messages[0].text =
                    "Show a commit link without exposing its Markdown syntax.".into();
                let message = &mut self.saved.chats[0].messages[1];
                message.reasoning.clear();
                message.text = "Commit: [`9e9b32d`](https://github.com/horrified-dev/hfx/commit/9e9b32d61e9cf50c9b5f598110fd40a8693111cb)\n— `docs: streamline README and add harness screenshots`\n\nLinks stay inline with **bold text**, `code`, and normal text.\n\nSee [the hfx repository](https://github.com/horrified-dev/hfx) for details.\n\nLiteral code stays literal: `[label](https://example.com)`".into();
                for message in &self.saved.chats[0].messages {
                    self.reveals.insert(
                        message.id,
                        (Reveal::complete(&message.text), Reveal::default()),
                    );
                }
            }
            if matches!(kind, "images" | "image-viewer") {
                let pixels = image::RgbaImage::from_fn(960, 540, |x, y| {
                    let grid = x % 96 == 0 || y % 54 == 0;
                    let line = (y as f32
                        - (410.0 - x as f32 * 0.27 + (x as f32 / 85.0).sin() * 40.0))
                        .abs()
                        < 4.0;
                    image::Rgba(if line {
                        [156, 198, 173, 255]
                    } else if grid {
                        [47, 58, 55, 255]
                    } else {
                        [25, 33, 31, 255]
                    })
                });
                let mut bytes = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(pixels)
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .unwrap();
                let image =
                    attachments::from_bytes("workspace-chart.png".into(), bytes.get_ref()).unwrap();
                self.saved.chats[0].messages[0].text = "Make a chart and show me the image.".into();
                let message = &mut self.saved.chats[0].messages[1];
                message.reasoning.clear();
                message.text =
                    "Here’s the chart. Click the image to enlarge it or save the PNG.".into();
                message.attachments.push(image.clone());
                message.activities = vec![
                    crate::state::Activity {
                        id: "render-preview".into(),
                        name: "run_command".into(),
                        arguments: serde_json::json!({"command":"python render_chart.py"})
                            .to_string()
                            .into(),
                        result: "Exit: 0\nSaved workspace-chart.png".into(),
                        status: ActionStatus::Complete,
                        elapsed: 0.4,
                        change: None,
                        images: Vec::new(),
                    },
                    crate::state::Activity {
                        id: "view-preview".into(),
                        name: "view_image".into(),
                        arguments: serde_json::json!({"path":"workspace-chart.png"})
                            .to_string()
                            .into(),
                        result: "Viewed image workspace-chart.png".into(),
                        status: ActionStatus::Complete,
                        elapsed: 0.1,
                        change: None,
                        images: vec![image.clone()],
                    },
                    crate::state::Activity {
                        id: "send-preview".into(),
                        name: "send_image".into(),
                        arguments: serde_json::json!({"path":"workspace-chart.png"})
                            .to_string()
                            .into(),
                        result: "Sent image workspace-chart.png".into(),
                        status: ActionStatus::Complete,
                        elapsed: 0.1,
                        change: None,
                        images: vec![image.clone()],
                    },
                ];
                for message in &self.saved.chats[0].messages {
                    self.reveals.insert(
                        message.id,
                        (Reveal::complete(&message.text), Reveal::default()),
                    );
                }
                if kind == "image-viewer" {
                    self.open_image(ctx, image);
                }
            }
            if matches!(kind, "actions" | "attachments" | "question") {
                let message = &mut self.saved.chats[0].messages[1];
                message.reasoning.clear();
                message.text = "Added a calmer welcome screen and checked the build.".into();
                message.activities = vec![
                    crate::state::Activity { id: "read-preview".into(), name: "read_file".into(), arguments: serde_json::json!({"path":"src/app.rs"}).to_string().into(), result: "Read the welcome screen layout.".into(), status: ActionStatus::Complete, elapsed: 0.1, change: None, images: Vec::new() },
                    crate::state::Activity { id: "edit-preview".into(), name: "write_file".into(), arguments: serde_json::json!({"path":"src/welcome.rs","content":"fn welcome() {\n    println!(\"Make something good.\");\n}\n"}).to_string().into(), result: "Wrote src/welcome.rs".into(), status: ActionStatus::Complete, elapsed: 0.2, change: Some(tools::file_change("src/welcome.rs", "fn welcome() {}\n", "fn welcome() {\n    println!(\"Make something good.\");\n}\n")), images: Vec::new() },
                    crate::state::Activity { id: "command-preview".into(), name: "run_command".into(), arguments: serde_json::json!({"command":"cargo check --locked"}).to_string().into(), result: "Finished dev profile in 1.2s".into(), status: ActionStatus::Complete, elapsed: 1.2, change: None, images: Vec::new() },
                ];
                self.reveals.insert(
                    message.id,
                    (Reveal::complete(&message.text), Reveal::default()),
                );
                if matches!(kind, "attachments" | "question") {
                    self.saved.chats[0].attachments.push(
                        attachments::from_bytes("welcome.rs".into(), b"fn welcome() {}\n").unwrap(),
                    );
                    let pixels = image::RgbaImage::from_fn(180, 100, |x, y| {
                        image::Rgba([28 + (x / 9) as u8, 42 + (y / 4) as u8, 48, 255])
                    });
                    let mut bytes = std::io::Cursor::new(Vec::new());
                    image::DynamicImage::ImageRgba8(pixels)
                        .write_to(&mut bytes, image::ImageFormat::Png)
                        .unwrap();
                    self.saved.chats[0].attachments.push(
                        attachments::from_bytes("Screenshot.png".into(), bytes.get_ref()).unwrap(),
                    );
                }
                if kind == "question" {
                    let question = tools::Question::parse(r#"{"question":"How should we organize the settings?","options":[{"label":"A panel beside the chat","description":"Keeps the conversation visible.","recommended":true},{"label":"A separate window","description":"More room for advanced controls.","recommended":false},{"label":"A full-screen page","description":"Focus on configuration.","recommended":false}]}"#).unwrap();
                    let (reply, _) = oneshot::channel();
                    self.question = Some(PendingQuestion {
                        id: "question-preview".into(),
                        chat: self.saved.selected,
                        selected: question.recommended(),
                        question,
                        deadline: Instant::now() + Duration::from_secs(30),
                        reply,
                        custom: String::new(),
                    });
                }
            }
            if kind == "approval" {
                let (reply, _) = oneshot::channel();
                self.pending=Some(Pending{command_mode:self.saved.settings.command_mode,call:ToolCall{id:"preview".into(),name:"write_file".into(),arguments:serde_json::json!({"path":"src/welcome.rs","content":"fn welcome() {\n    println!(\"Make something good.\");\n}\n"}).to_string()},reply,workspace:PathBuf::from(&self.project().path)});
            }
        }
    }
}

fn cancel_actions(message: &mut Message) {
    for action in &mut message.activities {
        if matches!(action.status, ActionStatus::Running | ActionStatus::Waiting) {
            action.status = ActionStatus::Cancelled;
        }
    }
}

fn changed_totals(chat: &Chat) -> (usize, usize, usize) {
    let mut paths = HashSet::new();
    let mut added = 0;
    let mut removed = 0;
    for change in chat
        .messages
        .iter()
        .flat_map(|m| &m.activities)
        .filter_map(|a| a.change.as_ref())
    {
        paths.insert(&change.path);
        added += change.added;
        removed += change.removed;
    }
    (paths.len(), added, removed)
}

fn show_diff(ui: &mut Ui, diff: &str) {
    ScrollArea::both()
        .id_salt("file_diff")
        .max_height(280.0)
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(diff).size(11.0).monospace()).selectable(true));
        });
}

impl eframe::App for Harness {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // eframe deliberately skips ui() for minimized/occluded windows. Keep
        // streaming, steering acknowledgements, queues and timeouts progressing
        // without depending on GL painting or compositor frame callbacks.
        self.poll(ctx);
        if background_window(ctx) && self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
            self.store.queue(&self.saved);
            self.last_autosave = Instant::now();
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.attachment_input(ctx, input);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.shortcuts(&ctx, now);
        let style = (
            self.saved.settings.font_size,
            self.saved.settings.reduced_motion,
        );
        if style != self.style_key {
            theme::install(&ctx, style.0, style.1);
            self.style_key = style;
        }
        self.titlebar(ui, now);
        self.rail(ui, now);
        self.sidebar_panel(ui, now);
        self.settings_panel(ui, now);
        self.main_panel(ui, now);
        self.overlays(&ctx, now);
        self.changes_window(&ctx);
        self.image_window(&ctx);
        if let Some((path, requested)) = &mut self.capture {
            let screenshot = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = screenshot {
                match image::save_buffer(
                    &*path,
                    image.as_raw(),
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(error) => eprintln!("Could not save preview: {error}"),
                }
                self.capture = None;
            } else if now > 0.8 && !*requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                *requested = true;
            } else {
                ctx.request_repaint_after(Duration::from_millis(40));
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if !self.preview {
            self.store.queue(&self.saved);
            self.last_autosave = Instant::now();
            // Remove the legacy inline history only after the new file is
            // durably saved, keeping eframe's own UI-state writes lightweight.
            if self.store.saved_once && !self.legacy_history_removed {
                storage.remove_string("hfx.v1");
                self.legacy_history_removed = true;
            }
        }
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        crate::lifecycle::begin_exit();
        let started = Instant::now();
        self.stop();
        self.cancel_auth();
        crate::lifecycle::trace("stop/cancel", started);
        let started = Instant::now();
        if let Err(error) = self.store.flush(&self.saved) {
            eprintln!("{error}");
        }
        crate::lifecycle::trace("final durable chat save", started);
    }
    fn auto_save_interval(&self) -> Duration {
        AUTOSAVE_INTERVAL
    }
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop();
        self.cancel_auth();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Activity;
    use eframe::App;

    fn draw(
        app: &mut Harness,
        ctx: &egui::Context,
        width: f32,
        height: f32,
        time: f64,
        mut events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> egui::FullOutput {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(width, height),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        app.raw_input_hook(ctx, &mut input);
        let mut output = ctx.run_ui(input, |ui| {
            let mut frame = eframe::Frame::_new_kittest();
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        // Headless layout tests intentionally have no GPU texture consumer.
        output.textures_delta.clear();
        output
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn text_position(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Pos2 {
        shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.pos + vec2(4.0, shape.galley.size().y / 2.0))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing UI text: {text}"))
    }

    fn context_ring_position(shapes: &[egui::epaint::ClippedShape]) -> egui::Pos2 {
        shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle)
                    if circle.radius == 9.0 && circle.stroke.color == theme::OUTLINE =>
                {
                    Some(circle.center)
                }
                _ => None,
            })
            .expect("context ring is visible")
    }

    fn pointer_button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn timeline_fixture() -> Message {
        let mut reply = Message::new(false, String::new(), 0.0, "Demo preview".into());
        let tool = |reply: &mut Message, name: &str, target: &str| {
            let index = reply.activities.len();
            reply.push_activity(Activity {
                id: format!("timeline-{index}"),
                name: name.into(),
                arguments: if name == "run_command" {
                    serde_json::json!({"command":target})
                } else {
                    serde_json::json!({"path":target})
                }
                .to_string()
                .into(),
                result: format!("Completed {target}").into(),
                status: ActionStatus::Complete,
                elapsed: 0.2,
                change: None,
                images: Vec::new(),
            });
        };
        reply.push_text(
            "I’ll inspect the existing file and command history before changing the layout.",
        );
        tool(&mut reply, "read_file", "src/app.rs");
        tool(&mut reply, "run_command", "rg action_history src");
        reply.push_text("\n\nThe history is now split into compact groups. Progress updates stay in the conversation where they happened.");
        tool(&mut reply, "write_file", "src/state.rs");
        tool(&mut reply, "read_file", "src/state.rs");
        tool(&mut reply, "run_command", "cargo test --locked timeline");
        reply.record_compaction();
        reply.context_checkpoint = Some(crate::state::ContextCheckpoint {
            connection: "demo".into(),
            history: Default::default(),
        });
        tool(&mut reply, "read_file", "README.md");
        tool(&mut reply, "run_command", "cargo fmt --all --check");
        reply.push_text("\n\nI’m checking saved history and streaming updates next. Each new batch of tools has its own disclosure.");
        tool(&mut reply, "write_file", "src/app.rs");
        tool(&mut reply, "read_file", "src/app.rs");
        tool(
            &mut reply,
            "run_command",
            "cargo test --locked app::tests::",
        );
        reply.push_text(
            "\n\nDone — progress updates and tool groups now alternate in chronological order.",
        );
        reply.elapsed = 52.0;
        reply
    }

    #[test]
    fn reply_timeline_renders_in_order_and_tool_groups_expand_independently() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.settings.show_reasoning = false;
        app.saved.settings.reduced_motion = true;
        app.saved.chats[0].messages = vec![timeline_fixture()];
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        // Test message geometry without the composer's viewport clipping: this
        // reply fits, but tests may inspect text in a multi-line wrapped galley.
        let positions = |output: &egui::FullOutput, prefix: &str| -> Vec<egui::Pos2> {
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.starts_with(prefix) => {
                        Some(text.pos + vec2(4.0, text.galley.size().y / 2.0))
                    }
                    _ => None,
                })
                .collect()
        };
        let reads = positions(&output, "Read files, Ran commands");
        let edits = positions(&output, "File edits, Read files, Ran commands");
        assert_eq!(reads.len(), 2);
        assert_eq!(edits.len(), 2);
        let first = positions(&output, "I’ll inspect")[0];
        let second = positions(&output, "The history is now")[0];
        let compacted = positions(&output, "Context automatically compacted")[0];
        let third = positions(&output, "I’m checking saved")[0];
        let done = positions(&output, "Done —")[0];
        assert!(first.y < reads[0].y && reads[0].y < second.y && second.y < edits[0].y);
        assert!(edits[0].y < compacted.y && compacted.y < reads[1].y && reads[1].y < third.y);
        assert!(third.y < edits[1].y && edits[1].y < done.y);
        assert!(app.action_targets.is_empty(), "all groups begin collapsed");
        click(&mut app, &ctx, reads[0], 0.2);
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.4,
            vec![],
            egui::Modifiers::NONE,
        );
        assert_eq!(
            app.action_targets.len(),
            2,
            "only the first group's arguments are parsed"
        );
        assert!(positions(&output, "Read src/app.rs").len() == 1);
        assert!(
            positions(&output, "Read README.md").is_empty(),
            "the other read group stays collapsed"
        );
        // Result/status updates must not reset the first group's open state.
        app.saved.chats[0].messages[0].activities[0].status = ActionStatus::Failed;
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.6,
            vec![],
            egui::Modifiers::NONE,
        );
        assert_eq!(positions(&output, "Read src/app.rs").len(), 1);
        assert_eq!(positions(&output, "Read README.md").len(), 0);
    }

    #[test]
    fn growing_a_live_tool_group_keeps_its_disclosure_open_when_the_label_changes() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.settings.show_reasoning = false;
        app.saved.settings.reduced_motion = true;
        let mut reply = timeline_fixture();
        reply.timeline.truncate(2);
        reply.text.truncate(match reply.timeline[0] {
            ReplyBlock::Text { end, .. } => end,
            _ => unreachable!(),
        });
        reply.activities.truncate(1);
        reply.timeline[1] = ReplyBlock::Tools { start: 0, end: 1 };
        reply.context_checkpoint = None;
        app.saved.chats[0].messages = vec![reply];
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        click(&mut app, &ctx, text_center(&output, "Read files"), 0.2);
        let reply = &mut app.saved.chats[0].messages[0];
        reply.push_activity(Activity {
            id: "new-call".into(),
            name: "run_command".into(),
            arguments: r#"{"command":"cargo test"}"#.into(),
            result: "ok".into(),
            status: ActionStatus::Running,
            elapsed: 0.0,
            change: None,
            images: Vec::new(),
        });
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.4,
            vec![],
            egui::Modifiers::NONE,
        );
        text_center(&output, "Read files, Ran commands");
        text_center(&output, "Ran cargo test · running");
        assert_eq!(app.action_targets.len(), 2);
    }

    #[test]
    fn progress_events_split_tool_batches_in_the_original_chat_without_losing_results() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
        let (events, _) = fake_active(&mut app);
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        events
            .send(Event::Text("First progress 🌿.".into()))
            .unwrap();
        for (id, name) in [("one", "read_file"), ("two", "run_command")] {
            events
                .send(Event::ToolStarted(ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments: "{}".into(),
                }))
                .unwrap();
            events
                .send(Event::ToolFinished {
                    id: id.into(),
                    output: tools::ToolOutput {
                        text: format!("Result {id}"),
                        status: ActionStatus::Complete,
                        change: None,
                        images: Vec::new(),
                    },
                    elapsed: 0.3,
                    show_in_reply: false,
                })
                .unwrap();
        }
        events.send(Event::Text("\n\n".into())).unwrap();
        events.send(Event::Text("Second progress.".into())).unwrap();
        events
            .send(Event::ToolStarted(ToolCall {
                id: "three".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }))
            .unwrap();
        events
            .send(Event::Compacted(crate::state::ContextCheckpoint {
                connection: "demo".into(),
                history: Default::default(),
            }))
            .unwrap();
        events
            .send(Event::Image(attachments::test_image()))
            .unwrap();
        events
            .send(Event::Text("\n\nFinal answer.".into()))
            .unwrap();
        app.poll(&ctx);
        let reply = &app.saved.chats[0].messages[1];
        assert!(reply.timeline_valid());
        assert_eq!(reply.timeline.len(), 7);
        assert!(matches!(
            reply.timeline[1],
            ReplyBlock::Tools { start: 0, end: 2 }
        ));
        assert!(matches!(
            reply.timeline[3],
            ReplyBlock::Tools { start: 2, end: 3 }
        ));
        assert_eq!(reply.activities[0].result.as_ref(), "Result one");
        assert_eq!(reply.activities[1].result.as_ref(), "Result two");
        assert!(app.saved.chats[1].messages.is_empty());
        app.stop();
        let reply = &app.saved.chats[0].messages[1];
        assert_eq!(reply.activities[2].status, ActionStatus::Cancelled);
        assert!(reply.timeline_valid());
        assert_eq!(reply.status, Status::Cancelled);
    }

    #[test]
    fn invalid_reply_timeline_falls_back_to_full_text_and_tools_without_panicking() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.settings.reduced_motion = true;
        let reply = &mut app.saved.chats[0].messages[1];
        reply.text = "👋 The full reply remains readable.".into();
        reply.timeline = vec![ReplyBlock::Text { start: 1, end: 3 }];
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        text_center(&output, "👋 The full reply remains readable.");
        text_center(&output, "File edits, Read files, Ran commands");
    }

    #[test]
    fn tool_activity_starts_collapsed_and_can_be_expanded_and_collapsed() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.settings.reduced_motion = true;
        let mut output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        assert!(
            app.action_targets.is_empty(),
            "tool rows are not rendered by default"
        );
        let header = "File edits, Read files, Ran commands";
        let pos = text_position(&output.shapes, header);
        for frame in 1..=3 {
            let events = if frame < 3 {
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, egui::PointerButton::Primary, frame == 1),
                ]
            } else {
                vec![]
            };
            output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                frame as f64 * 0.2,
                events,
                egui::Modifiers::NONE,
            );
        }
        assert_eq!(
            app.action_targets.len(),
            3,
            "all tool rows can still be expanded"
        );
        text_position(&output.shapes, "Ran cargo check --locked · 1.2s");
        let pos = text_position(&output.shapes, header);
        for frame in 4..=6 {
            let events = if frame < 6 {
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, egui::PointerButton::Primary, frame == 4),
                ]
            } else {
                vec![]
            };
            output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                frame as f64 * 0.2,
                events,
                egui::Modifiers::NONE,
            );
        }
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Ran cargo check"))));
        assert_eq!(
            app.saved.chats[0].messages[1].activities.len(),
            3,
            "collapse never erases activity"
        );
    }

    #[test]
    fn context_meter_tracks_current_chat_and_connection_and_handles_missing_or_oversized_usage() {
        let mut settings = Settings {
            provider: Provider::OpenAI,
            ..Default::default()
        };
        let mut chat = Chat::new(Uuid::new_v4());
        assert_eq!(ContextMeter::for_chat(&chat, &settings).used, None);
        let mut message = Message::new(false, "answer".into(), 0.0, "OpenAI API".into());
        message.context_connection = settings.context_key();
        message.context_tokens = 64000;
        chat.messages.push(message);
        let meter = ContextMeter::for_chat(&chat, &settings);
        assert_eq!(meter.fraction(), 0.5);
        assert_eq!(meter.usage_text(), "64,000 / 128,000 tokens");
        settings
            .context_windows
            .insert(settings.context_key(), 100000);
        assert_eq!(ContextMeter::for_chat(&chat, &settings).maximum, 100000);
        chat.messages[0].context_estimated = true;
        chat.messages[0].context_tokens = 150000;
        let meter = ContextMeter::for_chat(&chat, &settings);
        assert_eq!(
            meter.fraction(),
            1.0,
            "over-limit usage cannot overdraw the ring"
        );
        assert_eq!(meter.usage_text(), "~150,000 / 100,000 tokens");
        chat.messages[0].context_tokens = 0;
        assert_eq!(ContextMeter::for_chat(&chat, &settings).fraction(), 0.0);
        settings.openai_model = "another-model".into();
        assert_eq!(ContextMeter::for_chat(&chat, &settings).used, None);
        settings.openai_model = Settings::default().openai_model;
        let mut newer = Message::new(false, "other provider".into(), 0.0, "llama.cpp".into());
        newer.context_connection = "Llama|other-url|model".into();
        newer.context_tokens = 999;
        chat.messages.push(newer);
        assert_eq!(
            ContextMeter::for_chat(&chat, &settings).used,
            None,
            "never show an old connection's report as current"
        );
        assert_eq!(grouped_tokens(0), "0");
        assert_eq!(grouped_tokens(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn context_ring_is_left_of_reasoning_and_hover_shows_used_and_maximum_tokens() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.settings.provider = Provider::OpenAI;
        let connection = app.saved.settings.context_key();
        app.saved.chats[0].messages[1].context_connection = connection;
        app.saved.chats[0].messages[1].context_tokens = 64000;
        let mut output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let ring = context_ring_position(&output.shapes);
        let effort = text_position(&output.shapes, "high");
        assert!(
            ring.x + 14.0 < effort.x,
            "context meter sits immediately left of effort"
        );
        assert!((ring.y - effort.y).abs() < 2.0);
        for frame in 1..=7 {
            output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                frame as f64 * 0.2,
                if frame == 1 {
                    vec![egui::Event::PointerMoved(ring)]
                } else {
                    vec![]
                },
                egui::Modifiers::NONE,
            );
        }
        text_position(&output.shapes, "Context usage");
        text_position(&output.shapes, "64,000 / 128,000 tokens");
        text_position(
            &output.shapes,
            "Maximum source: Fallback · not reported by provider",
        );
        text_position(
            &output.shapes,
            "50.0% of context window · provider reported",
        );
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        let meter = ContextMeter::for_chat(&app.saved.chats[1], &app.saved.settings);
        assert_eq!(meter.used, None, "switching chats resets displayed usage");
        assert_eq!(meter.fraction(), 0.0);
    }

    #[test]
    fn chat_context_menu_renames_deletes_and_protects_running_chats() {
        for (action, running) in [
            ("Rename chat", false),
            ("Delete chat", false),
            ("Delete chat", true),
        ] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("chat".into()));
            let chat_id = app.saved.selected;
            // Keep the channel alive so polling doesn't complete the running chat.
            let (_tx, rx) = mpsc::channel();
            if running {
                app.active = Some(Active {
                    chat: chat_id,
                    message: app.saved.chats[0].messages[1].id,
                    workspace: PathBuf::from("."),
                    rx,
                    task: app.runtime.spawn(std::future::pending::<()>()),
                    started: Instant::now(),
                    steering: None,
                });
            }
            let mut output = draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                0.0,
                vec![],
                egui::Modifiers::NONE,
            );
            let pos = text_position(&output.shapes, &app.saved.chats[0].title);
            for frame in 1..=3 {
                let events = if frame < 3 {
                    vec![
                        egui::Event::PointerMoved(pos),
                        pointer_button(pos, egui::PointerButton::Secondary, frame == 1),
                    ]
                } else {
                    vec![]
                };
                output = draw(
                    &mut app,
                    &ctx,
                    720.0,
                    540.0,
                    frame as f64 * 0.2,
                    events,
                    egui::Modifiers::NONE,
                );
            }
            let pos = text_position(&output.shapes, action);
            for frame in 4..=5 {
                draw(
                    &mut app,
                    &ctx,
                    720.0,
                    540.0,
                    frame as f64 * 0.2,
                    vec![
                        egui::Event::PointerMoved(pos),
                        pointer_button(pos, egui::PointerButton::Primary, frame == 4),
                    ],
                    egui::Modifiers::NONE,
                );
            }
            if action == "Rename chat" {
                assert_eq!(app.rename.as_ref().map(|rename| rename.0), Some(chat_id));
            } else {
                assert_eq!(
                    app.saved.chats.iter().any(|chat| chat.id == chat_id),
                    running
                );
            }
        }
    }

    #[test]
    fn deleting_another_chat_preserves_the_live_turn_tools_and_steering() {
        for select_deleted in [false, true] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("actions".into()));
            let original = app.saved.selected;
            let reply_id = app.saved.chats[0].messages[1].id;
            app.saved.settings.provider = Provider::OpenAI;
            app.saved.settings.reduced_motion = true;
            app.saved.settings.show_reasoning = false;
            app.saved.chats[0].messages[1].text = "Live progress".into();
            let (events, mut steering) = fake_active(&mut app);
            app.saved.chats[0].messages[1].activities[0].status = ActionStatus::Running;
            app.saved.chats[0].messages[1].activities[1].status = ActionStatus::Waiting;
            app.saved.chats[0].draft = "Keep this guidance".into();
            app.submit(&ctx, true);
            let accepted = steering.try_recv().unwrap();

            let mut other = Chat::new(app.project().id);
            other.title = "Chat to delete".into();
            let other_id = other.id;
            let user = Message::new(true, "Old request".into(), 0.0, String::new());
            let removed_message = user.id;
            app.reveals
                .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
            other.messages.push(user);
            app.saved.chats.push(other);
            if select_deleted {
                app.saved.selected = other_id;
            }
            let mut output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                0.0,
                vec![],
                egui::Modifiers::NONE,
            );
            let pos = text_position(&output.shapes, "Chat to delete");
            for frame in 1..=3 {
                output = draw(
                    &mut app,
                    &ctx,
                    1180.0,
                    820.0,
                    frame as f64 * 0.2,
                    if frame < 3 {
                        vec![
                            egui::Event::PointerMoved(pos),
                            pointer_button(pos, egui::PointerButton::Secondary, frame == 1),
                        ]
                    } else {
                        vec![]
                    },
                    egui::Modifiers::NONE,
                );
            }
            let pos = text_position(&output.shapes, "Delete chat");
            let output = click(&mut app, &ctx, pos, 0.8);

            assert!(!app.saved.chats.iter().any(|c| c.id == other_id));
            assert!(!app.reveals.contains_key(&removed_message));
            assert_eq!(app.saved.selected, original);
            let active = app.active.as_ref().unwrap();
            assert_eq!((active.chat, active.message), (original, reply_id));
            assert!(!active.task.is_finished());
            let chat = &app.saved.chats[0];
            assert_eq!(chat.messages[1].status, Status::Streaming);
            assert_eq!(chat.messages[1].activities[0].status, ActionStatus::Running);
            assert_eq!(chat.messages[1].activities[1].status, ActionStatus::Waiting);
            assert!(!chat.queue_paused);
            assert_eq!(chat.queue[0].id, accepted.id);
            assert!(chat.queue[0].dispatched);
            text_center(&output, "Working");
            let tool_ids = chat.messages[1].activities[..2]
                .iter()
                .map(|a| a.id.clone())
                .collect::<Vec<_>>();

            // Deletion must not allow an already dispatched steer to be sent twice.
            app.steer_queued(original, accepted.id, &ctx);
            assert!(steering.try_recv().is_err());
            for id in tool_ids {
                events
                    .send(Event::ToolFinished {
                        id,
                        output: tools::ToolOutput {
                            text: "Finished after deletion".into(),
                            status: ActionStatus::Complete,
                            change: None,
                            images: Vec::new(),
                        },
                        elapsed: 0.2,
                        show_in_reply: false,
                    })
                    .unwrap();
            }
            events.send(Event::Steered(vec![accepted])).unwrap();
            events
                .send(Event::Text("Continued after deletion".into()))
                .unwrap();
            app.poll(&ctx);
            let chat = &app.saved.chats[0];
            assert!(chat.queue.is_empty());
            assert_eq!(chat.messages[1].status, Status::Complete);
            for activity in &chat.messages[1].activities[..2] {
                assert_eq!(activity.status, ActionStatus::Complete);
                assert_eq!(activity.result.as_ref(), "Finished after deletion");
            }
            assert_eq!(chat.messages[3].status, Status::Streaming);
            assert_eq!(chat.messages[3].text, "Continued after deletion");
            events.send(Event::Completed).unwrap();
            app.poll(&ctx);
            assert_eq!(app.saved.chats[0].messages[3].status, Status::Complete);
            assert!(app.active.is_none());
        }
    }

    #[test]
    fn compaction_events_update_the_original_chat_and_persist_without_erasing_visible_history() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("chat".into()));
        let chat = app.saved.selected;
        let message = app.saved.chats[0].messages[1].id;
        let visible_text = app.saved.chats[0].messages[1].text.clone();
        let (tx, rx) = mpsc::channel();
        app.active = Some(Active {
            chat,
            message,
            workspace: PathBuf::from("."),
            rx,
            task: app.runtime.spawn(std::future::pending::<()>()),
            started: Instant::now(),
            steering: None,
        });
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        tx.send(Event::CompactionStarted).unwrap();
        app.poll(&ctx);
        assert!(app.saved.chats[0].messages[1].compacting);
        let snapshot = crate::context::checkpoint(
            &app.saved.settings,
            vec![
                serde_json::json!({"role":"user","content":"Summary: keep the workspace decisions"}),
            ],
        );
        tx.send(Event::Compacted(snapshot)).unwrap();
        tx.send(Event::ContextUsage {
            tokens: 3000,
            estimated: true,
            connection: app.saved.settings.context_key(),
        })
        .unwrap();
        tx.send(Event::ResponsesContext(vec![
            serde_json::json!({"role":"assistant","content":"Continued answer"}),
        ]))
        .unwrap();
        tx.send(Event::Completed).unwrap();
        app.poll(&ctx);
        assert!(app.active.is_none());
        assert!(!app.saved.chats[0].messages[1].compacting);
        assert_eq!(app.saved.chats[0].messages[1].text, visible_text);
        assert!(app.saved.chats[1].messages.is_empty());
        let restored: Saved =
            serde_json::from_str(&serde_json::to_string(&app.saved).unwrap()).unwrap();
        let message = &restored.chats[0].messages[1];
        assert!(message.context_checkpoint.is_some());
        assert_eq!(message.context_tokens, 3000);
        assert!(message.response_items[0]["content"] == "Continued answer");
    }

    #[test]
    fn project_dialog_focus_validation_add_duplicates_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        let initial_projects = app.saved.projects.len();
        app.project_modal = true;
        app.project_focus = true;
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        assert!(ctx.memory(|m| m.has_focus(Id::new("project_folder_path"))));
        let output = draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.2,
            vec![],
            egui::Modifiers::NONE,
        );
        let pos = text_position(&output.shapes, "Add project");
        for frame in 2..=3 {
            draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                frame as f64 * 0.2,
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, egui::PointerButton::Primary, frame == 2),
                ],
                egui::Modifiers::NONE,
            );
        }
        assert!(app.project_modal, "blank submission is disabled");
        assert_eq!(app.saved.projects.len(), initial_projects);
        app.project_path = root.path().join("missing").display().to_string();
        ctx.memory_mut(|m| m.request_focus(Id::new("project_folder_path")));
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            0.8,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.project_modal);
        assert!(app.project_error.is_some());
        app.project_path = root.path().display().to_string();
        ctx.memory_mut(|m| m.request_focus(Id::new("project_folder_path")));
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            1.0,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(!app.project_modal);
        assert!(app.project_error.is_none());
        assert_eq!(app.saved.projects.len(), initial_projects + 1);
        assert_eq!(
            app.project().path,
            root.path().canonicalize().unwrap().display().to_string()
        );
        app.project_modal = true;
        app.project_path = root.path().display().to_string();
        ctx.memory_mut(|m| m.request_focus(Id::new("project_folder_path")));
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            1.2,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(
            app.saved.projects.len(),
            initial_projects + 1,
            "existing folders aren't duplicated"
        );
        app.project_modal = true;
        draw(
            &mut app,
            &ctx,
            720.0,
            540.0,
            1.4,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(!app.project_modal);
    }

    #[test]
    fn rename_dialog_focus_validation_save_and_cancel() {
        for action in ["Enter", "Save", "Cancel", "Escape", "Blank"] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("chat".into()));
            let original = app.saved.chats[0].title.clone();
            app.rename = Some((app.saved.selected, original.clone()));
            app.rename_focus = true;
            draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                0.0,
                vec![],
                egui::Modifiers::NONE,
            );
            draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                0.2,
                vec![],
                egui::Modifiers::NONE,
            );
            assert!(ctx.memory(|memory| memory.has_focus(Id::new("rename_chat_title"))));
            let name = if action == "Blank" {
                "   "
            } else {
                "  A better name 🌿  "
            };
            let output = draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                0.4,
                vec![egui::Event::Text(name.into())],
                egui::Modifiers::NONE,
            );
            assert_eq!(
                app.rename.as_ref().unwrap().1,
                name,
                "opening selects the old name"
            );
            match action {
                "Enter" | "Escape" => {
                    draw(
                        &mut app,
                        &ctx,
                        720.0,
                        540.0,
                        0.6,
                        vec![key(
                            if action == "Enter" {
                                egui::Key::Enter
                            } else {
                                egui::Key::Escape
                            },
                            egui::Modifiers::NONE,
                        )],
                        egui::Modifiers::NONE,
                    );
                }
                "Save" | "Cancel" | "Blank" => {
                    let pos = text_position(
                        &output.shapes,
                        if action == "Cancel" { "Cancel" } else { "Save" },
                    );
                    for frame in 3..=4 {
                        draw(
                            &mut app,
                            &ctx,
                            720.0,
                            540.0,
                            frame as f64 * 0.2,
                            vec![
                                egui::Event::PointerMoved(pos),
                                pointer_button(pos, egui::PointerButton::Primary, frame == 3),
                            ],
                            egui::Modifiers::NONE,
                        );
                    }
                }
                _ => unreachable!(),
            }
            if matches!(action, "Enter" | "Save") {
                assert_eq!(app.saved.chats[0].title, "A better name 🌿");
                assert!(app.rename.is_none());
            } else {
                assert_eq!(app.saved.chats[0].title, original);
                assert_eq!(app.rename.is_some(), action == "Blank");
            }
        }
    }

    #[test]
    fn typing_keeps_large_tool_history_shared_while_background_saving() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        let mut message = Message::new(
            false,
            "Finished the workspace changes.".into(),
            0.0,
            "OpenRouter".into(),
        );
        message.response_items = vec![serde_json::json!({"role":"tool","tool_call_id":"large","content":"x".repeat(8*1024*1024)})].into();
        let arguments: std::sync::Arc<str> =
            serde_json::json!({"path":"large.rs","content":"y".repeat(256*1024)})
                .to_string()
                .into();
        for index in 0..20 {
            message.activities.push(Activity {
                id: format!("call-{index}"),
                name: "write_file".into(),
                arguments: arguments.clone(),
                result: "z".repeat(65536).into(),
                status: ActionStatus::Complete,
                elapsed: 0.1,
                change: None,
                images: Vec::new(),
            });
        }
        let wire = message.response_items.clone();
        let id = message.id;
        app.reveals.insert(
            id,
            (
                Reveal::complete(&message.text),
                Reveal::complete(&message.reasoning),
            ),
        );
        app.saved.chats[0].messages.push(message);
        let root = tempfile::tempdir().unwrap();
        app.store = persistence::Store::new(Some(root.path().join("chats.json")));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        app.store.queue(&app.saved);
        let typed = "Typing stays responsive 🌿";
        let mut timings = Vec::new();
        for (index, character) in typed.chars().enumerate() {
            let start = Instant::now();
            draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                0.2 + index as f64 * 0.1,
                vec![egui::Event::Text(character.to_string())],
                egui::Modifiers::NONE,
            );
            timings.push(start.elapsed());
        }
        assert_eq!(app.saved.chats[0].draft, typed);
        assert!(std::sync::Arc::ptr_eq(
            &wire,
            &app.saved.chats[0].messages[0].response_items
        ));
        assert!(std::sync::Arc::ptr_eq(
            &arguments,
            &app.saved.chats[0].messages[0].activities[0].arguments
        ));
        assert!(
            app.action_targets.is_empty(),
            "collapsed tools do not parse large arguments while typing"
        );
        assert_eq!(app.saved.chats[0].messages[0].activities.len(), 20);
        timings.sort();
        eprintln!(
            "Typing with 14 MiB of tool history: median {:?}, slowest {:?}",
            timings[timings.len() / 2],
            timings.last().unwrap()
        );
        app.store.flush(&app.saved).unwrap();
        assert_eq!(
            persistence::read(&root.path().join("chats.json"))
                .unwrap()
                .unwrap()
                .chats[0]
                .draft,
            typed
        );
    }

    // Software rasterization keeps visual QA off the user's live desktop.
    // This opt-in test writes previews from real egui geometry and font textures.
    #[test]
    #[ignore = "writes headless visual QA artifacts"]
    fn export_headless_previews() {
        for (preview, width, height) in [
            ("settings", 1180, 820),
            ("settings", 720, 540),
            ("appearance", 1180, 820),
            ("appearance", 720, 540),
            ("tools", 1180, 820),
            ("tools", 720, 540),
            ("codex", 1180, 820),
            ("settings-details", 1180, 820),
            ("settings-details", 720, 540),
            ("appearance-details", 1180, 820),
            ("appearance-details", 720, 540),
            ("tools-details", 1180, 820),
            ("tools-details", 720, 540),
            ("tools-advanced", 1180, 820),
            ("markdown", 1180, 820),
            ("markdown", 720, 540),
            ("agent-tools", 1180, 820),
            ("agent-tools", 720, 540),
            ("web-tools", 1180, 820),
            ("web-tools", 720, 540),
            ("git-attribution", 1180, 820),
            ("git-attribution", 720, 540),
            ("icon-alignment", 1180, 820),
            ("icon-alignment", 720, 540),
            ("tool-timeline", 1180, 980),
            ("tool-timeline", 720, 540),
            ("message-queue", 1180, 820),
            ("message-queue", 720, 540),
            ("message-queue-paused", 720, 540),
            ("context-meter", 1180, 820),
            ("context-meter", 720, 540),
            ("context-meter-hover", 1180, 820),
            ("welcome", 1180, 820),
            ("welcome", 1180, 600),
            ("attachments", 1180, 820),
            ("question", 1180, 820),
            ("question", 720, 540),
            ("images", 1180, 820),
            ("images", 720, 540),
            ("image-viewer", 1180, 820),
            ("image-viewer", 720, 540),
            ("chat-menu", 1180, 820),
            ("chat-menu", 720, 540),
            ("chat-menu-hover", 720, 540),
            ("rename-dialog", 1180, 820),
            ("rename-dialog", 720, 540),
            ("rename-dialog-empty", 720, 540),
            ("project-dialog", 1180, 820),
            ("project-dialog", 720, 540),
            ("project-dialog-error", 720, 540),
        ] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            if let Ok(filter) = std::env::var("HFX_PREVIEW_FILTER")
                && !preview.starts_with(&filter)
            {
                continue;
            }
            let is_chat_menu = preview.starts_with("chat-menu");
            let mut app = Harness::new(
                &cc,
                Some(
                    if preview == "icon-alignment"
                        || preview.starts_with("context-meter")
                        || preview.starts_with("message-queue")
                        || preview.starts_with("tool-timeline")
                    {
                        "actions"
                    } else if is_chat_menu
                        || preview.starts_with("rename-dialog")
                        || preview.starts_with("project-dialog")
                    {
                        "chat"
                    } else {
                        preview
                    }
                    .into(),
                ),
            );
            app.saved.settings.reduced_motion = true;
            if preview == "agent-tools" {
                app.settings_open = true;
                app.settings_tab = 2;
            }
            if preview == "web-tools" {
                app.saved.settings.show_reasoning = false;
                let mut message = Message::new(false, "I searched the web and checked the source documentation.\n\nThe results include their original URLs, so you can verify the details.".into(), 0.0, "Codex".into());
                for (id, name, args, result) in [
                    (
                        "search",
                        "web_search",
                        serde_json::json!({"query":"Rust official documentation"}),
                        serde_json::json!({"kind":"web_search","results":[{"title":"Rust Documentation","url":"https://doc.rust-lang.org/","snippet":"Official language and standard library references."}]}),
                    ),
                    (
                        "fetch",
                        "web_fetch",
                        serde_json::json!({"url":"https://doc.rust-lang.org/"}),
                        serde_json::json!({"kind":"web_fetch","title":"Rust Documentation","url":"https://doc.rust-lang.org/","content":"Official Rust documentation, including the book, standard library, and compiler reference.","truncated":false}),
                    ),
                ] {
                    message.activities.push(crate::state::Activity {
                        id: id.into(),
                        name: name.into(),
                        arguments: args.to_string().into(),
                        result: result.to_string().into(),
                        status: ActionStatus::Complete,
                        elapsed: 0.3,
                        change: None,
                        images: Vec::new(),
                    });
                }
                app.reveals.insert(
                    message.id,
                    (Reveal::complete(&message.text), Reveal::default()),
                );
                app.saved.chats[0].messages = vec![message];
            }
            if preview == "icon-alignment" {
                app.saved.projects[0].name = "hfx".into();
                app.saved.settings.provider = Provider::Codex;
                let reply = Message::new(false, "The reply icon, project header, and composer folder share their text row’s vertical center.".into(), 0.0, "Codex".into());
                app.saved.chats[0].messages = vec![reply];
            }
            if preview.starts_with("tool-timeline") {
                app.saved.settings.show_reasoning = false;
                app.saved.chats[0].messages = vec![timeline_fixture()];
            }
            if preview.starts_with("context-meter") {
                app.saved.settings.provider = Provider::Codex;
                let connection = app.saved.settings.context_key();
                app.saved
                    .settings
                    .discovered_context_windows
                    .insert(connection.clone(), 272000);
                let message = &mut app.saved.chats[0].messages[1];
                message.context_tokens = 136000;
                message.context_connection = connection;
            }
            if preview.starts_with("message-queue") {
                app.saved.settings.provider = Provider::Codex;
                let connection = app.saved.settings.context_key();
                app.saved
                    .settings
                    .discovered_context_windows
                    .insert(connection.clone(), 272000);
                let message = &mut app.saved.chats[0].messages[1];
                message.text = "I’m checking the remaining changes and tests.".into();
                message.context_connection = connection;
                message.context_tokens = 45000;
                app.reveals.insert(
                    message.id,
                    (Reveal::complete(&message.text), Reveal::default()),
                );
                app.saved.chats[0].queue.push(QueuedMessage::new(
                    "That’s awesome — also add keyboard shortcuts.".into(),
                    Vec::new(),
                ));
                if preview.ends_with("paused") {
                    app.saved.chats[0].queue_paused = true;
                } else {
                    let (events, rx) = mpsc::channel();
                    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                    let message = &mut app.saved.chats[0].messages[1];
                    message.status = Status::Streaming;
                    app.active = Some(Active {
                        chat: app.saved.selected,
                        message: message.id,
                        workspace: PathBuf::from("."),
                        rx,
                        steering: Some(sender),
                        started: Instant::now(),
                        task: app.runtime.spawn(async move {
                            let _channels = (events, receiver);
                            std::future::pending::<()>().await;
                        }),
                    });
                }
            }
            if preview.starts_with("rename-dialog") {
                app.rename = Some((
                    app.saved.selected,
                    if preview.ends_with("empty") {
                        String::new()
                    } else {
                        "hey".into()
                    },
                ));
                app.rename_focus = true;
            }
            if preview.starts_with("project-dialog") {
                app.project_modal = true;
                app.project_focus = true;
                app.project_path = app.project().path.clone();
                if preview.ends_with("error") {
                    app.project_path = "/missing/project".into();
                    app.project_error = Some("Enter the path of an existing folder.".into());
                }
            }
            let mut menu_anchor = pos2(0.0, 0.0);
            let mut textures: HashMap<egui::TextureId, egui::ColorImage> = HashMap::new();
            let mut shapes = Vec::new();
            for frame in 0..8 {
                let mut events = Vec::new();
                if is_chat_menu {
                    if frame == 2 || frame == 3 {
                        events.push(egui::Event::PointerMoved(menu_anchor));
                        events.push(pointer_button(
                            menu_anchor,
                            egui::PointerButton::Secondary,
                            frame == 2,
                        ));
                    } else if frame >= 5 {
                        let pos = if preview.ends_with("hover") {
                            text_position(&shapes, "Delete chat")
                        } else {
                            pos2(600.0, 400.0)
                        };
                        events.push(egui::Event::PointerMoved(pos));
                    }
                }
                if preview == "web-tools" && matches!(frame, 2 | 3) {
                    let position = text_position(&shapes, "Searched web, Fetched pages");
                    events.push(egui::Event::PointerMoved(position));
                    events.push(pointer_button(
                        position,
                        egui::PointerButton::Primary,
                        frame == 2,
                    ));
                }
                if (preview.ends_with("-details") || preview.ends_with("-advanced")) && frame == 2 {
                    events.push(egui::Event::PointerMoved(pos2(width as f32 - 175.0, 260.0)));
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: vec2(
                            0.0,
                            if preview == "tools-details" {
                                -325.0
                            } else {
                                -4000.0
                            },
                        ),
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                if preview == "git-attribution" && frame == 2 {
                    events.push(egui::Event::PointerMoved(pos2(width as f32 - 175.0, 260.0)));
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: vec2(0.0, -360.0),
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                if preview == "context-meter-hover" && frame == 2 {
                    events.push(egui::Event::PointerMoved(context_ring_position(&shapes)));
                }
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(width as f32, height as f32),
                    )),
                    time: Some(frame as f64 * 0.2),
                    events,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let mut frame = eframe::Frame::_new_kittest();
                    app.logic(ui.ctx(), &mut frame);
                    app.ui(ui, &mut frame);
                });
                for (id, updates) in &output.textures_delta.set {
                    for update in updates {
                        let egui::ImageData::Color(image) = &update.image;
                        if let Some([x, y]) = update.pos {
                            let destination = textures.get_mut(id).expect("Texture was allocated");
                            for row in 0..image.height() {
                                let start = (y + row) * destination.width() + x;
                                destination.pixels[start..start + image.width()].copy_from_slice(
                                    &image.pixels[row * image.width()..(row + 1) * image.width()],
                                );
                            }
                        } else {
                            textures.insert(*id, (**image).clone());
                        }
                    }
                }
                output.textures_delta.clear();
                shapes = output.shapes;
                if is_chat_menu && frame == 1 {
                    menu_anchor = text_position(&shapes, &app.saved.chats[0].title);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let meshes = ctx.tessellate(shapes, 1.0);
            let mut canvas =
                image::RgbaImage::from_pixel(width, height, image::Rgba([23, 24, 25, 255]));
            for primitive in meshes {
                let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                    continue;
                };
                let texture = textures.get(&mesh.texture_id).expect("Mesh texture exists");
                let clip = primitive.clip_rect.intersect(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(width as f32, height as f32),
                ));
                for triangle in mesh.indices.as_chunks::<3>().0 {
                    let v = [
                        mesh.vertices[triangle[0] as usize],
                        mesh.vertices[triangle[1] as usize],
                        mesh.vertices[triangle[2] as usize],
                    ];
                    let cross = |a: egui::Pos2, b: egui::Pos2, p: egui::Pos2| {
                        (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
                    };
                    let area = cross(v[0].pos, v[1].pos, v[2].pos);
                    if area.abs() < 0.0001 {
                        continue;
                    }
                    let min_x = v
                        .iter()
                        .map(|v| v.pos.x)
                        .fold(f32::INFINITY, f32::min)
                        .floor()
                        .max(clip.left())
                        .max(0.0) as u32;
                    let max_x = v
                        .iter()
                        .map(|v| v.pos.x)
                        .fold(f32::NEG_INFINITY, f32::max)
                        .ceil()
                        .min(clip.right())
                        .min(width as f32) as u32;
                    let min_y = v
                        .iter()
                        .map(|v| v.pos.y)
                        .fold(f32::INFINITY, f32::min)
                        .floor()
                        .max(clip.top())
                        .max(0.0) as u32;
                    let max_y = v
                        .iter()
                        .map(|v| v.pos.y)
                        .fold(f32::NEG_INFINITY, f32::max)
                        .ceil()
                        .min(clip.bottom())
                        .min(height as f32) as u32;
                    for y in min_y..max_y {
                        for x in min_x..max_x {
                            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                            let weights = [
                                cross(v[1].pos, v[2].pos, p) / area,
                                cross(v[2].pos, v[0].pos, p) / area,
                                cross(v[0].pos, v[1].pos, p) / area,
                            ];
                            if weights.iter().any(|&w| w < 0.0) {
                                continue;
                            }
                            let u = (0..3).map(|i| weights[i] * v[i].uv.x).sum::<f32>();
                            let t = (0..3).map(|i| weights[i] * v[i].uv.y).sum::<f32>();
                            let tx =
                                ((u * texture.width() as f32) as usize).min(texture.width() - 1);
                            let ty =
                                ((t * texture.height() as f32) as usize).min(texture.height() - 1);
                            let sampled = texture.pixels[ty * texture.width() + tx].to_array();
                            let mut color = [0.0; 4];
                            for channel in 0..4 {
                                color[channel] = (0..3)
                                    .map(|i| weights[i] * v[i].color.to_array()[channel] as f32)
                                    .sum::<f32>()
                                    * sampled[channel] as f32
                                    / 255.0;
                            }
                            let pixel = canvas.get_pixel_mut(x, y);
                            for channel in 0..3 {
                                pixel[channel] = (color[channel]
                                    + pixel[channel] as f32 * (1.0 - color[3] / 255.0))
                                    .clamp(0.0, 255.0)
                                    as u8;
                            }
                        }
                    }
                }
            }
            std::fs::create_dir_all("artifacts").unwrap();
            canvas
                .save(format!("artifacts/headless-{preview}-{width}x{height}.png"))
                .unwrap();
        }
    }

    #[test]
    fn reply_and_project_icons_are_centered_on_their_rendered_text_rows() {
        for (width, height) in [(1180.0, 820.0), (720.0, 540.0)] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("chat".into()));
            app.saved.settings.reduced_motion = true;
            app.saved.projects[0].name = "Alignment project".into();
            app.saved.chats[0].messages = vec![Message::new(
                false,
                "Aligned reply".into(),
                0.0,
                "Codex".into(),
            )];
            let output = draw(
                &mut app,
                &ctx,
                width,
                height,
                0.0,
                vec![],
                egui::Modifiers::NONE,
            );
            let text_rect = |label: &str, size: f32| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if text.galley.job.text == label
                                && text.galley.job.sections[0].format.font_id.size == size =>
                        {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing text row: {label} ({size})"))
            };
            for (label, size, color, pieces) in [
                ("hfx", 13.0, theme::ACCENT, 3),
                ("Alignment project", 14.0, theme::MUTED, 2),
                ("Alignment project", 11.0, theme::MUTED, 2),
            ] {
                let text = text_rect(label, size);
                let region = egui::Rect::from_min_max(
                    text.min - vec2(30.0, 15.0),
                    pos2(text.left(), text.bottom() + 15.0),
                );
                let glyphs: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        let stroke_color = match &shape.shape {
                            egui::Shape::LineSegment { stroke, .. } => stroke.color,
                            egui::Shape::Path(path) => match path.stroke.color {
                                egui::epaint::ColorMode::Solid(color) => color,
                                _ => return None,
                            },
                            _ => return None,
                        };
                        let bounds = shape.shape.visual_bounding_rect();
                        (stroke_color == color && region.contains(bounds.center()))
                            .then_some(bounds)
                    })
                    .collect();
                assert_eq!(
                    glyphs.len(),
                    pieces,
                    "missing leading icon for {label} at {width}x{height}"
                );
                let bounds = glyphs.into_iter().reduce(|a, b| a.union(b)).unwrap();
                assert!(
                    (bounds.center().y - text.center().y).abs() < 1.0,
                    "{label} icon at {:?} is not centered on text {:?}",
                    bounds,
                    text
                );
            }
        }
    }

    #[test]
    fn git_attribution_email_ui_edits_validate_without_changing_custom_instructions() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("git-attribution".into()));
        app.saved.settings.system_prompt = "Keep my instructions".into();
        assert!(app.settings_open);
        assert_eq!(app.settings_tab, 2);
        let mut render = |time, mut events: Vec<egui::Event>, modifiers| {
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(340.0, 600.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| app.git_attribution_settings(ui),
            );
            output.textures_delta.clear();
            output
        };
        render(0.0, vec![], egui::Modifiers::NONE);
        ctx.memory_mut(|m| m.request_focus(Id::new("git_coauthor_email")));
        for (time, address, trailer, warning) in [
            (
                0.2,
                "12345+fixture@users.noreply.github.com",
                "Co-authored-by: hfx <12345+fixture@users.noreply.github.com>",
                false,
            ),
            (
                0.4,
                "invalid address",
                "Co-authored-by: hfx <hfx@local.invalid>",
                true,
            ),
        ] {
            let output = render(
                time,
                vec![
                    key(egui::Key::A, egui::Modifiers::COMMAND),
                    egui::Event::Paste(address.into()),
                ],
                egui::Modifiers::COMMAND,
            );
            text_position(&output.shapes, trailer);
            assert_eq!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Enter a plain email address"))), warning);
        }
        assert_eq!(app.saved.settings.git_coauthor_email, "invalid address");
        assert_eq!(app.saved.settings.system_prompt, "Keep my instructions");
    }

    #[test]
    fn settings_cards_fit_the_drawer_for_all_providers_and_tool_modes() {
        use crate::state::{CommandMode, SearchProvider};
        for width in [304.0, 372.0] {
            for font_size in [13.0, 15.5, 19.0] {
                let ctx = egui::Context::default();
                let cc = eframe::CreationContext::_new_kittest(ctx.clone());
                let mut app = Harness::new(&cc, Some("settings".into()));
                app.saved.settings.font_size = font_size;
                app.saved.settings.reveal_speed = 137.5;
                for tab in 0..3 {
                    for (provider, search, mode) in [
                        (
                            Provider::Codex,
                            SearchProvider::DuckDuckGo,
                            CommandMode::Trusted,
                        ),
                        (
                            Provider::OpenAI,
                            SearchProvider::Brave,
                            CommandMode::Sandbox,
                        ),
                        (
                            Provider::OpenRouter,
                            SearchProvider::Searxng,
                            CommandMode::Trusted,
                        ),
                        (
                            Provider::Llama,
                            SearchProvider::DuckDuckGo,
                            CommandMode::Sandbox,
                        ),
                        (Provider::Demo, SearchProvider::Brave, CommandMode::Trusted),
                    ] {
                        app.saved.settings.provider = provider;
                        app.saved.settings.search_provider = search;
                        app.saved.settings.command_mode = mode;
                        let mut output = ctx.run_ui(egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 2000.0))),
                            ..Default::default()
                        }, |ui| {
                            prefs::style(ui);
                            let right = ui.max_rect().right();
                            prefs::tabs(ui, &mut app.settings_tab);
                            match tab {
                                0 => app.provider_settings(ui),
                                1 => app.appearance_settings(ui),
                                _ => app.tool_settings(ui),
                            }
                            assert!(ui.min_rect().right() <= right + 0.5,
                                "tab {tab}, {provider:?}, width {width}: content extends to {}, expected <= {right}", ui.min_rect().right());
                        });
                        output.textures_delta.clear();
                        assert_eq!(app.saved.settings.font_size, font_size);
                        assert_eq!(app.saved.settings.reveal_speed, 137.5);
                    }
                }
            }
        }
    }

    #[test]
    fn settings_tabs_provider_selection_review_state_and_footer_are_interactive() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("settings".into()));
        app.saved.settings.reduced_motion = true;
        let mut output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        output = click(
            &mut app,
            &ctx,
            text_position(&output.shapes, "llama.cpp"),
            0.2,
        );
        assert_eq!(app.saved.settings.provider, Provider::Llama);
        output = click(
            &mut app,
            &ctx,
            text_position(&output.shapes, "Appearance"),
            0.4,
        );
        assert_eq!(app.settings_tab, 1);
        text_position(&output.shapes, "READING PREVIEW");
        output = click(
            &mut app,
            &ctx,
            text_position(&output.shapes, "Reasoning summaries"),
            0.6,
        );
        assert!(!app.saved.settings.show_reasoning);
        output = click(&mut app, &ctx, text_position(&output.shapes, "Tools"), 0.8);
        assert_eq!(app.settings_tab, 2);
        output = click(
            &mut app,
            &ctx,
            text_position(&output.shapes, "Review each tool action"),
            1.0,
        );
        assert!(app.saved.settings.review_actions);
        text_position(&output.shapes, "REVIEW MODE");
        output = click(
            &mut app,
            &ctx,
            text_position(&output.shapes, "Enable agent tools"),
            1.2,
        );
        assert!(!app.saved.settings.tools_enabled);
        text_position(&output.shapes, "TOOLS PAUSED");
        output = click(
            &mut app,
            &ctx,
            text_position(&output.shapes, "Review each tool action"),
            1.4,
        );
        assert!(
            app.saved.settings.review_actions,
            "disabled preferences must be retained"
        );
        click(&mut app, &ctx, text_position(&output.shapes, "Done"), 1.6);
        assert!(!app.settings_open);
    }

    #[test]
    fn settings_footer_stays_visible_when_scrolling_a_small_window() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("tools".into()));
        app.saved.settings.reduced_motion = true;
        let mut last = Vec::new();
        for frame in 0..5 {
            let events = if frame == 2 {
                vec![
                    egui::Event::PointerMoved(pos2(600.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: vec2(0.0, -4000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            let output = draw(
                &mut app,
                &ctx,
                720.0,
                540.0,
                frame as f64 * 0.2,
                events,
                egui::Modifiers::NONE,
            );
            let done = text_position(&output.shapes, "Done");
            assert!(done.y > 470.0 && done.y < 540.0);
            last = output.shapes;
        }
        let instructions = text_position(&last, "Assistant instructions");
        assert!(
            instructions.y > 170.0 && instructions.y < 470.0,
            "bottom settings must be reachable: {instructions:?}"
        );
    }

    #[test]
    fn panels_render_at_small_and_large_sizes() {
        for preview in [
            "welcome",
            "chat",
            "markdown",
            "settings",
            "codex",
            "openrouter",
            "approval",
            "appearance",
            "git-attribution",
            "menu",
            "actions",
            "attachments",
            "question",
            "images",
            "image-viewer",
        ] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some(preview.into()));
            for (width, height) in [(1180.0, 820.0), (720.0, 540.0)] {
                for frame in 0..3 {
                    let output = draw(
                        &mut app,
                        &ctx,
                        width,
                        height,
                        frame as f64 * 0.2,
                        vec![],
                        egui::Modifiers::NONE,
                    );
                    assert!(!output.shapes.is_empty(), "{preview} should render");
                }
            }
        }
    }

    fn fake_active(
        app: &mut Harness,
    ) -> (
        mpsc::Sender<Event>,
        tokio::sync::mpsc::UnboundedReceiver<Message>,
    ) {
        let chat = &mut app.saved.chats[0];
        let message = chat.messages.last_mut().unwrap();
        message.status = Status::Streaming;
        let (tx, rx) = mpsc::channel();
        let (steering, receive) = tokio::sync::mpsc::unbounded_channel();
        app.active = Some(Active {
            chat: chat.id,
            message: message.id,
            workspace: PathBuf::from("."),
            rx,
            task: app.runtime.spawn(std::future::pending::<()>()),
            started: Instant::now(),
            steering: Some(steering),
        });
        (tx, receive)
    }

    fn logic_tick(app: &mut Harness, ctx: &egui::Context, focused: bool, hidden: bool) -> Duration {
        let mut input = egui::RawInput::default();
        let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
        viewport.focused = Some(focused);
        viewport.occluded = Some(hidden);
        viewport.minimized = Some(hidden);
        // Deliberately leave raw.focused at its default; hidden run_logic must
        // use fresh native viewport info rather than the previous painted input.
        let mut interval = Duration::ZERO;
        let _ = ctx.run_logic(&input, |ctx| {
            interval = tick_interval(ctx);
            app.logic(ctx, &mut eframe::Frame::_new_kittest());
        });
        interval
    }

    #[test]
    fn closing_saves_cancelled_work_and_paused_queues_before_bounded_runtime_cleanup() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx);
        let mut app = Harness::new(&cc, Some("actions".into()));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        app.store = persistence::Store::new(Some(path.clone()));
        let chat = &mut app.saved.chats[0];
        chat.draft = "Keep the last typed text 🌿".into();
        chat.queue
            .push(QueuedMessage::new("Later".into(), Vec::new()));
        let message = chat.messages.last_mut().unwrap();
        message.status = Status::Streaming;
        message.activities[0].status = ActionStatus::Running;
        let (_tx, rx) = mpsc::channel();
        app.active = Some(Active {
            chat: chat.id,
            message: message.id,
            workspace: PathBuf::from("."),
            rx,
            steering: None,
            started: Instant::now(),
            task: app.runtime.spawn(std::future::pending::<()>()),
        });
        app.auth_task = Some(app.runtime.spawn(std::future::pending::<()>()));
        // Simulate eframe's save-before-on_exit ordering.
        app.store.queue(&app.saved);
        app.on_exit(None);
        assert!(app.active.is_none() && app.auth_task.is_none());
        drop(app);
        let restored = persistence::read(&path).unwrap().unwrap();
        let chat = &restored.chats[0];
        assert_eq!(chat.draft, "Keep the last typed text 🌿");
        assert!(chat.queue_paused);
        assert_eq!(chat.queue.len(), 1);
        assert_eq!(chat.messages.last().unwrap().status, Status::Cancelled);
        assert_eq!(
            chat.messages.last().unwrap().activities[0].status,
            ActionStatus::Cancelled
        );
    }

    #[test]
    fn completion_alerts_wait_for_the_queue_batch_and_ignore_stop_and_error() {
        let saved = Saved::default();
        let mut chat = saved.chats[0].clone();
        assert!(batch_finished(true, &chat));
        assert!(!batch_finished(false, &chat));
        chat.queue
            .push(QueuedMessage::new("Follow-up".into(), Vec::new()));
        assert!(!batch_finished(true, &chat));
        chat.queue_paused = true;
        assert!(batch_finished(true, &chat));
        assert!(!batch_finished(false, &chat));
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert!(legacy.desktop_notifications && legacy.completion_sound);
        let muted: Settings =
            serde_json::from_str(r#"{"desktop_notifications":false,"completion_sound":false}"#)
                .unwrap();
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&muted).unwrap()).unwrap();
        assert!(!restored.desktop_notifications && !restored.completion_sound);
    }

    #[test]
    fn hidden_logic_accepts_steering_and_stream_completion_without_painting() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
        let (events, mut steering) = fake_active(&mut app);
        events
            .send(Event::Text("Progress before steering 🌿.".into()))
            .unwrap();
        assert_eq!(logic_tick(&mut app, &ctx, false, true), BACKGROUND_TICK);
        assert_eq!(
            app.saved.chats[0].messages[1].text,
            "Progress before steering 🌿."
        );
        app.saved.chats[0].draft = "Guidance while hidden".into();
        app.saved.chats[0]
            .attachments
            .push(attachments::test_image());
        app.submit(&ctx, true);
        let accepted = steering.try_recv().unwrap();
        events.send(Event::Steered(vec![accepted])).unwrap();
        events
            .send(Event::Text("Continued while hidden 世界.".into()))
            .unwrap();
        events.send(Event::Completed).unwrap();
        logic_tick(&mut app, &ctx, false, true);
        let chat = &app.saved.chats[0];
        assert_eq!(chat.messages.len(), 4);
        assert_eq!(chat.messages[1].status, Status::Complete);
        assert_eq!(chat.messages[2].text, "Guidance while hidden");
        assert_eq!(chat.messages[2].attachments.len(), 1);
        assert_eq!(chat.messages[3].text, "Continued while hidden 世界.");
        assert_eq!(chat.messages[3].status, Status::Complete);
        assert!(chat.queue.is_empty() && app.active.is_none());
        assert!(chat.messages[1].timeline_valid() && chat.messages[3].timeline_valid());
        assert_eq!(
            app.reveals[&chat.messages[3].id]
                .0
                .visible(&chat.messages[3].text),
            chat.messages[3].text
        );
    }

    #[test]
    fn hidden_logic_finishes_a_turn_and_starts_its_queued_followup_without_painting() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let (events, _) = fake_active(&mut app);
        app.saved.chats[0].draft = "Automatic follow-up".into();
        app.send(&ctx);
        let original = app.saved.selected;
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        events.send(Event::Completed).unwrap();
        logic_tick(&mut app, &ctx, false, true);
        assert_eq!(app.active.as_ref().unwrap().chat, original);
        assert_eq!(app.saved.chats[0].messages[2].text, "Automatic follow-up");
        assert!(app.saved.chats[0].queue.is_empty());
        assert!(app.saved.chats[1].messages.is_empty());
        app.stop();
    }

    #[test]
    fn hidden_logic_autosaves_streamed_progress_without_a_painted_frame() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("chats.json");
        app.store = persistence::Store::new(Some(path.clone()));
        app.last_autosave = Instant::now() - AUTOSAVE_INTERVAL;
        app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
        let (events, _) = fake_active(&mut app);
        events
            .send(Event::Text("Saved while minimized 🌿.".into()))
            .unwrap();
        logic_tick(&mut app, &ctx, false, true);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !app.store.saved_once && Instant::now() < deadline {
            app.store.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.store.saved_once,
            "background logic must queue a save, not wait for ui()"
        );
        let restored = persistence::read(&path).unwrap().unwrap();
        assert_eq!(
            restored.chats[0].messages[1].text,
            "Saved while minimized 🌿."
        );
        assert!(restored.chats[0].messages[1].timeline_valid());
        app.stop();
    }

    #[test]
    fn hidden_logic_times_out_a_question_without_approving_workspace_actions() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("question".into()));
        let pending = app.question.as_mut().unwrap();
        let expected = pending
            .question
            .answer(pending.question.recommended(), true);
        pending.selected = 1;
        pending.custom = "Not submitted".into();
        pending.deadline = Instant::now() - Duration::from_millis(1);
        let (reply, mut answer) = oneshot::channel();
        pending.reply = reply;
        let (approval, mut decision) = oneshot::channel();
        app.pending = Some(Pending {
            command_mode: app.saved.settings.command_mode,
            call: ToolCall {
                id: "approval".into(),
                name: "write_file".into(),
                arguments: "{}".into(),
            },
            reply: approval,
            workspace: PathBuf::from("."),
        });
        logic_tick(&mut app, &ctx, false, true);
        assert_eq!(answer.try_recv().unwrap(), expected);
        assert!(app.question.is_none());
        assert!(app.pending.is_some() && decision.try_recv().is_err());
        app.stop();
    }

    #[test]
    fn unfocused_logic_throttles_animation_and_drains_event_bursts_in_bounded_batches() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.chats[0].messages[1] = Message::new(false, String::new(), 0.0, "Demo".into());
        let (events, _) = fake_active(&mut app);
        let count = EVENTS_PER_TICK * 3 + 17;
        for _ in 0..count {
            events.send(Event::Text("🌿".into())).unwrap();
        }
        events.send(Event::Completed).unwrap();
        assert_eq!(logic_tick(&mut app, &ctx, false, false), BACKGROUND_TICK);
        let reply = &app.saved.chats[0].messages[1];
        assert!(!reply.text.is_empty() && reply.text.len() <= EVENTS_PER_TICK * "🌿".len());
        assert!(
            app.active.is_some(),
            "one tick must yield before draining a whole burst"
        );
        assert_eq!(app.reveals[&reply.id].0.visible(&reply.text), reply.text);
        for _ in 0..100 {
            if app.active.is_none() {
                break;
            }
            logic_tick(&mut app, &ctx, false, false);
        }
        assert!(app.active.is_none());
        assert_eq!(app.saved.chats[0].messages[1].text, "🌿".repeat(count));
        assert_eq!(app.saved.chats[0].messages[1].status, Status::Complete);
        assert_eq!(logic_tick(&mut app, &ctx, true, false), FOREGROUND_TICK);
    }

    #[test]
    fn enter_queues_while_running_then_fifo_drains_without_changing_the_draft_or_selected_chat() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let (events, _steering) = fake_active(&mut app);
        let original = app.saved.selected;
        let active_message = app.active.as_ref().unwrap().message;
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        app.saved.chats[0].draft = "Follow-up one".into();
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.2,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.active.as_ref().unwrap().message, active_message);
        assert_eq!(
            app.saved.chats[0].messages.len(),
            2,
            "unsent queue is not model history"
        );
        assert_eq!(app.saved.chats[0].queue[0].text, "Follow-up one");
        app.saved.chats[0].draft = "Follow-up two".into();
        app.saved.chats[0]
            .attachments
            .push(attachments::test_image());
        app.send(&ctx);
        assert_eq!(app.saved.chats[0].queue.len(), 2);
        app.saved.chats[0].draft = "Keep typing this draft".into();
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        events.send(Event::Completed).unwrap();
        app.poll(&ctx);
        assert_eq!(app.active.as_ref().unwrap().chat, original);
        assert_eq!(app.saved.chats[0].messages[2].text, "Follow-up one");
        assert_eq!(app.saved.chats[0].queue[0].text, "Follow-up two");
        assert_eq!(app.saved.chats[0].queue[0].attachments.len(), 1);
        assert_eq!(app.saved.chats[0].draft, "Keep typing this draft");
        assert_eq!(app.saved.selected, app.saved.chats[1].id);
        app.stop();
        assert!(app.saved.chats[0].queue_paused);
    }

    #[test]
    fn steer_button_sends_once_and_acknowledgement_splits_history_in_the_original_chat() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        app.saved.settings.provider = Provider::OpenAI;
        let (events, mut steering) = fake_active(&mut app);
        app.saved.chats[0].draft = "Focus on tests instead".into();
        app.saved.chats[0]
            .attachments
            .push(attachments::test_image());
        app.send(&ctx);
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let pos = text_center(&output, "Steer");
        click(&mut app, &ctx, pos, 0.2);
        let user = steering.try_recv().unwrap();
        assert_eq!(user.attachments.len(), 1);
        assert!(app.saved.chats[0].queue[0].dispatched);
        app.steer_queued(app.saved.selected, user.id, &ctx);
        assert!(
            steering.try_recv().is_err(),
            "a steer cannot be dispatched twice"
        );
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        events
            .send(Event::ResponsesContext(vec![
                serde_json::json!({"role":"assistant","content":"Before steering"}),
            ]))
            .unwrap();
        events.send(Event::Steered(vec![user.clone()])).unwrap();
        events.send(Event::Text("After steering".into())).unwrap();
        events.send(Event::Completed).unwrap();
        app.poll(&ctx);
        let messages = &app.saved.chats[0].messages;
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1].status, Status::Complete);
        assert_eq!(messages[1].response_items[0]["content"], "Before steering");
        assert_eq!(messages[2].id, user.id);
        assert_eq!(messages[3].text, "After steering");
        assert!(app.saved.chats[0].queue.is_empty());
        assert!(app.saved.chats[1].messages.is_empty());
    }

    #[test]
    fn stop_error_and_late_steering_preserve_unsent_messages_without_auto_running() {
        for error in [false, true] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("actions".into()));
            let (events, _steering) = fake_active(&mut app);
            app.saved.chats[0].draft = "Unapplied steer".into();
            app.submit(&ctx, true);
            assert!(app.saved.chats[0].queue[0].dispatched);
            if error {
                events.send(Event::Error("fixture error".into())).unwrap();
                app.poll(&ctx);
            } else {
                app.stop();
                app.poll(&ctx);
            }
            assert!(app.active.is_none());
            assert!(app.saved.chats[0].queue_paused);
            assert!(!app.saved.chats[0].queue[0].dispatched);
            assert_eq!(app.saved.chats[0].queue[0].text, "Unapplied steer");
        }
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let (events, steering) = fake_active(&mut app);
        drop(steering);
        app.saved.chats[0].draft = "Late steer".into();
        app.submit(&ctx, true);
        assert!(!app.saved.chats[0].queue[0].dispatched);
        events.send(Event::Completed).unwrap();
        app.poll(&ctx);
        assert_eq!(
            app.saved.chats[0].messages[2].text, "Late steer",
            "late steering becomes a normal follow-up"
        );
        app.stop();
    }

    #[test]
    fn stop_commits_an_acknowledged_steer_but_keeps_unacknowledged_work_queued() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let (events, mut steering) = fake_active(&mut app);
        app.saved.chats[0].draft = "Accepted guidance".into();
        app.submit(&ctx, true);
        let accepted = steering.try_recv().unwrap();
        app.saved.chats[0].draft = "Not yet accepted".into();
        app.submit(&ctx, true);
        events.send(Event::Steered(vec![accepted])).unwrap();
        events
            .send(Event::Text("Partial new answer".into()))
            .unwrap();
        app.stop();
        let chat = &app.saved.chats[0];
        assert_eq!(chat.messages[2].text, "Accepted guidance");
        assert_eq!(chat.messages[3].text, "Partial new answer");
        assert_eq!(chat.messages[3].status, Status::Cancelled);
        assert_eq!(chat.queue.len(), 1);
        assert_eq!(chat.queue[0].text, "Not yet accepted");
        assert!(chat.queue_paused);
        assert!(!chat.queue[0].dispatched);
    }

    #[test]
    fn command_enter_steers_only_the_active_chat_and_paused_queues_wait_for_resume() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let (_events, mut steering) = fake_active(&mut app);
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        app.saved.chats[0].draft = "Command-enter guidance".into();
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.2,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
            egui::Modifiers::COMMAND,
        );
        assert_eq!(steering.try_recv().unwrap().text, "Command-enter guidance");
        let other = Chat::new(app.project().id);
        app.saved.selected = other.id;
        app.saved.chats.push(other);
        app.saved.chats[1].draft = "Only the other chat".into();
        app.submit(&ctx, true);
        assert!(
            steering.try_recv().is_err(),
            "cannot steer a different chat's run"
        );
        assert_eq!(app.saved.chats[1].queue[0].text, "Only the other chat");
        app.stop();
        app.saved.chats[1].draft = "Still paused".into();
        app.send(&ctx);
        assert!(app.active.is_none());
        assert_eq!(app.saved.chats[1].queue.len(), 2);
        assert!(app.saved.chats[1].queue_paused);
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.4,
            vec![],
            egui::Modifiers::NONE,
        );
        click(&mut app, &ctx, text_center(&output, "Resume"), 0.6);
        assert_eq!(app.active.as_ref().unwrap().chat, app.saved.chats[1].id);
        assert_eq!(app.saved.chats[1].messages[0].text, "Only the other chat");
        app.stop();
    }

    #[test]
    fn queue_edit_preserves_attachments_and_composer_draft_and_can_cancel() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("actions".into()));
        let queued = QueuedMessage::new("old".into(), vec![attachments::test_image()]);
        let id = queued.id;
        app.saved.chats[0].queue.push(queued);
        app.saved.chats[0].queue_paused = true;
        app.saved.chats[0].draft = "Keep this draft".into();
        app.queue_edit = Some((app.saved.selected, id, "Edited message".into()));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.1,
            vec![],
            egui::Modifiers::NONE,
        );
        click(&mut app, &ctx, text_center(&output, "Save"), 0.2);
        assert!(app.queue_edit.is_none());
        assert_eq!(app.saved.chats[0].queue[0].text, "Edited message");
        assert_eq!(app.saved.chats[0].queue[0].attachments.len(), 1);
        assert_eq!(app.saved.chats[0].draft, "Keep this draft");
        app.queue_edit = Some((app.saved.selected, id, "Discard me".into()));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.4,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.queue_edit.is_none());
        assert_eq!(app.saved.chats[0].queue[0].text, "Edited message");
    }

    #[test]
    fn enter_sends_and_stop_targets_original_chat_after_switching() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        app.saved.chats[0].draft = "Hello".into();
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.2,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.active.is_some());
        assert_eq!(app.saved.chats[0].messages[0].text, "Hello");
        assert!(app.saved.chats[0].draft.is_empty());
        let original = app.saved.selected;
        app.new_chat(app.project().id, 0.3);
        assert_ne!(app.saved.selected, original);
        app.stop();
        assert_eq!(app.saved.chats[0].messages[1].status, Status::Cancelled);
        assert!(app.saved.chats[1].messages.is_empty());
    }

    #[test]
    fn pasted_files_stay_in_the_original_chat_and_attachment_only_send_works() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pasted file.rs");
        std::fs::write(&path, "fn pasted() {}\n").unwrap();
        let url = reqwest::Url::from_file_path(&path).unwrap();
        let mut input = egui::RawInput {
            events: vec![egui::Event::Paste(url.to_string())],
            ..Default::default()
        };
        app.raw_input_hook(&ctx, &mut input);
        assert!(input.events.is_empty());
        assert_eq!(app.attachment_jobs.len(), 1);
        let original = app.saved.selected;
        app.new_chat(app.project().id, 0.2);
        for _ in 0..100 {
            app.poll_attachments(&ctx);
            if app.attachment_jobs.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.attachment_jobs.is_empty());
        assert_eq!(app.saved.chats[0].attachments[0].name, "pasted file.rs");
        assert!(app.saved.chats[1].attachments.is_empty());
        assert!(app.saved.chats[0].draft.is_empty());
        app.saved.selected = original;
        app.send(&ctx);
        assert!(app.active.is_some());
        assert!(app.saved.chats[0].attachments.is_empty());
        assert_eq!(app.saved.chats[0].messages[0].attachments.len(), 1);
        assert_eq!(app.saved.chats[0].title, "pasted file.rs");
        app.stop();
    }

    #[test]
    fn question_timeout_submits_recommendation_even_when_another_option_was_selected() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("question".into()));
        let (reply, mut answer) = oneshot::channel();
        let pending = app.question.as_mut().unwrap();
        pending.selected = 2;
        pending.deadline = Instant::now() - Duration::from_millis(1);
        pending.reply = reply;
        app.poll(&ctx);
        assert!(app.question.is_none());
        let result: serde_json::Value = serde_json::from_str(&answer.try_recv().unwrap()).unwrap();
        assert_eq!(result["answer"], "A panel beside the chat");
        assert_eq!(result["automatic"], true);
    }

    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == label
                {
                    return Some(text.pos + text.galley.size() * 0.5);
                }
                None
            })
            .unwrap_or_else(|| panic!("Missing visible text: {label}"))
    }

    fn click(
        app: &mut Harness,
        ctx: &egui::Context,
        pos: egui::Pos2,
        time: f64,
    ) -> egui::FullOutput {
        draw(
            app,
            ctx,
            1180.0,
            820.0,
            time,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            egui::Modifiers::NONE,
        );
        draw(
            app,
            ctx,
            1180.0,
            820.0,
            time + 0.01,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            egui::Modifiers::NONE,
        )
    }

    #[test]
    fn returned_image_thumbnail_opens_viewer_and_escape_closes_it() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("images".into()));
        app.saved.settings.reduced_motion = true;
        let mut output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        for frame in 1..4 {
            output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                frame as f64 * 0.2,
                vec![],
                egui::Modifiers::NONE,
            );
        }
        let position = text_center(&output, "workspace-chart.png");
        assert!(app.image_preview.is_none());
        click(&mut app, &ctx, position, 1.0);
        assert_eq!(
            app.image_preview.as_ref().unwrap().image.name,
            "workspace-chart.png"
        );
        let output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            1.2,
            vec![],
            egui::Modifiers::NONE,
        );
        text_center(&output, "Save PNG…");
        text_center(&output, "Fit");
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            1.4,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.image_preview.is_none());
        assert_eq!(app.saved.chats[0].messages[1].attachments.len(), 1);
    }

    #[test]
    fn image_events_store_viewed_images_in_actions_and_sent_images_in_replies() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("chat".into()));
        let id = app.saved.chats[0].messages[1].id;
        let (tx, rx) = mpsc::channel();
        let task = app.runtime.spawn(std::future::pending::<()>());
        app.active = Some(Active {
            chat: app.saved.selected,
            message: id,
            workspace: PathBuf::from("."),
            rx,
            task,
            started: Instant::now(),
            steering: None,
        });
        for (name, show_in_reply) in [("view_image", false), ("send_image", true)] {
            tx.send(Event::ToolStarted(ToolCall {
                id: name.into(),
                name: name.into(),
                arguments: "{}".into(),
            }))
            .unwrap();
            tx.send(Event::ToolFinished {
                id: name.into(),
                output: tools::ToolOutput {
                    text: "Image".into(),
                    status: ActionStatus::Complete,
                    change: None,
                    images: vec![attachments::test_image()],
                },
                elapsed: 0.1,
                show_in_reply,
            })
            .unwrap();
        }
        tx.send(Event::Image(attachments::test_image())).unwrap();
        tx.send(Event::Completed).unwrap();
        app.poll(&ctx);
        let message = &app.saved.chats[0].messages[1];
        assert_eq!(message.attachments.len(), 2);
        assert_eq!(message.activities.len(), 2);
        assert!(
            message
                .activities
                .iter()
                .all(|a| a.status == ActionStatus::Complete && a.images.len() == 1)
        );
        assert!(app.active.is_none());
    }

    #[test]
    fn question_selection_and_send_return_the_selected_option() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("question".into()));
        let (reply, mut answer) = oneshot::channel();
        app.question.as_mut().unwrap().reply = reply;
        let mut output = draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        for frame in 1..4 {
            output = draw(
                &mut app,
                &ctx,
                1180.0,
                820.0,
                frame as f64 * 0.1,
                vec![],
                egui::Modifiers::NONE,
            );
        }
        let position = text_center(&output, "2. A separate window");
        output = click(&mut app, &ctx, position, 0.5);
        assert_eq!(app.question.as_ref().unwrap().selected, 1);
        assert!(answer.try_recv().is_err(), "Choosing must wait for Send");
        let position = text_center(&output, "Send");
        click(&mut app, &ctx, position, 0.7);
        assert!(app.question.is_none());
        let result: serde_json::Value = serde_json::from_str(&answer.try_recv().unwrap()).unwrap();
        assert_eq!(result["answer"], "A separate window");
        assert_eq!(result["automatic"], false);
    }

    #[test]
    fn custom_question_reply_enter_sends_without_altering_chat_draft() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("question".into()));
        app.saved.chats[0].draft = "Keep this draft".into();
        let (reply, mut answer) = oneshot::channel();
        app.question.as_mut().unwrap().reply = reply;
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        let id = Id::new(("question_reply", &app.question.as_ref().unwrap().id));
        ctx.memory_mut(|m| m.request_focus(id));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.2,
            vec![egui::Event::Paste("Use a floating inspector".into())],
            egui::Modifiers::NONE,
        );
        assert_eq!(
            app.question.as_ref().unwrap().custom,
            "Use a floating inspector"
        );
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.3,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.question.is_none());
        let result: serde_json::Value = serde_json::from_str(&answer.try_recv().unwrap()).unwrap();
        assert_eq!(result["answer"], "Use a floating inspector");
        assert_eq!(app.saved.chats[0].draft, "Keep this draft");
    }

    #[test]
    fn welcome_suggestions_never_cross_into_the_composer() {
        for (width, height) in [
            (1180.0, 820.0),
            (1180.0, 680.0),
            (1180.0, 600.0),
            (720.0, 540.0),
        ] {
            let ctx = egui::Context::default();
            let cc = eframe::CreationContext::_new_kittest(ctx.clone());
            let mut app = Harness::new(&cc, Some("welcome".into()));
            app.saved.settings.reduced_motion = true;
            let mut output = draw(
                &mut app,
                &ctx,
                width,
                height,
                0.0,
                vec![],
                egui::Modifiers::NONE,
            );
            for frame in 1..4 {
                output = draw(
                    &mut app,
                    &ctx,
                    width,
                    height,
                    frame as f64 * 0.2,
                    vec![],
                    egui::Modifiers::NONE,
                );
            }
            let composer_top = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::epaint::Shape::Text(text) = &shape.shape
                        && text.galley.job.text.contains("Ask, build, fix, explore")
                    {
                        return Some(text.pos.y - 50.0);
                    }
                    None
                })
                .next()
                .expect("Composer text should be visible");
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && [
                        "Explore the codebase",
                        "Build something new",
                        "Find an improvement",
                        "Map out the project",
                        "From idea to first draft",
                        "A fresh pair of eyes",
                    ]
                    .contains(&text.galley.job.text.as_str())
                {
                    assert!(
                        text.pos.y + text.galley.size().y <= composer_top,
                        "Suggestions must fit above the composer at {width}x{height}"
                    );
                    assert!(shape.clip_rect.bottom() <= composer_top + 10.0);
                }
            }
        }
    }

    #[test]
    fn shift_enter_adds_newline_without_starting_generation() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Harness::new(&cc, Some("welcome".into()));
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.0,
            vec![],
            egui::Modifiers::NONE,
        );
        app.saved.chats[0].draft = "Hello".into();
        draw(
            &mut app,
            &ctx,
            1180.0,
            820.0,
            0.2,
            vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)],
            egui::Modifiers::SHIFT,
        );
        assert!(app.active.is_none());
        assert!(app.saved.chats[0].draft.contains('\n'));
    }
}

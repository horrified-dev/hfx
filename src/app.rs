mod attachments_ui;
mod composer;
mod conversation;
mod dialogs;
mod previews;
mod safety;
mod settings;
#[cfg(test)]
mod tests;
mod turns;

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

struct TrustRequest {
    project: Uuid,
    chat: Option<Uuid>,
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
    recovery: Option<crate::recovery::Recovery>,
    recovery_open: bool,
    recovery_rx: Option<Receiver<Result<crate::recovery::Outcome, String>>>,
    trust_request: Option<TrustRequest>,
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
        let (mut saved, recovery) = crate::recovery::load(state_path.as_deref(), || {
            if preview.is_some() {
                Saved::default()
            } else {
                cc.storage
                    .and_then(|storage| eframe::get_value(storage, "hfx.v1"))
                    .unwrap_or_default()
            }
        });
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
            store: match &recovery {
                Some(recovery) => persistence::Store::blocked(recovery.error.clone()),
                None => persistence::Store::new(state_path),
            },
            recovery_open: recovery.is_some(),
            recovery,
            recovery_rx: None,
            trust_request: None,
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
                self.recovery_banner(ui);
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
            if self.recovery_open {
                self.recovery_open = false;
            } else if self.trust_request.is_some() {
                self.trust_request = None;
            } else if self.image_preview.is_some() {
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
        self.safety_dialogs(&ctx);
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

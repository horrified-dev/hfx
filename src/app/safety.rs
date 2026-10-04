//! Explicit chat recovery and project host-access acknowledgement.
use super::*;

impl Harness {
    pub(super) fn recovery_banner(&mut self, ui: &mut Ui) {
        if self.recovery.is_some() {
            egui::Panel::top("recovery_banner").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Chat recovery required · saving is disabled")
                            .color(theme::ERROR),
                    );
                    if ui.button("Review recovery").clicked() {
                        self.recovery_open = true;
                    }
                });
            });
        }
    }

    pub(super) fn poll_recovery(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.recovery_rx else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(tick_interval(ctx));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Recovery task stopped; saving remains disabled.".into())
            }
        };
        self.recovery_rx = None;
        match result {
            Ok(outcome) => {
                let Some(recovery) = self.recovery.take() else {
                    return;
                };
                let notice = match outcome {
                    crate::recovery::Outcome::Reloaded(mut state) => {
                        state.restore();
                        self.saved = *state;
                        self.reveals.clear();
                        self.textures.clear();
                        self.thumbnails.clear();
                        self.action_targets.clear();
                        self.queue_edit = None;
                        self.rename = None;
                        self.trust_request = None;
                        self.focus_composer = true;
                        "Original chat history reloaded".to_owned()
                    }
                    crate::recovery::Outcome::BackedUp(backup) => {
                        format!("Original preserved at {}", backup.display())
                    }
                };
                self.store = persistence::Store::new(Some(recovery.path));
                self.store.queue(&self.saved);
                self.recovery_open = false;
                self.notify(notice, ctx.input(|i| i.time));
            }
            Err(error) => {
                if let Some(recovery) = &mut self.recovery {
                    recovery.error = error;
                }
                self.recovery_open = true;
            }
        }
    }

    pub(super) fn trust_settings(&mut self, ui: &mut Ui) {
        let project = self.project().id;
        let trusted = self.project().is_trusted();
        prefs::card(ui, "project_trust", |ui| {
            prefs::heading(
                ui,
                Icon::Folder,
                "Project host access",
                "A separate decision for each project",
            );
            prefs::pill(
                ui,
                if trusted {
                    "PROJECT TRUSTED"
                } else {
                    "PROJECT NOT TRUSTED"
                },
                if trusted { prefs::GREEN } else { prefs::AMBER },
            );
            prefs::help(
                ui,
                "Trust permits enabled host commands and MCP. It is not isolation: code and tools can read credentials or change files outside this project. Revocation stops the active project run; stop previously launched background services separately. Review mode still applies to tool calls. Existing projects require acknowledgement too.",
            );
            if trusted {
                if ui.button("Revoke project trust").clicked() {
                    self.revoke_project_trust(project);
                }
            } else if ui.button("Review project trust").clicked() {
                self.trust_request = Some(TrustRequest::for_project(self.project(), None));
            }
        });
    }

    pub(super) fn revoke_project_trust(&mut self, project: Uuid) {
        // Stop a captured trusted run as well as preventing future runs.
        if self.active.as_ref().is_some_and(|active| {
            self.saved
                .chats
                .iter()
                .any(|chat| chat.id == active.chat && chat.project == project)
        }) {
            self.stop();
        }
        self.mcp_probe.cancel();
        if let Some(project) = self.saved.projects.iter_mut().find(|p| p.id == project) {
            project.trusted_path = None;
        }
        self.store.queue(&self.saved);
    }

    pub(super) fn approve_project_trust(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let request = self
            .trust_request
            .as_ref()
            .ok_or("No project trust request")?;
        let project = self
            .saved
            .projects
            .iter_mut()
            .find(|p| p.id == request.project)
            .ok_or("Project no longer exists")?;
        project.trust_workspace(request.workspace.as_ref().map_err(Clone::clone)?)?;
        self.resume_after_trust(ctx);
        Ok(())
    }

    fn resume_after_trust(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.trust_request.take()
            && let Some(chat_id) = request.chat
            && let Some(chat) = self
                .saved
                .chats
                .iter_mut()
                .find(|c| c.id == chat_id && c.project == request.project)
        {
            chat.queue_paused = false;
            self.start_next(chat_id, ctx);
        }
        self.store.queue(&self.saved);
    }

    pub(super) fn safety_dialogs(&mut self, ctx: &egui::Context) {
        if self.recovery_open
            && let Some(recovery) = &self.recovery
        {
            let mut action = None;
            let busy = self.recovery_rx.is_some();
            let modal = safety_modal(ctx, "chat_recovery").show(ctx, |ui| {
                safety_style(ui, ctx);
                if safety_header(
                    ui,
                    Icon::Clock,
                    "Protect your chat history",
                    "Your original file stays untouched",
                ) {
                    self.recovery_open = false;
                }
                ui.add_space(14.0);
                ScrollArea::vertical()
                    .id_salt("recovery_details")
                    .max_height(safety_body_height(ctx, 240.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        safety_notice(
                            ui,
                            "SAVING IS PAUSED",
                            "Your saved history could not be loaded. The original has not been overwritten. Startup saves, autosaves and exit saves are blocked until you resolve this.",
                        );
                        ui.add_space(10.0);
                        safety_inset(ui, |ui| {
                            safety_caption(ui, "ORIGINAL CHAT FILE");
                            safety_path(ui, &recovery.path.display().to_string());
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&recovery.error)
                                        .size(11.0)
                                        .color(theme::ERROR),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        });
                        ui.add_space(10.0);
                        safety_copy(
                            ui,
                            "Reload a repaired original to replace this temporary session, or preserve the original in a private backup before enabling saves for this session.",
                        );
                        prefs::help(ui, "Reloading discards this temporary session's unsaved changes.");
                    });
                safety_divider(ui);
                ui.add_enabled_ui(self.active.is_none() && !busy, |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 36.0),
                        Layout::right_to_left(Align::Center).with_main_wrap(true),
                        |ui| {
                            if theme::dialog_action(ui, "Back up original and enable saving", true)
                                .clicked()
                            {
                                action = Some(true);
                            }
                            if theme::dialog_action(ui, "Retry loading original", false).clicked() {
                                action = Some(false);
                            }
                        },
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if busy { "Hide recovery progress" } else { "Continue without saving" })
                                    .size(12.0)
                                    .color(theme::MUTED),
                            )
                            .frame(false),
                        )
                        .on_hover_text(if busy {
                            "Hide this dialog without cancelling recovery. Saving is enabled only if the recovery you requested succeeds."
                        } else {
                            "Keep using this temporary session. Saving stays disabled."
                        })
                        .clicked()
                    {
                        self.recovery_open = false;
                    }
                    if busy {
                        ui.spinner();
                        prefs::help(ui, "Working on your history…");
                    } else if self.active.is_some() {
                        prefs::help(ui, "Stop generation before recovering history.");
                    }
                });
            });
            if modal.should_close() {
                self.recovery_open = false;
            }
            if let Some(backup) = action {
                let recovery = recovery.clone();
                let (tx, rx) = mpsc::channel();
                self.recovery_rx = Some(rx);
                let repaint = ctx.clone();
                self.runtime.spawn_blocking(move || {
                    let result = if backup {
                        recovery.backup()
                    } else {
                        recovery.retry()
                    };
                    let _ = tx.send(result);
                    repaint.request_repaint();
                });
            }
            // Never stack the trust decision above history recovery.
            return;
        }
        if let Some(request) = &self.trust_request {
            let Some(project) = self.saved.projects.iter().find(|p| p.id == request.project) else {
                self.trust_request = None;
                return;
            };
            let trust_error = match &request.workspace {
                Ok(expected) if expected.to_str().is_none() => Some("Project directory is not valid UTF-8; trust was not granted.".to_owned()),
                Ok(expected) => project.workspace().and_then(|current| {
                    if current == *expected { Ok(()) } else { Err("Project directory changed. Close this dialog and review the new directory before granting trust.".to_owned()) }
                }).err(),
                Err(error) => Some(error.clone()),
            };
            let mut action = None;
            let modal = safety_modal(ctx, "project_trust").show(ctx, |ui| {
                safety_style(ui, ctx);
                if safety_header(
                    ui,
                    Icon::Terminal,
                    "Trust this project with host access?",
                    "A separate decision for this workspace",
                ) {
                    action = Some(0);
                }
                ui.add_space(14.0);
                ScrollArea::vertical()
                    .id_salt("project_trust_details")
                    .max_height(safety_body_height(ctx, 200.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        safety_inset(ui, |ui| {
                            ui.horizontal(|ui| {
                                theme::inline_icon(ui, Icon::Folder, 15.0, theme::ACCENT);
                                ui.add(
                                    egui::Label::new(RichText::new(&project.name).size(13.0).strong())
                                        .wrap(),
                                );
                            });
                            if let Ok(canonical) = &request.workspace {
                                safety_caption(ui, "CANONICAL PROJECT DIRECTORY");
                                safety_path(ui, &canonical.display().to_string());
                                if canonical.as_path() != std::path::Path::new(&project.path) {
                                    prefs::help(ui, &format!("Opened from: {}", project.path));
                                }
                            } else {
                                safety_caption(ui, "PROJECT DIRECTORY");
                                safety_path(ui, &project.path);
                            }
                        });
                        if let Some(error) = &trust_error {
                            ui.add_space(10.0);
                            safety_notice(ui, "TRUST NOT GRANTED", error);
                        }
                        ui.add_space(10.0);
                        safety_notice(
                            ui,
                            "HOST ACCESS · NOT A SANDBOX",
                            "Commands and local MCP servers run as you, with access to credentials, Git/SSH, network and desktop. They can change files outside this project. No sudo is granted. Trust only code and servers you understand.",
                        );
                        ui.add_space(6.0);
                        prefs::help(
                            ui,
                            "Trust is saved for this canonical directory. Review mode still applies to tool calls, but does not approve MCP server startup.",
                        );
                        ui.add_space(10.0);
                        safety_caption(ui, "OTHER WAYS TO CONTINUE · GLOBAL TOOL PREFERENCES");
                        if safety_alternative(
                            ui,
                            Icon::Stop,
                            "Continue without shell commands or MCP",
                        )
                        .clicked()
                        {
                            action = Some(2);
                        }
                        if ui
                            .add_enabled_ui(cfg!(target_os = "linux"), |ui| {
                                safety_alternative(
                                    ui,
                                    Icon::Sidebar,
                                    "Use strict sandbox (disable MCP)",
                                )
                            })
                            .inner
                            .on_hover_text("Linux, bubblewrap and overlay support are required. Errors never fall back to host access.")
                            .clicked()
                        {
                            action = Some(3);
                        }
                        prefs::help(
                            ui,
                            "Alternatives change global tool preferences. Strict sandbox requires Linux, bubblewrap and overlay support; errors never fall back to host access. No choice is made automatically.",
                        );
                    });
                safety_divider(ui);
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 36.0),
                    Layout::right_to_left(Align::Center).with_main_wrap(true),
                    |ui| {
                        if ui.add_enabled_ui(trust_error.is_none(), |ui| {
                            theme::dialog_action(ui, "Trust project and continue", true)
                        }).inner.clicked() {
                            action = Some(1);
                        }
                        if theme::dialog_action(ui, "Cancel", false).clicked() {
                            action = Some(0);
                        }
                    },
                );
            });
            if modal.should_close() && action.is_none() {
                action = Some(0);
            }
            match action {
                Some(0) => self.trust_request = None, // Queue remains paused.
                Some(1) => {
                    if let Err(error) = self.approve_project_trust(ctx) {
                        self.notify(error, ctx.input(|i| i.time));
                    }
                }
                Some(2) => {
                    self.saved.settings.commands_enabled = false;
                    self.saved.settings.mcp_enabled = false;
                    self.resume_after_trust(ctx);
                }
                Some(3) => {
                    self.saved.settings.command_mode = crate::state::CommandMode::Sandbox;
                    self.saved.settings.mcp_enabled = false;
                    self.resume_after_trust(ctx);
                }
                _ => {}
            }
        }
    }
}

// Shared presentation keeps the safety dialogs aligned with project/rename
// dialogs without putting trust or recovery decisions in the styling helpers.
fn safety_modal(ctx: &egui::Context, id: &str) -> egui::Modal {
    egui::Modal::new(Id::new(id))
        .frame(
            Frame::window(&ctx.style_of(egui::Theme::Dark))
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, theme::OUTLINE))
                .corner_radius(14)
                .inner_margin(24),
        )
        .backdrop_color(egui::Color32::from_black_alpha(150))
}

fn safety_style(ui: &mut Ui, ctx: &egui::Context) {
    prefs::style(ui);
    ui.set_width((ctx.content_rect().width() - 112.0).clamp(240.0, 540.0));
    ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
}

fn safety_header(ui: &mut Ui, glyph: Icon, title: &str, subtitle: &str) -> bool {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, 11, theme::ACCENT.gamma_multiply(0.10));
        theme::icon(ui.painter(), rect.center(), 20.0, glyph, theme::ACCENT);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 38.0).max(100.0));
            ui.add(egui::Label::new(RichText::new(title).size(20.0).strong()).wrap());
            ui.label(RichText::new(subtitle).size(11.0).color(theme::DIM));
        });
        let id = ui.id().with("safety_close_icon");
        let button = egui::Button::new(egui::Atom::custom(id, vec2(18.0, 18.0)))
            .frame(false)
            .min_size(vec2(28.0, 28.0))
            .atom_ui(ui);
        if button.response.hovered() {
            ui.painter()
                .rect_filled(button.response.rect, 8, theme::HOVER);
        }
        if button.response.has_focus() {
            ui.painter().rect_stroke(
                button.response.rect,
                8,
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(rect) = button.rect(id) {
            theme::icon(
                ui.painter(),
                rect.center(),
                18.0,
                Icon::Close,
                if button.response.hovered() {
                    theme::TEXT
                } else {
                    theme::MUTED
                },
            );
        }
        button.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Close dialog")
        });
        button
            .response
            .on_hover_text("Close dialog · Esc")
            .clicked()
    })
    .inner
}

fn safety_body_height(ctx: &egui::Context, chrome_height: f32) -> f32 {
    // Recovery has an extra footer row; reserve it rather than scrolling away
    // the header or decision buttons when paths or errors are unusually long.
    (ctx.content_rect().height() - chrome_height).clamp(100.0, 480.0)
}

fn safety_caption(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).size(10.0).strong().color(theme::DIM));
}

fn safety_copy(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::MUTED)).wrap());
}

fn safety_path(ui: &mut Ui, path: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(path)
                .monospace()
                .size(11.0)
                .color(theme::CODE),
        )
        .wrap()
        .selectable(true),
    );
}

fn safety_inset(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::NONE
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::LINE))
        .corner_radius(10)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn safety_notice(ui: &mut Ui, title: &str, text: &str) {
    Frame::NONE
        .fill(prefs::AMBER.gamma_multiply(0.06))
        .stroke(Stroke::new(1.0, prefs::AMBER.gamma_multiply(0.24)))
        .corner_radius(10)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(10.0).strong().color(prefs::AMBER));
            safety_copy(ui, text);
        });
}

fn safety_alternative(ui: &mut Ui, glyph: Icon, label: &str) -> egui::Response {
    ui.scope(|ui| {
        let visuals = &mut ui.visuals_mut().widgets.inactive;
        visuals.weak_bg_fill = theme::CARD;
        visuals.bg_fill = theme::CARD;
        visuals.bg_stroke = Stroke::new(1.0, theme::LINE);
        let id = ui.id().with(("safety_alternative", label));
        let button = egui::Button::new((
            egui::Atom::custom(id, vec2(14.0, 14.0)),
            RichText::new(label).size(12.0),
            egui::Atom::grow(),
        ))
        .min_size(vec2(ui.available_width(), 34.0))
        .gap(9.0)
        .corner_radius(8)
        .atom_ui(ui);
        if let Some(rect) = button.rect(id) {
            theme::icon(ui.painter(), rect.center(), 14.0, glyph, theme::MUTED);
        }
        if button.response.has_focus() {
            ui.painter().rect_stroke(
                button.response.rect,
                8,
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        button.response
    })
    .inner
}

fn safety_divider(ui: &mut Ui) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
}

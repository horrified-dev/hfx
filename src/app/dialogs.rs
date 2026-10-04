//! Dialogs for the native harness.
use super::*;

impl Harness {
    pub(super) fn overlays(&mut self, ctx: &egui::Context, now: f64) {
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
                } else if matches!(pending.call.name.as_str(), "write_file" | "edit_file") {
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
                } else if matches!(pending.call.name.as_str(), "write_file" | "edit_file") {
                    ui.label(
                        RichText::new(args["path"].as_str().unwrap_or("Unknown file"))
                            .color(theme::ACCENT),
                    );
                    ui.label(
                        RichText::new(
                            if pending.call.name == "edit_file" { "Approval replaces the unique old text with the new text below. Stale or ambiguous edits fail without writing." } else { "Approval replaces the file with the complete content below." },
                        )
                        .size(12.0)
                        .color(theme::MUTED),
                    );
                    if pending.call.name == "edit_file" {
                        ui.label("Old text:");
                        ScrollArea::both().id_salt("review_old_text").max_height(120.0).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(args["old_text"].as_str().unwrap_or("")).monospace()).selectable(true));
                        });
                        ui.label("New text:");
                        if let Some(digest) = args["expected_sha256"].as_str() { ui.monospace(format!("Expected SHA-256: {digest}")); }
                    }
                    ScrollArea::both()
                        .id_salt("review_content")
                        .max_height(if pending.call.name == "edit_file" { (ctx.content_rect().height() - 440.0).clamp(70.0, 180.0) } else { (ctx.content_rect().height() - 300.0).max(100.0) })
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if pending.call.name == "edit_file" { args["new_text"].as_str().unwrap_or("") } else { args["content"].as_str().unwrap_or("") })
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
                        } else if matches!(pending.call.name.as_str(), "write_file" | "edit_file") {
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
}

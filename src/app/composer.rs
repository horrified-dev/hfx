//! Composer for the native harness.
use super::*;

impl Harness {
    pub(super) fn queue_height(&self) -> f32 {
        let count = self.saved.chats[self.selected_index()].queue.len();
        if count == 0 {
            0.0
        } else {
            52.0 + count.min(3) as f32 * 40.0
        }
    }

    pub(super) fn queued_messages(&mut self, ui: &mut Ui) {
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

    pub(super) fn composer(&mut self, ui: &mut Ui) {
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
                                    ui.scope(|ui| {
                                        ui.set_max_width(160.0);
                                        ui.add(egui::Label::new(
                                            RichText::new(&self.project().name).size(11.0).color(theme::MUTED),
                                        ).truncate()).on_hover_text(&self.project().path);
                                    });
                                    ui.label(RichText::new("/").size(11.0).color(theme::DIM));
                                    let status = self.git_probe.status_for(std::path::Path::new(&self.project().path));
                                    ui.add(egui::Label::new(
                                        RichText::new(status.label()).size(11.0).color(theme::MUTED),
                                    ).truncate()).on_hover_text(status.detail());
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
}

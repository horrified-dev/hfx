//! Conversation for the native harness.
use super::*;

impl Harness {
    pub(super) fn question_card(&mut self, ui: &mut Ui) {
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

    pub(super) fn change_badge(&mut self, ui: &mut Ui) {
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
            let mut tooltip = "View saved edit history. The badge excludes committed, reverted, and Git-ignored edits; historical diffs are kept.".to_owned();
            if let Some(error) = &self.edit_probe.error {
                tooltip.push_str(&format!("\nGit reconciliation is paused: {error}"));
            }
            if response.on_hover_text(tooltip).clicked() {
                self.changes_open = true;
            }
        });
        ui.add_space(5.0);
    }

    pub(super) fn action_history(
        &mut self,
        ui: &mut Ui,
        message: &Message,
        start: usize,
        end: usize,
    ) {
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
            if activities
                .iter()
                .any(|a| a.name == name || (name == "write_file" && a.name == "edit_file"))
            {
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
                    "write_file" | "edit_file" => (
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
                        "write_file" | "edit_file" => Icon::Paperclip,
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

    pub(super) fn changes_window(&mut self, ctx: &egui::Context) {
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

    pub(super) fn conversation(&mut self, ui: &mut Ui, now: f64) {
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

    /// Copy only the tiny cursors; rendered text continues to borrow the saved
    /// message, including reasoning that may be hidden or collapsed this frame.
    pub(super) fn reply_text<'a>(&self, message: &'a Message) -> (&'a str, &'a str) {
        self.reveals
            .get(&message.id)
            .copied()
            .map(|(answer, reasoning)| {
                (
                    answer.visible(&message.text),
                    reasoning.visible(&message.reasoning),
                )
            })
            .unwrap_or((&message.text, &message.reasoning))
    }

    pub(super) fn message(&mut self, ui: &mut Ui, message: &Message, now: f64) {
        let alpha = if self.saved.settings.reduced_motion || message.born == 0.0 {
            1.0
        } else {
            motion::ease(((now - message.born) / 0.3) as f32)
        };
        ui.set_opacity(ui.opacity() * alpha);
        let (answer, reasoning) = self.reply_text(message);
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
                    theme::markdown(ui, answer, self.saved.settings.font_size, theme::TEXT);
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
                            theme::markdown(ui, reasoning, 13.0, theme::MUTED);
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
                theme::markdown(ui, answer, self.saved.settings.font_size, theme::TEXT);
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
}

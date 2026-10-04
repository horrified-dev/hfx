//! Settings for the native harness.
use super::*;

impl Harness {
    pub(super) fn settings_panel(&mut self, ui: &mut Ui, now: f64) {
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

    pub(super) fn appearance_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn tool_settings(&mut self, ui: &mut Ui) {
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
        self.trust_settings(ui);
        self.command_and_web_settings(ui);
        let canonical = self.project().workspace();
        let project_trusted = canonical
            .as_ref()
            .is_ok_and(|path| self.project().trusts_workspace(path));
        let root = canonical.unwrap_or_else(|_| PathBuf::from(&self.project().path));
        self.mcp_probe.show(
            ui,
            &mut self.saved.settings,
            root,
            &self.runtime,
            project_trusted,
        );
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

    pub(super) fn command_and_web_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn git_attribution_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn provider_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn context_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn generation_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn connection_settings(&mut self, ui: &mut Ui) {
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

    pub(super) fn start_auth(&mut self, ctx: &egui::Context, action: AuthAction) {
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

    pub(super) fn cancel_auth(&mut self) {
        if let Some(task) = self.auth_task.take() {
            task.abort();
        }
        self.auth_rx = None;
        self.auth_url = None;
    }

    pub(super) fn codex_settings(&mut self, ui: &mut Ui) {
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
}

//! Native MCP settings and an asynchronous, cancellable discovery-only probe.
use crate::{
    mcp, settings_ui as prefs,
    state::Settings,
    theme::{self, Icon},
};
use eframe::egui::{self, RichText, ScrollArea, Ui};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

#[derive(Default)]
pub struct Probe {
    config: String,
    receive: Option<Receiver<Result<Vec<String>, String>>>,
    task: Option<tokio::task::JoinHandle<()>>,
    result: Option<Result<Vec<String>, String>>,
}

impl Probe {
    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.receive = None;
        self.result = None;
    }

    pub fn poll(&mut self, ctx: &egui::Context, interval: Duration) {
        let Some(receive) = &self.receive else {
            return;
        };
        match receive.try_recv() {
            Ok(result) => {
                self.result = Some(result);
                self.receive = None;
                self.task = None;
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(interval.max(Duration::from_millis(60)))
            }
            Err(TryRecvError::Disconnected) => {
                self.result = Some(Err("MCP discovery task stopped unexpectedly.".into()));
                self.receive = None;
                self.task = None;
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        settings: &mut Settings,
        root: PathBuf,
        runtime: &tokio::runtime::Runtime,
        project_trusted: bool,
    ) {
        prefs::card(ui, "mcp", |ui| {
            prefs::heading(
                ui,
                Icon::Plus,
                "MCP servers",
                "Connect external tools to every provider",
            );
            ui.add_enabled_ui(settings.tools_enabled, |ui| {
                prefs::toggle(
                    ui,
                    &mut settings.mcp_enabled,
                    "Enable MCP",
                    "Use tools from servers you explicitly configure.",
                );
            });
            prefs::callout(
                ui,
                "EXTERNAL ACCESS · Not sandboxed\nLocal servers run as you and inherit your environment. Remote servers receive tool arguments. Review mode approves tool calls, not server startup; use only trusted servers. MCP is blocked in strict sandbox mode.",
                prefs::AMBER,
            );
            prefs::label(ui, "Server configuration · JSON");
            ui.add(
                egui::TextEdit::multiline(&mut settings.mcp_config)
                    .id_salt("mcp_config")
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(8),
            );
            prefs::help(
                ui,
                "Use mcpServers with command/args for stdio or url for Streamable HTTP. Config is saved in plaintext: use ${ENV_VAR} in env and bearerTokenEnv for secrets. See docs/mcp.md. Changes apply to the next run.",
            );
            let valid = match mcp::Config::parse(&settings.mcp_config) {
                Ok(config) => {
                    let count = config.mcp_servers.values().filter(|s| s.enabled).count();
                    prefs::help(
                        ui,
                        &format!(
                            "{count} enabled server(s). Discovery runs at the start of each agent task."
                        ),
                    );
                    true
                }
                Err(error) => {
                    ui.label(RichText::new(error).size(11.0).color(theme::ERROR));
                    false
                }
            };
            let config = serde_json::json!([
                settings.mcp_config,
                settings.mcp_enabled,
                settings.tools_enabled,
                settings.commands_enabled,
                settings.command_mode,
                project_trusted,
                root
            ])
            .to_string();
            if self.config != config {
                self.config = config;
                self.result = None;
                self.receive = None;
                if let Some(task) = self.task.take() {
                    task.abort();
                }
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(
                    valid
                        && project_trusted
                        && settings.tools_enabled
                        && settings.mcp_enabled
                        && self.receive.is_none(),
                    |ui| {
                        if prefs::primary(ui, "Test MCP servers").clicked() {
                            let settings = settings.clone();
                            let (send, receive) = mpsc::channel();
                            self.receive = Some(receive);
                            self.result = None;
                            let ctx = ui.ctx().clone();
                            self.task = Some(runtime.spawn(async move {
                                let result = mcp::Session::connect(&settings, &root)
                                    .await
                                    .map(|session| session.summary());
                                let _ = send.send(result);
                                ctx.request_repaint();
                            }));
                        }
                    },
                );
            });
            if !project_trusted {
                prefs::help(
                    ui,
                    "Acknowledge Project host access above before testing MCP servers.",
                );
            }
            prefs::help(
                ui,
                "Test starts enabled local servers and lists tools, but does not invoke tools. No project config is auto-loaded.",
            );
            if self.receive.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    prefs::help(ui, "Connecting and discovering tools…");
                    if ui.button("Cancel test").clicked() {
                        if let Some(task) = self.task.take() {
                            task.abort();
                        }
                        self.receive = None;
                    }
                });
            }
            if let Some(result) = &self.result {
                match result {
                    Ok(tools) => {
                        prefs::pill(
                            ui,
                            &format!("{} MCP TOOLS AVAILABLE", tools.len()),
                            prefs::GREEN,
                        );
                        ScrollArea::vertical()
                            .id_salt("mcp_discovered_tools")
                            .max_height(160.0)
                            .show(ui, |ui| {
                                for tool in tools {
                                    ui.label(RichText::new(tool).monospace().size(11.0));
                                }
                            });
                    }
                    Err(error) => {
                        ui.label(RichText::new(error).size(11.0).color(theme::ERROR));
                    }
                }
            }
        });
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(
        probe: &mut Probe,
        settings: &mut Settings,
        runtime: &tokio::runtime::Runtime,
        root: &std::path::Path,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        time: f64,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(340.0, 1200.0),
                )),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                prefs::style(ui);
                probe.show(ui, settings, root.into(), runtime, true);
            },
        );
        output.textures_delta.clear();
        output
    }

    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    (text.galley.job.text == label).then_some(text.pos + text.galley.size() / 2.0)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Missing UI text {label}"))
    }

    fn click(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn mcp_settings_toggle_probe_and_config_changes_are_interactive_without_auto_calls() {
        let runtime = crate::lifecycle::TaskRuntime::new();
        let mut server = runtime.block_on(crate::mcp::tests::http_fixture(false));
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        theme::install(&ctx, 15.0, true);
        let mut settings = crate::mcp::tests::http_settings(&server.url);
        settings.mcp_enabled = false;
        let mut probe = Probe::default();
        let output = render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            vec![],
            0.0,
        );
        let test = text_center(&output, "Test MCP servers");
        // Disabled Test never connects, even when a server is configured.
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            click(test, true),
            0.1,
        );
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            click(test, false),
            0.2,
        );
        assert!(probe.task.is_none());
        assert!(server.requests.try_recv().is_err());
        let enable = text_center(&output, "Enable MCP");
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            click(enable, true),
            0.3,
        );
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            click(enable, false),
            0.4,
        );
        assert!(settings.mcp_enabled);
        assert!(probe.task.is_none(), "Enabling MCP must not start servers");
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            click(test, true),
            0.5,
        );
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            click(test, false),
            0.6,
        );
        assert!(probe.task.is_some());
        for _ in 0..100 {
            probe.poll(&ctx, Duration::from_millis(10));
            if probe.result.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(probe.result.as_ref().unwrap().as_ref().unwrap().len(), 3);
        render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            vec![],
            0.7,
        );
        let output = render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            vec![],
            0.8,
        );
        text_center(&output, "3 MCP TOOLS AVAILABLE");
        let mut methods = Vec::new();
        while let Ok(request) = server.requests.try_recv() {
            assert_ne!(request["method"], "tools/call");
            methods.push(request["method"].as_str().unwrap().to_owned());
        }
        assert_eq!(methods.iter().filter(|s| *s == "tools/list").count(), 2);
        settings.mcp_config = "invalid draft".into();
        let output = render(
            &mut probe,
            &mut settings,
            &runtime,
            root.path(),
            &ctx,
            vec![],
            0.9,
        );
        assert!(probe.result.is_none());
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Invalid MCP configuration"))));
        assert!(server.requests.try_recv().is_err());
    }

    #[test]
    fn untrusted_projects_cannot_start_mcp_discovery() {
        let runtime = crate::lifecycle::TaskRuntime::new();
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        theme::install(&ctx, 15.0, true);
        let mut probe = Probe::default();
        let mut settings = Settings { mcp_enabled: true, mcp_config: serde_json::json!({"mcpServers":{"fixture":{"command":"sh","args":["-c","touch unexpected-server-start"]}}}).to_string(), ..Default::default() };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(340.0, 1400.0),
                )),
                ..Default::default()
            },
            |ui| {
                prefs::style(ui);
                probe.show(ui, &mut settings, root.path().into(), &runtime, false);
            },
        );
        output.textures_delta.clear();
        let test = text_center(&output, "Test MCP servers");
        for pressed in [true, false] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: click(test, pressed),
                    ..Default::default()
                },
                |ui| {
                    prefs::style(ui);
                    probe.show(ui, &mut settings, root.path().into(), &runtime, false);
                },
            );
            output.textures_delta.clear();
        }
        assert!(probe.task.is_none() && probe.receive.is_none());
        assert!(!root.path().join("unexpected-server-start").exists());
    }
}

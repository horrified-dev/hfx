//! Previews for the native harness.
use super::*;

impl Harness {
    pub(super) fn load_preview(&mut self, ctx: &egui::Context, kind: &str) {
        let kind = kind
            .strip_suffix("-details")
            .or_else(|| kind.strip_suffix("-advanced"))
            .unwrap_or(kind);
        if kind == "trust" {
            self.trust_request = Some(TrustRequest {
                project: self.project().id,
                chat: None,
            });
            return;
        }
        if kind == "recovery" {
            self.recovery = Some(crate::recovery::Recovery {
                path: PathBuf::from("example-profile/chats.json"),
                error: "Scripted preview: invalid saved JSON. Original history is preserved."
                    .into(),
            });
            self.recovery_open = true;
            self.store = persistence::Store::blocked("Scripted preview".into());
            return;
        }
        if kind == "edit-approval" {
            let (reply, _) = oneshot::channel();
            self.pending = Some(Pending { command_mode: self.saved.settings.command_mode, call: ToolCall { id: "edit-preview".into(), name: "edit_file".into(), arguments: serde_json::json!({"path":"src/welcome.rs", "old_text":"fn welcome() {}", "new_text":"fn welcome() { println!(\"Make something good.\"); }", "expected_sha256":null}).to_string() }, reply, workspace: PathBuf::from(&self.project().path) });
            return;
        }
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

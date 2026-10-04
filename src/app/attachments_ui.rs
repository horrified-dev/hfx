//! Attachments ui for the native harness.
use super::*;

impl Harness {
    pub(super) fn load_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let chat = self.saved.selected;
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let _ = tx.send(attachments::load_paths(&paths));
            repaint.request_repaint();
        });
        self.attachment_jobs.push((chat, rx));
    }

    pub(super) fn pick_files(&mut self, ctx: &egui::Context) {
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

    pub(super) fn paste_clipboard(&mut self, ctx: &egui::Context) {
        let chat = self.saved.selected;
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let _ = tx.send(attachments::clipboard_files_or_image());
            repaint.request_repaint();
        });
        self.attachment_jobs.push((chat, rx));
    }

    pub(super) fn attachment_input(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
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

    pub(super) fn poll_attachments(&mut self, ctx: &egui::Context) {
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

    pub(super) fn attachment_strip(&mut self, ui: &mut Ui, files: &[Attachment], removable: bool) {
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

    pub(super) fn open_image(&mut self, ctx: &egui::Context, image: Attachment) {
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

    pub(super) fn image_window(&mut self, ctx: &egui::Context) {
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
}

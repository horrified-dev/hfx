//! Native project selection and app-only project removal.
use super::*;

impl Harness {
    pub(super) fn pick_project(&mut self, ctx: &egui::Context) {
        if self.project_picker_rx.is_some() {
            return;
        }
        let directory = self.project().path.clone();
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        self.runtime.spawn_blocking(move || {
            let mut dialog = rfd::FileDialog::new().set_title("Add a project");
            if !directory.is_empty() {
                dialog = dialog.set_directory(directory);
            }
            let _ = tx.send(dialog.pick_folder());
            repaint.request_repaint();
        });
        self.project_picker_rx = Some(rx);
    }

    pub(super) fn add_project_path(&mut self, path: PathBuf, now: f64) -> Result<(), String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Cannot open project: {e}"))?;
        if !path.is_dir() {
            return Err("Choose an existing project folder.".into());
        }
        let path = path
            .to_str()
            .ok_or("The project path is not valid UTF-8.")?
            .to_owned();
        let id = if let Some(project) = self.saved.projects.iter().find(|p| p.path == path) {
            project.id
        } else {
            let project = Project::from_path(path);
            let id = project.id;
            self.saved.projects.push(project);
            id
        };
        self.new_chat(id, now);
        // Drop the empty no-project placeholder after adding a real workspace.
        self.saved.repair_chat_selection();
        self.store.queue(&self.saved);
        Ok(())
    }

    pub(super) fn poll_project_picker(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.project_picker_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(path) => {
                self.project_picker_rx = None;
                self.focus_composer = true;
                if let Some(path) = path
                    && let Err(error) = self.add_project_path(path, ctx.input(|i| i.time))
                {
                    self.notify(error, ctx.input(|i| i.time));
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.project_picker_rx = None;
                self.notify(
                    "The system folder picker closed unexpectedly.",
                    ctx.input(|i| i.time),
                );
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(tick_interval(ctx)),
        }
    }

    fn project_is_running(&self, project: Uuid) -> bool {
        self.active.as_ref().is_some_and(|active| {
            self.saved
                .chats
                .iter()
                .any(|c| c.id == active.chat && c.project == project)
        })
    }

    pub(super) fn remove_project_from_app(&mut self, project: Uuid, now: f64) -> bool {
        if self.project_is_running(project) || !self.saved.projects.iter().any(|p| p.id == project)
        {
            return false;
        }
        self.remember_connection();
        let removed_chats: HashSet<_> = self
            .saved
            .chats
            .iter()
            .filter(|c| c.project == project)
            .map(|c| c.id)
            .collect();
        let removed_messages: HashSet<_> = self
            .saved
            .chats
            .iter()
            .filter(|c| c.project == project)
            .flat_map(|c| &c.messages)
            .map(|m| m.id)
            .collect();
        let selection_removed = removed_chats.contains(&self.saved.selected);
        self.saved.projects.retain(|p| p.id != project);
        self.saved.chats.retain(|c| c.project != project);
        self.attachment_jobs
            .retain(|(chat, _)| !removed_chats.contains(chat));
        self.reveals.retain(|id, _| !removed_messages.contains(id));
        self.action_targets
            .retain(|(id, _), _| !removed_messages.contains(id));
        if self
            .rename
            .as_ref()
            .is_some_and(|(chat, _)| removed_chats.contains(chat))
        {
            self.rename = None;
        }
        if self
            .queue_edit
            .as_ref()
            .is_some_and(|(chat, _, _)| removed_chats.contains(chat))
        {
            self.queue_edit = None;
        }
        if self
            .trust_request
            .as_ref()
            .is_some_and(|r| r.project == project)
        {
            self.trust_request = None;
        }
        self.saved.repair_chat_selection();
        if selection_removed {
            self.load_chat_connection();
            self.view_started = now;
            self.focus_composer = true;
            self.changes_open = false;
            self.image_preview = None;
        }
        self.remove_project = None;
        self.store.queue(&self.saved);
        self.notify(
            "Project removed from hfx. Files on disk were not changed.",
            now,
        );
        true
    }

    pub(super) fn project_removal_dialog(&mut self, ctx: &egui::Context, now: f64) {
        let Some(id) = self.remove_project else {
            return;
        };
        let Some(project) = self.saved.projects.iter().find(|p| p.id == id).cloned() else {
            self.remove_project = None;
            return;
        };
        let running = self.project_is_running(id);
        let chats = self.saved.chats.iter().filter(|c| c.project == id).count();
        let mut remove = false;
        let modal = egui::Modal::new(Id::new("remove_project"))
            .frame(Frame::window(&ctx.style_of(egui::Theme::Dark))
                .fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::OUTLINE))
                .corner_radius(14).inner_margin(24))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 80.0).clamp(180.0, 400.0));
                ui.heading(format!("Remove {}?", project.name));
                ui.label(RichText::new(&project.path).size(12.0).color(theme::MUTED));
                ui.add_space(12.0);
                ui.label(format!("This removes the project and its {chats} chat(s), including queued messages, from hfx."));
                ui.label("Your project folder and files on disk will NOT be deleted.");
                if running {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Stop this project’s active run before removing it.").color(theme::ERROR));
                }
                ui.add_space(16.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    remove = ui.add_enabled_ui(!running, |ui| theme::dialog_action(ui, "Remove project", true)).inner.clicked();
                    if theme::dialog_action(ui, "Cancel", false).clicked() {
                        self.remove_project = None;
                    }
                });
            });
        if remove {
            self.remove_project_from_app(id, now);
        } else if modal.should_close() {
            self.remove_project = None;
        }
    }
}

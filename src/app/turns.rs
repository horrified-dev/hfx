//! Turns for the native harness.
use super::*;

impl Harness {
    pub(super) fn send(&mut self, ctx: &egui::Context) {
        self.submit(ctx, false);
    }

    pub(super) fn submit(&mut self, ctx: &egui::Context, steer: bool) {
        let index = self.selected_index();
        let chat_id = self.saved.chats[index].id;
        let text = self.saved.chats[index].draft.trim().to_owned();
        if (text.is_empty() && self.saved.chats[index].attachments.is_empty())
            || self.attachment_jobs.iter().any(|(id, _)| *id == chat_id)
        {
            return;
        }
        let chat = &mut self.saved.chats[index];
        let queued = QueuedMessage::new(text, std::mem::take(&mut chat.attachments));
        let id = queued.id;
        if chat.queue.is_empty() {
            chat.queue_paused = false;
        }
        chat.queue.push(queued);
        chat.draft.clear();
        if steer {
            self.steer_queued(chat_id, id, ctx);
        }
        if self.active.is_none() {
            self.start_next(chat_id, ctx);
        }
        self.focus_composer = true;
        self.store.queue(&self.saved);
    }

    pub(super) fn steer_queued(&mut self, chat_id: Uuid, id: Uuid, ctx: &egui::Context) {
        let Some(active) = self.active.as_ref().filter(|a| a.chat == chat_id) else {
            return;
        };
        let Some(sender) = &active.steering else {
            return;
        };
        let Some(queued) = self
            .saved
            .chats
            .iter_mut()
            .find(|c| c.id == chat_id)
            .and_then(|c| c.queue.iter_mut().find(|q| q.id == id && !q.dispatched))
        else {
            return;
        };
        if sender.send(queued.message(ctx.input(|i| i.time))).is_ok() {
            queued.dispatched = true;
        }
        // If the stream already completed, leave it queued for a normal follow-up.
    }

    pub(super) fn start_next(&mut self, chat_id: Uuid, ctx: &egui::Context) -> bool {
        if self.active.is_some() || self.recovery_rx.is_some() || self.trust_request.is_some() {
            return false;
        }
        let Some(index) = self.saved.chats.iter().position(|c| c.id == chat_id) else {
            return false;
        };
        let chat = &self.saved.chats[index];
        let Some(queued) = chat.queue.first() else {
            return false;
        };
        if chat.queue_paused
            || self
                .queue_edit
                .as_ref()
                .is_some_and(|(chat, id, _)| *chat == chat_id && *id == queued.id)
        {
            return false;
        }
        let now = ctx.input(|i| i.time);
        let settings = self.saved.settings.clone();
        if settings.provider == Provider::Codex
            && (!self.auth.account().signed_in || self.auth_url.is_some())
        {
            self.saved.chats[index].queue_paused = true;
            self.settings_open = true;
            self.settings_tab = 0;
            self.notify(
                "Complete OpenAI sign-in, then resume the queued message",
                now,
            );
            return false;
        }
        let project = self.saved.chats[index].project;
        let Some(project_state) = self.saved.projects.iter().find(|p| p.id == project) else {
            return false;
        };
        let workspace = match project_state.workspace() {
            Ok(workspace) => workspace,
            Err(error) => {
                self.saved.chats[index].queue_paused = true;
                self.notify(error, now);
                return false;
            }
        };
        let project_trusted = project_state.trusts_workspace(&workspace);
        if settings.provider != Provider::Demo
            && settings.requires_project_trust()
            && !project_trusted
        {
            self.saved.chats[index].queue_paused = true;
            self.trust_request = Some(TrustRequest {
                project,
                chat: Some(chat_id),
                workspace: Ok(workspace),
            });
            self.store.queue(&self.saved);
            return false;
        }
        let chat = &mut self.saved.chats[index];
        let queued = chat.queue.remove(0);
        let user = queued.message(now);
        if chat.messages.is_empty() {
            chat.title = if user.text.is_empty() {
                user.attachments
                    .first()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "Attachments".into())
            } else {
                user.text
                    .lines()
                    .next()
                    .unwrap_or("New chat")
                    .chars()
                    .take(42)
                    .collect()
            };
        }
        self.reveals
            .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
        chat.messages.push(user);
        let history = chat.messages.clone();
        let session = chat.id.to_string();
        let codex_auth = (settings.provider == Provider::Codex).then(|| self.auth.clone());
        let mut message = Message::new(false, String::new(), now, settings.provider.label().into());
        message.status = Status::Streaming;
        let id = message.id;
        self.reveals
            .insert(id, (Reveal::default(), Reveal::default()));
        chat.messages.push(message);
        let (tx, rx) = mpsc::channel();
        let (steer_tx, steer_rx) = tokio::sync::mpsc::unbounded_channel();
        let real_steering = settings.provider != Provider::Demo;
        let repaint = ctx.clone();
        let request_workspace = workspace.clone();
        let task = self.runtime.spawn(async move {
            backend::run_with_steering(
                Request {
                    project_trusted,
                    settings,
                    workspace: request_workspace,
                    history,
                    session,
                    codex_auth,
                },
                tx,
                Some(steer_rx),
            )
            .await;
            repaint.request_repaint();
        });
        self.active = Some(Active {
            chat: chat_id,
            message: id,
            workspace,
            rx,
            task,
            started: Instant::now(),
            steering: real_steering.then_some(steer_tx),
        });
        if chat_id == self.saved.selected {
            self.focus_composer = true;
        }
        self.store.queue(&self.saved);
        true
    }

    pub(super) fn pause_queues(&mut self) {
        for chat in &mut self.saved.chats {
            if !chat.queue.is_empty() {
                chat.queue_paused = true;
            }
            for queued in &mut chat.queue {
                queued.dispatched = false;
            }
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.task.abort();
            // Commit already acknowledged steering/tool results before cancelling.
            while let Ok(event) = active.rx.try_recv() {
                self.apply_event(&mut active, event);
            }
            if let Some(message) = self
                .saved
                .chats
                .iter_mut()
                .find(|c| c.id == active.chat)
                .and_then(|c| c.messages.iter_mut().find(|m| m.id == active.message))
            {
                if message.status == Status::Streaming {
                    message.status = Status::Cancelled;
                }
                message.compacting = false;
                message.elapsed = active.started.elapsed().as_secs_f32();
                cancel_actions(message);
            }
        }
        self.pause_queues();
        self.pending = None;
        self.question = None;
    }

    pub(super) fn apply_event(&mut self, active: &mut Active, event: Event) -> Option<bool> {
        if let Event::Steered(users) = event {
            let chat = self.saved.chats.iter_mut().find(|c| c.id == active.chat)?;
            let prior = chat.messages.iter_mut().find(|m| m.id == active.message)?;
            let provider = prior.provider.clone();
            prior.status = Status::Complete;
            prior.compacting = false;
            prior.elapsed = active.started.elapsed().as_secs_f32();
            let empty = prior.text.is_empty()
                && prior.reasoning.is_empty()
                && prior.activities.is_empty()
                && prior.attachments.is_empty()
                && prior.context_checkpoint.is_none()
                && prior.response_items.is_empty();
            if empty {
                chat.messages.retain(|m| m.id != active.message);
                self.reveals.remove(&active.message);
            }
            let born = users.last().map_or(0.0, |user| user.born);
            for user in users {
                chat.queue.retain(|q| q.id != user.id);
                self.reveals
                    .insert(user.id, (Reveal::complete(&user.text), Reveal::default()));
                chat.messages.push(user);
            }
            let mut reply = Message::new(false, String::new(), born, provider);
            reply.status = Status::Streaming;
            active.message = reply.id;
            active.started = Instant::now();
            self.reveals
                .insert(reply.id, (Reveal::default(), Reveal::default()));
            chat.messages.push(reply);
            self.store.queue(&self.saved);
            return None;
        }
        let message = self
            .saved
            .chats
            .iter_mut()
            .find(|c| c.id == active.chat)
            .and_then(|c| c.messages.iter_mut().find(|m| m.id == active.message))?;
        message.elapsed = active.started.elapsed().as_secs_f32();
        match event {
            Event::Steered(_) => unreachable!(),
            Event::Text(t) => message.push_text(&t),
            Event::Reasoning(t) => message.reasoning.push_str(&t),
            Event::ReasoningDetails(details) => {
                std::sync::Arc::make_mut(&mut message.reasoning_details).extend(details)
            }
            Event::ResponsesContext(items) => message.response_items = items.into(),
            Event::Usage(n) => message.tokens += n,
            Event::ModelTime(seconds) => message.model_seconds += seconds,
            Event::ContextUsage {
                tokens,
                estimated,
                connection,
            } => {
                message.context_connection = connection;
                message.context_tokens = tokens;
                message.context_estimated = estimated;
            }
            Event::CompactionStarted => message.compacting = true,
            Event::Compacted(checkpoint) => {
                message.record_compaction();
                message.context_checkpoint = Some(checkpoint);
                message.response_items = Default::default();
                message.compacting = false;
            }
            Event::Image(image) => message.push_image(image),
            Event::ToolStarted(call) => message.push_activity(crate::state::Activity {
                id: call.id,
                name: call.name,
                arguments: call.arguments.into(),
                result: "".into(),
                status: ActionStatus::Running,
                elapsed: 0.0,
                change: None,
                images: Vec::new(),
            }),
            Event::ToolRunning(id) => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == id) {
                    activity.status = ActionStatus::Running;
                }
            }
            Event::ToolFinished {
                id,
                output,
                elapsed,
                show_in_reply,
            } => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == id) {
                    activity.result = output.text.into();
                    activity.status = output.status;
                    activity.elapsed = elapsed;
                    activity.change = output.change;
                    activity.images = output.images.clone();
                }
                if show_in_reply {
                    for image in output.images {
                        message.push_image(image);
                    }
                }
                if self.question.as_ref().is_some_and(|q| q.id == id) {
                    self.question = None;
                }
            }
            Event::AskUser {
                id,
                question,
                deadline,
                reply,
            } => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == id) {
                    activity.status = ActionStatus::Waiting;
                }
                let selected = question.recommended();
                self.question = Some(PendingQuestion {
                    id,
                    chat: active.chat,
                    question,
                    deadline,
                    reply,
                    selected,
                    custom: String::new(),
                });
            }
            Event::Approval {
                call,
                reply,
                command_mode,
            } => {
                if let Some(activity) = message.activities.iter_mut().find(|a| a.id == call.id) {
                    activity.status = ActionStatus::Waiting;
                }
                self.pending = Some(Pending {
                    command_mode,
                    call,
                    reply,
                    workspace: active.workspace.clone(),
                })
            }
            Event::Completed => {
                message.compacting = false;
                message.status = Status::Complete;
                return Some(true);
            }
            Event::Error(error) => {
                message.compacting = false;
                message.status = Status::Failed;
                message.error = Some(error);
                cancel_actions(message);
                return Some(false);
            }
        }
        None
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        self.poll_recovery(ctx);
        if let Some(error) = self.store.poll() {
            self.notify(error, ctx.input(|i| i.time));
        }
        let interval = tick_interval(ctx);
        let mut preferred = None;
        if let Some(mut active) = self.active.take() {
            let mut outcome = None;
            let tick_started = Instant::now();
            for index in 0..EVENTS_PER_TICK {
                // A burst accumulated while hidden must not monopolize the
                // window thread. This is a soft budget: finish the current event.
                if index > 0 && tick_started.elapsed() >= EVENT_TICK_BUDGET {
                    break;
                }
                match active.rx.try_recv() {
                    Ok(event) => {
                        outcome = self.apply_event(&mut active, event);
                        if outcome.is_some() {
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.apply_event(
                            &mut active,
                            Event::Error("The generation task ended unexpectedly.".into()),
                        );
                        outcome = Some(false);
                        break;
                    }
                }
            }
            if let Some(success) = outcome {
                for queued in self
                    .saved
                    .chats
                    .iter_mut()
                    .filter(|c| c.id == active.chat)
                    .flat_map(|c| &mut c.queue)
                {
                    queued.dispatched = false;
                }
                self.pending = None;
                self.question = None;
                if success {
                    preferred = Some(active.chat);
                } else {
                    self.pause_queues();
                }
                if !self.preview
                    && self
                        .saved
                        .chats
                        .iter()
                        .find(|chat| chat.id == active.chat)
                        .is_some_and(|chat| batch_finished(success, chat))
                {
                    self.notify("Work complete", ctx.input(|i| i.time));
                    notifications::completed(
                        self.saved.settings.desktop_notifications,
                        self.saved.settings.completion_sound,
                    );
                }
                // Hidden eframe frames may never reach its UI autosave path.
                // Persist terminal state even if no further repaint is needed.
                self.store.queue(&self.saved);
            } else {
                self.active = Some(active);
                ctx.request_repaint_after(interval);
            }
        }
        if self.active.is_none() {
            let next = preferred
                .filter(|id| {
                    self.saved
                        .chats
                        .iter()
                        .any(|c| c.id == *id && !c.queue.is_empty() && !c.queue_paused)
                })
                .or_else(|| {
                    self.saved
                        .chats
                        .iter()
                        .find(|c| !c.queue.is_empty() && !c.queue_paused)
                        .map(|c| c.id)
                });
            if let Some(chat) = next {
                self.start_next(chat, ctx);
            }
        }
        self.poll_attachments(ctx);
        if self
            .question
            .as_ref()
            .is_some_and(|q| Instant::now() >= q.deadline)
            && let Some(question) = self.question.take()
        {
            let answer = question
                .question
                .answer(question.question.recommended(), true);
            let _ = question.reply.send(answer);
        }
        if let Some(question) = &self.question {
            ctx.request_repaint_after(
                interval.min(question.deadline.saturating_duration_since(Instant::now())),
            );
        }
        if let Some(rx) = &self.auth_rx {
            let mut done = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    AuthEvent::Snapshot {
                        account,
                        models,
                        error,
                    } => {
                        self.auth_account = account;
                        self.auth_models = models;
                        for (model, limit) in self.auth.context_windows() {
                            let mut settings = self.saved.settings.clone();
                            settings.provider = Provider::Codex;
                            settings.codex_model = model;
                            self.saved
                                .settings
                                .discovered_context_windows
                                .insert(settings.context_key(), limit);
                        }
                        self.auth_error = error;
                        self.auth_url = None;
                        done = true;
                    }
                    AuthEvent::LoginUrl(url) => {
                        if codex::valid_login_url(&url) {
                            ctx.open_url(egui::OpenUrl::new_tab(url.clone()));
                            self.auth_url = Some(url);
                        }
                    }
                    AuthEvent::Error(error) => {
                        self.auth_error = Some(error);
                        self.auth_url = None;
                        done = true;
                    }
                }
            }
            if done {
                self.auth_rx = None;
                self.auth_task = None;
            } else {
                ctx.request_repaint_after(interval.max(Duration::from_millis(80)));
            }
        }
        if !self.preview {
            self.auth_account = self.auth.account();
        }
        let dt = ctx.input(|i| i.stable_dt);
        let instant = self.saved.settings.reduced_motion || background_window(ctx);
        let mut animating = false;
        for chat in &self.saved.chats {
            for message in &chat.messages {
                let (text, reasoning) = self.reveals.entry(message.id).or_insert_with(|| {
                    (
                        Reveal::complete(&message.text),
                        Reveal::complete(&message.reasoning),
                    )
                });
                animating |=
                    text.step(&message.text, dt, self.saved.settings.reveal_speed, instant);
                animating |= reasoning.step(
                    &message.reasoning,
                    dt,
                    self.saved.settings.reveal_speed,
                    instant,
                );
            }
        }
        if animating {
            ctx.request_repaint_after(interval);
        }
        self.mcp_probe.poll(ctx, interval);
        let git_root = PathBuf::from(&self.project().path);
        self.git_probe.update(ctx, &self.runtime, git_root.clone());
        self.poll_pending_edits(ctx, git_root);
        if let Some(rx) = &self.probe_rx {
            if let Ok(result) = rx.try_recv() {
                self.probe_result = Some(result);
                self.probe_rx = None;
            } else {
                ctx.request_repaint_after(interval.max(Duration::from_millis(60)));
            }
        }
    }
}

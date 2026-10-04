//! Exact message-height measurements for the selected conversation only.
//!
//! No estimated heights: cold/invalid rows go through the normal renderer once.
//! Cached rows reserve their full space and their auto-ID slot when off-screen.
use super::*;

#[derive(Default)]
pub(super) struct ConversationLayout {
    chat: Option<Uuid>,
    environment: Option<Environment>,
    pub(super) rows: Vec<Row>,
    pub(super) focused: Option<Id>,
    #[cfg(test)]
    pub(super) force_full: bool,
    #[cfg(test)]
    pub(super) rendered: usize,
    #[cfg(test)]
    pub(super) viewport: Option<Viewport>,
}

struct Environment {
    width: f32,
    pixels_per_point: f32,
    font_size: f32,
    show_reasoning: bool,
    preview: bool,
    pending: bool,
    style: std::sync::Arc<egui::Style>,
    fonts: egui::FontDefinitions,
}

#[derive(Default)]
pub(super) struct Row {
    pub(super) id: Uuid,
    key: Option<RenderKey>,
    pub(super) height: f32,
    pub(super) focused: Option<Id>,
    settle_until: f64,
}

#[cfg(test)]
pub(super) struct Viewport {
    pub(super) id: Id,
    pub(super) content_height: f32,
    pub(super) offset: f32,
    pub(super) rect: egui::Rect,
}

impl ConversationLayout {
    pub(super) fn prepare(
        &mut self,
        ui: &Ui,
        chat: Uuid,
        messages: &[Message],
        settings: &Settings,
        preview: bool,
        pending: bool,
    ) {
        let width = ui.available_width();
        let unchanged = self.chat == Some(chat)
            && self.environment.as_ref().is_some_and(|old| {
                old.width == width
                    && old.pixels_per_point == ui.pixels_per_point()
                    && old.font_size == settings.font_size
                    && old.show_reasoning == settings.show_reasoning
                    && old.preview == preview
                    && old.pending == pending
                    && (std::sync::Arc::ptr_eq(&old.style, ui.style())
                        || old.style.as_ref() == ui.style().as_ref())
                    && ui
                        .ctx()
                        .fonts(|fonts| same_fonts(&old.fonts, fonts.definitions()))
            });
        if !unchanged {
            self.rows.clear();
            self.chat = Some(chat);
            self.environment = Some(Environment {
                width,
                pixels_per_point: ui.pixels_per_point(),
                font_size: settings.font_size,
                show_reasoning: settings.show_reasoning,
                preview,
                pending,
                style: ui.style().clone(),
                fonts: ui.ctx().fonts(|fonts| fonts.definitions().clone()),
            });
        }
        self.rows.resize_with(messages.len(), Row::default);
        for (row, message) in self.rows.iter_mut().zip(messages) {
            if row.id != message.id {
                *row = Row {
                    id: message.id,
                    ..Default::default()
                };
            }
        }
        #[cfg(test)]
        if ui.ctx().current_pass_index() == 0 {
            self.rendered = 0;
        }
    }

    // Event handlers invalidate even same-length/in-place content changes. The
    // cheap render key below additionally catches reveal progress and replacements
    // without hashing/copying potentially multi-megabyte message bodies per frame.
    pub(super) fn invalidate(&mut self, message: Uuid) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == message) {
            row.key = None;
        }
    }
}

// Retain the old Arcs so replacement cannot reuse their addresses. Comparing
// identities detects font-data/tweak replacement without scanning font bytes.
fn same_fonts(old: &egui::FontDefinitions, new: &egui::FontDefinitions) -> bool {
    old.families == new.families
        && old.font_data.len() == new.font_data.len()
        && old
            .font_data
            .iter()
            .zip(&new.font_data)
            .all(|((a, old), (b, new))| a == b && std::sync::Arc::ptr_eq(old, new))
}

impl Row {
    pub(super) fn reusable(&self, key: RenderKey, now: f64) -> bool {
        self.key == Some(key) && now >= self.settle_until
    }

    pub(super) fn measure(&mut self, key: RenderKey, height: f32, now: f64, animation: f32) {
        if self.key != Some(key) || self.height != height {
            // A disclosure can keep changing after it leaves the viewport.
            // Continue measuring until its native egui animation has settled.
            self.settle_until = now + f64::from(animation) * 2.0;
        }
        self.key = Some(key);
        self.height = height;
    }

    pub(super) fn interacted(&mut self, now: f64, animation: f32) {
        self.settle_until = now + f64::from(animation) * 2.0;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RenderKey {
    user: bool,
    status: Status,
    text: [(usize, usize); 3],
    counts: [usize; 4],
    checkpoint: bool,
    context_tokens: u64,
    context_estimated: bool,
    tokens: u64,
    elapsed: u32,
    error: Option<(usize, usize)>,
}

pub(super) fn render_key(message: &Message, answer: &str, reasoning: &str) -> RenderKey {
    let text = |text: &str| (text.as_ptr() as usize, text.len());
    RenderKey {
        user: message.user,
        status: message.status,
        text: [text(&message.provider), text(answer), text(reasoning)],
        counts: [
            message.activities.len(),
            message.attachments.len(),
            message.timeline.as_ptr() as usize,
            message.timeline.len(),
        ],
        checkpoint: message.context_checkpoint.is_some(),
        context_tokens: message.context_tokens,
        context_estimated: message.context_estimated,
        tokens: message.tokens,
        elapsed: message.elapsed.to_bits(),
        error: message.error.as_deref().map(text),
    }
}

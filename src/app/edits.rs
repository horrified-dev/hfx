//! Retire only the checked pending edit records; preserve saved action history.
use super::*;
use crate::pending_edits::Edit;

impl Harness {
    pub(super) fn pending_edits(&self) -> Vec<Edit> {
        let project = self.project().id;
        self.saved
            .chats
            .iter()
            .filter(|chat| chat.project == project)
            .flat_map(|chat| {
                chat.messages.iter().flat_map(move |message| {
                    message
                        .activities
                        .iter()
                        .enumerate()
                        .filter_map(move |(activity, action)| {
                            action
                                .change
                                .as_ref()
                                .filter(|change| !change.settled)
                                .map(|change| Edit {
                                    chat: chat.id,
                                    message: message.id,
                                    activity,
                                    change: change.clone(),
                                })
                        })
                })
            })
            .collect()
    }

    pub(super) fn settle_edits(&mut self, edits: Vec<Edit>) {
        let project = self.project().id;
        let mut changed = false;
        for edit in edits {
            if let Some(change) = self
                .saved
                .chats
                .iter_mut()
                .find(|chat| chat.id == edit.chat && chat.project == project)
                .and_then(|chat| {
                    chat.messages
                        .iter_mut()
                        .find(|message| message.id == edit.message)
                })
                .and_then(|message| message.activities.get_mut(edit.activity))
                .and_then(|action| action.change.as_mut())
                .filter(|change| **change == edit.change)
            {
                change.settled = true;
                changed = true;
            }
        }
        if changed {
            self.store.queue(&self.saved);
        }
    }

    pub(super) fn poll_pending_edits(&mut self, ctx: &egui::Context, root: PathBuf) {
        let edits = self.pending_edits();
        let settled = self.edit_probe.update(ctx, &self.runtime, root, edits);
        self.settle_edits(settled);
    }
}

//! Undo/redo confirmation.
//!
//! Ctrl+Z and Ctrl+Y never touch a file on their own: this window lists every path the entry
//! covers first, and the mutation only runs on Confirm. It is a detached window like the delete
//! and rename dialogs, so the viewer stays visible while the user decides.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_history_confirmation_modal(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.history_confirm.clone() else {
            return;
        };

        let direction = state.direction;
        let title = state.action.title(direction);
        let summary = state.action.summary(direction);
        let preview_items: Vec<DeleteModalItemInfo> = state
            .action
            .paths()
            .iter()
            .map(|path| self.history_item_info(path))
            .collect();
        let unrestorable = state.unrestorable.clone();

        let mut place_frames = state.place_frames;
        let window_size = history_dialog_window_size(preview_items.len());
        let viewport_id = history_dialog_viewport_id();
        let mut choice = None;

        // The frame reports the native title-bar close button; the body reports its buttons and
        // Esc, exactly like the other confirmation dialogs.
        let frame_choice = dialog_window_frame(
            ctx,
            viewport_id,
            &title,
            window_size,
            &mut place_frames,
            |ctx, ui| {
                if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                    choice = Some(DialogChoice::Cancel);
                }
                if ctx.input(|input| {
                    input.key_pressed(egui::Key::Enter)
                        && !input.modifiers.ctrl
                        && !input.modifiers.shift
                        && !input.modifiers.alt
                }) {
                    choice = Some(DialogChoice::Confirm);
                }

                dialog_heading(ui, &title, &summary);
                if !unrestorable.is_empty() {
                    ui.add_space(8.0);
                    let names = unrestorable
                        .iter()
                        .map(|path| {
                            path.file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| path.to_string_lossy().to_string())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.label(
                        egui::RichText::new(format!(
                            "Not in the Recycle Bin any more, so these are skipped: {names}"
                        ))
                        .size(11.5)
                        .color(UI_TEXT_STRONG),
                    );
                }
                ui.add_space(12.0);

                let list_height = (ui.available_height() - DIALOG_FOOTER_HEIGHT).max(96.0);
                egui::ScrollArea::vertical()
                    .id_salt("history_dialog_list")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for item in &preview_items {
                            self.draw_modal_file_card(ui, ctx, item, None, false);
                            ui.add_space(8.0);
                        }
                    });

                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(
                        "Nothing changes until you confirm. A file that can no longer be \
                         restored is reported and left out of the history.",
                    )
                    .size(10.5)
                    .color(UI_TEXT_FAINT),
                );
                ui.add_space(12.0);
                ui_divider(ui);
                ui.add_space(10.0);
                let confirm_label = match direction {
                    HistoryDirection::Undo => "Undo",
                    HistoryDirection::Redo => "Redo",
                };
                if let Some(chosen) = dialog_footer(ui, confirm_label, "Cancel") {
                    choice = Some(chosen);
                }
            },
        );
        state.place_frames = place_frames;

        let Some(choice) = choice.or(frame_choice) else {
            self.history_confirm = Some(state);
            return;
        };

        // The window goes away in this frame instead of staying blank until the next pass.
        ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
        self.history_confirm = None;

        if choice == DialogChoice::Confirm {
            self.commit_history_confirmation(direction);
        }
    }

    /// A card for one file of a history entry. A file that is still deleted on disk has no size
    /// or dimensions to probe here, so the stash's captured-metadata fallback is used instead.
    fn history_item_info(&self, path: &PathBuf) -> DeleteModalItemInfo {
        if path.exists() {
            self.delete_modal_item_info(path)
        } else {
            self.stash_item_info(path)
        }
    }
}

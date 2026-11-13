//! Delete and rename modals.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Rename dialog in its own native window: the viewer and the Stash stay usable behind it,
    /// and the window carries the Settings palette and buttons.
    pub(crate) fn draw_rename_modal(&mut self, ctx: &egui::Context) {
        let Some(rename_state) = self.rename_overlay.clone() else {
            return;
        };

        let preview_items: Vec<DeleteModalItemInfo> = rename_state
            .items
            .iter()
            .map(|item| self.delete_modal_item_info(&item.original_path))
            .collect();
        let item_count = preview_items.len();
        let title = if item_count == 1 {
            "Rename File".to_string()
        } else {
            format!("Rename {} Files", item_count)
        };
        let summary = if item_count == 1 {
            "Choose a new name for the selected file.".to_string()
        } else {
            "Edit each filename below. Every rename is validated before anything is moved."
                .to_string()
        };
        let confirm_label = if item_count == 1 {
            "Rename File"
        } else {
            "Rename Files"
        };

        let mut edited_state = rename_state;
        let mut place_frames = self.rename_dialog_place_frames;
        let window_size = rename_dialog_window_size(item_count);
        let viewport_id = rename_dialog_viewport_id();
        let mut choice = None;

        // The frame reports the native title-bar close button; the body reports its buttons
        // and Esc. Discarding the frame's answer left that close button doing nothing.
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
                if let Some(error) = edited_state.error_message.as_ref() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(error).size(11.5).color(UI_TEXT_STRONG));
                }
                ui.add_space(12.0);

                let list_height = (ui.available_height() - DIALOG_FOOTER_HEIGHT).max(96.0);
                egui::ScrollArea::vertical()
                    .id_salt("rename_dialog_list")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, item) in preview_items.iter().enumerate() {
                            self.draw_modal_file_card(
                                ui,
                                ctx,
                                item,
                                Some(&mut edited_state.items[index].draft_name),
                                edited_state.just_opened && index == 0,
                            );
                            ui.add_space(8.0);
                        }
                    });

                edited_state.just_opened = false;

                ui.add_space(12.0);
                ui_divider(ui);
                ui.add_space(10.0);
                if let Some(chosen) = dialog_footer(ui, confirm_label, "Cancel") {
                    choice = Some(chosen);
                }
            },
        );
        self.rename_dialog_place_frames = place_frames;

        // Typed edits live in the local clone while the window draws, so they are stored back
        // every frame: a text field must keep what the user types, and `just_opened` has to
        // clear or the first field would steal focus back on every pass.
        self.rename_overlay = Some(edited_state);

        let Some(choice) = choice.or(frame_choice) else {
            return;
        };

        match choice {
            DialogChoice::Cancel => {
                ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
                self.cancel_inline_rename();
            }
            DialogChoice::Confirm => {
                // A rejected rename keeps the window open with its error message on screen.
                self.commit_inline_rename();
                if self.rename_overlay.is_none() {
                    ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
                }
            }
        }
    }
}

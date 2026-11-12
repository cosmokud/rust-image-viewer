//! Modal preview, metadata chips and the item card.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// True when the dialog thumbnail for `path` is known to be undrawable: the preview paints
    /// the X and never runs another decode for it until the file changes.
    pub(crate) fn modal_thumbnail_is_unloadable(&mut self, path: &Path) -> bool {
        let Some(stamp) = self.unloadable_files.get(path) else {
            return false;
        };
        let stamp = *stamp;
        let current = FileStamp::of_path(path);
        if current.is_some() && current != stamp {
            self.unloadable_files.remove(path);
            self.folder_placeholder_thumbnail_failures.remove(path);
            self.modal_thumbnail_cache.remove(&path.to_path_buf());
            return false;
        }
        true
    }

    pub(crate) fn draw_modal_thumbnail_preview(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        path: &PathBuf,
    ) {
        let thumbnail_size = egui::vec2(84.0, 84.0);
        let (rect, _) = ui.allocate_exact_size(thumbnail_size, egui::Sense::hover());
        ui_paint_surface(ui.painter(), rect, 10.0, UI_CONTENT, Some(UI_BORDER));

        let inner = rect.shrink(4.0);
        let unloadable = self.modal_thumbnail_is_unloadable(path.as_path());
        if unloadable {
            paint_unloadable_cross(ui.painter(), inner, UI_TEXT_DIM);
        } else if let Some((texture_id, image_size)) =
            self.ensure_modal_thumbnail_texture(ctx, path)
        {
            let available = inner.shrink2(egui::vec2(2.0, 2.0));
            let scale = if image_size.x <= 0.0 || image_size.y <= 0.0 {
                1.0
            } else {
                (available.width() / image_size.x)
                    .min(available.height() / image_size.y)
                    .max(0.01)
            };
            let fitted_size = egui::vec2(image_size.x * scale, image_size.y * scale);
            let image_rect = egui::Rect::from_center_size(rect.center(), fitted_size);
            ui.painter().image(
                texture_id,
                image_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else if !unloadable {
            let placeholder = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_uppercase())
                .unwrap_or_else(|| "FILE".to_string());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                placeholder,
                egui::TextStyle::Button.resolve(ui.style()),
                UI_TEXT_DIM,
            );
        }
    }

    pub(crate) fn draw_modal_metadata_chips(
        ui: &mut egui::Ui,
        file_size_label: &str,
        dimensions_label: &str,
    ) {
        let render_chip = |ui: &mut egui::Ui, text: &str| {
            egui::Frame::none()
                .fill(UI_SURFACE)
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(text).color(UI_TEXT).size(11.5));
                });
        };

        ui.horizontal_wrapped(|ui| {
            render_chip(ui, file_size_label);
            render_chip(ui, dimensions_label);
        });
    }

    pub(crate) fn draw_modal_file_card(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        item: &DeleteModalItemInfo,
        draft_name: Option<&mut String>,
        request_focus: bool,
    ) {
        // Two fills instead of a stroke: the same border trick the Settings cards use.
        egui::Frame::none()
            .fill(UI_BORDER)
            .rounding(12.0)
            .inner_margin(egui::Margin::same(1.0))
            .show(ui, |ui| {
                egui::Frame::none()
                    .fill(UI_WINDOW)
                    .rounding(11.0)
                    .inner_margin(egui::Margin::same(11.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            self.draw_modal_thumbnail_preview(ui, ctx, &item.path);
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.set_min_height(84.0);
                                match draft_name {
                                    Some(draft_name) => {
                                        let response = ui.add(
                                            egui::TextEdit::singleline(draft_name)
                                                .margin(egui::Margin::symmetric(8.0, 6.0))
                                                .desired_width(ui.available_width().max(180.0))
                                                .clip_text(false),
                                        );
                                        if request_focus {
                                            response.request_focus();
                                        }
                                    }
                                    None => {
                                        ui.label(
                                            egui::RichText::new(&item.display_name)
                                                .color(UI_TEXT_STRONG)
                                                .size(13.5),
                                        );
                                    }
                                }

                                ui.add_space(8.0);
                                Self::draw_modal_metadata_chips(
                                    ui,
                                    &item.file_size_label,
                                    &item.dimensions_label,
                                );
                                ui.add_space(8.0);
                                let parent_label = item
                                    .path
                                    .parent()
                                    .map(|parent| parent.to_string_lossy().to_string())
                                    .unwrap_or_else(|| item.path.to_string_lossy().to_string());
                                ui.label(
                                    egui::RichText::new(parent_label)
                                        .monospace()
                                        .color(UI_TEXT_FAINT)
                                        .size(10.5),
                                );
                            });
                        });
                    });
            });
    }

    /// Delete confirmation in its own native window: the viewer stays visible and usable while
    /// the user decides, and the window carries the Settings palette and buttons.
    pub(crate) fn draw_delete_confirmation_modal(&mut self, ctx: &egui::Context) {
        let (targets, title, summary) =
            if let Some(path) = self.pending_single_delete_target.clone() {
                (
                    vec![path],
                    "Delete File to Recycle Bin?".to_string(),
                    "This will move the selected file to the Recycle Bin.".to_string(),
                )
            } else if !self.pending_marked_delete_targets.is_empty() {
                let targets = self.pending_marked_delete_targets.clone();
                let target_count = targets.len();
                (
                    targets,
                    "Delete Marked Files to Recycle Bin?".to_string(),
                    format!(
                        "This will move {} marked files to the Recycle Bin.",
                        target_count
                    ),
                )
            } else {
                return;
            };

        let preview_items: Vec<DeleteModalItemInfo> = targets
            .iter()
            .map(|path| self.delete_modal_item_info(path))
            .collect();

        let mut place_frames = self.delete_dialog_place_frames;
        let window_size = delete_dialog_window_size(preview_items.len());
        let viewport_id = delete_dialog_viewport_id();
        let mut choice = None;

        dialog_window_frame(
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
                ui.add_space(12.0);

                let list_height = (ui.available_height() - DIALOG_FOOTER_HEIGHT).max(96.0);
                egui::ScrollArea::vertical()
                    .id_salt("delete_dialog_list")
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
                        "Set confirm_delete_to_recycle_bin = false in your preset .ini to skip \
                         this confirmation.",
                    )
                    .size(10.5)
                    .color(UI_TEXT_FAINT),
                );
                ui.add_space(12.0);
                ui_divider(ui);
                ui.add_space(10.0);
                if let Some(chosen) = dialog_footer(ui, "Delete to Recycle Bin", "Cancel") {
                    choice = Some(chosen);
                }
            },
        );
        self.delete_dialog_place_frames = place_frames;

        let Some(choice) = choice else {
            return;
        };

        // Closing in this frame drops the window instead of leaving it blank until the next pass.
        ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
        match choice {
            DialogChoice::Confirm => self.perform_delete_targets(targets),
            DialogChoice::Cancel => {
                self.pending_single_delete_target = None;
                self.pending_marked_delete_targets.clear();
                self.modal_thumbnail_cache.clear();
            }
        }
    }
}

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
        ui.painter().rect_filled(
            rect,
            12.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14),
        );
        ui.painter().rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 28),
            ),
        );

        let unloadable = self.modal_thumbnail_is_unloadable(path.as_path());
        if unloadable {
            paint_unloadable_cross(ui.painter(), rect, egui::Color32::from_rgb(214, 90, 82));
        } else if let Some((texture_id, image_size)) =
            self.ensure_modal_thumbnail_texture(ctx, path)
        {
            let available = rect.shrink2(egui::vec2(6.0, 6.0));
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
                egui::Color32::from_rgb(188, 202, 220),
            );
        }
    }

    pub(crate) fn draw_modal_metadata_chips(
        ui: &mut egui::Ui,
        file_size_label: &str,
        dimensions_label: &str,
    ) {
        let render_chip = |ui: &mut egui::Ui,
                           text: &str,
                           fill: egui::Color32,
                           stroke: egui::Stroke,
                           color: egui::Color32| {
            egui::Frame::none()
                .fill(fill)
                .stroke(stroke)
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(text).color(color).size(12.0));
                });
        };

        ui.horizontal_wrapped(|ui| {
            render_chip(
                ui,
                file_size_label,
                egui::Color32::from_rgba_unmultiplied(58, 76, 98, 180),
                egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(130, 168, 196, 180),
                ),
                egui::Color32::from_rgb(222, 233, 243),
            );
            render_chip(
                ui,
                dimensions_label,
                egui::Color32::from_rgba_unmultiplied(72, 68, 38, 180),
                egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(224, 192, 108, 180),
                ),
                egui::Color32::from_rgb(245, 225, 171),
            );
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
        egui::Frame::none()
            .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 10))
            .stroke(egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 24),
            ))
            .rounding(14.0)
            .inner_margin(egui::Margin::same(12.0))
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
                                        .color(egui::Color32::WHITE)
                                        .strong()
                                        .size(15.0),
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
                                .color(egui::Color32::from_rgb(146, 162, 178))
                                .size(11.5),
                        );
                    });
                });
            });
    }

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

        let mut cancel = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let mut confirm = ctx.input(|input| {
            input.key_pressed(egui::Key::Enter)
                && !input.modifiers.ctrl
                && !input.modifiers.shift
                && !input.modifiers.alt
        });
        let screen_rect = ctx.screen_rect();

        egui::Area::new(egui::Id::new("delete_confirmation_backdrop"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, screen_rect.size());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(5, 7, 10, 190),
                );
            });

        let list_height = (preview_items.len() as f32 * 108.0)
            .clamp(120.0, (screen_rect.height() - 260.0).max(120.0));
        let modal_size = egui::vec2(
            (screen_rect.width() - 48.0).clamp(420.0, 680.0),
            (228.0 + list_height).clamp(280.0, screen_rect.height() - 36.0),
        );
        let modal_pos = screen_rect.center() - modal_size * 0.5;
        egui::Area::new(egui::Id::new("delete_confirmation_modal"))
            .fixed_pos(modal_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_min_size(modal_size);
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(18, 22, 28, 252))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40),
                    ))
                    .rounding(18.0)
                    .inner_margin(egui::Margin::same(18.0))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(title)
                                    .color(egui::Color32::WHITE)
                                    .strong()
                                    .size(18.0),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(summary)
                                    .color(egui::Color32::from_rgb(210, 216, 224))
                                    .size(14.0),
                            );
                            ui.add_space(12.0);

                            egui::ScrollArea::vertical()
                                .max_height((modal_size.y - 158.0).max(120.0))
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for item in &preview_items {
                                        self.draw_modal_file_card(ui, ctx, item, None, false);
                                        ui.add_space(8.0);
                                    }
                                });

                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(
                                    "Set confirm_delete_to_recycle_bin = false in your preset .ini to skip this confirmation.",
                                )
                                .color(egui::Color32::from_rgb(130, 168, 196))
                                .size(12.0),
                            );
                            ui.add_space(16.0);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let delete_button = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("Delete to Recycle Bin")
                                            .color(egui::Color32::WHITE),
                                    )
                                    .min_size(egui::vec2(170.0, 32.0))
                                    .fill(egui::Color32::from_rgb(176, 52, 52))
                                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(132, 36, 36)))
                                    .rounding(4.0),
                                );
                                if delete_button.clicked() {
                                    confirm = true;
                                }

                                let cancel_button = ui.add(
                                    egui::Button::new("Cancel")
                                        .min_size(egui::vec2(100.0, 32.0))
                                        .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 24))
                                        .stroke(egui::Stroke::new(
                                            1.0_f32,
                                            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 48),
                                        ))
                                        .rounding(4.0),
                                );
                                if cancel_button.clicked() {
                                    cancel = true;
                                }
                            });
                        });
                    });
            });

        if cancel {
            self.pending_single_delete_target = None;
            self.pending_marked_delete_targets.clear();
            self.modal_thumbnail_cache.clear();
        } else if confirm {
            self.perform_delete_targets(targets);
        }
    }
}

//! Delete and rename modals.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
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

        let mut edited_state = rename_state;
        let mut cancel = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let mut confirm = ctx.input(|input| {
            input.key_pressed(egui::Key::Enter)
                && !input.modifiers.ctrl
                && !input.modifiers.shift
                && !input.modifiers.alt
        });
        let screen_rect = ctx.screen_rect();

        egui::Area::new(egui::Id::new("rename_dialog_backdrop"))
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
            .clamp(120.0, (screen_rect.height() - 272.0).max(120.0));
        let modal_size = egui::vec2(
            (screen_rect.width() - 48.0).clamp(440.0, 720.0),
            (244.0 + list_height).clamp(300.0, screen_rect.height() - 36.0),
        );
        let modal_pos = screen_rect.center() - modal_size * 0.5;

        egui::Area::new(egui::Id::new("rename_dialog_modal"))
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
                            if let Some(error) = edited_state.error_message.as_ref() {
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new(error)
                                        .color(egui::Color32::from_rgb(255, 148, 148))
                                        .size(12.5),
                                );
                            }
                            ui.add_space(12.0);

                            egui::ScrollArea::vertical()
                                .max_height((modal_size.y - 170.0).max(120.0))
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

                            ui.add_space(16.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let confirm_label = if item_count == 1 {
                                        "Rename File"
                                    } else {
                                        "Rename Files"
                                    };
                                    let rename_button = ui.add(
                                        egui::Button::new(
                                            egui::RichText::new(confirm_label)
                                                .color(egui::Color32::WHITE),
                                        )
                                        .min_size(egui::vec2(132.0, 32.0))
                                        .fill(egui::Color32::from_rgb(48, 122, 198))
                                        .stroke(egui::Stroke::new(
                                            1.0_f32,
                                            egui::Color32::from_rgb(38, 92, 162),
                                        ))
                                        .rounding(6.0),
                                    );
                                    if rename_button.clicked() {
                                        confirm = true;
                                    }

                                    let cancel_button = ui.add(
                                        egui::Button::new("Cancel")
                                            .min_size(egui::vec2(100.0, 32.0))
                                            .fill(egui::Color32::from_rgba_unmultiplied(
                                                255, 255, 255, 24,
                                            ))
                                            .stroke(egui::Stroke::new(
                                                1.0_f32,
                                                egui::Color32::from_rgba_unmultiplied(
                                                    255, 255, 255, 48,
                                                ),
                                            ))
                                            .rounding(6.0),
                                    );
                                    if cancel_button.clicked() {
                                        cancel = true;
                                    }
                                },
                            );
                        });
                    });
            });

        if cancel {
            self.cancel_inline_rename();
            return;
        }

        self.rename_overlay = Some(edited_state);
        if confirm {
            self.commit_inline_rename();
        }
    }
}

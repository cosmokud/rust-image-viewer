//! The main video seekbar.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_webp_fps_controls(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        popup_id: egui::Id,
        media_path: &Path,
        frame_count: usize,
        total_duration_ms: u64,
    ) {
        let default_custom_fps = self.sync_custom_fps_with_current_media_default(
            media_path,
            frame_count,
            total_duration_ms,
        );
        let max_custom_fps = Self::ANIMATED_IMAGE_CUSTOM_MAX_FPS;
        self.webp_custom_fps = self.webp_custom_fps.clamp(1, max_custom_fps);
        if self.webp_custom_fps_input.trim().is_empty() {
            self.webp_custom_fps_input = self.webp_custom_fps.to_string();
        }

        let old_item_spacing_x = ui.spacing().item_spacing.x;
        ui.spacing_mut().item_spacing.x = 4.0;

        // Parent row is right-to-left, so add slider first, then button.
        // This keeps the button on the left of the slider while the whole pair
        // stays tucked right beside the WEBP/GIF label.
        let mut custom_fps = self.webp_custom_fps.clamp(1, max_custom_fps);
        let (slider_rect, slider_response) =
            ui.allocate_exact_size(egui::vec2(80.0, 22.0), egui::Sense::click_and_drag());

        let slider_double_clicked = slider_response.double_clicked()
            || (slider_response.hovered()
                && ctx.input(|i| {
                    i.pointer
                        .button_double_clicked(egui::PointerButton::Primary)
                }));

        if slider_double_clicked {
            custom_fps = default_custom_fps;
        } else if (slider_response.clicked() || slider_response.dragged())
            && slider_response.interact_pointer_pos().is_some()
        {
            if let Some(pos) = slider_response.interact_pointer_pos() {
                let t = ((pos.x - slider_rect.left()) / slider_rect.width()).clamp(0.0, 1.0);
                custom_fps = (1.0 + t * (max_custom_fps.saturating_sub(1)) as f32).round() as u32;
            }
        }
        if slider_response.hovered() {
            let wheel_delta = ctx.input(|i| i.smooth_scroll_delta.y);
            if wheel_delta != 0.0 {
                self.arm_media_slider_wheel_guard();
                if wheel_delta > 0.0 {
                    custom_fps = custom_fps.saturating_add(1).min(max_custom_fps);
                } else {
                    custom_fps = custom_fps.saturating_sub(1).max(1);
                }
            }
        }

        if custom_fps != self.webp_custom_fps {
            self.webp_custom_fps = custom_fps;
            self.webp_custom_fps_input = custom_fps.to_string();
            self.webp_fps_override = Some(custom_fps);
            self.webp_show_custom_fps_slider = true;
            ctx.request_repaint();
        }

        if ui.is_rect_visible(slider_rect) {
            let track_height = 4.0;
            let track_rect = egui::Rect::from_min_max(
                egui::pos2(
                    slider_rect.left(),
                    slider_rect.center().y - track_height * 0.5,
                ),
                egui::pos2(
                    slider_rect.right(),
                    slider_rect.center().y + track_height * 0.5,
                ),
            );
            let t = (self.webp_custom_fps.saturating_sub(1) as f32
                / max_custom_fps.saturating_sub(1) as f32)
                .clamp(0.0, 1.0);
            let active_rect = egui::Rect::from_min_max(
                track_rect.left_top(),
                egui::pos2(
                    track_rect.left() + track_rect.width() * t,
                    track_rect.bottom(),
                ),
            );
            let handle_center = egui::pos2(
                track_rect.left() + track_rect.width() * t,
                track_rect.center().y,
            );
            let handle_radius = if slider_response.hovered() || slider_response.dragged() {
                7.0
            } else {
                5.5
            };

            ui.painter()
                .rect_filled(track_rect, 2.0, egui::Color32::from_gray(70));
            ui.painter()
                .rect_filled(active_rect, 2.0, egui::Color32::from_rgb(76, 175, 80));
            ui.painter()
                .circle_filled(handle_center, handle_radius, egui::Color32::WHITE);
        }

        let fps_list_button = ui.add_sized(
            [60.0, 22.0],
            egui::Button::new(format!("{} FPS", self.webp_custom_fps)),
        );
        if fps_list_button.clicked() {
            ui.memory_mut(|mem| mem.toggle_popup(popup_id));
        }

        let close_on_click_outside = egui::popup::PopupCloseBehavior::CloseOnClickOutside;
        let _ = egui::popup::popup_below_widget(
            ui,
            popup_id,
            &fps_list_button,
            close_on_click_outside,
            |ui| {
                ui.set_min_width(120.0);

                for fps in [120_u32, 90, 60, 30, 24] {
                    if ui.button(format!("{} FPS", fps)).clicked() {
                        let clamped = fps.clamp(1, max_custom_fps);
                        self.webp_custom_fps = clamped;
                        self.webp_custom_fps_input = clamped.to_string();
                        self.webp_fps_override = Some(clamped);
                        self.webp_show_custom_fps_slider = true;
                        ui.memory_mut(|mem| mem.close_popup());
                        ctx.request_repaint();
                    }
                }

                ui.separator();
                ui.label(format!("Manual FPS (1-{})", max_custom_fps));
                ui.horizontal(|ui| {
                    let text_response = ui.add_sized(
                        [64.0, 22.0],
                        egui::TextEdit::singleline(&mut self.webp_custom_fps_input),
                    );
                    let apply_clicked = ui.button("Set").clicked();
                    let apply_with_enter =
                        text_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if apply_clicked || apply_with_enter {
                        if let Ok(raw_fps) = self.webp_custom_fps_input.trim().parse::<u64>() {
                            let clamped = raw_fps.clamp(1, max_custom_fps as u64) as u32;
                            self.webp_custom_fps = clamped;
                            self.webp_custom_fps_input = clamped.to_string();
                            self.webp_fps_override = Some(clamped);
                            self.webp_show_custom_fps_slider = true;
                            ui.memory_mut(|mem| mem.close_popup());
                            ctx.request_repaint();
                        }
                    }
                });

                if ui.button("Default").clicked() {
                    self.webp_fps_override = None;
                    self.webp_custom_fps = default_custom_fps;
                    self.webp_custom_fps_input = default_custom_fps.to_string();
                    self.webp_show_custom_fps_slider = true;
                    ui.memory_mut(|mem| mem.close_popup());
                    ctx.request_repaint();
                }

                ui.rect_contains_pointer(ui.min_rect())
            },
        );

        ui.spacing_mut().item_spacing.x = old_item_spacing_x;
    }
}

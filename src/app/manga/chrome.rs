//! Manga HUD chrome: toggle button and zoom bar.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Draw zoom HUD (bottom-right in fullscreen)
    pub(crate) fn draw_manga_zoom_bar(&mut self, ctx: &egui::Context) {
        if !self.mode().is_fullscreen() || !self.show_manga_zoom_bar {
            self.show_manga_zoom_bar = false;
            // Reset hold states when bar is hidden
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
            return;
        }

        // Only show for viewable media (including manga mode)
        if self.mode().is_solo()
            && !matches!(
                self.current_media_type,
                Some(MediaType::Image | MediaType::Video)
            )
        {
            self.show_manga_zoom_bar = false;
            // Reset hold states when bar is hidden
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
            return;
        }

        let screen_rect = ctx.screen_rect();
        let scrollbar_padding = Self::BOTTOM_RIGHT_OVERLAY_SCROLLBAR_PADDING; // Padding to avoid scrollbar
        let margin = Self::BOTTOM_RIGHT_OVERLAY_MARGIN;

        let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));

        // If the primary button is no longer held, stop any latched zoom-repeat.
        // Important: don't rely on `is_pointer_button_down_on()` to clear the hold state,
        // because the HUD can shift during zooming (e.g. scrollbar appearing), causing
        // the pointer to no longer be considered "on" the widget even though the user
        // is still holding the mouse button.
        if !primary_down {
            self.manga_zoom_plus_held = false;
            self.manga_zoom_minus_held = false;
        }

        // Don't auto-hide while buttons are being held
        let is_holding_button = self.manga_zoom_plus_held || self.manga_zoom_minus_held;

        // If video controls are visible, lift the zoom HUD above them.
        let video_controls_offset = if self.show_video_controls {
            56.0 + 8.0
        } else {
            0.0
        };

        // Calculate zoom change from held buttons BEFORE drawing UI
        let mut zoom_delta_from_hold: f32 = 0.0;

        if is_holding_button && primary_down {
            // Calculate acceleration based on hold duration
            let hold_duration = self.manga_zoom_hold_start.elapsed().as_secs_f32();

            // Slower zoom: starts at 0.5% per frame, increases to 2% after 1 second
            let base_speed = 0.005; // 0.5% per frame at 60fps = 30% per second
            let acceleration = (hold_duration * 2.0).min(3.0); // Up to 3x acceleration
            let speed = base_speed * (1.0 + acceleration);

            if self.manga_zoom_plus_held {
                zoom_delta_from_hold = speed;
            } else if self.manga_zoom_minus_held {
                zoom_delta_from_hold = -speed;
            }
        }

        // Apply zoom from hold before drawing
        if zoom_delta_from_hold != 0.0 {
            let old_zoom = self.zoom.max(0.0001);
            let factor = if zoom_delta_from_hold > 0.0 {
                1.0 + zoom_delta_from_hold
            } else {
                1.0 / (1.0 - zoom_delta_from_hold)
            };
            let new_zoom = self.clamp_zoom(self.zoom * factor);

            if (new_zoom - old_zoom).abs() > 0.0001 {
                let zoom_ratio = new_zoom / old_zoom;

                if self.mode().is_manga() {
                    if self.mode().is_masonry() {
                        let center_pos =
                            egui::pos2(self.screen_size.x * 0.5, self.screen_size.y * 0.5);
                        if self.apply_masonry_zoom_at_screen_pos(new_zoom, center_pos) {
                            self.manga_finish_direct_zoom_change();
                        }
                    } else {
                        let center_pos =
                            egui::pos2(self.screen_size.x * 0.5, self.screen_size.y * 0.5);
                        if self.apply_strip_zoom_at_screen_pos(new_zoom, center_pos) {
                            self.manga_finish_direct_zoom_change();
                        }
                    }
                } else {
                    // Non-manga mode: simple ratio-based offset adjustment
                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                    self.offset = self.offset * zoom_ratio;
                    self.remember_current_fullscreen_view_state();
                    self.maybe_refresh_current_solo_image_lod();
                }
            }

            ctx.request_repaint(); // Ensure continuous updates while holding
        }

        let bar_size = egui::Vec2::new(Self::MANGA_HUD_PANEL_WIDTH, Self::MANGA_HUD_PANEL_HEIGHT);
        let bar_pos = egui::pos2(
            screen_rect.max.x - bar_size.x - margin - scrollbar_padding,
            screen_rect.max.y - bar_size.y - margin - video_controls_offset,
        );

        if self.mode().is_masonry() {
            let rows_bar_pos =
                egui::pos2(bar_pos.x, bar_pos.y - Self::MANGA_HUD_PANEL_VERTICAL_STEP);
            egui::Area::new(egui::Id::new("masonry_rows_bar"))
                .fixed_pos(rows_bar_pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let panel_rect = egui::Rect::from_min_size(rows_bar_pos, bar_size);
                    ui.painter().rect_filled(
                        panel_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180),
                    );

                    let inner_rect = panel_rect.shrink2(egui::vec2(
                        (Self::MANGA_HUD_PANEL_WIDTH - Self::MANGA_HUD_PANEL_INNER_WIDTH) * 0.5,
                        (Self::MANGA_HUD_PANEL_HEIGHT - Self::MANGA_HUD_PANEL_INNER_HEIGHT) * 0.5,
                    ));
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().slider_width = 80.0;
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let (rows_label_rect, _) = ui
                                .allocate_exact_size(egui::vec2(32.0, 24.0), egui::Sense::hover());
                            ui.painter().text(
                                rows_label_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "Cols",
                                egui::FontId::proportional(12.0),
                                egui::Color32::from_rgb(220, 220, 220),
                            );

                            let (rows_minus_rect, rows_minus_resp) = ui
                                .allocate_exact_size(egui::vec2(20.0, 24.0), egui::Sense::click());
                            if ui.is_rect_visible(rows_minus_rect) {
                                let minus_bg = if rows_minus_resp.is_pointer_button_down_on() {
                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36)
                                } else if rows_minus_resp.hovered() {
                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rows_minus_rect, 4.0, minus_bg);
                                ui.painter().text(
                                    rows_minus_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "−",
                                    egui::FontId::proportional(15.0),
                                    egui::Color32::WHITE,
                                );
                            }
                            if rows_minus_resp.clicked() {
                                self.set_masonry_items_per_row(
                                    self.masonry_items_per_row.saturating_sub(1),
                                );
                                self.touch_bottom_overlays();
                            }

                            let mut slider_rows = self.masonry_items_per_row as i32;
                            let rows_slider = egui::Slider::new(&mut slider_rows, 2..=10)
                                .show_value(false)
                                .clamping(egui::SliderClamping::Always);
                            let rows_resp = ui.add_sized([80.0, 24.0], rows_slider);

                            if rows_resp.changed() {
                                self.set_masonry_items_per_row(slider_rows as usize);
                            }

                            let (rows_plus_rect, rows_plus_resp) = ui
                                .allocate_exact_size(egui::vec2(20.0, 24.0), egui::Sense::click());
                            if ui.is_rect_visible(rows_plus_rect) {
                                let plus_bg = if rows_plus_resp.is_pointer_button_down_on() {
                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36)
                                } else if rows_plus_resp.hovered() {
                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rows_plus_rect, 4.0, plus_bg);
                                ui.painter().text(
                                    rows_plus_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "+",
                                    egui::FontId::proportional(15.0),
                                    egui::Color32::WHITE,
                                );
                            }
                            if rows_plus_resp.clicked() {
                                self.set_masonry_items_per_row(
                                    self.masonry_items_per_row.saturating_add(1),
                                );
                                self.touch_bottom_overlays();
                            }

                            if rows_resp.hovered() || rows_resp.dragged() {
                                self.touch_bottom_overlays();
                            }

                            if rows_minus_resp.hovered() || rows_plus_resp.hovered() {
                                self.touch_bottom_overlays();
                            }

                            let rows_value = format!("{}", self.masonry_items_per_row);
                            let (rows_value_rect, _) = ui
                                .allocate_exact_size(egui::vec2(40.0, 24.0), egui::Sense::hover());
                            ui.painter().text(
                                rows_value_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                rows_value,
                                egui::FontId::proportional(12.0),
                                egui::Color32::from_rgb(200, 200, 200),
                            );
                        });
                    });
                });
        }

        egui::Area::new(egui::Id::new("manga_zoom_bar"))
            .fixed_pos(bar_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let panel_rect = egui::Rect::from_min_size(bar_pos, bar_size);
                ui.painter().rect_filled(
                    panel_rect,
                    6.0,
                    egui::Color32::from_rgba_unmultiplied(40, 40, 40, 180),
                );

                let inner_rect = panel_rect.shrink2(egui::vec2(
                    (Self::MANGA_HUD_PANEL_WIDTH - Self::MANGA_HUD_PANEL_INNER_WIDTH) * 0.5,
                    (Self::MANGA_HUD_PANEL_HEIGHT - Self::MANGA_HUD_PANEL_INNER_HEIGHT) * 0.5,
                ));
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                    let display_zoom = self.zoom;
                    let max_zoom = self.max_zoom_factor();

                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.spacing_mut().slider_width = 100.0;
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let (minus_rect, minus_resp) =
                            ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());

                        if ui.is_rect_visible(minus_rect) {
                            let minus_bg = if minus_resp.is_pointer_button_down_on() {
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36)
                            } else if minus_resp.hovered() {
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(minus_rect, 4.0, minus_bg);
                            ui.painter().text(
                                minus_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "−",
                                egui::FontId::proportional(16.0),
                                egui::Color32::WHITE,
                            );
                        }

                        if minus_resp.is_pointer_button_down_on() {
                            if !self.manga_zoom_minus_held {
                                self.manga_zoom_minus_held = true;
                                self.manga_zoom_plus_held = false;
                                self.manga_zoom_hold_start = Instant::now();
                                if self.mode().is_manga() {
                                    self.apply_manga_zoom_step(false);
                                } else {
                                    self.apply_fullscreen_zoom_step(false);
                                }
                            }
                            self.touch_bottom_overlays();
                        }

                        let mut slider_value = display_zoom;
                        let slider = egui::Slider::new(&mut slider_value, 0.1..=max_zoom)
                            .show_value(false)
                            .clamping(egui::SliderClamping::Always);
                        let slider_resp = ui.add_sized([100.0, 24.0], slider);

                        if slider_resp.changed() && slider_resp.dragged() {
                            let old_zoom = self.zoom.max(0.0001);
                            let new_zoom = self.clamp_zoom(slider_value);

                            if (new_zoom - old_zoom).abs() > 0.0001 {
                                let zoom_ratio = new_zoom / old_zoom;

                                if self.mode().is_manga() {
                                    if self.mode().is_masonry() {
                                        let center_pos = egui::pos2(
                                            self.screen_size.x * 0.5,
                                            self.screen_size.y * 0.5,
                                        );
                                        if self
                                            .apply_masonry_zoom_at_screen_pos(new_zoom, center_pos)
                                        {
                                            self.manga_finish_direct_zoom_change();
                                        }
                                    } else {
                                        let center_pos = egui::pos2(
                                            self.screen_size.x * 0.5,
                                            self.screen_size.y * 0.5,
                                        );
                                        if self.apply_strip_zoom_at_screen_pos(new_zoom, center_pos)
                                        {
                                            self.manga_finish_direct_zoom_change();
                                        }
                                    }
                                } else {
                                    self.zoom = new_zoom;
                                    self.zoom_target = new_zoom;
                                    self.zoom_velocity = 0.0;
                                    self.offset = self.offset * zoom_ratio;
                                    self.remember_current_fullscreen_view_state();
                                }
                            }

                            self.manga_zoom_plus_held = false;
                            self.manga_zoom_minus_held = false;
                        }

                        let (plus_rect, plus_resp) =
                            ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());

                        if ui.is_rect_visible(plus_rect) {
                            let plus_bg = if plus_resp.is_pointer_button_down_on() {
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36)
                            } else if plus_resp.hovered() {
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(plus_rect, 4.0, plus_bg);
                            ui.painter().text(
                                plus_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "+",
                                egui::FontId::proportional(16.0),
                                egui::Color32::WHITE,
                            );
                        }

                        if plus_resp.is_pointer_button_down_on() {
                            if !self.manga_zoom_plus_held {
                                self.manga_zoom_plus_held = true;
                                self.manga_zoom_minus_held = false;
                                self.manga_zoom_hold_start = Instant::now();
                                if self.mode().is_manga() {
                                    self.apply_manga_zoom_step(true);
                                } else {
                                    self.apply_fullscreen_zoom_step(true);
                                }
                            }
                            self.touch_bottom_overlays();
                        }

                        let zoom_value = format!("{:.0}%", (display_zoom * 100.0).round());
                        let (zoom_label_rect, _) =
                            ui.allocate_exact_size(egui::vec2(48.0, 24.0), egui::Sense::hover());
                        ui.painter().text(
                            egui::pos2(zoom_label_rect.left() + 2.0, zoom_label_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            zoom_value,
                            egui::FontId::proportional(12.0),
                            egui::Color32::from_rgb(200, 200, 200),
                        );
                    });
                });
            });
    }

    /// Apply a single zoom step for manga mode (used for initial click)
    pub(crate) fn apply_manga_zoom_step(&mut self, zoom_in: bool) {
        let step = self.config.zoom_step;
        let old_zoom = self.zoom.max(0.0001);
        let new_zoom = if zoom_in {
            self.clamp_zoom(self.zoom * step)
        } else {
            self.clamp_zoom(self.zoom / step)
        };

        if (new_zoom - old_zoom).abs() > 0.0001 {
            let center_pos = egui::pos2(self.screen_size.x * 0.5, self.screen_size.y * 0.5);
            if self.mode().is_masonry() {
                if self.apply_masonry_zoom_at_screen_pos(new_zoom, center_pos) {
                    self.manga_finish_direct_zoom_change();
                }
            } else if self.apply_strip_zoom_at_screen_pos(new_zoom, center_pos) {
                self.manga_finish_direct_zoom_change();
            }
        }
    }
}

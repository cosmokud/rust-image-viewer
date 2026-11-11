//! WebP frame-rate override controls.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Draw GIF seekbar and controls for non-manga mode
    pub(crate) fn draw_gif_seekbar_inner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(ref img) = self.image else {
            return;
        };
        if !img.is_animated() {
            return;
        }

        let frame_count = img.frame_count();
        let current_frame = img.current_frame_index();
        let total_duration_ms = img.total_duration_ms();
        let mut display_frame_count = frame_count;
        if !self.anim_stream_done {
            let base = self.anim_seekbar_total_frames.unwrap_or(frame_count.max(1));
            display_frame_count = base.max(current_frame + 1).max(1);
            self.anim_seekbar_total_frames = Some(display_frame_count);
        }
        let position_fraction = if !self.anim_stream_done {
            if display_frame_count > 1 {
                current_frame as f32 / (display_frame_count - 1) as f32
            } else {
                0.0
            }
        } else {
            img.position_fraction() as f32
        };
        let animated_label = Self::animated_image_label_for_path(
            self.image_list.get(self.current_index).or(Some(&img.path)),
        );
        let show_fps_controls = Self::path_uses_animated_fps_override(img.path.as_path());
        let animated_media_path = img.path.clone();
        let mut file_navigation_requested: Option<VideoFileNavigation> = None;

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
            let seek_bar_height = 6.0;
            let available_width = ui.available_width();
            let (seek_rect, seek_response) = ui.allocate_exact_size(
                egui::Vec2::new(available_width, seek_bar_height + 8.0),
                egui::Sense::click_and_drag(),
            );

            let bar_inner = egui::Rect::from_min_size(
                egui::pos2(
                    seek_rect.min.x,
                    seek_rect.center().y - seek_bar_height / 2.0,
                ),
                egui::Vec2::new(seek_rect.width(), seek_bar_height),
            );

            // Background bar
            ui.painter()
                .rect_filled(bar_inner, 3.0, egui::Color32::from_gray(60));

            // Progress bar
            let display_fraction = if self.gif_seeking {
                self.gif_seek_preview_frame
                    .map(|f| f as f32 / (display_frame_count - 1).max(1) as f32)
                    .unwrap_or(position_fraction)
            } else {
                position_fraction
            };
            let progress_width = bar_inner.width() * display_fraction;
            if progress_width > 0.0 {
                let progress_rect = egui::Rect::from_min_size(
                    bar_inner.min,
                    egui::Vec2::new(progress_width, seek_bar_height),
                );
                ui.painter()
                    .rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(76, 175, 80));
                // Green for GIF
            }

            // Seek handle
            let handle_x = bar_inner.min.x + progress_width;
            let handle_center = egui::pos2(handle_x, bar_inner.center().y);
            let handle_radius = if seek_response.hovered() || seek_response.dragged() {
                8.0
            } else {
                6.0
            };
            ui.painter()
                .circle_filled(handle_center, handle_radius, egui::Color32::WHITE);

            // Handle seeking
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released =
                ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

            if seek_response.is_pointer_button_down_on() && !self.gif_seeking {
                self.gif_seeking = true;
            }

            if self.gif_seeking && primary_down {
                if let Some(pos) = seek_response
                    .interact_pointer_pos()
                    .or_else(|| ctx.input(|i| i.pointer.hover_pos()))
                {
                    let seek_fraction =
                        ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    self.gif_seek_preview_frame = Some(target_frame);

                    // Update the actual frame
                    if let Some(ref mut img) = self.image {
                        img.set_frame(target_frame);
                        self.texture = None; // Force texture rebuild
                    }
                }
                ctx.request_repaint();
            }

            if seek_response.clicked() && !self.gif_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction =
                        ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    if let Some(ref mut img) = self.image {
                        img.set_frame(target_frame);
                        self.texture = None;
                    }
                    ctx.request_repaint();
                }
            }

            if self.gif_seeking && (primary_released || !primary_down) {
                self.gif_seeking = false;
                self.gif_seek_preview_frame = None;
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                // Play/Pause button
                let play_btn = ui.add(
                    egui::Button::new(if self.gif_paused { "▶" } else { "⏸" })
                        .min_size(egui::vec2(32.0, 24.0)),
                );

                if play_btn.clicked() {
                    self.gif_paused = !self.gif_paused;
                }

                ui.add_space(4.0);

                let prev_btn = Self::video_control_icon_button(
                    ui,
                    VideoControlIcon::Previous,
                    self.navigation_tooltip_previous(),
                    None,
                    false,
                );
                if prev_btn.clicked() {
                    file_navigation_requested = Some(VideoFileNavigation::Previous);
                }

                let next_btn = Self::video_control_icon_button(
                    ui,
                    VideoControlIcon::Next,
                    self.navigation_tooltip_next(),
                    None,
                    false,
                );
                if next_btn.clicked() {
                    file_navigation_requested = Some(VideoFileNavigation::Next);
                }

                ui.add_space(8.0);

                // Frame display
                let duration_secs = total_duration_ms as f64 / 1000.0;
                let current_time = (position_fraction as f64 * duration_secs).max(0.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Frame {}/{} ({:.1}s / {:.1}s)",
                        current_frame + 1,
                        frame_count,
                        current_time,
                        duration_secs
                    ))
                    .color(egui::Color32::WHITE)
                    .size(12.0),
                );

                // Animated image indicator on right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(animated_label)
                            .color(egui::Color32::from_rgb(76, 175, 80))
                            .size(14.0),
                    );
                    if show_fps_controls {
                        ui.add_space(2.0);
                        self.draw_webp_fps_controls(
                            ui,
                            ctx,
                            Self::solo_webp_fps_popup_id(),
                            animated_media_path.as_path(),
                            frame_count,
                            total_duration_ms as u64,
                        );
                    }
                });
            });
        });

        if let Some(navigation) = file_navigation_requested {
            match navigation {
                VideoFileNavigation::Previous => self.navigate_prev_for_video_mode(),
                VideoFileNavigation::Next => self.navigate_next_for_video_mode(),
            }
        }
    }
}

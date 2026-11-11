//! The main control overlays (bottom bar, sliders).

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Draw video controls bar at the bottom of the screen
    pub(crate) fn draw_video_controls(&mut self, ctx: &egui::Context) {
        // Skip if we're in manga mode (manga has its own controls)
        if self.mode().is_strip() {
            return;
        }

        // Check if we have a video or animated GIF
        let has_video = self.video_player.is_some() || self.is_video_playback_preview_mode();
        let has_animated_gif = self.image.as_ref().map_or(false, |img| img.is_animated());

        if !has_video && !has_animated_gif {
            return;
        }

        if !self.show_video_controls {
            self.mouse_over_video_controls = false;
            return;
        }

        let screen_rect = ctx.screen_rect();
        let bar_height = 56.0; // Increased height for bottom padding
        let bottom_padding = 8.0; // Gap at the bottom so buttons don't look cramped

        // Draw control bar
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(0.0, screen_rect.height() - bar_height),
            egui::Vec2::new(screen_rect.width(), bar_height),
        );

        egui::Area::new(egui::Id::new("video_control_bar"))
            .fixed_pos(bar_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let painter = ui.painter();

                // Semi-transparent background
                painter.rect_filled(
                    bar_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(20, 20, 20, 230),
                );

                // Check if mouse is over this bar
                self.mouse_over_video_controls =
                    self.mouse_over_video_controls || ui.rect_contains_pointer(bar_rect);

                // Create inner rect with padding (more on bottom for visual breathing room)
                let inner_rect = egui::Rect::from_min_max(
                    egui::pos2(bar_rect.min.x + 8.0, bar_rect.min.y + 6.0),
                    egui::pos2(bar_rect.max.x - 8.0, bar_rect.max.y - bottom_padding - 4.0),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                    ui.set_min_height(inner_rect.height());

                    if self.video_player.is_some() {
                        self.draw_video_seekbar_inner(ui, ctx);
                    } else if self.is_video_playback_preview_mode() {
                        self.draw_video_playback_unavailable_controls_inner(ui);
                    } else if has_animated_gif {
                        self.draw_gif_seekbar_inner(ui, ctx);
                    }
                });
            });
    }
}

//! GIF/animated seekbar.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Draw video/GIF controls bar for manga reading mode at the bottom of the screen.
    /// Shows seekbar and audio controls for the currently focused video,
    /// or a GIF seekbar for animated images.
    pub(crate) fn draw_manga_video_controls(&mut self, ctx: &egui::Context) {
        // Only show in manga mode fullscreen
        if self.mode().is_solo() || !self.mode().is_fullscreen() {
            return;
        }

        if !self.mode().shows_preview_video_controls() {
            return;
        }

        if !self.show_video_controls {
            return;
        }

        let focused_idx = self.manga_focused_video_index;

        // Determine the type of focused media
        let focused_media_type = if let Some(idx) = focused_idx {
            self.manga_loader
                .as_ref()
                .and_then(|loader| loader.get_media_type(idx))
        } else {
            // Check if current image is an animated GIF/WebP
            let current_idx = self.manga_get_focused_media_index();
            self.manga_loader
                .as_ref()
                .and_then(|loader| loader.get_media_type(current_idx))
        };

        // Check if we have a video playing or an animated image
        let has_video =
            focused_idx.is_some() && matches!(focused_media_type, Some(MangaMediaType::Video));
        let has_animated = matches!(focused_media_type, Some(MangaMediaType::AnimatedImage));

        if !has_video && !has_animated {
            return;
        }

        let screen_rect = ctx.screen_rect();
        let bar_height = 56.0;
        let bottom_padding = 8.0;

        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(0.0, screen_rect.height() - bar_height),
            egui::Vec2::new(screen_rect.width(), bar_height),
        );

        egui::Area::new(egui::Id::new("manga_video_control_bar"))
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

                self.mouse_over_video_controls =
                    self.mouse_over_video_controls || ui.rect_contains_pointer(bar_rect);

                let inner_rect = egui::Rect::from_min_max(
                    egui::pos2(bar_rect.min.x + 8.0, bar_rect.min.y + 6.0),
                    egui::pos2(bar_rect.max.x - 8.0, bar_rect.max.y - bottom_padding - 4.0),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                    ui.set_min_height(inner_rect.height());

                    if has_video {
                        self.draw_manga_video_seekbar(ui, ctx, focused_idx.unwrap());
                    } else if has_animated {
                        let current_idx = self.manga_get_focused_media_index();
                        self.draw_manga_gif_seekbar(ui, ctx, current_idx);
                    }
                });
            });
    }
}

//! Video control cluster.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_video_playback_unavailable_controls_inner(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let play_btn = ui.add(egui::Button::new("▶").min_size(egui::vec2(32.0, 24.0)));
            if play_btn.clicked() {
                self.queue_video_playback_unavailable_popup();
            }

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(Self::gstreamer_missing_video_error_text())
                    .color(egui::Color32::from_rgb(255, 190, 135))
                    .size(11.0),
            );
        });
    }

    pub(crate) fn commit_video_seek_mode(&self) -> VideoSeekMode {
        match self.config.video_seek_policy {
            VideoSeekPolicy::Adaptive | VideoSeekPolicy::Accurate => VideoSeekMode::Accurate,
            VideoSeekPolicy::Keyframe => VideoSeekMode::Keyframe,
        }
    }
}

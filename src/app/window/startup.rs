//! First-frame readiness, window showing, no-file startup window.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn show_window_if_ready(&mut self, ctx: &egui::Context) {
        if self.startup_window_shown {
            return;
        }

        if !self.startup_ready_to_show() {
            return;
        }

        if matches!(self.current_media_type, Some(MediaType::Video)) {
            let size = if let Some((vid_w, vid_h)) = self.media_display_dimensions() {
                self.floating_layout_size_for_media(
                    vid_w as f32,
                    vid_h as f32,
                    self.monitor_size_points(ctx),
                )
                .map(|(_, size)| size)
                .unwrap_or(egui::Vec2::new(800.0, 600.0))
            } else {
                egui::Vec2::new(800.0, 600.0)
            };

            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.center_window_on_monitor(ctx, size);
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.startup_window_shown = true;
        self.needs_repaint = true;
    }

    pub(crate) fn text_needs_cjk_fonts(text: &str) -> bool {
        // Check common CJK Unicode blocks (Han, Hiragana, Katakana, Hangul).
        text.chars().any(|ch| {
            let c = ch as u32;
            (0x3400..=0x4DBF).contains(&c) // CJK Unified Ideographs Extension A
                || (0x4E00..=0x9FFF).contains(&c) // CJK Unified Ideographs
                || (0xF900..=0xFAFF).contains(&c) // CJK Compatibility Ideographs
                || (0x3040..=0x309F).contains(&c) // Hiragana
                || (0x30A0..=0x30FF).contains(&c) // Katakana
                || (0x31F0..=0x31FF).contains(&c) // Katakana Phonetic Extensions
                || (0x1100..=0x11FF).contains(&c) // Hangul Jamo
                || (0xAC00..=0xD7AF).contains(&c) // Hangul Syllables
        })
    }
}

pub(crate) fn startup_window_for_no_file(
    screen_size: egui::Vec2,
) -> (egui::Vec2, egui::Pos2, bool) {
    let size = egui::Vec2::new(500.0, 500.0);
    let pos = egui::Pos2::new(
        ((screen_size.x - size.x) * 0.5).max(0.0),
        ((screen_size.y - size.y) * 0.5).max(0.0),
    );
    (size, pos, true)
}

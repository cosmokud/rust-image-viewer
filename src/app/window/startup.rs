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

        if !self.mode().is_fullscreen() && matches!(self.current_media_type, Some(MediaType::Video))
        {
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

            let position = self.floating_centred_position(ctx, size);
            // Clear stale pre-show placement so async WM_PLACE_RECT cannot overwrite startup rect.
            self.pending_viewer_rect = None;
            self.transition_cover = None;
            self.transition_cover_waits = 0;
            self.cover_next_placement = false;
            self.floating_window_position = None;

            let mut placed_synchronously = false;
            #[cfg(target_os = "windows")]
            {
                crate::windows_env::clear_posted_window_rect();
                let ppp = ctx.pixels_per_point();
                let hwnd = self.resolved_main_hwnd();
                if ppp.is_finite() && ppp > 0.01 && !hwnd.is_null() {
                    let outer = self.outer_size_for_inner(ctx, size);
                    let width = (outer.x * ppp).round() as i32;
                    let height = (outer.y * ppp).round() as i32;
                    let left = (position.x * ppp).round() as i32;
                    let top = (position.y * ppp).round() as i32;
                    placed_synchronously =
                        crate::windows_env::set_window_rect(hwnd, left, top, width, height);
                }
            }
            if !placed_synchronously {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                self.send_outer_position(ctx, position);
            }
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

pub(crate) fn startup_window_for_video(
    screen_size: egui::Vec2,
    gstreamer_available: bool,
) -> (egui::Vec2, egui::Pos2, bool) {
    let size = egui::Vec2::new(800.0, 600.0);
    let pos = egui::Pos2::new(
        ((screen_size.x - size.x) * 0.5).max(0.0),
        ((screen_size.y - size.y) * 0.5).max(0.0),
    );
    // Keep window hidden via WS_VISIBLE=false until first frame; stay on-screen so OS/egui rects stay valid.
    (size, pos, !gstreamer_available)
}

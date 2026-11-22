//! First-frame readiness, window showing, no-file startup window.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const STARTUP_REVEAL_TIMEOUT: Duration = Duration::from_millis(150);
const STARTUP_REVEAL_SIZE_TOLERANCE_PT: f32 = 2.0;
const STARTUP_REVEAL_MAX_MISMATCHED_FRAMES: u8 = 3;

/// Tracks post-show frames while DWM-cloaked so uncloaking happens after a frame at the target size swaps.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StartupRevealState {
    armed_at: Instant,
    target_inner_size: Option<egui::Vec2>,
    matched_frames: u8,
    painted_frames: u8,
}

impl StartupRevealState {
    pub(crate) fn new(target_inner_size: Option<egui::Vec2>, now: Instant) -> Self {
        Self {
            armed_at: now,
            target_inner_size,
            matched_frames: 0,
            painted_frames: 0,
        }
    }

    pub(crate) fn note_painted_frame(&mut self, frame_size: egui::Vec2) {
        self.painted_frames = self.painted_frames.saturating_add(1);
        let size_matches = match self.target_inner_size {
            None => true,
            Some(target) => {
                (frame_size.x - target.x).abs() <= STARTUP_REVEAL_SIZE_TOLERANCE_PT
                    && (frame_size.y - target.y).abs() <= STARTUP_REVEAL_SIZE_TOLERANCE_PT
            }
        };
        if size_matches {
            self.matched_frames = self.matched_frames.saturating_add(1);
        }
    }

    pub(crate) fn ready_to_uncloak(&self, now: Instant) -> bool {
        self.matched_frames >= 1
            || self.painted_frames >= STARTUP_REVEAL_MAX_MISMATCHED_FRAMES
            || now.saturating_duration_since(self.armed_at) >= STARTUP_REVEAL_TIMEOUT
    }
}

impl ImageViewer {
    /// Returns true on the exact frame that transitions `startup_window_shown` to true.
    pub(crate) fn show_window_if_ready(&mut self, ctx: &egui::Context) -> bool {
        if self.startup_window_shown {
            return false;
        }

        if !self.startup_ready_to_show() {
            return false;
        }

        let mut target_inner_size = None;
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
            target_inner_size = Some(size);

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
        if self.startup_window_cloaked {
            self.startup_reveal = Some(StartupRevealState::new(target_inner_size, Instant::now()));
            ctx.request_repaint();
        }
        self.needs_repaint = true;
        true
    }

    /// Uncloaks the startup window at the top of the frame *after* a target-sized frame has swapped.
    pub(crate) fn maybe_uncloak_startup_window(&mut self) {
        if !self.startup_window_cloaked || !self.startup_window_shown {
            return;
        }
        let should_uncloak = self
            .startup_reveal
            .as_ref()
            .map_or(true, |reveal| reveal.ready_to_uncloak(Instant::now()));
        if !should_uncloak {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            let hwnd = self.resolved_main_hwnd();
            if !hwnd.is_null() {
                crate::windows_env::set_window_cloaked(hwnd, false);
            }
        }
        self.startup_window_cloaked = false;
        self.startup_reveal = None;
    }

    pub(crate) fn note_startup_reveal_frame_painted(&mut self, frame_size: egui::Vec2) {
        if let Some(reveal) = self.startup_reveal.as_mut() {
            reveal.note_painted_frame(frame_size);
        }
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

//! Zoom clamps, fit maths and background colours.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn clamp_zoom(&self, zoom: f32) -> f32 {
        zoom.clamp(0.1, self.max_zoom_factor())
    }

    pub(crate) fn fit_zoom_for_target_height(&self, target_height: f32, media_height: f32) -> f32 {
        if target_height <= 0.0 || media_height <= 0.0 {
            return 1.0;
        }

        // Layout fit must support very tall media where the correct fit can be < 0.1x.
        // Keep the interactive zoom floor at 0.1x, but allow fit calculations to go lower.
        (target_height / media_height)
            .max(0.0001)
            .min(self.max_zoom_factor())
    }

    pub(crate) fn fit_zoom_for_target_bounds(
        &self,
        target_size: egui::Vec2,
        media_size: egui::Vec2,
    ) -> f32 {
        if target_size.x <= 0.0
            || target_size.y <= 0.0
            || media_size.x <= 0.0
            || media_size.y <= 0.0
        {
            return 1.0;
        }

        let fit_x = target_size.x / media_size.x;
        let fit_y = target_size.y / media_size.y;

        // Fit to whichever axis is limiting first.
        fit_x.min(fit_y).max(0.0001).min(self.max_zoom_factor())
    }

    pub(crate) fn startup_ready_to_show(&self) -> bool {
        if self.error_message.is_some() || self.is_video_playback_unavailable_active() {
            return true;
        }

        match self.current_media_type {
            None => true,
            Some(MediaType::Image) => self.image.is_some(),
            Some(MediaType::Video) => {
                // For videos, we need ALL of these conditions to show the window:
                // 1. Video dimensions are known (first frame decoded)
                // 2. Layout has been applied (pending_media_layout is false)
                // 3. Video texture exists (first frame is ready to display)
                // This ensures the window appears with the correct size AND the first frame visible.
                // Safety fallback: don't stay hidden forever.
                let ready = self.media_display_dimensions().is_some()
                    && !self.pending_media_layout
                    && self.video_texture.is_some();
                ready || self.startup_hide_started_at.elapsed() > Duration::from_secs(2)
            }
        }
    }

    pub(crate) fn background_clear_color(&self) -> [f32; 4] {
        let [r, g, b] = self.config.background_rgb;
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
    }

    /// Zoom at a specific point
    pub(crate) fn zoom_at(&mut self, center: egui::Pos2, factor: f32, available_rect: egui::Rect) {
        let old_zoom = self.zoom;
        self.zoom = self.clamp_zoom(self.zoom * factor);

        // In fullscreen we allow panning and cursor-follow zoom.
        // In floating mode we keep the image centered and let the window autosize instead.
        if self.mode().is_fullscreen() {
            let rect_center = available_rect.center();
            let cursor_offset = center - rect_center;

            let zoom_ratio = self.zoom / old_zoom;
            self.offset = self.offset * zoom_ratio - cursor_offset * (zoom_ratio - 1.0);
            self.remember_current_fullscreen_view_state();
        } else {
            self.offset = egui::Vec2::ZERO;
        }
        self.maybe_refresh_current_solo_image_lod();
    }

    // ============ MANGA READING MODE METHODS ============

    pub(crate) fn mode(&self) -> Mode {
        self.mode_state.mode()
    }

    pub(crate) fn mode_transition(&mut self, event: ModeEvent) -> Mode {
        self.mode_state.transition(event)
    }

    pub(crate) fn clear_strip_return_context(&mut self) {
        let should_clear_preserved_masonry_cache =
            self.strip_return_preserve_masonry_cache && self.mode().is_solo();

        self.mode_state.clear_strip_return();
        self.strip_return_masonry_state = None;
        self.strip_return_masonry_list_snapshot = None;
        self.strip_return_preserve_masonry_cache = false;

        if should_clear_preserved_masonry_cache && !self.has_resident_masonry_runtime_cache() {
            self.clear_manga_runtime_cache(true);
        }
    }

    pub(crate) fn activate_strip_return_context(
        &mut self,
        layout_mode: MangaLayoutMode,
        opened_index: usize,
    ) {
        self.mode_state.set_strip_return_layout(layout_mode);
        self.strip_return_preserve_masonry_cache =
            (layout_mode).is_grid() && self.mode().is_manga();
        self.strip_return_masonry_list_snapshot =
            if (layout_mode).is_grid() && self.mode().is_manga() {
                Some(self.image_list.clone())
            } else {
                None
            };
        self.strip_return_masonry_state = if (layout_mode).is_grid() && self.mode().is_manga() {
            Some(MasonryReturnState {
                zoom: self.zoom,
                zoom_target: self.zoom_target,
                offset: self.offset,
                scroll_offset: self.manga_scroll_offset,
                scroll_target: self.manga_scroll_target,
                opened_index: opened_index.min(self.image_list.len().saturating_sub(1)),
                list_signature: self.image_list_signature,
            })
        } else {
            None
        };
    }
}

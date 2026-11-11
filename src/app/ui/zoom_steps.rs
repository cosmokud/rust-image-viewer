//! Zoom step actions.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Apply a single zoom step in fullscreen image mode (used for initial click)
    pub(crate) fn apply_fullscreen_zoom_step(&mut self, zoom_in: bool) {
        let step = self.config.zoom_step;
        let old_zoom = self.zoom.max(0.0001);
        let new_zoom = if zoom_in {
            self.clamp_zoom(self.zoom * step)
        } else {
            self.clamp_zoom(self.zoom / step)
        };

        if (new_zoom - old_zoom).abs() > 0.0001 {
            let zoom_ratio = new_zoom / old_zoom;
            self.zoom = new_zoom;
            self.zoom_target = new_zoom;
            self.zoom_velocity = 0.0;
            self.offset = self.offset * zoom_ratio;
            self.remember_current_fullscreen_view_state();
            self.maybe_refresh_current_solo_image_lod();
        }
    }

    /// Apply pointer-anchored zoom for long-strip mode using screen-space cursor position.
    /// Keeps the content point under the cursor stable while zooming.
    pub(crate) fn apply_strip_zoom_at_screen_pos(
        &mut self,
        new_zoom: f32,
        anchor_screen: egui::Pos2,
    ) -> bool {
        if self.mode().is_solo() || self.mode().is_masonry() {
            return false;
        }

        let old_zoom = self.zoom.max(0.0001);
        let new_zoom = self.clamp_zoom(new_zoom);
        if (new_zoom - old_zoom).abs() <= 0.0001 {
            return false;
        }

        let anchor_screen_x = anchor_screen.x;
        let anchor_screen_y = anchor_screen.y.clamp(0.0, self.screen_size.y.max(1.0));
        let anchor_abs_y = self.manga_scroll_offset.max(0.0) + anchor_screen_y;
        let anchor_idx = self
            .manga_index_at_y(anchor_abs_y)
            .min(self.image_list.len().saturating_sub(1));
        let page_start_y = self.manga_page_start_y(anchor_idx);
        let page_height = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let page_fraction_y = ((anchor_abs_y - page_start_y) / page_height).clamp(0.0, 1.0);

        let page_width = self.manga_get_image_display_width(anchor_idx).max(0.0001);
        let page_left = (self.screen_size.x - page_width) * 0.5 + self.offset.x;
        let page_fraction_x = ((anchor_screen_x - page_left) / page_width).clamp(0.0, 1.0);

        self.zoom = new_zoom;
        self.zoom_target = new_zoom;
        self.zoom_velocity = 0.0;
        self.invalidate_manga_layout_cache_for_zoom();

        let new_page_width = self.manga_get_image_display_width(anchor_idx).max(0.0001);
        let centered_left = (self.screen_size.x - new_page_width) * 0.5;
        self.offset.x = anchor_screen_x - page_fraction_x * new_page_width - centered_left;

        let new_page_start_y = self.manga_page_start_y(anchor_idx);
        let new_page_height = self.manga_page_height_cached(anchor_idx).max(0.0001);
        let new_anchor_abs_y = new_page_start_y + page_fraction_y * new_page_height;
        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        let new_scroll = (new_anchor_abs_y - anchor_screen_y).clamp(0.0, max_scroll);
        self.manga_scroll_offset = new_scroll;
        self.manga_scroll_target = new_scroll;
        self.manga_scroll_velocity = 0.0;

        true
    }
}

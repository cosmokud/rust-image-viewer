//! Layout cache invalidation.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn invalidate_manga_layout_cache_for_zoom(&mut self) {
        if self.mode().is_masonry() {
            return;
        }

        let len = self.image_list.len();
        let screen_y = self.screen_size.y.round();
        let zoom = (self.zoom * 10_000.0).round() / 10_000.0;
        let cached_zoom = self.manga_total_height_cache_zoom.max(0.0001);

        let can_rescale_cached_layout = self.manga_total_height_cache_valid
            && len > 0
            && self.manga_layout_offsets.len() == len.saturating_add(1)
            && self.manga_total_height_cache_len == len
            && (self.manga_total_height_cache_screen_y - screen_y).abs() <= 1e-6
            && zoom.is_finite()
            && cached_zoom.is_finite()
            && (zoom - cached_zoom).abs() > 1e-6;

        if can_rescale_cached_layout {
            let ratio = zoom / cached_zoom;
            if ratio.is_finite() && ratio > 0.0 {
                for y in &mut self.manga_layout_offsets {
                    *y *= ratio;
                }
                self.manga_total_height_cache = (self.manga_total_height_cache * ratio).max(0.0);
                self.manga_total_height_cache_zoom = zoom;
                self.manga_total_height_cache_screen_y = screen_y;
                self.manga_total_height_cache_len = len;
                self.manga_total_height_cache_valid = true;
                self.manga_strip_spatial_index = None;
                return;
            }
        }

        self.invalidate_manga_layout_cache();
    }

    pub(crate) fn toggle_long_strip_mode(&mut self) {
        self.toggle_strip_mode(MangaLayoutMode::LongStrip);
    }
}

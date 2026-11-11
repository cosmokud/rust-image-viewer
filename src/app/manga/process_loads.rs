//! Draining the decode results into textures.

use crate::app::*;

impl ImageViewer {
    /// Calculate total height of all images in manga mode
    pub(crate) fn manga_total_height(&mut self) -> f32 {
        if self.mode().is_solo() || self.image_list.is_empty() {
            self.invalidate_manga_layout_cache();
            self.masonry_layout_items.clear();
            self.masonry_layout_total_height = 0.0;
            return 0.0;
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            return self.masonry_layout_total_height * self.zoom.max(0.0001);
        }

        // Quantize inputs used for cache invalidation.
        // On some platforms/backends, `ctx.screen_rect()` and derived sizes can vary by tiny
        // sub-pixel amounts frame-to-frame, which would otherwise force an O(n) recompute
        // every call and make wheel scrolling feel laggy.
        let zoom = (self.zoom * 10_000.0).round() / 10_000.0;
        let screen_y = self.screen_size.y.round();
        let len = self.image_list.len();

        let needs_recompute = !self.manga_total_height_cache_valid
            || (self.manga_total_height_cache_zoom - zoom).abs() > 1e-6
            || (self.manga_total_height_cache_screen_y - screen_y).abs() > 1e-6
            || self.manga_total_height_cache_len != len;

        if needs_recompute {
            let mut total = 0.0;
            self.manga_layout_offsets.clear();
            self.manga_strip_spatial_index = None;
            self.manga_layout_offsets.reserve(len + 1);
            self.manga_layout_offsets.push(0.0);
            for idx in 0..len {
                let h = self.manga_get_image_display_height(idx).max(0.0);
                total += h;
                self.manga_layout_offsets.push(total);
            }
            self.manga_total_height_cache = total;
            self.manga_total_height_cache_zoom = zoom;
            self.manga_total_height_cache_screen_y = screen_y;
            self.manga_total_height_cache_len = len;
            self.manga_total_height_cache_valid = true;
        }

        self.manga_total_height_cache
    }
}

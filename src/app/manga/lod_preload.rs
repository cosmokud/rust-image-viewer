//! Preload target sides and navigation priming.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_prime_navigation_destination(&mut self, target: usize) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let target = target.min(self.image_list.len().saturating_sub(1));
        let target_texture_side = self.manga_target_texture_side_for_preload(target, &[]);
        let (downscale_filter, gif_filter) = self.manga_decode_filters_for_strip_mode();
        let force_triangle_filters = self.manga_should_force_triangle_filters();

        if let Some(ref mut loader) = self.manga_loader {
            loader.prime(
                &self.image_list,
                target,
                self.max_texture_side,
                target_texture_side,
                downscale_filter,
                gif_filter,
                force_triangle_filters,
            );
        }

        self.manga_preload_cooldown = 0;
        self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
        self.manga_update_preload_queue();
    }

    pub(crate) fn manga_strip_preload_window_counts(
        &self,
        visible_item_equivalent: f32,
    ) -> (usize, usize) {
        MangaLoader::strip_window_for_visible_equivalent(visible_item_equivalent)
    }

    pub(crate) fn manga_strip_visible_item_equivalent(
        &mut self,
        visible_indices: &[usize],
        viewport_top: f32,
        viewport_bottom: f32,
    ) -> f32 {
        if self.mode().is_masonry() || visible_indices.is_empty() {
            return visible_indices.len().max(1) as f32;
        }

        let mut visible_item_equivalent = 0.0f32;
        for &idx in visible_indices {
            let item_top = self.manga_page_start_y(idx);
            let item_height = self.manga_page_height_cached(idx).max(0.0001);
            let item_bottom = item_top + item_height;
            let overlap = (viewport_bottom.min(item_bottom) - viewport_top.max(item_top)).max(0.0);
            visible_item_equivalent += (overlap / item_height).clamp(0.0, 1.0);
        }

        visible_item_equivalent.max(1.0)
    }

    pub(crate) fn invalidate_manga_layout_cache(&mut self) {
        self.manga_total_height_cache_valid = false;
        self.manga_layout_offsets.clear();
        self.manga_strip_spatial_index = None;
        self.masonry_spatial_index = None;
        self.masonry_layout_list_signature = 0;
        self.masonry_layout_valid = false;
        self.masonry_layout_invalidation_deferred = false;
        self.masonry_pending_dimension_updates.clear();
    }
}

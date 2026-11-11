//! Manga cache clearing.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Clear the manga image cache to free GPU memory
    pub(crate) fn manga_clear_cache(&mut self) {
        self.clear_manga_runtime_cache(false);

        self.invalidate_manga_layout_cache();
        self.masonry_layout_items.clear();
        self.masonry_layout_total_height = 0.0;
    }

    /// Determine the focused media index in manga mode.
    /// The focused item is the one with the most viewport coverage (center-weighted).
    /// Returns the index of the media item that should be actively playing.
    pub(crate) fn manga_get_focused_media_index(&mut self) -> usize {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return self.current_index;
        }

        let len = self.image_list.len();

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            if self.masonry_layout_items.len() != len {
                return self.current_index.min(len.saturating_sub(1));
            }

            let zoom = self.zoom.max(0.0001);

            let viewport_top = self.manga_scroll_offset.max(0.0);
            let viewport_h = self.screen_size.y.max(1.0);
            let viewport_bottom = viewport_top + viewport_h;
            let viewport_left = -self.offset.x;
            let viewport_right = viewport_left + self.screen_size.x.max(1.0);
            let viewport_center_x = viewport_left + self.screen_size.x * 0.5;
            let viewport_center_y = viewport_top + viewport_h * 0.5;

            let candidate_indices: Vec<usize> = if self.manga_use_rtree_backend() {
                let layout_left = viewport_left / zoom;
                let layout_right = viewport_right / zoom;
                let layout_top = viewport_top / zoom;
                let layout_bottom = viewport_bottom / zoom;
                self.manga_ensure_masonry_spatial_index()
                    .map(|index| {
                        index.query_indices(layout_left, layout_top, layout_right, layout_bottom)
                    })
                    .unwrap_or_default()
            } else {
                (0..self.masonry_layout_items.len()).collect()
            };

            let mut best_idx = self.current_index.min(len.saturating_sub(1));
            let mut best_score = f32::MAX;

            for idx in candidate_indices {
                let Some(item) = self.masonry_layout_items.get(idx) else {
                    continue;
                };

                let item_top = item.y * zoom;
                let item_bottom = item_top + item.height * zoom;
                let item_left = item.x * zoom;
                let item_right = item_left + item.width * zoom;
                if item_top >= viewport_bottom || item_bottom <= viewport_top {
                    continue;
                }
                if item_left >= viewport_right || item_right <= viewport_left {
                    continue;
                }

                let cx = item_left + item.width * zoom * 0.5;
                let cy = item_top + item.height * zoom * 0.5;
                let dx = cx - viewport_center_x;
                let dy = cy - viewport_center_y;
                let score = dx * dx * 0.25 + dy * dy;
                if score < best_score {
                    best_score = score;
                    best_idx = idx;
                }
            }

            if best_score.is_finite() {
                return best_idx;
            }

            // Fallback when all items are panned out of horizontal viewport.
            let mut fallback_score = f32::MAX;
            for (idx, item) in self.masonry_layout_items.iter().enumerate() {
                let cy = (item.y + item.height * 0.5) * zoom;
                let score = (cy - viewport_center_y).abs();
                if score < fallback_score {
                    fallback_score = score;
                    best_idx = idx;
                }
            }
            return best_idx;
        }

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_h = self.screen_size.y.max(1.0);
        let viewport_bottom = viewport_top + viewport_h;
        let viewport_center = viewport_top + viewport_h * 0.5;

        let mut candidate_indices = if self.manga_use_rtree_backend() {
            self.manga_query_visible_indices_rtree(viewport_top, viewport_bottom)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        if candidate_indices.is_empty() {
            // Only consider items intersecting the viewport.
            let start_idx = self.manga_index_at_y(viewport_top);
            let mut end_idx = self.manga_index_at_y(viewport_bottom);
            if end_idx < start_idx {
                end_idx = start_idx;
            }
            candidate_indices.extend(start_idx..=end_idx.min(len.saturating_sub(1)));
        }

        self.manga_ensure_layout_cache();
        if self.manga_layout_offsets.len() != len + 1 {
            return self.current_index.min(len.saturating_sub(1));
        }

        let mut best_idx = self.current_index.min(len.saturating_sub(1));
        let mut best_center_distance = f32::MAX;

        // Use prefix sums directly for speed.
        for idx in candidate_indices {
            if idx >= len || idx + 1 >= self.manga_layout_offsets.len() {
                continue;
            }
            let start = self.manga_layout_offsets[idx];
            let end = self.manga_layout_offsets[idx + 1];
            let center = (start + end) * 0.5;
            let center_distance = (center - viewport_center).abs();
            if center_distance < best_center_distance {
                best_center_distance = center_distance;
                best_idx = idx;
            }
        }

        best_idx
    }
}

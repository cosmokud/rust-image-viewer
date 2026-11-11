//! Preload queue construction.

use crate::app::*;

impl ImageViewer {
    /// Calculate the number of pages currently visible on screen.
    /// This is used for zoom-aware preloading - at low zoom, many pages are visible.
    pub(crate) fn manga_calculate_visible_page_count(&mut self) -> usize {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return 1;
        }

        if self.manga_use_rtree_backend() {
            return self.manga_collect_visible_indices().len().max(1);
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            if self.masonry_layout_items.is_empty() {
                return 1;
            }

            let viewport_top = self.manga_scroll_offset.max(0.0);
            let viewport_bottom = viewport_top + self.screen_size.y;
            let zoom = self.zoom.max(0.0001);
            let count = self
                .masonry_layout_items
                .iter()
                .filter(|item| {
                    let item_top = item.y * zoom;
                    let item_bottom = item_top + item.height * zoom;
                    item_top < viewport_bottom && item_bottom > viewport_top
                })
                .count();

            return count.max(1);
        }

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y;

        // Find first visible index
        let first_idx = self.manga_index_at_y(viewport_top);

        // Count how many pages fit in the viewport
        let mut count = 0usize;
        let mut y = self.manga_page_start_y(first_idx);

        for idx in first_idx..self.image_list.len() {
            let page_height = self.manga_page_height_cached(idx);
            let page_bottom = y + page_height;

            // Check if page is at least partially visible
            if y < viewport_bottom && page_bottom > viewport_top {
                count += 1;
            }

            // Stop if we've passed the viewport
            if y >= viewport_bottom {
                break;
            }

            y = page_bottom;
        }

        count.max(1) // At least 1 page is always "visible"
    }
}

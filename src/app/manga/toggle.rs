//! The big solo <-> manga mode transition.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Get the scroll offset to show a specific image at the top
    pub(crate) fn manga_get_scroll_offset_for_index(&mut self, target_index: usize) -> f32 {
        if self.mode().is_grid() {
            self.masonry_ensure_layout_cache();
            if let Some(item) = self.masonry_layout_items.get(target_index) {
                return (item.y * self.zoom.max(0.0001)).max(0.0);
            }
            return 0.0;
        }

        // Keep paging aligned with the currently rendered strip geometry.
        self.manga_page_start_y(target_index.min(self.image_list.len().saturating_sub(1)))
    }
}

//! Total height / layout-cache for strip mode.

use crate::app::*;

impl ImageViewer {
    /// Ensure the cached manga layout offsets are available.
    ///
    /// This uses `manga_total_height` as the single cache rebuild point.
    pub(crate) fn manga_ensure_layout_cache(&mut self) {
        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            return;
        }

        let _ = self.manga_total_height();

        // Be defensive: if the cache says it's valid but the vector size is wrong,
        // force a rebuild next call.
        let expected = self.image_list.len().saturating_add(1);
        if self.manga_total_height_cache_valid && self.manga_layout_offsets.len() != expected {
            self.manga_total_height_cache_valid = false;
        }
    }

    /// Find the page index that contains absolute strip coordinate `y`.
    pub(crate) fn manga_index_at_y(&mut self, y: f32) -> usize {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return self
                .current_index
                .min(self.image_list.len().saturating_sub(1));
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            let len = self.image_list.len();
            if self.masonry_layout_items.len() != len {
                return self.current_index.min(len.saturating_sub(1));
            }

            let zoom = self.zoom.max(0.0001);

            let viewport_center_x = self.screen_size.x * 0.5 - self.offset.x;

            let row_candidates: Option<Vec<usize>> = if self.manga_use_rtree_backend() {
                let query_y = y / zoom;
                self.manga_ensure_masonry_spatial_index()
                    .map(|index| index.query_vertical_band(query_y, query_y))
            } else {
                None
            };

            let mut best_row_idx = None;
            let mut best_row_score = f32::MAX;

            if let Some(candidates) = row_candidates {
                for idx in candidates {
                    let Some(item) = self.masonry_layout_items.get(idx) else {
                        continue;
                    };
                    let center_x = (item.x + item.width * 0.5) * zoom;
                    let score = (center_x - viewport_center_x).abs();
                    if score < best_row_score {
                        best_row_score = score;
                        best_row_idx = Some(idx);
                    }
                }
            } else {
                for (idx, item) in self.masonry_layout_items.iter().enumerate() {
                    let top = item.y * zoom;
                    let bottom = top + item.height * zoom;
                    if y >= top && y <= bottom {
                        let center_x = (item.x + item.width * 0.5) * zoom;
                        let score = (center_x - viewport_center_x).abs();
                        if score < best_row_score {
                            best_row_score = score;
                            best_row_idx = Some(idx);
                        }
                    }
                }
            }
            if let Some(idx) = best_row_idx {
                return idx;
            }

            let mut best_idx = self.current_index.min(len.saturating_sub(1));
            let mut best_score = f32::MAX;
            for (idx, item) in self.masonry_layout_items.iter().enumerate() {
                let center_y = (item.y + item.height * 0.5) * zoom;
                let score = (center_y - y).abs();
                if score < best_score {
                    best_score = score;
                    best_idx = idx;
                }
            }
            return best_idx;
        }

        self.manga_ensure_layout_cache();

        let len = self.image_list.len();
        if self.manga_layout_offsets.len() != len + 1 {
            return self.current_index.min(len.saturating_sub(1));
        }

        let total = *self.manga_layout_offsets.last().unwrap_or(&0.0);
        let y = if y.is_finite() {
            y.clamp(0.0, total.max(0.0))
        } else {
            0.0
        };

        // Use only start offsets (len entries). Find insertion point for start <= y.
        let starts = &self.manga_layout_offsets[..len];
        let insertion = starts.partition_point(|&start| start <= y);
        insertion.saturating_sub(1).min(len.saturating_sub(1))
    }
}

//! Masonry layout maths and layout-cache construction.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_item_source_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .and_then(|(w, h)| if w > 0 && h > 0 { Some((w, h)) } else { None })
            .or_else(|| {
                self.masonry_layout_items.get(index).and_then(|item| {
                    (item.source_width > 0 && item.source_height > 0)
                        .then_some((item.source_width, item.source_height))
                })
            })
    }

    pub(crate) fn masonry_slot_aspect_ratio(&self, index: usize) -> f32 {
        self.masonry_slot_source_dimensions(index)
            .map(|(w, h)| (h as f32 / w as f32).clamp(0.2, 5.0))
            .unwrap_or(1.4)
    }

    pub(crate) fn masonry_item_source_size(&self, index: usize) -> Option<egui::Vec2> {
        self.masonry_layout_items
            .get(index)
            .and_then(|item| item.source_size())
            .or_else(|| {
                self.masonry_slot_source_dimensions(index)
                    .map(|(w, h)| egui::vec2(w as f32, h as f32))
            })
    }

    pub(crate) fn fit_size_preserving_aspect(
        source_size: egui::Vec2,
        bounds_size: egui::Vec2,
    ) -> egui::Vec2 {
        if source_size.x <= 0.0
            || source_size.y <= 0.0
            || bounds_size.x <= 0.0
            || bounds_size.y <= 0.0
        {
            return bounds_size;
        }

        let scale = (bounds_size.x / source_size.x).min(bounds_size.y / source_size.y);
        egui::vec2(
            (source_size.x * scale).min(bounds_size.x),
            (source_size.y * scale).min(bounds_size.y),
        )
    }

    pub(crate) fn masonry_item_display_size(
        &self,
        index: usize,
        slot_size: egui::Vec2,
    ) -> egui::Vec2 {
        self.masonry_item_source_size(index)
            .map(|source_size| Self::fit_size_preserving_aspect(source_size, slot_size))
            .unwrap_or(slot_size)
    }

    pub(crate) fn masonry_item_display_rect(
        &self,
        index: usize,
        slot_rect: egui::Rect,
    ) -> egui::Rect {
        let display_size = self.masonry_item_display_size(index, slot_rect.size());
        egui::Rect::from_center_size(slot_rect.center(), display_size)
    }

    pub(crate) fn masonry_ensure_layout_cache(&mut self) {
        if self.image_list.is_empty() {
            self.masonry_layout_items.clear();
            self.masonry_spatial_index = None;
            self.masonry_layout_total_height = 0.0;
            self.masonry_layout_list_signature = 0;
            self.masonry_layout_valid = false;
            return;
        }

        if !self.mode().is_masonry() {
            return;
        }

        let screen_x = self.screen_size.x.round();
        let len = self.image_list.len();
        let items_per_row = self.masonry_items_per_row.clamp(2, 10);

        let needs_recompute = !self.masonry_layout_valid
            || (self.masonry_layout_screen_x - screen_x).abs() > 1e-6
            || self.masonry_layout_items_per_row != items_per_row
            || self.masonry_layout_len != len
            || self.masonry_layout_list_signature != self.image_list_signature;

        if !needs_recompute {
            return;
        }

        let rebuild_started = Instant::now();

        const SIDE_PADDING: f32 = 16.0;
        const TOP_PADDING: f32 = 10.0;
        const BOTTOM_PADDING: f32 = 10.0;
        const GUTTER: f32 = 12.0;

        let available_width = (self.screen_size.x - SIDE_PADDING * 2.0).max(20.0);
        let columns = items_per_row.max(1);
        let total_gutter = GUTTER * (columns.saturating_sub(1) as f32);
        let column_width = ((available_width - total_gutter) / columns as f32).max(1.0);
        let used_width = column_width * columns as f32 + total_gutter;
        let start_x = ((self.screen_size.x - used_width) * 0.5).max(0.0);

        let gallery_mode = self.mode().is_gallery();
        let mut column_heights = vec![TOP_PADDING; columns];
        self.masonry_layout_items.clear();
        self.masonry_layout_items
            .resize(len, MasonryItemLayout::default());

        for idx in 0..len {
            let mut target_col = 0usize;
            let mut min_height = column_heights[0];
            for col in 1..columns {
                if column_heights[col] < min_height {
                    min_height = column_heights[col];
                    target_col = col;
                }
            }

            let x = start_x + target_col as f32 * (column_width + GUTTER);
            let y = column_heights[target_col];

            let (source_width, source_height, height) = if gallery_mode {
                (0, 0, column_width.max(20.0))
            } else {
                let (source_width, source_height) =
                    self.masonry_slot_source_dimensions(idx).unwrap_or((0, 0));
                let aspect_ratio = self.masonry_slot_aspect_ratio(idx);
                (
                    source_width,
                    source_height,
                    (column_width * aspect_ratio).max(20.0),
                )
            };

            self.masonry_layout_items[idx] = MasonryItemLayout {
                x,
                y,
                width: column_width,
                height,
                source_width,
                source_height,
            };

            column_heights[target_col] = y + height + GUTTER;
        }

        let mut content_bottom = TOP_PADDING;
        for h in column_heights {
            if h > content_bottom {
                content_bottom = h;
            }
        }
        if len > 0 {
            content_bottom = (content_bottom - GUTTER).max(TOP_PADDING);
        }

        self.masonry_layout_total_height = (content_bottom + BOTTOM_PADDING).max(0.0);
        self.masonry_layout_screen_x = screen_x;
        self.masonry_layout_items_per_row = items_per_row;
        self.masonry_layout_len = len;
        self.masonry_layout_list_signature = self.image_list_signature;
        self.masonry_layout_valid = true;
        self.masonry_spatial_index = None;
        self.perf_metrics
            .record(Metric::MasonryLayoutRebuildMs, rebuild_started.elapsed());
    }

    pub(crate) fn masonry_item_screen_rect(&self, index: usize) -> Option<egui::Rect> {
        self.masonry_layout_items
            .get(index)
            .copied()
            .map(|item| item.to_screen_rect(self.zoom, self.offset.x, self.manga_scroll_offset))
            .map(|slot_rect| self.masonry_item_display_rect(index, slot_rect))
    }

    pub(crate) fn masonry_should_apply_dimension_updates_now(
        &mut self,
        updated_indices: &[usize],
    ) -> bool {
        if updated_indices.is_empty() || !self.mode().is_masonry() || self.mode().is_solo() {
            return !updated_indices.is_empty();
        }

        if !self.masonry_layout_valid || self.masonry_layout_items.is_empty() {
            return true;
        }

        let visible_indices = self.manga_collect_visible_indices();
        if visible_indices.is_empty() {
            return true;
        }

        let first_visible = *visible_indices.first().unwrap_or(&0);
        let last_visible = *visible_indices.last().unwrap_or(&first_visible);
        let buffer = self.masonry_items_per_row.clamp(2, 10).saturating_mul(4);
        let start = first_visible.saturating_sub(buffer);
        let end = last_visible
            .saturating_add(buffer)
            .min(self.image_list.len().saturating_sub(1));

        updated_indices
            .iter()
            .any(|idx| *idx >= start && *idx <= end)
    }

    pub(crate) fn masonry_queue_dimension_updates<I>(&mut self, updated_indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        if self.mode().is_gallery() {
            self.masonry_pending_dimension_updates.clear();
            self.masonry_layout_invalidation_deferred = false;
            return;
        }

        for idx in updated_indices {
            self.masonry_pending_dimension_updates.insert(idx);
        }

        if !self.masonry_pending_dimension_updates.is_empty() {
            self.masonry_layout_invalidation_deferred = true;
        }
    }

    pub(crate) fn masonry_flush_pending_dimension_updates(&mut self, force: bool) -> bool {
        crate::diag::note_stage("dimension_flush");
        if self.masonry_pending_dimension_updates.is_empty() {
            self.masonry_layout_invalidation_deferred = false;
            return false;
        }

        let pending_indices: Vec<usize> = self
            .masonry_pending_dimension_updates
            .iter()
            .copied()
            .collect();
        let should_apply =
            force || self.masonry_should_apply_dimension_updates_now(&pending_indices);
        if !should_apply {
            self.perf_metrics.count(Metric::MasonryDimCommitDeferred, 1);
            self.masonry_layout_invalidation_deferred = true;
            return false;
        }

        let pending_count = self.masonry_pending_dimension_updates.len() as u64;
        self.masonry_pending_dimension_updates.clear();
        self.masonry_layout_invalidation_deferred = false;
        self.invalidate_manga_layout_cache();
        self.perf_metrics.count(
            if force {
                Metric::MasonryDimCommitIdle
            } else {
                Metric::MasonryDimCommitVisible
            },
            1,
        );
        self.perf_metrics
            .count(Metric::MasonryDimCommitItems, pending_count);

        if force && !self.masonry_metadata_preload_active {
            self.restore_masonry_scroll_after_metadata_preload();
        }

        true
    }

    pub(crate) fn manga_index_at_screen_pos(&mut self, pos: egui::Pos2) -> Option<usize> {
        if self.mode().is_solo() || !self.mode().is_fullscreen() || self.image_list.is_empty() {
            return None;
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();

            if self.manga_use_rtree_backend() {
                let zoom = self.zoom.max(0.0001);
                let layout_x = (pos.x - self.offset.x) / zoom;
                let layout_y = (self.manga_scroll_offset.max(0.0) + pos.y) / zoom;
                let eps = 0.0001;

                let candidates = self
                    .manga_ensure_masonry_spatial_index()
                    .map(|index| {
                        index.query_indices(
                            layout_x - eps,
                            layout_y - eps,
                            layout_x + eps,
                            layout_y + eps,
                        )
                    })
                    .unwrap_or_default();

                for idx in candidates {
                    if let Some(rect) = self.masonry_item_screen_rect(idx) {
                        if rect.contains(pos) {
                            return Some(idx);
                        }
                    }
                }

                return None;
            }

            for (idx, _) in self.image_list.iter().enumerate() {
                if let Some(rect) = self.masonry_item_screen_rect(idx) {
                    if rect.contains(pos) {
                        return Some(idx);
                    }
                }
            }
            return None;
        }

        let idx = self.manga_index_at_y(self.manga_scroll_offset.max(0.0) + pos.y);
        let display_width = self.manga_get_image_display_width(idx);
        let display_height = self.manga_page_height_cached(idx);
        let x = (self.screen_size.x - display_width) * 0.5 + self.offset.x;
        let y = self.manga_page_start_y(idx) - self.manga_scroll_offset;
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::Vec2::new(display_width, display_height),
        );

        if rect.contains(pos) {
            Some(idx)
        } else {
            None
        }
    }
}

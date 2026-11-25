//! Spatial index maintenance and visibility queries.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_ensure_masonry_spatial_index(&mut self) -> Option<&MangaSpatialIndex> {
        if !self.mode().is_masonry() || self.image_list.is_empty() {
            self.masonry_spatial_index = None;
            return None;
        }

        self.masonry_ensure_layout_cache();
        if self.masonry_layout_items.is_empty() {
            self.masonry_spatial_index = None;
            return None;
        }

        let needs_rebuild = self
            .masonry_spatial_index
            .as_ref()
            .map(|index| index.len() != self.masonry_layout_items.len())
            .unwrap_or(true);

        if needs_rebuild {
            let rebuild_started = Instant::now();
            let mut rects = Vec::with_capacity(self.masonry_layout_items.len());
            for (idx, item) in self.masonry_layout_items.iter().enumerate() {
                rects.push(SpatialRect::new(
                    idx,
                    item.x,
                    item.y,
                    item.x + item.width.max(0.0),
                    item.y + item.height.max(0.0),
                ));
            }
            self.masonry_spatial_index = Some(MangaSpatialIndex::from_rects(rects));
            self.perf_metrics
                .record(Metric::MasonrySpatialRebuildMs, rebuild_started.elapsed());
        }

        self.masonry_spatial_index.as_ref()
    }

    pub(crate) fn manga_ensure_strip_spatial_index(&mut self) -> Option<&MangaSpatialIndex> {
        if self.mode().is_masonry() || self.mode().is_solo() || self.image_list.is_empty() {
            self.manga_strip_spatial_index = None;
            return None;
        }

        self.manga_ensure_layout_cache();
        let len = self.image_list.len();
        if self.manga_layout_offsets.len() != len.saturating_add(1) {
            self.manga_strip_spatial_index = None;
            return None;
        }

        let needs_rebuild = self
            .manga_strip_spatial_index
            .as_ref()
            .map(|index| index.len() != len)
            .unwrap_or(true);

        if needs_rebuild {
            let mut rects = Vec::with_capacity(len);
            for idx in 0..len {
                let start_y = self.manga_layout_offsets[idx];
                let end_y = self.manga_layout_offsets[idx + 1].max(start_y + 0.0001);
                rects.push(SpatialRect::new(
                    idx,
                    -STRIP_QUERY_HALF_WIDTH,
                    start_y,
                    STRIP_QUERY_HALF_WIDTH,
                    end_y,
                ));
            }
            self.manga_strip_spatial_index = Some(MangaSpatialIndex::from_rects(rects));
        }

        self.manga_strip_spatial_index.as_ref()
    }

    pub(crate) fn manga_query_visible_indices_rtree_into(
        &mut self,
        viewport_top: f32,
        viewport_bottom: f32,
        out: &mut Vec<usize>,
    ) -> bool {
        if self.mode().is_masonry() {
            let zoom = self.zoom.max(0.0001);
            let min_y = viewport_top / zoom;
            let max_y = viewport_bottom / zoom;
            if let Some(index) = self.manga_ensure_masonry_spatial_index() {
                index.query_vertical_band_into(min_y, max_y, out);
                return true;
            }
            return false;
        }

        if let Some(index) = self.manga_ensure_strip_spatial_index() {
            index.query_vertical_band_into(viewport_top, viewport_bottom, out);
            return true;
        }
        false
    }

    pub(crate) fn manga_collect_visible_indices_linear_into(&mut self, out: &mut Vec<usize>) {
        out.clear();
        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            if self.masonry_layout_items.is_empty() {
                return;
            }

            let viewport_top = self.manga_scroll_offset.max(0.0);
            let viewport_bottom = viewport_top + self.screen_size.y.max(1.0);
            let zoom = self.zoom.max(0.0001);

            for (idx, item) in self.masonry_layout_items.iter().enumerate() {
                let item_top = item.y * zoom;
                let item_bottom = item_top + item.height * zoom;
                if item_top < viewport_bottom && item_bottom > viewport_top {
                    out.push(idx);
                }
            }
            return;
        }

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y.max(1.0);
        let first_idx = self.manga_index_at_y(viewport_top);

        let mut y = self.manga_page_start_y(first_idx);
        for idx in first_idx..self.image_list.len() {
            let page_h = self.manga_page_height_cached(idx).max(1.0);
            let page_bottom = y + page_h;

            if y < viewport_bottom && page_bottom > viewport_top {
                out.push(idx);
            }

            if y > viewport_bottom {
                break;
            }

            y = page_bottom;
        }
    }

    pub(crate) fn manga_collect_visible_indices_into(&mut self, out: &mut Vec<usize>) {
        out.clear();
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let query_started = Instant::now();

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y.max(1.0);

        if self.manga_use_rtree_backend()
            && self.manga_query_visible_indices_rtree_into(viewport_top, viewport_bottom, out)
        {
            self.perf_metrics
                .record(Metric::MangaVisibleQueryMs, query_started.elapsed());
            self.perf_metrics.count(Metric::MangaVisibleQueryRtree, 1);
            return;
        }

        self.manga_collect_visible_indices_linear_into(out);
        self.perf_metrics
            .record(Metric::MangaVisibleQueryMs, query_started.elapsed());
        self.perf_metrics.count(Metric::MangaVisibleQueryLinear, 1);
    }

    pub(crate) fn manga_collect_visible_indices(&mut self) -> Vec<usize> {
        let mut out = Vec::new();
        self.manga_collect_visible_indices_into(&mut out);
        out
    }

    pub(crate) fn manga_target_texture_side_for_preload(
        &mut self,
        anchor_index: usize,
        visible_indices: &[usize],
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);

        if self.mode().is_solo() || self.image_list.is_empty() {
            return max_side;
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            let zoom = self.zoom.max(0.0001);
            let mut visible_max_side = 0.0f32;

            for &idx in visible_indices.iter().take(96) {
                if let Some(item) = self.masonry_layout_items.get(idx) {
                    let slot_size = egui::vec2(item.width * zoom, item.height * zoom);
                    let display_size = self.masonry_item_display_size(idx, slot_size);
                    visible_max_side = visible_max_side.max(display_size.x.max(display_size.y));
                }
            }

            let mut target_side =
                self.masonry_target_texture_side_from_display_side(visible_max_side.max(1.0));
            if self.masonry_navigation_active_for_heavy_work() {
                target_side = target_side.min(self.masonry_navigation_target_side_cap());
            }
            return target_side;
        }

        let mut visible_max_side = 0.0f32;
        for &idx in visible_indices.iter().take(6) {
            let display_w = self.manga_get_image_display_width(idx);
            let display_h = self.manga_get_image_display_height(idx);
            visible_max_side = visible_max_side.max(display_w.max(display_h));
        }

        if visible_max_side <= 0.0 && anchor_index < self.image_list.len() {
            let display_w = self.manga_get_image_display_width(anchor_index);
            let display_h = self.manga_get_image_display_height(anchor_index);
            visible_max_side = display_w.max(display_h);
        }

        self.manga_strip_target_texture_side_from_display_side(visible_max_side.max(1.0))
    }
}

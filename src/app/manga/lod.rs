//! Strip LOD target-side computation.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_lod_upgrade_hysteresis_factor(&self) -> f32 {
        if !self.mode().is_masonry() {
            1.0
        } else if self.masonry_quality_refine_frames_remaining > 0 {
            0.80
        } else {
            let visible_hint = self.masonry_visible_density_hint();
            if visible_hint >= 120 {
                0.98
            } else if visible_hint >= 72 {
                0.94
            } else if visible_hint >= 36 {
                0.90
            } else {
                0.86
            }
        }
    }

    pub(crate) fn masonry_schedule_quality_refine(&mut self) {
        self.masonry_quality_refine_due_at =
            Some(Instant::now() + Duration::from_millis(Self::MASONRY_QUALITY_REFINE_SETTLE_MS));
        self.masonry_quality_refine_frames_remaining = 0;
    }

    pub(crate) fn masonry_begin_quality_refine_if_due(
        &mut self,
        navigation_active_for_quality_work: bool,
    ) -> bool {
        if !self.mode().is_masonry() || self.mode().is_solo() || navigation_active_for_quality_work
        {
            return false;
        }

        let Some(due_at) = self.masonry_quality_refine_due_at else {
            return false;
        };

        if Instant::now() < due_at {
            return false;
        }

        self.masonry_quality_refine_due_at = None;
        self.masonry_quality_refine_frames_remaining = Self::MASONRY_QUALITY_REFINE_MAX_FRAMES;
        self.manga_last_preload_update = Instant::now() - Duration::from_millis(100);
        true
    }

    pub(crate) fn masonry_target_texture_side_from_display_side_with_navigation(
        &self,
        item_screen_max_side: f32,
        navigation_active: bool,
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let zoom = self.zoom.max(0.0001);
        let rows = self.masonry_items_per_row.clamp(2, 10) as f32;

        // Baseline uses viewport width split by rows and is normalized to a 1024px
        // reference at 3440px / 5 rows / 100% zoom before quality boost.
        let baseline_item_width = (self.screen_size.x.max(1.0) / rows)
            * zoom
            * Self::MANGA_MASONRY_ZOOM_QUALITY_BASELINE_SCALE;
        let basis = item_screen_max_side
            .max(baseline_item_width.min(item_screen_max_side.max(1.0) * 1.08))
            .max(48.0);
        let scaled =
            (basis * self.masonry_zoom_quality_boost(zoom) * self.manga_lod_target_scale_factor())
                .ceil() as u32;
        let scaled = if navigation_active {
            scaled.min(self.masonry_navigation_target_side_cap())
        } else {
            scaled.min(self.masonry_fill_target_side_cap())
        };

        scaled.clamp(self.masonry_dynamic_target_min_side(), max_side)
    }

    pub(crate) fn masonry_target_texture_side_from_display_side(
        &self,
        item_screen_max_side: f32,
    ) -> u32 {
        self.masonry_target_texture_side_from_display_side_with_navigation(
            item_screen_max_side,
            self.masonry_navigation_active_for_heavy_work(),
        )
    }

    pub(crate) fn manga_strip_target_texture_side_from_display_side(
        &self,
        item_screen_max_side: f32,
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let scaled = (item_screen_max_side.max(1.0)
            * Self::MANGA_DYNAMIC_TARGET_OVERSCAN
            * self.manga_lod_target_scale_factor())
        .ceil() as u32;

        scaled.clamp(Self::MANGA_DYNAMIC_TARGET_MIN_SIDE.min(max_side), max_side)
    }

    pub(crate) fn masonry_item_current_display_size(&mut self, index: usize) -> Option<egui::Vec2> {
        if !self.mode().is_masonry() {
            return None;
        }

        if let Some(rect) = self.masonry_item_screen_rect(index) {
            return Some(rect.size());
        }

        self.masonry_ensure_layout_cache();
        self.masonry_layout_items.get(index).map(|item| {
            let slot_size = egui::vec2(
                item.width * self.zoom.max(0.0001),
                item.height * self.zoom.max(0.0001),
            );
            self.masonry_item_display_size(index, slot_size)
        })
    }
}

//! Visible-tile retry scheduling and target sides.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_should_heal_placeholder_retry(&self, index: usize) -> bool {
        self.manga_ttv_pending
            .get(&index)
            .is_some_and(|started_at| {
                started_at.elapsed()
                    >= Duration::from_millis(Self::MANGA_PLACEHOLDER_STALL_RETRY_MS)
            })
    }

    pub(crate) fn manga_retry_target_side_for_rect(
        &mut self,
        index: usize,
        image_rect: egui::Rect,
    ) -> u32 {
        let max_side = self.max_texture_side.max(1);
        let target_scale = self.manga_lod_target_scale_factor();

        let target_side = if self.mode().is_masonry() {
            let navigation_active = self.masonry_navigation_active_for_heavy_work();
            self.masonry_target_texture_side_for_index_with_navigation(
                index,
                navigation_active && !self.manga_autoscroll_active,
            )
        } else {
            ((image_rect.width().max(image_rect.height())
                * Self::MANGA_DYNAMIC_TARGET_OVERSCAN
                * target_scale)
                .ceil() as u32)
                .clamp(Self::MANGA_DYNAMIC_TARGET_MIN_SIDE.min(max_side), max_side)
        };

        self.manga_clamp_target_side_to_source(index, target_side)
            .clamp(1, max_side)
    }

    pub(crate) fn manga_texture_upgrade_needed(
        &self,
        existing_width: u32,
        existing_height: u32,
        desired_width: u32,
        desired_height: u32,
        media_type: MangaMediaType,
    ) -> bool {
        let existing_width = existing_width.max(1);
        let existing_height = existing_height.max(1);
        let desired_width = desired_width.max(1);
        let desired_height = desired_height.max(1);

        if desired_width <= existing_width && desired_height <= existing_height {
            return false;
        }

        let existing_side = existing_width.max(existing_height);
        let desired_side = desired_width.max(desired_height);
        let existing_pixels = existing_width as u64 * existing_height as u64;
        let desired_pixels = desired_width as u64 * desired_height as u64;

        if self.mode().is_masonry() {
            if media_type == MangaMediaType::StaticImage {
                return desired_width > existing_width
                    || desired_height > existing_height
                    || desired_pixels > existing_pixels;
            }

            if media_type == MangaMediaType::Video {
                let side_growth = desired_side.saturating_sub(existing_side);
                let meaningful_pixel_growth =
                    desired_pixels > existing_pixels.saturating_mul(11) / 10;
                return desired_pixels > existing_pixels
                    && (desired_width > existing_width || desired_height > existing_height)
                    && (side_growth >= 16 || meaningful_pixel_growth);
            }
        }

        let (ratio, delta) = if self.mode().is_masonry() {
            let dense_view = self.masonry_visible_density_hint() >= 48;
            let moving_media = matches!(
                media_type,
                MangaMediaType::Video | MangaMediaType::AnimatedImage
            );

            if dense_view || moving_media {
                (1.32f32, 128u32)
            } else {
                (
                    Self::MANGA_TEXTURE_UPGRADE_MASONRY_MIN_RATIO,
                    Self::MANGA_TEXTURE_UPGRADE_MASONRY_MIN_DELTA_SIDE,
                )
            }
        } else {
            (
                Self::MANGA_TEXTURE_UPGRADE_MIN_RATIO,
                Self::MANGA_TEXTURE_UPGRADE_MIN_DELTA_SIDE,
            )
        };

        let hysteresis = self.manga_lod_upgrade_hysteresis_factor();
        let ratio = (ratio * hysteresis).clamp(1.05, 2.0);
        let delta = ((delta as f32) * hysteresis).round().max(16.0) as u32;

        let ratio_threshold = (existing_side as f32 * ratio).ceil() as u32;
        let delta_threshold = existing_side.saturating_add(delta);
        desired_pixels > existing_pixels && desired_side >= ratio_threshold.max(delta_threshold)
    }

    pub(crate) fn manga_use_rtree_backend(&self) -> bool {
        let use_rtree = match self.config.manga_virtualization_backend {
            MangaVirtualizationBackend::Linear => false,
            MangaVirtualizationBackend::RTree => true,
            MangaVirtualizationBackend::Auto => {
                self.image_list.len() >= Self::MANGA_VIRTUALIZATION_AUTO_RTREE_MIN_ITEMS
            }
        };

        // Mirror masonry's smooth-navigation behavior: avoid rebuilding strip
        // spatial indices while the user is actively zooming/panning/scrolling.
        if !self.mode().is_masonry() && self.manga_navigation_active_for_heavy_work() {
            return false;
        }

        use_rtree
    }
}

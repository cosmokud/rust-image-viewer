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
            let strip_cap = self.manga_strip_lod_side_cap();
            ((image_rect.width().max(image_rect.height())
                * Self::MANGA_DYNAMIC_TARGET_OVERSCAN
                * target_scale)
                .ceil() as u32)
                .clamp(
                    Self::MANGA_DYNAMIC_TARGET_MIN_SIDE.min(strip_cap),
                    strip_cap,
                )
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
                // Avoid re-decoding 24 MP tiles on trivial sub-pixel/DPI deltas: require >=15%
                // side growth or >=96px delta before triggering a quality upgrade.
                let side_growth = desired_side.saturating_sub(existing_side);
                let meaningful_growth = (desired_side as f32 >= existing_side as f32 * 1.15)
                    || side_growth >= Self::MANGA_TEXTURE_UPGRADE_MASONRY_MIN_DELTA_SIDE;
                return desired_pixels > existing_pixels && meaningful_growth;
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

    /// Upgrades settled visible textures whose mipmaps were suppressed mid-scroll.
    /// Waits out the post-scroll settle debounce before starting re-decodes and is bounded
    /// to at most one deferred-mipmap upgrade in flight or queued per frame.
    /// Returns `(queued_retry_this_frame, any_visible_upgrade_pending)`.
    pub(crate) fn manga_tick_deferred_mipmap_upgrades(
        &mut self,
        visible: &[usize],
    ) -> (bool, bool) {
        if !self.mode().is_manga() || self.mode().is_solo() || self.image_list.is_empty() {
            self.manga_deferred_mipmap_wake_at = None;
            return (false, false);
        }
        if self.manga_navigation_active_for_heavy_work() || self.manga_strip_focused_video_playing()
        {
            self.manga_deferred_mipmap_wake_at = None;
            return (false, false);
        }
        if self.mode().is_masonry()
            && (self.masonry_navigation_was_active
                || self.masonry_quality_refine_due_at.is_some()
                || self.masonry_quality_refine_frames_remaining > 0
                || self.masonry_metadata_preload_active)
        {
            self.manga_deferred_mipmap_wake_at = None;
            return (false, false);
        }

        let settle_remaining = self.manga_settle_debounce_remaining();
        let now = Instant::now();
        let max_side = self.max_texture_side.max(1);
        let mut queued_retry = false;
        let mut in_progress = false;
        let mut still_pending = false;
        let mut debounce_waiting = false;

        for &idx in visible {
            let Some(path) = self.image_list.get(idx).cloned() else {
                continue;
            };
            let Some((tex_w, tex_h, media_type, mipmap_deferred)) = self
                .manga_texture_cache
                .peek_texture_mipmap_info_for_path(idx, path.as_path())
            else {
                continue;
            };
            if !mipmap_deferred {
                continue;
            }

            let video_thumb_allowed = media_type != MangaMediaType::Video
                || gstreamer_runtime_available()
                || cfg!(target_os = "windows");
            let retry_blocked = self.manga_media_is_unloadable(idx)
                || self
                    .manga_loader
                    .as_ref()
                    .is_some_and(|loader| loader.retry_blocked(idx));
            if media_type == MangaMediaType::AnimatedImage || !video_thumb_allowed || retry_blocked
            {
                self.manga_texture_cache
                    .clear_mipmap_deferred_for_path(idx, path.as_path());
                continue;
            }

            let settled_policy =
                self.manga_upload_policy_with_nav(idx, media_type, (tex_w, tex_h), false);
            if !settled_policy.mipmap_enabled((tex_w, tex_h)) {
                self.manga_texture_cache
                    .clear_mipmap_deferred_for_path(idx, path.as_path());
                continue;
            }

            if settle_remaining.is_some() {
                debounce_waiting = true;
                continue;
            }

            if self
                .manga_decoded_mailbox
                .iter()
                .any(|decoded| decoded.index == idx && decoded.path == path)
            {
                in_progress = true;
                still_pending = true;
                continue;
            }

            if self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.is_index_in_flight(idx))
            {
                in_progress = true;
                still_pending = true;
                continue;
            }

            if self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.retry_waiting(idx, now))
            {
                continue;
            }

            if queued_retry || in_progress {
                still_pending = true;
                continue;
            }

            let raw_target_side = if self.mode().is_masonry() {
                let display_size = self
                    .masonry_item_current_display_size(idx)
                    .unwrap_or_else(|| egui::vec2(tex_w.max(1) as f32, tex_h.max(1) as f32));
                self.manga_bucket_side_covering_rect(
                    idx,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, display_size),
                )
            } else {
                let display_size = self.manga_strip_item_current_display_size(idx);
                if media_type == MangaMediaType::Video {
                    self.manga_retry_target_side_for_rect(
                        idx,
                        egui::Rect::from_min_size(egui::Pos2::ZERO, display_size),
                    )
                } else {
                    self.manga_strip_target_texture_side_from_display_side(
                        display_size.x.max(display_size.y),
                    )
                }
            }
            .max(tex_w.max(tex_h));
            let target_texture_side = self
                .manga_clamp_target_side_to_source(idx, raw_target_side)
                .clamp(1, max_side);
            let (downscale_filter, gif_filter) = self.manga_decode_filters_for_strip_mode();
            let force_triangle_filters = self.manga_should_force_triangle_filters();

            let requested = self.manga_loader.as_mut().is_some_and(|loader| {
                loader.retry_visible(&VisibleRetry {
                    image_list: &self.image_list,
                    index: idx,
                    max_texture_side: self.max_texture_side,
                    target_texture_side,
                    downscale_filter,
                    gif_filter,
                    force_triangle_filters,
                    repair: RetryRepair::None,
                })
            });
            if requested {
                self.perf_metrics.count(Metric::MangaRetryEnqueued, 1);
                self.manga_record_target_side_sample(target_texture_side);
                queued_retry = true;
                in_progress = true;
                still_pending = true;
            } else if self.manga_loader.is_some() {
                still_pending = true;
            }
        }

        self.manga_deferred_mipmap_wake_at = match (debounce_waiting, settle_remaining) {
            (true, Some(remaining)) => Some(now + remaining),
            _ => None,
        };

        (queued_retry, still_pending)
    }
}

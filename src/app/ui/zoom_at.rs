//! Zoom-at-pointer for strip and masonry.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Apply pointer-anchored zoom for masonry mode using screen-space cursor position.
    /// Keeps the content point under the cursor stable while zooming.
    pub(crate) fn apply_masonry_zoom_at_screen_pos(
        &mut self,
        new_zoom: f32,
        anchor_screen: egui::Pos2,
    ) -> bool {
        if !self.mode().is_masonry() {
            return false;
        }

        let old_zoom = self.zoom.max(0.0001);
        let new_zoom = self.clamp_zoom(new_zoom);
        if (new_zoom - old_zoom).abs() <= 0.0001 {
            return false;
        }

        // Convert the anchor screen point to masonry content-space coordinates before zoom.
        let content_x = (anchor_screen.x - self.offset.x) / old_zoom;
        let content_y = (anchor_screen.y + self.manga_scroll_offset) / old_zoom;

        self.zoom = new_zoom;
        self.zoom_target = new_zoom;
        self.zoom_velocity = 0.0;
        self.invalidate_manga_layout_cache_for_zoom();

        // Rebuild transforms so the same content-space point maps back to the same screen point.
        self.offset.x = anchor_screen.x - content_x * new_zoom;

        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        let new_scroll = (content_y * new_zoom - anchor_screen.y).clamp(0.0, max_scroll);
        self.manga_scroll_offset = new_scroll;
        self.manga_scroll_target = new_scroll;
        self.manga_scroll_velocity = 0.0;

        true
    }

    pub(crate) fn manga_request_retry_for_visible_item(
        &mut self,
        index: usize,
        display_target_side: u32,
        quality_upgrade: bool,
        media_type: MangaMediaType,
        navigation_active_for_visible_retry: bool,
    ) -> bool {
        if self.mode().is_solo() {
            return false;
        }

        // A file already known to be undrawable never gets another decode request. The mark is
        // cleared only when the file itself changes (see `manga_refresh_unloadable_media`).
        if self.manga_media_is_unloadable(index) {
            return false;
        }

        if self.image_list.is_empty() {
            return false;
        }

        let max_side = self.max_texture_side.max(1);
        let is_masonry = self.mode().is_masonry();
        let prefer_visible_quality_during_autoscroll = is_masonry && self.manga_autoscroll_active;
        let defer_visible_quality =
            navigation_active_for_visible_retry && !prefer_visible_quality_during_autoscroll;
        let min_target_side = if is_masonry {
            Self::MANGA_MASONRY_DYNAMIC_TARGET_MIN_SIDE
        } else {
            Self::MANGA_DYNAMIC_TARGET_MIN_SIDE
        }
        .min(max_side);
        let strip_placeholder_during_playback = !is_masonry
            && self.manga_strip_focused_video_playing()
            && Some(index) != self.manga_focused_video_index;

        if quality_upgrade && defer_visible_quality {
            self.perf_metrics.count(Metric::MangaUpgradeDeferredNav, 1);
            return false;
        }

        if strip_placeholder_during_playback && quality_upgrade {
            self.perf_metrics
                .count(Metric::MangaUpgradeDeferredStripPlayback, 1);
            return false;
        }

        let desired_target_side = self
            .manga_clamp_target_side_to_source(index, display_target_side)
            .clamp(1, max_side);
        let masonry_fill_cap = if is_masonry {
            if defer_visible_quality {
                self.masonry_navigation_target_side_cap()
            } else {
                self.masonry_fill_target_side_cap()
            }
        } else {
            max_side
        };

        let preload_floor_side = if strip_placeholder_during_playback {
            min_target_side
        } else {
            self.manga_target_texture_side.min(max_side)
        };

        // During active masonry navigation, prioritize fast placeholder fill over texture quality.
        let target_texture_side = if media_type == MangaMediaType::StaticImage {
            desired_target_side.clamp(min_target_side, max_side)
        } else if is_masonry && !quality_upgrade {
            let fill_cap = masonry_fill_cap
                .min(self.manga_target_texture_side.max(min_target_side))
                .max(min_target_side);
            let side = desired_target_side.min(fill_cap).max(min_target_side);
            if side < desired_target_side {
                self.perf_metrics.count(
                    if defer_visible_quality {
                        Metric::MangaRetryLowLodNav
                    } else {
                        Metric::MangaRetryLowLodFill
                    },
                    1,
                );
            }
            side
        } else {
            desired_target_side
                .max(preload_floor_side)
                .clamp(min_target_side, max_side)
        };
        let (downscale_filter, gif_filter) = self.manga_decode_filters_for_strip_mode();
        let force_triangle_filters = self.manga_should_force_triangle_filters();
        let current_texture_dims = self
            .image_list
            .get(index)
            .cloned()
            .and_then(|path| {
                self.manga_texture_cache
                    .get_texture_info_for_path(index, path.as_path())
            })
            .map(|(_, tex_w, tex_h)| (tex_w, tex_h))
            .unwrap_or((0, 0));
        let desired_texture_dims = self.manga_bucket_dimensions_for_side(
            index,
            target_texture_side,
            egui::vec2(
                current_texture_dims.0.max(1) as f32,
                current_texture_dims.1.max(1) as f32,
            ),
        );

        let mut requested = self
            .manga_loader
            .as_mut()
            .map(|loader| {
                loader.retry_visible(&VisibleRetry {
                    image_list: &self.image_list,
                    index,
                    max_texture_side: self.max_texture_side,
                    target_texture_side,
                    downscale_filter,
                    gif_filter,
                    force_triangle_filters,
                    repair: RetryRepair::None,
                })
            })
            .unwrap_or(false);

        if !requested
            && quality_upgrade
            && current_texture_dims.0 > 0
            && self.manga_texture_upgrade_needed(
                current_texture_dims.0,
                current_texture_dims.1,
                desired_texture_dims.0,
                desired_texture_dims.1,
                media_type,
            )
        {
            if let Some(loader) = self.manga_loader.as_mut() {
                requested = loader.retry_visible(&VisibleRetry {
                    image_list: &self.image_list,
                    index,
                    max_texture_side: self.max_texture_side,
                    target_texture_side,
                    downscale_filter,
                    gif_filter,
                    force_triangle_filters,
                    repair: RetryRepair::ForceReload,
                });
            }

            if requested {
                self.perf_metrics.count(Metric::MangaRetryForcedRequeue, 1);
            }
        }

        if !requested
            && !quality_upgrade
            && current_texture_dims == (0, 0)
            && self.manga_should_heal_placeholder_retry(index)
        {
            if let Some(loader) = self.manga_loader.as_mut() {
                requested = loader.retry_visible(&VisibleRetry {
                    image_list: &self.image_list,
                    index,
                    max_texture_side: self.max_texture_side,
                    target_texture_side,
                    downscale_filter,
                    gif_filter,
                    force_triangle_filters,
                    repair: RetryRepair::ResetState,
                });
            }

            if requested {
                self.perf_metrics.count(Metric::MangaRetryStallHeal, 1);
            }
        }

        if requested {
            self.perf_metrics.count(Metric::MangaRetryEnqueued, 1);
            self.manga_record_target_side_sample(target_texture_side);
        } else {
            self.perf_metrics.count(Metric::MangaRetryRejected, 1);
        }

        requested
    }
}

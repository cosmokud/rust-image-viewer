//! Visible-index bookkeeping for the manga loader.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_navigation_active_for_heavy_work(&self) -> bool {
        let recent_navigation_window = Duration::from_millis(90);
        let scrollbar_recently_moving = self.manga_scrollbar_dragging
            && self
                .masonry_scrollbar_last_motion_at
                .is_some_and(|started_at| started_at.elapsed() <= recent_navigation_window);
        let autoscroll_recently_moving = self.manga_autoscroll_active
            && self
                .masonry_autoscroll_last_motion_at
                .is_some_and(|started_at| started_at.elapsed() <= recent_navigation_window);

        self.mode().is_manga()
            && (scrollbar_recently_moving
                || autoscroll_recently_moving
                || self.is_panning
                || self.manga_zoom_plus_held
                || self.manga_zoom_minus_held
                || self.manga_wheel_scroll_active
                || (self.manga_scroll_target - self.manga_scroll_offset).abs() > 0.25
                || self.manga_scroll_velocity.abs() > 0.25
                || self.manga_video_seeking
                || self.manga_video_volume_dragging
                || self.gif_seeking)
    }

    pub(crate) fn masonry_navigation_active_for_heavy_work(&self) -> bool {
        self.mode().is_masonry() && self.manga_navigation_active_for_heavy_work()
    }

    pub(crate) fn folder_placeholder_heavy_work_deferred(&self) -> bool {
        self.manga_navigation_active_for_heavy_work()
    }

    pub(crate) fn draw_fps_overlay(&self, ctx: &egui::Context) {
        if !self.config.show_fps {
            return;
        }

        let fps = if self.fps_overlay_smoothed.is_finite() {
            self.fps_overlay_smoothed.max(0.0)
        } else {
            0.0
        };
        let mut text = if fps > 0.0 && self.fps_overlay_last_dt_s > 0.0 {
            let ms = (self.fps_overlay_last_dt_s * 1000.0).max(0.0);
            format!("{fps:.0} FPS  ({ms:.1} ms)")
        } else {
            "0 FPS  (idle)".to_string()
        };

        let decode_text = {
            let caps = detect_video_acceleration_capabilities();
            let (
                prefer_hardware_decode,
                disable_hardware_decode,
                enable_cuda_decode,
                enable_d3d12_decode,
            ) = self.effective_video_decoder_preferences();

            let active_decode = if disable_hardware_decode || !caps.hardware_decode_available {
                "SW"
            } else if enable_d3d12_decode {
                "HW+D3D12"
            } else if enable_cuda_decode {
                "HW+CUDA"
            } else if prefer_hardware_decode {
                "HW"
            } else {
                "AUTO"
            };

            format!(
                " | DEC {} (hw:{} cuda:{})",
                active_decode,
                if caps.hardware_decode_available {
                    "on"
                } else {
                    "off"
                },
                if caps.cuda_available { "on" } else { "off" },
            )
        };
        text.push_str(&decode_text);

        if self.mode().is_manga() {
            if let Some((p50, p95, samples)) = self.manga_ttv_percentiles_ms() {
                text.push_str(&format!(
                    " | TTV p50/p95 {p50:.0}/{p95:.0} ms (n={samples})"
                ));
            }

            if let Some(backlog) = self.manga_loader.as_ref().map(|loader| loader.backlog()) {
                text.push_str(&format!(
                    " | U{} L{}/{} D{}/{} V{}/{}",
                    self.manga_upload_batch_limit,
                    backlog.loads,
                    self.manga_pending_loads_peak,
                    backlog.decoded,
                    self.manga_pending_decoded_peak,
                    self.manga_visible_indices_last,
                    self.manga_visible_indices_peak
                ));
            } else {
                text.push_str(&format!(" | U{}", self.manga_upload_batch_limit));
            }
        }

        let index_stats = self.media_directory_index.stats();
        text.push_str(&format!(
            " | IDX H{} M{}",
            index_stats.hits, index_stats.misses
        ));

        let decoded_hits = self.perf_metrics.counter(Metric::DecodedImageCacheHit);
        let decoded_misses = self.perf_metrics.counter(Metric::DecodedImageCacheMiss);
        if decoded_hits > 0 || decoded_misses > 0 {
            text.push_str(&format!(" | DC H{} M{}", decoded_hits, decoded_misses));
        }

        let metadata_stats = metadata_cache_stats();
        if metadata_stats.dimension_hits > 0
            || metadata_stats.dimension_misses > 0
            || metadata_stats.dimension_expired > 0
            || metadata_stats.dimension_evicted > 0
            || metadata_stats.dimension_write_drops > 0
        {
            text.push_str(&format!(
                " | MC D{}/{} E{} V{} X{}",
                metadata_stats.dimension_hits,
                metadata_stats.dimension_misses,
                metadata_stats.dimension_expired,
                metadata_stats.dimension_evicted,
                metadata_stats.dimension_write_drops,
            ));
        }

        if let Some(p95) = self.perf_metrics.p95(Metric::MediaIndexLookupMs) {
            text.push_str(&format!(" p95:{p95:.2}ms"));
        }

        if self.mode().is_manga() {
            if let Some(p95) = self.perf_metrics.p95(Metric::MangaUploadPassMs) {
                text.push_str(&format!(" | UP p95:{p95:.2}ms"));
            }

            if let Some(p95) = self.perf_metrics.p95(Metric::MangaDecodeQueueWaitMs) {
                text.push_str(&format!(" | QW p95:{p95:.2}ms"));
            }
            if let Some(p95) = self.perf_metrics.p95(Metric::MangaDecodeWorkerMs) {
                text.push_str(&format!(" | DEC p95:{p95:.2}ms"));
            }
            if let Some(p95) = self.perf_metrics.p95(Metric::MangaDecodeResizeMs) {
                text.push_str(&format!(" | RSZ p95:{p95:.2}ms"));
            }
            if let Some(p95) = self.perf_metrics.p95(Metric::MangaUploadTextureMs) {
                text.push_str(&format!(" | UTX p95:{p95:.2}ms"));
            }
            if let Some(p95) = self.perf_metrics.p95(Metric::MasonryLayoutRebuildMs) {
                text.push_str(&format!(" | LY p95:{p95:.2}ms"));
            }
            if let Some(p95) = self.perf_metrics.p95(Metric::MasonrySpatialRebuildMs) {
                text.push_str(&format!(" | SI p95:{p95:.2}ms"));
            }
            if let Some(p95) = self.perf_metrics.p95(Metric::MangaVisibleQueryMs) {
                text.push_str(&format!(" | VQ p95:{p95:.2}ms"));
            }

            let uploaded_total = self.perf_metrics.counter(Metric::MangaUploadedTextures);
            if uploaded_total > 0 {
                text.push_str(&format!(" | UTot {}", uploaded_total));
            }

            let visible_query_rtree = self.perf_metrics.counter(Metric::MangaVisibleQueryRtree);
            let visible_query_linear = self.perf_metrics.counter(Metric::MangaVisibleQueryLinear);
            if visible_query_rtree > 0 || visible_query_linear > 0 {
                text.push_str(&format!(
                    " | VQ R/L {}/{}",
                    visible_query_rtree, visible_query_linear
                ));
            }

            if self.mode().is_masonry() {
                text.push_str(&format!(
                    " | DQ {}",
                    self.masonry_pending_dimension_updates.len()
                ));

                text.push_str(&format!(" | DM {}", self.manga_decoded_mailbox.len()));

                let dim_commit_visible = self.perf_metrics.counter(Metric::MasonryDimCommitVisible);
                let dim_commit_idle = self.perf_metrics.counter(Metric::MasonryDimCommitIdle);
                let dim_commit_deferred =
                    self.perf_metrics.counter(Metric::MasonryDimCommitDeferred);
                if dim_commit_visible > 0 || dim_commit_idle > 0 || dim_commit_deferred > 0 {
                    text.push_str(&format!(
                        " | DQ V/I/D {}/{}/{}",
                        dim_commit_visible, dim_commit_idle, dim_commit_deferred
                    ));
                }

                let decoded_mailbox_drop =
                    self.perf_metrics.counter(Metric::MangaDecodedMailboxDrop);
                if decoded_mailbox_drop > 0 {
                    text.push_str(&format!(" | DMdrop {}", decoded_mailbox_drop));
                }
            }

            let retry_enqueued = self.perf_metrics.counter(Metric::MangaRetryEnqueued);
            let retry_rejected = self.perf_metrics.counter(Metric::MangaRetryRejected);
            if retry_enqueued > 0 || retry_rejected > 0 {
                text.push_str(&format!(" | RR {}/{}", retry_enqueued, retry_rejected));
            }

            let side_low = self.perf_metrics.counter(Metric::MangaTargetSideLow);
            let side_mid = self.perf_metrics.counter(Metric::MangaTargetSideMid);
            let side_high = self.perf_metrics.counter(Metric::MangaTargetSideHigh);
            if side_low > 0 || side_mid > 0 || side_high > 0 {
                text.push_str(&format!(
                    " | TS L/M/H {}/{}/{}",
                    side_low, side_mid, side_high
                ));
            }

            let deferred_nav = self.perf_metrics.counter(Metric::MangaUpgradeDeferredNav);
            let low_lod_nav = self.perf_metrics.counter(Metric::MangaRetryLowLodNav);
            if deferred_nav > 0 || low_lod_nav > 0 {
                text.push_str(&format!(" | NavDQ {} NavLL {}", deferred_nav, low_lod_nav));
            }
        }

        // Keep it below the title/breadcrumb bars when visible.
        let y_offset = if self.show_controls {
            self.top_controls_visible_height() + 8.0
        } else {
            8.0
        };
        egui::Area::new(egui::Id::new("fps_overlay"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, y_offset))
            .show(ctx, |ui| {
                // Use a no-wrap galley + explicit rect sizing to prevent wrapping.
                let font = egui::FontId::proportional(13.0);
                let text_color = egui::Color32::WHITE;
                let galley = ui
                    .painter()
                    .layout_no_wrap(text.clone(), font.clone(), text_color);

                let padding_x = 10.0;
                let padding_y = 6.0;
                let min_w = 170.0; // Keep a stable width even when FPS is short.

                let size = egui::Vec2::new(
                    (galley.rect.width() + padding_x * 2.0).max(min_w),
                    galley.rect.height() + padding_y * 2.0,
                );

                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    6.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    font,
                    text_color,
                );
            });
    }

    pub(crate) fn manga_mark_placeholder_visible(&mut self, index: usize) {
        if self.mode().is_solo() {
            return;
        }
        // A tile nothing will ever load is not "pending work". Keeping it here drove the 16ms
        // background poll (and thus a full-frame repaint) for as long as the corrupt file was
        // on screen, which is where the sustained 100% CPU came from.
        if self.manga_media_is_unloadable(index)
            || self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.retry_blocked(index))
        {
            return;
        }
        self.manga_ttv_pending
            .entry(index)
            .or_insert_with(Instant::now);
    }
}

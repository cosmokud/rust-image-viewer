//! Decoded-frame mailbox used by the manga uploader.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_record_ttv_sample(&mut self, elapsed: Duration) {
        let ms = elapsed.as_secs_f32() * 1000.0;
        if !ms.is_finite() || ms <= 0.0 {
            return;
        }

        if self.manga_ttv_samples_ms.len() >= Self::MANGA_TTV_SAMPLE_CAP {
            self.manga_ttv_samples_ms.pop_front();
        }
        self.manga_ttv_samples_ms.push_back(ms);
    }

    pub(crate) fn manga_prune_ttv_pending(&mut self) {
        self.manga_ttv_pending
            .retain(|_, started_at| started_at.elapsed() <= Self::MANGA_TTV_PENDING_MAX_AGE);
    }

    pub(crate) fn manga_record_target_side_sample(&mut self, side: u32) {
        self.perf_metrics.count(Metric::MangaTargetSideSamples, 1);

        if side <= 192 {
            self.perf_metrics.count(Metric::MangaTargetSideLow, 1);
        } else if side <= 512 {
            self.perf_metrics.count(Metric::MangaTargetSideMid, 1);
        } else {
            self.perf_metrics.count(Metric::MangaTargetSideHigh, 1);
        }
    }

    pub(crate) fn manga_ttv_percentiles_ms(&self) -> Option<(f32, f32, usize)> {
        if self.manga_ttv_samples_ms.is_empty() {
            return None;
        }

        let mut sorted: Vec<f32> = self
            .manga_ttv_samples_ms
            .iter()
            .copied()
            .filter(|v| v.is_finite() && *v > 0.0)
            .collect();
        if sorted.is_empty() {
            return None;
        }

        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let p50_idx = ((n - 1) as f32 * 0.50).round() as usize;
        let p95_idx = ((n - 1) as f32 * 0.95).round() as usize;
        Some((sorted[p50_idx], sorted[p95_idx], n))
    }

    pub(crate) fn current_frame_budget_ms(&self) -> f32 {
        let hz = self
            .fps_display_refresh_hz
            .filter(|hz| hz.is_finite() && *hz >= 24.0 && *hz <= 1000.0)
            .unwrap_or(60.0);
        1000.0 / hz
    }

    pub(crate) fn manga_inline_upload_budget_ms(&self, navigation_active: bool) -> f32 {
        let frame_budget_ms = self.current_frame_budget_ms();
        if navigation_active {
            (frame_budget_ms * 0.25).clamp(0.8, 2.0)
        } else {
            (frame_budget_ms * 0.45).clamp(1.8, 6.0)
        }
    }

    /// Refresh-rate-scaled per-frame RGBA upload byte budget covering deferred GL upload + mipmaps.
    pub(crate) fn manga_upload_byte_budget(&self, navigation_active: bool) -> usize {
        let frame_budget_ms = self.current_frame_budget_ms();
        let mib = 1024.0 * 1024.0;
        if navigation_active {
            (frame_budget_ms * 1.0 * mib)
                .round()
                .clamp(4.0 * mib, 14.0 * mib) as usize
        } else {
            (frame_budget_ms * 1.6 * mib)
                .round()
                .clamp(6.0 * mib, 24.0 * mib) as usize
        }
    }

    /// Backs off GPU texture uploads for one cooldown frame when the previous VSync-paced frame
    /// that uploaded textures exceeded 1.35x the display frame period (covering `SwapBuffers`).
    pub(crate) fn manga_should_backoff_uploads(&self) -> bool {
        if self.manga_uploads_last_pass == 0
            || !self.manga_last_upload_vsync_paced
            || !self.fps_last_dt_s.is_finite()
            || self.fps_last_dt_s <= 0.0
        {
            return false;
        }
        (self.fps_last_dt_s * 1000.0) > self.current_frame_budget_ms() * 1.35
    }

    pub(crate) fn manga_upload_p95_soft_hard_budgets_ms(&self) -> (f32, f32) {
        let frame_budget_ms = self.current_frame_budget_ms();
        let soft = (frame_budget_ms * 0.27).clamp(1.1, Self::MANGA_UPLOAD_P95_SOFT_BUDGET_MS);
        let hard = (frame_budget_ms * 0.45).clamp(1.8, Self::MANGA_UPLOAD_P95_HARD_BUDGET_MS);
        (soft, hard)
    }

    pub(crate) fn manga_compute_upload_batch_limit(
        &self,
        pending_loads: usize,
        pending_decoded: usize,
    ) -> usize {
        if self.manga_strip_focused_video_playing() {
            return 1;
        }

        let mut limit = Self::MANGA_UPLOAD_BATCH_BASE;

        if self.mode().is_masonry() {
            limit += 2;
        }

        // Lower zoom usually means many more items are visible; prioritize fast fill.
        if self.zoom <= 0.75 {
            limit += 2;
        }
        if self.zoom <= 0.50 {
            limit += 2;
        }

        // Increase throughput when decode backlog is building.
        if pending_decoded >= 8 {
            limit += 2;
        }
        if pending_decoded >= 16 {
            limit += 2;
        }
        if pending_loads >= 24 {
            limit += 1;
        }

        // If many visible placeholders are waiting, bias toward lower latency.
        if self.manga_ttv_pending.len() >= 8 {
            limit += 2;
        }

        let frame_budget_ms = self.current_frame_budget_ms();
        let (soft_budget_ms, hard_budget_ms) = self.manga_upload_p95_soft_hard_budgets_ms();

        // Adapt upload budget to measured upload pass latency, scaled to display refresh period.
        if let Some(upload_p95_ms) = self.perf_metrics.p95(Metric::MangaUploadPassMs) {
            if upload_p95_ms >= hard_budget_ms {
                limit = limit.saturating_sub(4);
            } else if upload_p95_ms >= soft_budget_ms {
                limit = limit.saturating_sub(2);
            } else if upload_p95_ms <= soft_budget_ms * 0.35 && pending_decoded >= 6 {
                limit += 1;
            }
        }

        // Guard UI smoothness by reacting to recent frame time relative to display refresh period.
        // `fps_last_dt_s` is updated from active render frames only.
        if self.fps_last_dt_s.is_finite() && self.fps_last_dt_s > 0.0 {
            let frame_ms = self.fps_last_dt_s * 1000.0;
            if frame_ms >= frame_budget_ms * 1.30 {
                limit = limit.saturating_sub(2);
            } else if frame_ms >= frame_budget_ms * 1.08 {
                limit = limit.saturating_sub(1);
            } else if frame_ms <= frame_budget_ms * 0.75 && pending_decoded >= 10 {
                limit += 1;
            }
        }

        // During active scroll/navigation in any vertical-scroll view, prioritize frame-time
        // consistency over fill rate to avoid UI-thread upload bursts.
        if self.manga_navigation_active_for_heavy_work() {
            return if self.mode().is_masonry() {
                Self::MANGA_UPLOAD_BATCH_MIN
            } else {
                1
            };
        }

        limit.clamp(Self::MANGA_UPLOAD_BATCH_MIN, Self::MANGA_UPLOAD_BATCH_MAX)
    }

    pub(crate) fn manga_decoded_mailbox_band(
        index: usize,
        visible_set: &HashSet<usize>,
        anchor_index: usize,
        near_radius: usize,
    ) -> u8 {
        if visible_set.contains(&index) {
            0
        } else if index.abs_diff(anchor_index) <= near_radius {
            1
        } else {
            2
        }
    }

    pub(crate) fn manga_sort_decoded_mailbox_for_upload(
        &mut self,
        visible_set: &HashSet<usize>,
        anchor_index: usize,
        navigation_active: bool,
    ) -> usize {
        let near_radius = if self.mode().is_masonry() {
            self.masonry_items_per_row.clamp(2, 10).saturating_mul(3)
        } else {
            visible_set.len().max(2)
        };
        let texture_cache = &self.manga_texture_cache;

        self.manga_decoded_mailbox
            .make_contiguous()
            .sort_by_key(|decoded| {
                let band = Self::manga_decoded_mailbox_band(
                    decoded.index,
                    visible_set,
                    anchor_index,
                    near_radius,
                );
                // Blank visible tiles must upload ahead of upgrades for tiles already on screen.
                let upgrade_penalty = if navigation_active
                    && texture_cache.contains_for_path(decoded.index, decoded.path.as_path())
                {
                    1u8
                } else {
                    0u8
                };
                let distance = decoded.index.abs_diff(anchor_index);
                let media_penalty =
                    if navigation_active && decoded.media_type == MangaMediaType::Video {
                        1u8
                    } else {
                        0u8
                    };
                let lod_penalty = if navigation_active {
                    decoded.requested_side
                } else {
                    0
                };

                (band, upgrade_penalty, distance, media_penalty, lod_penalty)
            });

        near_radius
    }

    /// One-line file identity for diag forensics: name + on-disk size (one stat, diag paths only).
    pub(crate) fn diag_file_label(path: &std::path::Path) -> String {
        let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        format!(
            "{} ({}B)",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?"),
            bytes
        )
    }

    /// Prefix length of the (band-sorted) mailbox that fits both the item cap and the byte cap.
    /// `entries` must already be sorted by distance band, so the tail is the farthest speculative
    /// entry. Visible entries are never dropped, even when they alone exceed the byte budget.
    pub(crate) fn manga_decoded_mailbox_keep_count(
        entries: &[(usize, usize)],
        visible_set: &HashSet<usize>,
    ) -> usize {
        let mut keep = entries.len().min(Self::MANGA_DECODED_MAILBOX_MAX_ITEMS);
        let mut total_bytes: usize = entries.iter().take(keep).map(|(_, bytes)| bytes).sum();

        while keep > 0 && total_bytes > Self::MANGA_DECODED_MAILBOX_MAX_BYTES {
            let (index, bytes) = entries[keep - 1];
            if visible_set.contains(&index) {
                break;
            }
            total_bytes -= bytes;
            keep -= 1;
        }

        keep
    }

    pub(crate) fn manga_prune_decoded_mailbox(
        &mut self,
        visible_set: &HashSet<usize>,
        anchor_index: usize,
        navigation_active: bool,
    ) -> usize {
        let near_radius = self.manga_sort_decoded_mailbox_for_upload(
            visible_set,
            anchor_index,
            navigation_active,
        );

        let mut budget = std::mem::take(&mut self.manga_mailbox_budget_scratch);
        budget.clear();
        budget.extend(
            self.manga_decoded_mailbox
                .iter()
                .map(|decoded| (decoded.index, decoded.pixel_bytes())),
        );
        let keep = Self::manga_decoded_mailbox_keep_count(&budget, visible_set);
        self.manga_mailbox_budget_scratch = budget;

        if keep >= self.manga_decoded_mailbox.len() {
            return near_radius;
        }

        let dropped_items: Vec<DecodedImage> = self.manga_decoded_mailbox.drain(keep..).collect();
        let dropped_count = dropped_items.len() as u64;
        if let Some(loader) = self.manga_loader.as_mut() {
            loader.forget_indices(dropped_items.iter().map(|decoded| decoded.index));
        }
        crate::manga_loader::drop_decoded_batch_off_thread(dropped_items);

        if dropped_count > 0 {
            self.perf_metrics
                .count(Metric::MangaDecodedMailboxDrop, dropped_count);
        }

        near_radius
    }

    pub(crate) fn masonry_sync_loader_visible_index(&mut self, previous_visible_index: usize) {
        if !self.mode().is_masonry() || self.current_index == previous_visible_index {
            return;
        }

        let jumped_far = self.current_index.abs_diff(previous_visible_index) > 32;
        if let Some(loader) = self.manga_loader.as_mut() {
            loader.sync_visible_index(self.current_index, jumped_far);
        }
    }
}

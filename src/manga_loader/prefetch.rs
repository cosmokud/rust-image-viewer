//! Preload/prefetch window maths.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::first_frame::FirstFrame;
#[allow(unused_imports)]
use crate::image_loader::{
    get_media_type, is_supported_image, is_supported_video, probe_image_dimensions, LoadedImage,
    MediaType,
};
#[allow(unused_imports)]
use crate::image_resize::downscale_rgba_if_needed;
#[allow(unused_imports)]
use crate::metadata_cache::{
    lookup_cached_dimensions, lookup_cached_dimensions_batch, store_cached_dimensions,
    CachedMediaKind,
};
#[allow(unused_imports)]
use crate::video_player::gstreamer_runtime_available;
#[allow(unused_imports)]
use crate::video_thumbnail::{
    probe_video_dimensions_with_gstreamer, probe_video_dimensions_without_gstreamer,
};
#[allow(unused_imports)]
use crate::viewer_mode::MangaLayoutMode;
#[allow(unused_imports)]
use crate::FileStamp;
#[allow(unused_imports)]
use crossbeam_channel::{Receiver, Sender, TrySendError};
#[allow(unused_imports)]
use hashbrown::{HashMap, HashSet};
#[allow(unused_imports)]
use image::imageops::FilterType;
#[allow(unused_imports)]
use lru::LruCache;
#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use rayon::prelude::*;
#[allow(unused_imports)]
use std::num::NonZeroUsize;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

impl MangaLoader {
    /// Preload counts (ahead, behind) from the applied visible-item signal.
    pub(crate) fn calculate_preload_counts(&self) -> (usize, usize) {
        let visible_items = self.visible_page_count.max(1);
        let ahead = visible_items
            .saturating_mul(PRELOAD_LOOK_AHEAD_MULTIPLIER)
            .clamp(MIN_PRELOAD_AHEAD, MAX_PRELOAD_AHEAD);
        let behind = visible_items
            .saturating_mul(PRELOAD_LOOK_BEHIND_MULTIPLIER)
            .clamp(MIN_PRELOAD_BEHIND, MAX_PRELOAD_BEHIND);

        (ahead, behind)
    }

    pub(crate) fn strip_window_counts(visible_item_equivalent: f32) -> (usize, usize) {
        let visible = visible_item_equivalent.max(1.0);
        let behind = (visible * STRIP_PRELOAD_LOOK_BEHIND_MULTIPLIER)
            .ceil()
            .max(1.0) as usize;
        let ahead = (visible * STRIP_PRELOAD_LOOK_AHEAD_MULTIPLIER)
            .ceil()
            .max(1.0) as usize;

        (behind, ahead)
    }

    /// (ahead, behind) preload counts for the current layout signal, direction-agnostic.
    pub(crate) fn effective_preload_counts(&self) -> (usize, usize) {
        match self.strip_visible_item_equivalent {
            Some(visible_item_equivalent) => {
                let (behind, ahead) = Self::strip_window_counts(visible_item_equivalent);
                (ahead, behind)
            }
            None => self.calculate_preload_counts(),
        }
    }

    /// Dimension-probe window around the anchor, biased by scroll direction.
    /// Grid layouts get row-scaled 384/768 caps; strip gets its fractional look window.
    pub(crate) fn probe_window(demand: &Demand<'_>) -> (usize, usize) {
        if demand.layout.is_grid() {
            let rows = demand.masonry_rows.clamp(2, 10);
            let directional_visible = demand.visible_indices.len().max(1);
            let forward = directional_visible
                .saturating_mul(2)
                .max(rows.saturating_mul(8));
            let backward = directional_visible.max(rows.saturating_mul(4));
            let max_behind = 384usize.saturating_mul(rows);
            let max_ahead = 768usize.saturating_mul(rows);

            if demand.moving_up {
                (forward.min(max_behind), backward.min(max_ahead))
            } else {
                (backward.min(max_behind), forward.min(max_ahead))
            }
        } else {
            let visible_item_equivalent = demand
                .strip_visible_item_equivalent
                .unwrap_or(demand.visible_indices.len() as f32);
            let (look_backward, look_forward) = Self::strip_window_counts(visible_item_equivalent);

            if demand.moving_up {
                (look_forward, look_backward)
            } else {
                (look_backward, look_forward)
            }
        }
    }

    /// Entry-count capacity target: visible density × zoom tier, byte-clamped for grid layouts.
    pub(crate) fn cache_capacity_target(
        visible_indices_count: usize,
        demand: &Demand<'_>,
    ) -> usize {
        let visible = demand.visible_page_count.max(visible_indices_count).max(1);
        let mut target = if demand.layout.is_grid() {
            let rows = demand.masonry_rows.clamp(2, 10);
            let zoom_factor = if demand.zoom <= 0.35 {
                4
            } else if demand.zoom <= 0.55 {
                3
            } else if demand.zoom <= 0.80 {
                2
            } else {
                1
            };
            let density_factor = 3 + rows;

            visible
                .saturating_mul(density_factor)
                .saturating_mul(zoom_factor)
                .clamp(MANGA_CACHE_MIN_ENTRIES, MANGA_CACHE_MAX_ENTRIES)
        } else {
            let zoom_factor = if demand.zoom <= 0.45 {
                3
            } else if demand.zoom <= 0.75 {
                2
            } else {
                1
            };

            visible
                .saturating_mul(5)
                .saturating_mul(zoom_factor)
                .clamp(MANGA_CACHE_MIN_ENTRIES, 512)
        };

        if demand.layout.is_grid() {
            // Byte-aware clamp: oversized textures must not inflate the entry-count target.
            // 1.5 x RGBA is the decode+texture peak per entry.
            let est_side = demand.target_texture_side.max(1) as usize;
            let est_bytes_per_texture = est_side
                .saturating_mul(est_side)
                .saturating_mul(4)
                .saturating_mul(3)
                / 2;
            let budget_bytes = if demand.navigation_active {
                MASONRY_CACHE_BUDGET_BYTES_NAV
            } else {
                MASONRY_CACHE_BUDGET_BYTES_IDLE
            };
            let byte_limited_capacity = (budget_bytes / est_bytes_per_texture.max(1))
                .clamp(MANGA_CACHE_MIN_ENTRIES, MANGA_CACHE_MAX_ENTRIES);
            let keep_floor = demand
                .visible_indices
                .len()
                .max(1)
                .max(demand.masonry_rows.clamp(2, 10).saturating_mul(4));
            target = target.min(byte_limited_capacity.max(keep_floor));
        }

        if demand.preserve_recent_cache {
            let retain_floor = demand
                .previous_cache_capacity
                .saturating_mul(7)
                .saturating_div(8)
                .max(demand.visible_indices.len().max(1));
            target = target.max(retain_floor);
        }

        target
    }

    /// Symmetric keep window around the anchor, scaled by the preload bias and the cache budget.
    pub(crate) fn keep_window(
        target_cache_capacity: usize,
        demand: &Demand<'_>,
        counts: (usize, usize),
    ) -> (usize, usize) {
        let (keep_ahead, keep_behind) = counts;

        let (mut final_keep_behind, mut final_keep_ahead) = if demand.layout.is_grid() {
            let rows = demand.masonry_rows.clamp(2, 10);
            let symmetric = keep_ahead
                .max(keep_behind)
                .saturating_mul((rows + 1) / 2)
                .max(12usize.saturating_mul(rows));
            (symmetric, symmetric)
        } else if demand.scrolling_up {
            (keep_ahead, keep_behind)
        } else {
            (keep_behind, keep_ahead)
        };

        let visible_budget = demand
            .visible_indices
            .len()
            .max(1)
            .min(target_cache_capacity.max(1));
        let keep_budget_total = target_cache_capacity.saturating_sub(visible_budget);
        if keep_budget_total == 0 {
            final_keep_behind = 0;
            final_keep_ahead = 0;
        } else {
            let requested_total = final_keep_behind.saturating_add(final_keep_ahead);
            if requested_total > keep_budget_total {
                let mut scaled_behind =
                    final_keep_behind.saturating_mul(keep_budget_total) / requested_total.max(1);
                let mut scaled_ahead = keep_budget_total.saturating_sub(scaled_behind);

                if final_keep_behind > 0 && scaled_behind == 0 {
                    scaled_behind = 1.min(keep_budget_total);
                    scaled_ahead = keep_budget_total.saturating_sub(scaled_behind);
                }
                if final_keep_ahead > 0 && scaled_ahead == 0 && keep_budget_total > 1 {
                    scaled_ahead = 1;
                    scaled_behind = keep_budget_total.saturating_sub(scaled_ahead);
                }

                final_keep_behind = scaled_behind;
                final_keep_ahead = scaled_ahead;
            }
        }

        (final_keep_behind, final_keep_ahead)
    }

    /// Strip-window (behind, ahead) for a fractional viewport coverage signal.
    pub fn strip_window_for_visible_equivalent(visible_item_equivalent: f32) -> (usize, usize) {
        Self::strip_window_counts(visible_item_equivalent)
    }

    /// Jump-prime window (behind, ahead) for a destination far from the current demand.
    pub(crate) fn prime_window(&self) -> (usize, usize) {
        if self.last_layout.is_grid() {
            let rows = self.last_masonry_rows.clamp(2, 10);
            let visible = self.last_visible_indices_count.max(1);
            (
                visible.max(rows.saturating_mul(4)),
                visible.saturating_mul(2).max(rows.saturating_mul(8)),
            )
        } else {
            Self::strip_window_counts(self.strip_visible_item_equivalent.unwrap_or(1.0))
        }
    }

    /// Load a navigation destination before the viewport actually arrives there.
    /// Same windows and priority path as a `demand` at that index, minus the cache plan.
    pub fn prime(
        &mut self,
        image_list: &[PathBuf],
        index: usize,
        max_texture_side: u32,
        target_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        force_triangle_filters: bool,
    ) {
        let index = index.min(image_list.len().saturating_sub(1));
        let (behind, ahead) = self.prime_window();
        let start = index.saturating_sub(behind);
        let end = index.saturating_add(ahead).min(image_list.len());
        self.queue_dimension_range(image_list, start, end);
        let target_texture_side =
            Self::quantize_target_texture_side(target_texture_side, max_texture_side);
        self.rearm_failed_in_window(image_list, start, end);
        self.prefetch(
            image_list,
            index,
            max_texture_side,
            target_texture_side,
            downscale_filter,
            gif_filter,
            force_triangle_filters,
        );
    }

    pub(crate) fn prefetch(
        &mut self,
        image_list: &[PathBuf],
        visible_index: usize,
        max_texture_side: u32,
        target_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        force_triangle_filters: bool,
    ) {
        if image_list.is_empty() {
            return;
        }

        let target_texture_side =
            Self::quantize_target_texture_side(target_texture_side, max_texture_side);
        let (request_downscale_filter, request_gif_filter) = if force_triangle_filters {
            (FilterType::Triangle, FilterType::Triangle)
        } else {
            (downscale_filter, gif_filter)
        };

        // Detect a far jump (e.g., dragging the scrollbar, Home/End, or any other big reposition).
        // On a far jump we cancel older work and make the target page the only "urgent" item.
        let prev_visible_index = self.last_visible_index;
        let index_delta = visible_index.abs_diff(prev_visible_index);
        let is_large_jump = index_delta > LARGE_JUMP_INDEX_THRESHOLD;
        if is_large_jump {
            self.cancel_all();
        }

        // Update scroll direction based on visible index change
        if visible_index > prev_visible_index {
            self.scroll_direction = 1; // Scrolling down
        } else if visible_index < prev_visible_index {
            self.scroll_direction = -1; // Scrolling up
        }
        self.last_visible_index = visible_index;

        // Apply scroll direction bias to the counts set by `demand`
        let (base_ahead, base_behind) = self.effective_preload_counts();

        // Apply scroll direction bias
        let ahead = if self.scroll_direction > 0 {
            base_ahead
        } else {
            base_behind
        };
        let behind = if self.scroll_direction > 0 {
            base_behind
        } else {
            base_ahead
        };

        let start_idx = visible_index.saturating_sub(behind);
        let end_idx = (visible_index + ahead + 1).min(image_list.len());

        // A video that scrolled out of the window gets a fresh probe chance on the way back:
        // both its gate cooldown and (for videos only) the download budget spent on blocked
        // probes are dropped once the index leaves the window.
        clear_probe_cooldowns_outside(start_idx, end_idx);
        let mut reentered: Vec<usize> = Vec::new();
        if !self.probe_blocked.is_empty() {
            let (start, end) = (start_idx, end_idx);
            reentered = self
                .probe_blocked
                .keys()
                .copied()
                .filter(|index| *index < start || *index >= end)
                .collect();
            self.probe_blocked
                .retain(|index, _| *index >= start && *index < end);
        }
        // An image with a spent budget is genuinely broken (corrupt header), so only videos are
        // revived here: their budget was burnt by gate timeouts rather than by the file.
        reentered.retain(|index| {
            image_list
                .get(*index)
                .is_some_and(|path| is_supported_video(path))
        });
        self.rearm_indices(reentered);
        // A video tile inside the window whose budget is spent waits for its revive window; the
        // ceiling has to pace re-attempts here or nothing would ever re-request it again.
        self.revive_spent_video_thumbnails(image_list, start_idx, end_idx);

        // Collect indices that need loading
        let mut requests: Vec<LoadRequest> = Vec::new();
        let generation = self.current_generation;
        let now = Instant::now();

        {
            let loading = self.loading_indices.read();
            let loaded = self.loaded_levels.read();
            let retry = self.retry_state.read();

            for idx in start_idx..end_idx {
                let request_target_texture_side = self.limit_target_texture_side_for_index(
                    idx,
                    target_texture_side,
                    max_texture_side,
                );

                // Check if file is a supported media type (image or video)
                let is_image = is_supported_image(&image_list[idx]);
                let is_video = is_supported_video(&image_list[idx]);
                if !is_image && !is_video {
                    continue;
                }

                // Skip if already loading or loaded at sufficient LOD.
                if loading.get(&idx).copied() == Some(generation)
                    || loaded
                        .get(&idx)
                        .is_some_and(|side| *side >= request_target_texture_side)
                {
                    continue;
                }

                if let Some(retry_state) = retry.get(&idx) {
                    let retry_due = now >= retry_state.next_retry_at;
                    if retry_state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS || !retry_due {
                        continue;
                    }
                }

                // Gate-blocked video: wait out the probe backoff instead of burning attempts.
                if self
                    .probe_blocked
                    .get(&idx)
                    .is_some_and(|state| now < state.next_probe_at)
                {
                    continue;
                }

                // Calculate priority based on distance from visible index
                // and scroll direction
                let distance = (idx as i32 - visible_index as i32).abs();
                let direction_bonus = if self.scroll_direction > 0 {
                    if idx > visible_index {
                        0
                    } else {
                        10
                    }
                } else {
                    if idx < visible_index {
                        0
                    } else {
                        10
                    }
                };

                // On a far jump, mark the actual target page as "urgent" so it can preempt
                // any neighbor prefetch and be decoded first.
                let priority = if is_large_jump && idx == visible_index {
                    -100_000
                } else {
                    distance + direction_bonus
                };

                requests.push(LoadRequest {
                    generation,
                    index: idx,
                    path: image_list[idx].clone(),
                    max_texture_side,
                    target_texture_side: request_target_texture_side,
                    downscale_filter: request_downscale_filter,
                    gif_filter: request_gif_filter,
                    priority,
                    queued_at: Instant::now(),
                });
            }
        }

        // Send requests (sorted by priority). IMPORTANT:
        // Only mark an index as "loading" after the request is successfully enqueued.
        // If the channel is full during fast scrollbar drags, dropping a request while still
        // marking it as loading will permanently wedge that item in the UI.
        //
        // In-flight cap: a window that slides every frame used to accumulate one extra decode per
        // step (`loads:57` in the crash log). Bound the in-flight set to the prefetch window or
        // two decodes per worker, whichever is larger. Priorities sort the visible index (and any
        // urgent demand) first, and those are exempt, so the cap can only cut speculative work.
        let window = end_idx.saturating_sub(start_idx);
        let max_inflight = window.max(rayon::current_num_threads().saturating_mul(2));
        let mut inflight = self.loading_indices.read().len();

        requests.sort_by_key(|r| r.priority);
        for req in requests {
            let idx = req.index;
            let exempt = idx == visible_index || req.priority < 0;
            if inflight >= max_inflight && !exempt {
                continue;
            }
            match self.request_tx.try_send(req) {
                Ok(()) => {
                    self.loading_indices.write().insert(idx, generation);
                    inflight = inflight.saturating_add(1);
                }
                Err(TrySendError::Full(_req)) => {
                    // Backpressure: stop here so already-enqueued high priority work runs first.
                    // Remaining items will be retried next frame.
                    break;
                }
                Err(TrySendError::Disconnected(_req)) => {
                    break;
                }
            }
        }

        self.stats.images_pending = self.loading_indices.read().len();
    }
}

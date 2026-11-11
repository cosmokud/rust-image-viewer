//! Read-only loader queries: backlog, demand accounting, pacing probes and diag counts.

#![allow(unused_imports)]

use super::*;

use crate::first_frame::FirstFrame;
use crate::image_loader::{
    get_media_type, is_supported_image, is_supported_video, probe_image_dimensions, LoadedImage,
    MediaType,
};
use crate::image_resize::downscale_rgba_if_needed;
use crate::metadata_cache::{
    lookup_cached_dimensions, lookup_cached_dimensions_batch, store_cached_dimensions,
    CachedMediaKind,
};
use crate::video_player::gstreamer_runtime_available;
use crate::video_thumbnail::{
    probe_video_dimensions_with_gstreamer, probe_video_dimensions_without_gstreamer,
};
use crate::viewer_mode::MangaLayoutMode;
use crate::FileStamp;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use hashbrown::{HashMap, HashSet};
use image::imageops::FilterType;
use lru::LruCache;
use parking_lot::RwLock;
use rayon::prelude::*;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

impl MangaLoader {
    /// Pending counts for repaint scheduling. Authoritative source for "are we still loading?".
    pub fn backlog(&self) -> Backlog {
        Backlog {
            loads: self.loading_indices.read().len(),
            decoded: self.result_rx.len(),
            dimension_results: self.dim_result_rx.len(),
            dimension_probes: self.dim_pending.len(),
        }
    }

    /// (in-flight decode loads, retry-state entries) for the diag frame line.
    pub fn diag_counts(&self) -> (usize, usize) {
        (
            self.loading_indices.read().len(),
            self.retry_state.read().len(),
        )
    }

    /// True when a visible index still needs a request: not in flight and no loaded level
    /// reaching the target side. `backlog` only counts in-flight work, so a caller that pumps
    /// the preload queue off `backlog` alone stalls once batches are dropped (generation
    /// change, full result channel) and needs pointer input to recover. Level-triggered signal.
    pub fn demand_outstanding(
        &self,
        image_list: &[PathBuf],
        visible_indices: &[usize],
        max_texture_side: u32,
        target_texture_side: u32,
    ) -> bool {
        let loading = self.loading_indices.read();
        let loaded = self.loaded_levels.read();
        let retry = self.retry_state.read();
        let now = Instant::now();

        visible_indices.iter().any(|&idx| {
            let Some(path) = image_list.get(idx) else {
                return false;
            };
            if !is_supported_image(path) && !is_supported_video(path) {
                return false;
            }

            // Same skip gates as `prefetch`, so this is true only while a request is issuable.
            if loading.get(&idx).copied() == Some(self.current_generation) {
                return false;
            }
            let request_target = self.limit_target_texture_side_for_index(
                idx,
                target_texture_side,
                max_texture_side,
            );
            if let Some(side) = loaded.get(&idx) {
                // Unknown source dims can put `request_target` above what the file can reach;
                // any stored level then counts, else the pump would never go idle.
                if *side >= request_target || !self.dimension_cache.contains_key(&idx) {
                    return false;
                }
            }
            // Gate backoff still open: a request would be refused, so this is not issuable
            // demand. The parked loop is woken by `probe_blocked_visible_wake` instead of
            // repainting a full frame every pump for the whole window.
            if self
                .probe_blocked
                .get(&idx)
                .is_some_and(|state| now < state.next_probe_at)
            {
                return false;
            }
            let spent = retry
                .get(&idx)
                .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS);
            if !spent {
                return true;
            }
            // A video thumbnail budget re-opens on a schedule, so its visible tile is outstanding
            // work once that window has elapsed; until then it is parked (its wake-up is
            // scheduled). An image at the ceiling lets the app go idle for good.
            is_supported_video(path) && self.video_revive_due(idx, now)
        })
    }

    /// Cheap "is any tile pacing the gate at all" probe: one map scan, no visible-set work, so the
    /// idle frame path can skip the spatial query entirely.
    pub fn probe_gate_pacing(&self, now: Instant) -> bool {
        self.probe_blocked
            .values()
            .any(|state| now < state.next_probe_at)
    }

    /// Remaining backoff of the soonest-usable visible tile the gate already blocked, `ZERO` when
    /// one is usable right now. `None` when no visible tile is gate-blocked.
    ///
    /// The remaining time is the whole reason this returns a duration: the frame loop parks
    /// between attempts and wakes exactly when the next one becomes issuable, instead of pumping
    /// a full-frame repaint for the whole backoff window.
    pub fn probe_blocked_visible_wake(
        &self,
        visible_indices: &[usize],
        now: Instant,
    ) -> Option<Duration> {
        // Empty map (the common case) costs one load and no scan.
        if self.probe_blocked.is_empty() {
            return None;
        }
        visible_indices
            .iter()
            .filter_map(|idx| self.probe_blocked.get(idx))
            .map(|state| state.next_probe_at.saturating_duration_since(now))
            .min()
    }

    /// True while the probe gate paces a video attempt. Hover is only worth a pointer hit test
    /// then: a spent budget is already re-opened by the preload window on its own schedule, so a
    /// per-frame spatial query would buy nothing for it.
    pub fn video_thumbnail_pacing(&self, now: Instant) -> bool {
        self.probe_blocked
            .values()
            .any(|state| now < state.next_probe_at)
    }

    /// Number of cached dimensions for indices in `[0, total_len)`.
    pub fn cached_dimensions_count(&self, total_len: usize) -> usize {
        if total_len == 0 {
            return 0;
        }

        self.dimension_cache
            .keys()
            .filter(|&&idx| idx < total_len)
            .count()
    }
}

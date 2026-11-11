//! Per-index backoff, probe-gate accounting and index bookkeeping.

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
    /// Drop this index's gate cooldown and probe backoff. Used when the tile comes back into view
    /// or is hovered: a video that scrolled away and returned gets a fresh probed attempt.
    pub fn clear_probe_cooldown_for(&mut self, index: usize) {
        clear_probe_cooldown(index);
        self.probe_blocked.remove(&index);
    }

    /// True while a video index waits out its `ProbeBlockedState` backoff after gate misses.
    /// Paces probe attempts only; the caller still treats the tile as outstanding work.
    pub(crate) fn probe_backoff_active(&self, index: usize, now: Instant) -> bool {
        self.probe_blocked
            .get(&index)
            .is_some_and(|state| now < state.next_probe_at)
    }

    /// Clear the gate-failure history for `index`: a payload actually reached the texture cache.
    ///
    /// Leaving the entry in place kept `demand_outstanding` true for a tile that had already
    /// resolved, so the parked frame loop kept waking every pump interval for the session.
    pub fn note_probe_succeeded(&mut self, index: usize) {
        self.probe_blocked.remove(&index);
        self.video_revive.remove(&index);
    }

    /// Record that the last attempt for `index` could not run its gated probe.
    pub fn note_probe_blocked(&mut self, index: usize) {
        let now = Instant::now();
        self.probe_blocked
            .entry(index)
            .or_insert_with(|| ProbeBlockedState {
                misses: 0,
                next_probe_at: now,
            })
            .note_miss(now);
    }

    /// Drop every memo that makes `index` look settled: retry ceiling, in-flight/loaded levels,
    /// gate cooldown and probe backoff. Used when a dropped payload or a re-entered window means
    /// the tile never actually got a chance to resolve.
    pub fn rearm_index(&mut self, index: usize) {
        self.reset_index_state_for(index);
        self.release_inflight(index);
    }

    /// `rearm_index` for a batch, skipping indices that already have a texture.
    pub fn rearm_indices<I>(&mut self, indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        for index in indices {
            self.rearm_index(index);
        }
    }

    /// Record that `index` cannot be drawn at all: pin it at the retry ceiling (so prefetch
    /// and every visible-retry path skip it) and drop its in-flight/loaded bookkeeping.
    ///
    /// This is the "X card" state. It is cleared only by [`Self::rearm_failed_indices`], i.e.
    /// when the file itself changed, which is what keeps a corrupt GIF off the CPU entirely.
    pub fn mark_unloadable(&mut self, index: usize, path: &Path) {
        let stamp = FileStamp::of_path(path);
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.retry_state.write().insert(
            index,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp,
                stamp_checked_at: Instant::now(),
            },
        );
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Record that `index` produced no usable texture, keeping the bounded retry backoff intact
    /// so a corrupt file cannot spin the pump.
    pub fn register_upload_failure(&mut self, index: usize, path: &Path) {
        Self::register_decode_failure(
            &self.loaded_levels,
            &self.retry_state,
            index,
            FileStamp::of_path(path),
        );
    }

    /// Release a stranded dimension probe so the index can be probed again.
    pub fn drop_dim_pending(&mut self, index: usize) {
        self.dim_pending.remove(&index);
    }

    /// True once `index` exhausted its retry budget; nothing else will re-request it, so the
    /// caller must stop treating it as outstanding work.
    pub fn retry_blocked(&self, index: usize) -> bool {
        self.retry_state
            .read()
            .get(&index)
            .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS)
    }

    /// Highest level the loader believes is loaded for `index` (may be set with no texture).
    pub fn loaded_level_for(&self, index: usize) -> Option<u32> {
        self.loaded_levels.read().get(&index).copied()
    }

    /// True while a request for `index` is in flight for the active generation.
    pub fn is_index_in_flight(&self, index: usize) -> bool {
        self.loading_indices.read().get(&index).copied() == Some(self.current_generation)
    }

    /// Snapshot of in-flight work for the caller's stall tracking:
    /// `(indices decoding, indices awaiting a dimension probe)`.
    pub fn inflight_index_snapshot(&self) -> (Vec<usize>, Vec<usize>) {
        (
            self.loading_indices.read().keys().copied().collect(),
            self.dim_pending.iter().copied().collect(),
        )
    }

    /// Sync jump-tracking to an externally handled visible-index change.
    ///
    /// Masonry scrollbar dragging can jump far enough that the UI already knows it has
    /// repositioned. Cancel stale work once, then stop `prefetch` from treating the same
    /// jump as a fresh one.
    pub fn sync_visible_index(&mut self, visible_index: usize, cancel_pending: bool) {
        let previous_index = self.last_visible_index;

        if visible_index > previous_index {
            self.scroll_direction = 1;
        } else if visible_index < previous_index {
            self.scroll_direction = -1;
        }

        if cancel_pending {
            self.cancel_all();
        }

        self.last_visible_index = visible_index;
    }
}

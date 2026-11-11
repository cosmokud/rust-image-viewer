//! Clear/forget/release/cancel and the dimension queries that go with them.

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
    pub(crate) fn clear_with_dimension_policy(&mut self, preserve_dimensions: bool) {
        // Increment generation to invalidate pending requests
        self.current_generation += 1;
        self.generation
            .store(self.current_generation, Ordering::Release);

        // Clear indices
        self.loading_indices.write().clear();
        self.loaded_levels.write().clear();
        self.retry_state.write().clear();
        self.probe_blocked.clear();
        self.video_revive.clear();

        // Clear dimension cache
        if !preserve_dimensions {
            self.dimension_cache.clear();
        }

        // Clear any queued dimension probes
        self.dim_pending.clear();
        clear_all_probe_cooldowns();

        // Drain dimension result channel
        while self.dim_result_rx.try_recv().is_ok() {}

        // Drain result channel
        while self.result_rx.try_recv().is_ok() {}

        // Note: we can't directly drain the request channel from here because we only own
        // the Sender; cancellation is handled via generation checks in the coordinator.
        self.stats = LoaderStats::default();
    }

    /// Clear all caches and reset state (called when exiting manga mode).
    pub fn clear(&mut self) {
        self.clear_with_dimension_policy(false);
    }

    /// Clear runtime state but keep the current list's cached dimensions warm.
    /// Drop runtime state; keep the cached dimensions warm. Distinct from `cancel_all`:
    /// this drops dimensions, `cancel_all` keeps them and only cancels pending work.
    pub fn clear_preserving_dimensions(&mut self) {
        self.clear_with_dimension_policy(true);
    }

    /// Forget loaded/retry bookkeeping for evicted indices so they can be re-requested.
    /// In-flight requests are untouched; only `reset_index_state` clears those.
    pub fn forget_indices<I>(&mut self, indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        let mut loaded = self.loaded_levels.write();
        let mut retry = self.retry_state.write();
        for index in indices {
            loaded.remove(&index);
            retry.remove(&index);
        }
    }

    /// Release one index's in-flight and loaded bookkeeping but keep its failure history.
    ///
    /// Used for work that has been in flight far too long: the tile must be able to re-request,
    /// but a file that already exhausted its retry budget must stay at the ceiling. Clearing the
    /// retry state here instead would give a corrupt file a fresh budget every stale sweep.
    pub fn release_inflight(&mut self, index: usize) {
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Drop all bookkeeping for one index so a stale/placeholder tile can re-request.
    pub fn reset_index_state_for(&mut self, index: usize) {
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.retry_state.write().remove(&index);
        self.probe_blocked.remove(&index);
        self.clear_probe_cooldown_for(index);
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Cancel all pending work (large jumps like Home/End, fast scrollbar drags, exit).
    /// Stale in-flight results are dropped by generation checks in the coordinator.
    pub fn cancel_all(&mut self) {
        // Increment generation to invalidate in-flight requests
        // The coordinator thread will check this and skip stale requests
        self.current_generation += 1;
        self.generation
            .store(self.current_generation, Ordering::Release);

        // Clear loading indices (they'll be re-requested around the new position)
        self.loading_indices.write().clear();

        // Drain result channel to clear stale decoded images.
        // Any drained decoded index may have been marked as loaded by workers,
        // so clear loaded bookkeeping for those entries.
        let mut drained_indices: Vec<usize> = Vec::new();
        while let Ok(decoded) = self.result_rx.try_recv() {
            drained_indices.push(decoded.index);
        }
        if !drained_indices.is_empty() {
            let mut loaded = self.loaded_levels.write();
            let mut retry = self.retry_state.write();
            for idx in drained_indices {
                loaded.remove(&idx);
                retry.remove(&idx);
            }
        }

        // Clear any queued dimension probes and drain stale results
        self.dim_pending.clear();
        while self.dim_result_rx.try_recv().is_ok() {}

        self.stats.images_pending = 0;
    }

    /// Get cached dimensions for an index (width, height only).
    pub fn get_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.dimension_cache.get(&index).map(|(w, h, _)| (*w, *h))
    }

    /// Get cached media info for an index (width, height, media_type).
    /// Get media type for an index.
    pub fn get_media_type(&self, index: usize) -> Option<MangaMediaType> {
        self.dimension_cache.get(&index).map(|(_, _, mt)| *mt)
    }

    /// Update dimensions for a video (called when actual dimensions are known from playback).
    pub fn update_video_dimensions(&mut self, index: usize, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }

        if let Some(entry) = self.dimension_cache.get_mut(&index) {
            if entry.0 == width && entry.1 == height && entry.2 == MangaMediaType::Video {
                return false;
            }

            // Ignore tiny frame-size jitter from some decoders; it should not reshuffle masonry.
            if entry.2 == MangaMediaType::Video {
                let width_delta = entry.0.abs_diff(width);
                let height_delta = entry.1.abs_diff(height);
                if width_delta <= 2 && height_delta <= 2 {
                    let old_aspect = entry.0 as f32 / entry.1.max(1) as f32;
                    let new_aspect = width as f32 / height as f32;
                    if (old_aspect - new_aspect).abs() <= 0.003 {
                        return false;
                    }
                }
            }

            entry.0 = width;
            entry.1 = height;
            entry.2 = MangaMediaType::Video;
            true
        } else {
            self.dimension_cache
                .insert(index, (width, height, MangaMediaType::Video));
            true
        }
    }
}

impl MangaTextureCache {
    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.pinned_entries.clear();
        self.unpinned_entries.clear();
        self.pinned_indices.clear();
    }
}

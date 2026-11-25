//! `MangaTextureCache`: the uploaded texture LRU used by the manga view.

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

/// LRU-style texture cache for manga mode.
/// Keeps track of usage order for eviction.
pub struct MangaTextureCache {
    /// Non-evictable entries for currently visible indices.
    pub(crate) pinned_entries: HashMap<usize, MangaTextureEntry>,
    /// Evictable entries, ordered by recency.
    pub(crate) unpinned_entries: LruCache<usize, MangaTextureEntry>,
    /// Maximum number of entries to cache
    pub(crate) max_entries: usize,
    /// Indices that should not be evicted while still visible.
    pub(crate) pinned_indices: HashSet<usize>,
    /// Last sorted pinned slice, used to skip redundant set_pinned_indices churn.
    pub(crate) last_pinned_sorted: Vec<usize>,
    /// Sum of RGBA bytes (`width * height * 4`) across all currently cached textures.
    pub(crate) resident_bytes: usize,
    /// Soft GPU memory budget enforced during LRU eviction.
    pub(crate) vram_budget_bytes: usize,
}

#[derive(Clone)]
pub(crate) struct MangaTextureEntry {
    pub(crate) path: PathBuf,
    pub(crate) texture: egui::TextureHandle,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) media_type: MangaMediaType,
    pub(crate) mipmap_deferred: bool,
}

impl MangaTextureCache {
    #[inline]
    pub(crate) fn entry_rgba_bytes(width: u32, height: u32) -> usize {
        (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4)
    }

    pub fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }

    pub(crate) fn total_entries(&self) -> usize {
        self.pinned_entries.len() + self.unpinned_entries.len()
    }

    pub(crate) fn evict_to_capacity_with_policy(
        &mut self,
        allow_normal_eviction: bool,
        safety_ceiling_bytes: usize,
    ) -> Vec<usize> {
        let mut evicted = Vec::new();

        while (allow_normal_eviction
            && (self.total_entries() > self.max_entries
                || self.resident_bytes > self.vram_budget_bytes))
            || self.resident_bytes > safety_ceiling_bytes
        {
            let Some((idx, entry)) = self.unpinned_entries.pop_lru() else {
                // All remaining entries are pinned; cannot evict further.
                break;
            };

            let removed_bytes = Self::entry_rgba_bytes(entry.width, entry.height);
            self.resident_bytes = self.resident_bytes.saturating_sub(removed_bytes);

            if self.pinned_indices.remove(&idx) {
                self.last_pinned_sorted.clear();
            }
            evicted.push(idx);
        }

        evicted
    }

    pub fn set_limits_and_maybe_evict(
        &mut self,
        max_entries: usize,
        vram_budget_bytes: usize,
        defer_non_critical_eviction: bool,
    ) -> Vec<usize> {
        self.max_entries = max_entries.max(1);
        self.vram_budget_bytes = vram_budget_bytes.max(1);
        let capacity = NonZeroUsize::new(self.max_entries).expect("cache capacity is non-zero");
        self.unpinned_entries.resize(capacity);
        let safety_ceiling = safety_ceiling_bytes_for_budget(self.vram_budget_bytes);
        self.evict_to_capacity_with_policy(!defer_non_critical_eviction, safety_ceiling)
    }

    pub fn set_pinned_indices<I>(&mut self, pinned: I)
    where
        I: IntoIterator<Item = usize>,
    {
        let new_pinned: HashSet<usize> = pinned.into_iter().collect();

        let to_unpin: Vec<usize> = self
            .pinned_indices
            .difference(&new_pinned)
            .copied()
            .collect();
        for idx in to_unpin {
            if let Some(entry) = self.pinned_entries.remove(&idx) {
                self.unpinned_entries.put(idx, entry);
            }
        }

        let to_pin: Vec<usize> = new_pinned
            .difference(&self.pinned_indices)
            .copied()
            .collect();
        for idx in to_pin {
            if let Some(entry) = self.unpinned_entries.pop(&idx) {
                self.pinned_entries.insert(idx, entry);
            }
        }

        self.pinned_indices = new_pinned;
        self.last_pinned_sorted.clear();
        self.last_pinned_sorted
            .extend(self.pinned_indices.iter().copied());
        self.last_pinned_sorted.sort_unstable();
    }

    /// Fast slice variant for sorted, deduplicated visible indices: returns immediately
    /// without allocations when the pinned set is unchanged from the previous frame.
    pub fn set_pinned_indices_slice(&mut self, pinned: &[usize]) {
        if pinned == self.last_pinned_sorted.as_slice() {
            return;
        }
        self.set_pinned_indices(pinned.iter().copied());
    }

    /// Increment frame counter (call once per frame).
    pub fn tick(&mut self) {
        // LruCache updates recency on access, so no per-frame bookkeeping is needed.
    }

    pub fn get_texture_info_for_path(
        &mut self,
        index: usize,
        path: &Path,
    ) -> Option<(egui::TextureId, u32, u32)> {
        if let Some(entry) = self.pinned_entries.get(&index) {
            return (entry.path.as_path() == path).then_some((
                entry.texture.id(),
                entry.width,
                entry.height,
            ));
        }

        self.unpinned_entries.get(&index).and_then(|entry| {
            (entry.path.as_path() == path).then_some((
                entry.texture.id(),
                entry.width,
                entry.height,
            ))
        })
    }

    pub fn peek_texture_dimensions_for_path(
        &self,
        index: usize,
        path: &Path,
    ) -> Option<(u32, u32)> {
        self.pinned_entries
            .get(&index)
            .and_then(|entry| (entry.path.as_path() == path).then_some((entry.width, entry.height)))
            .or_else(|| {
                self.unpinned_entries.peek(&index).and_then(|entry| {
                    (entry.path.as_path() == path).then_some((entry.width, entry.height))
                })
            })
    }

    pub fn get_texture_handle_info_for_path(
        &mut self,
        index: usize,
        path: &Path,
    ) -> Option<(egui::TextureHandle, u32, u32, MangaMediaType)> {
        if let Some(entry) = self.pinned_entries.get(&index) {
            return (entry.path.as_path() == path).then_some((
                entry.texture.clone(),
                entry.width,
                entry.height,
                entry.media_type,
            ));
        }

        self.unpinned_entries.get(&index).and_then(|entry| {
            (entry.path.as_path() == path).then_some((
                entry.texture.clone(),
                entry.width,
                entry.height,
                entry.media_type,
            ))
        })
    }

    pub fn contains_for_path(&self, index: usize, path: &Path) -> bool {
        self.pinned_entries
            .get(&index)
            .is_some_and(|entry| entry.path.as_path() == path)
            || self
                .unpinned_entries
                .peek(&index)
                .is_some_and(|entry| entry.path.as_path() == path)
    }

    pub(crate) fn peek_texture_mipmap_info_for_path(
        &self,
        index: usize,
        path: &Path,
    ) -> Option<(u32, u32, MangaMediaType, bool)> {
        self.pinned_entries
            .get(&index)
            .and_then(|entry| {
                (entry.path.as_path() == path).then_some((
                    entry.width,
                    entry.height,
                    entry.media_type,
                    entry.mipmap_deferred,
                ))
            })
            .or_else(|| {
                self.unpinned_entries.peek(&index).and_then(|entry| {
                    (entry.path.as_path() == path).then_some((
                        entry.width,
                        entry.height,
                        entry.media_type,
                        entry.mipmap_deferred,
                    ))
                })
            })
    }

    pub fn is_mipmap_deferred_for_path(&self, index: usize, path: &Path) -> bool {
        self.peek_texture_mipmap_info_for_path(index, path)
            .is_some_and(|(_, _, _, deferred)| deferred)
    }

    pub fn clear_mipmap_deferred_for_path(&mut self, index: usize, path: &Path) -> bool {
        if let Some(entry) = self.pinned_entries.get_mut(&index) {
            if entry.path.as_path() == path && entry.mipmap_deferred {
                entry.mipmap_deferred = false;
                return true;
            }
            return false;
        }

        if let Some(entry) = self.unpinned_entries.peek_mut(&index) {
            if entry.path.as_path() == path && entry.mipmap_deferred {
                entry.mipmap_deferred = false;
                return true;
            }
        }

        false
    }

    /// Insert a texture into the cache with explicit media type.
    /// Returns evicted indices if cache was full.
    pub fn insert_with_type(
        &mut self,
        index: usize,
        path: PathBuf,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
        media_type: MangaMediaType,
    ) -> Vec<usize> {
        self.insert_with_type_with_mipmap_deferred(
            index, path, texture, width, height, media_type, false,
        )
    }

    /// Insert a texture into the cache with explicit media type and whether mipmap generation
    /// was deferred due to active navigation. Returns evicted indices if cache was full.
    pub fn insert_with_type_with_mipmap_deferred(
        &mut self,
        index: usize,
        path: PathBuf,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
        media_type: MangaMediaType,
        mipmap_deferred: bool,
    ) -> Vec<usize> {
        self.insert_with_type_and_eviction_policy(
            index,
            path,
            texture,
            width,
            height,
            media_type,
            mipmap_deferred,
            false,
        )
    }

    /// Insert a texture while optionally deferring non-critical eviction until scroll settles
    /// (unless `resident_bytes` exceeds the 1.25x VRAM safety ceiling).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_with_type_and_eviction_policy(
        &mut self,
        index: usize,
        path: PathBuf,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
        media_type: MangaMediaType,
        mipmap_deferred: bool,
        defer_non_critical_eviction: bool,
    ) -> Vec<usize> {
        let new_bytes = Self::entry_rgba_bytes(width, height);
        let entry = MangaTextureEntry {
            path,
            texture,
            width,
            height,
            media_type,
            mipmap_deferred,
        };

        if let Some(prev) = self.pinned_entries.remove(&index) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(Self::entry_rgba_bytes(prev.width, prev.height));
        }
        if let Some(prev) = self.unpinned_entries.pop(&index) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(Self::entry_rgba_bytes(prev.width, prev.height));
        }

        self.resident_bytes = self.resident_bytes.saturating_add(new_bytes);
        if self.pinned_indices.contains(&index) {
            self.pinned_entries.insert(index, entry);
        } else {
            self.unpinned_entries.put(index, entry);
        }

        let safety_ceiling = if defer_non_critical_eviction {
            safety_ceiling_bytes_for_budget(self.vram_budget_bytes)
        } else {
            self.vram_budget_bytes
        };
        self.evict_to_capacity_with_policy(!defer_non_critical_eviction, safety_ceiling)
    }

    /// Update an existing texture in the cache (for video frame updates).
    /// Does not evict anything, just replaces the existing entry.
    pub fn update_texture(
        &mut self,
        index: usize,
        path: &Path,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) -> bool {
        let new_bytes = Self::entry_rgba_bytes(width, height);
        if let Some(entry) = self.pinned_entries.get_mut(&index) {
            if entry.path.as_path() != path {
                return false;
            }
            let prev_bytes = Self::entry_rgba_bytes(entry.width, entry.height);
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(prev_bytes)
                .saturating_add(new_bytes);
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            entry.mipmap_deferred = false;
            return true;
        }

        if let Some(entry) = self.unpinned_entries.get_mut(&index) {
            if entry.path.as_path() != path {
                return false;
            }
            let prev_bytes = Self::entry_rgba_bytes(entry.width, entry.height);
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(prev_bytes)
                .saturating_add(new_bytes);
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            entry.mipmap_deferred = false;
            return true;
        }

        false
    }

    /// Remove an entry from the cache.
    pub fn remove(&mut self, index: usize) {
        if let Some(prev) = self.pinned_entries.remove(&index) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(Self::entry_rgba_bytes(prev.width, prev.height));
        }
        if let Some(prev) = self.unpinned_entries.pop(&index) {
            self.resident_bytes = self
                .resident_bytes
                .saturating_sub(Self::entry_rgba_bytes(prev.width, prev.height));
        }
        if self.pinned_indices.remove(&index) {
            self.last_pinned_sorted.clear();
        }
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.total_entries() == 0
    }

    /// Get indices of all cached textures into a caller-owned scratch buffer.
    pub fn cached_indices_into(&self, out: &mut Vec<usize>) {
        out.clear();
        out.reserve(self.total_entries());
        out.extend(self.pinned_entries.keys().copied());
        out.extend(self.unpinned_entries.iter().map(|(idx, _)| *idx));
    }

    /// Get `(index, rgba_bytes)` for all cached textures into a caller-owned scratch buffer.
    pub fn cached_entries_bytes_into(&self, out: &mut Vec<(usize, usize)>) {
        out.clear();
        out.reserve(self.total_entries());
        out.extend(
            self.pinned_entries
                .iter()
                .map(|(&idx, entry)| (idx, Self::entry_rgba_bytes(entry.width, entry.height))),
        );
        out.extend(
            self.unpinned_entries
                .iter()
                .map(|(&idx, entry)| (idx, Self::entry_rgba_bytes(entry.width, entry.height))),
        );
    }
}

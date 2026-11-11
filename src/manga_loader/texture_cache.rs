//! `MangaTextureCache`: the uploaded texture LRU used by the manga view.

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
}

#[derive(Clone)]
pub(crate) struct MangaTextureEntry {
    pub(crate) path: PathBuf,
    pub(crate) texture: egui::TextureHandle,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) media_type: MangaMediaType,
}

impl MangaTextureCache {
    pub(crate) fn total_entries(&self) -> usize {
        self.pinned_entries.len() + self.unpinned_entries.len()
    }

    pub(crate) fn evict_to_capacity(&mut self) -> Vec<usize> {
        let mut evicted = Vec::new();

        while self.total_entries() > self.max_entries {
            let Some((idx, _)) = self.unpinned_entries.pop_lru() else {
                // All remaining entries are pinned; cannot evict further.
                break;
            };

            self.pinned_indices.remove(&idx);
            evicted.push(idx);
        }

        evicted
    }

    pub fn set_max_entries(&mut self, max_entries: usize) -> Vec<usize> {
        self.max_entries = max_entries.max(1);
        let capacity = NonZeroUsize::new(self.max_entries).expect("cache capacity is non-zero");
        self.unpinned_entries.resize(capacity);
        self.evict_to_capacity()
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
        let entry = MangaTextureEntry {
            path,
            texture,
            width,
            height,
            media_type,
        };

        if self.pinned_indices.contains(&index) {
            self.unpinned_entries.pop(&index);
            self.pinned_entries.insert(index, entry);
        } else {
            self.pinned_entries.remove(&index);
            self.unpinned_entries.put(index, entry);
        }

        self.evict_to_capacity()
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
        if let Some(entry) = self.pinned_entries.get_mut(&index) {
            if entry.path.as_path() != path {
                return false;
            }
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            return true;
        }

        if let Some(entry) = self.unpinned_entries.get_mut(&index) {
            if entry.path.as_path() != path {
                return false;
            }
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            return true;
        }

        false
    }

    /// Remove an entry from the cache.
    pub fn remove(&mut self, index: usize) {
        self.pinned_entries.remove(&index);
        self.unpinned_entries.pop(&index);
        self.pinned_indices.remove(&index);
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.total_entries() == 0
    }

    /// Get indices of all cached textures.
    pub fn cached_indices(&self) -> Vec<usize> {
        let mut indices = Vec::with_capacity(self.total_entries());
        indices.extend(self.pinned_entries.keys().copied());
        indices.extend(self.unpinned_entries.iter().map(|(idx, _)| *idx));
        indices
    }
}

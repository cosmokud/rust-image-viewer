//! `MangaLoader::demand`: what to load next given what is visible.

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
    /// The only way the viewer says "this is what is on screen".
    ///
    /// Owns the whole derived sequence: LOD side, dimension-probe window, preload queue,
    /// cache capacity target, pinned set and keep-window eviction. The texture cache stays
    /// with the caller (it holds egui handles), so the plan comes back rather than gets applied.
    pub fn demand(&mut self, demand: &Demand<'_>) -> DemandPlan {
        let image_list = demand.image_list;
        let visible_indices_count = demand.visible_indices.len().max(1);
        self.last_layout = demand.layout;
        self.last_visible_indices_count = visible_indices_count;
        self.last_masonry_rows = demand.masonry_rows.clamp(2, 10);
        self.strip_visible_item_equivalent = demand
            .strip_visible_item_equivalent
            .filter(|value| value.is_finite())
            .map(|value| value.max(1.0));

        let target_texture_side =
            Self::quantize_target_texture_side(demand.target_texture_side, demand.max_texture_side);
        let cache_capacity = Self::cache_capacity_target(visible_indices_count, demand);
        self.cache_capacity_last = cache_capacity;
        // Cache-bounded visible count: capacity is derived from the raw count, then feeds back.
        self.visible_page_count = visible_indices_count.min(cache_capacity.max(1)).max(1);

        let (behind, ahead) = Self::probe_window(demand);
        let cache_start = demand.anchor_index.saturating_sub(behind);
        let cache_end = (demand.anchor_index + ahead).min(image_list.len());
        self.queue_dimension_range(image_list, cache_start, cache_end);
        self.rearm_failed_in_window(image_list, cache_start, cache_end);

        self.prefetch(
            image_list,
            demand.anchor_index,
            demand.max_texture_side,
            target_texture_side,
            demand.downscale_filter,
            demand.gif_filter,
            demand.force_triangle_filters,
        );

        let (keep_behind, keep_ahead) =
            Self::keep_window(cache_capacity, demand, self.effective_preload_counts());
        let keep_start = demand.anchor_index.saturating_sub(keep_behind);
        let keep_end = (demand
            .anchor_index
            .saturating_add(keep_ahead)
            .saturating_add(1))
        .min(image_list.len());
        let visible: HashSet<usize> = demand.visible_indices.iter().copied().collect();
        let mut evicted_indices = Vec::new();
        for &idx in demand.cached_indices {
            if !(keep_start..keep_end).contains(&idx) && !visible.contains(&idx) {
                self.forget_indices(std::iter::once(idx));
                evicted_indices.push(idx);
            }
        }

        // Wanted texture-cache state; main.rs owns the cache (egui handles) and applies it.
        DemandPlan {
            cache_capacity,
            pinned_indices: demand.visible_indices.to_vec(),
            evicted_indices,
        }
    }
}

//! `MangaLoader::demand`: what to load next given what is visible.

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
        // Defer synchronous FileStamp::of_path probes while actively scrolling/navigating.
        if !demand.navigation_active {
            self.rearm_failed_in_window(image_list, cache_start, cache_end);
        }

        self.prefetch(
            image_list,
            demand.anchor_index,
            demand.max_texture_side,
            target_texture_side,
            demand.downscale_filter,
            demand.gif_filter,
            demand.force_triangle_filters,
        );

        let vram_budget_bytes = if demand.navigation_active {
            MANGA_CACHE_BUDGET_BYTES_NAV
        } else {
            MANGA_CACHE_BUDGET_BYTES_IDLE
        };
        let safety_ceiling_bytes = safety_ceiling_bytes_for_budget(vram_budget_bytes);
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

        // Defer non-critical eviction (`glDeleteTextures`) while actively scrolling/navigating
        // unless resident GPU memory exceeds the 1.25x safety ceiling.
        let defer_eviction =
            demand.navigation_active && demand.cached_resident_bytes <= safety_ceiling_bytes;

        if !defer_eviction {
            if demand.layout.is_grid() {
                for &idx in demand.cached_indices {
                    if !(keep_start..keep_end).contains(&idx) && !visible.contains(&idx) {
                        evicted_indices.push(idx);
                    }
                }
            } else {
                // Keep Long Strip textures warm across scroll sweeps as long as resident VRAM
                // fits inside the real byte budget (and entry count stays within `cache_capacity`),
                // evicting furthest-first only when over budget.
                let over_vram_budget = demand.cached_resident_bytes > vram_budget_bytes;
                let over_entry_cap = demand.cached_indices.len() > cache_capacity;
                if over_vram_budget || over_entry_cap {
                    let fallback_entry_bytes = (target_texture_side as usize)
                        .saturating_mul(target_texture_side as usize)
                        .saturating_mul(4)
                        .max(1);
                    let entry_bytes_for = |idx: usize| -> usize {
                        demand
                            .cached_entry_bytes
                            .iter()
                            .find(|&&(entry_idx, _)| entry_idx == idx)
                            .map(|&(_, bytes)| bytes)
                            .unwrap_or(fallback_entry_bytes)
                    };
                    let mut candidates: Vec<(usize, usize, usize)> = demand
                        .cached_indices
                        .iter()
                        .copied()
                        .filter(|&idx| {
                            !(keep_start..keep_end).contains(&idx) && !visible.contains(&idx)
                        })
                        .map(|idx| (idx.abs_diff(demand.anchor_index), idx, entry_bytes_for(idx)))
                        .collect();
                    candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

                    let mut remaining_bytes = demand.cached_resident_bytes;
                    let mut remaining_count = demand.cached_indices.len();
                    for (_, idx, bytes) in candidates {
                        if remaining_bytes <= vram_budget_bytes && remaining_count <= cache_capacity
                        {
                            break;
                        }
                        remaining_bytes = remaining_bytes.saturating_sub(bytes);
                        remaining_count = remaining_count.saturating_sub(1);
                        evicted_indices.push(idx);
                    }
                }
            }

            if !evicted_indices.is_empty() {
                self.forget_indices(evicted_indices.iter().copied());
            }
        }

        // Wanted texture-cache state; main.rs owns the cache (egui handles) and applies it.
        DemandPlan {
            cache_capacity,
            vram_budget_bytes,
            pinned_indices: demand.visible_indices.to_vec(),
            evicted_indices,
        }
    }
}

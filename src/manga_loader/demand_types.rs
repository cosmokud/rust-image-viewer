//! The demand description the UI hands in, and the plan that comes back.

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

/// What the viewer currently shows. One `demand` call replaces the hand-sequenced
/// visible-count + dimension-probe + preload + cache-budget sequence that used to live in main.rs.
pub struct Demand<'a> {
    pub image_list: &'a [PathBuf],
    pub layout: MangaLayoutMode,
    /// Index at the viewport top; anchors probe, preload and keep windows.
    pub anchor_index: usize,
    pub visible_indices: &'a [usize],
    /// Whole-page count for the viewport when it differs from `visible_indices` (strip fallback).
    pub visible_page_count: usize,
    /// Long-strip fractional viewport coverage; None for grid layouts.
    pub strip_visible_item_equivalent: Option<f32>,
    pub max_texture_side: u32,
    /// Display-derived side before LOD quantization; quantized inside the loader.
    pub target_texture_side: u32,
    pub downscale_filter: FilterType,
    pub gif_filter: FilterType,
    pub force_triangle_filters: bool,
    pub masonry_rows: usize,
    pub zoom: f32,
    /// Scroll offset moved up this frame (keep window bias).
    pub scrolling_up: bool,
    /// Scrolling up or backward velocity (probe window bias).
    pub moving_up: bool,
    pub navigation_active: bool,
    pub preserve_recent_cache: bool,
    /// Capacity from the previous demand; used by the retain floor.
    pub previous_cache_capacity: usize,
    /// Indices the texture cache currently holds; the keep window evicts from this set.
    pub cached_indices: &'a [usize],
}

/// Loader-side answers for `Demand`. main.rs applies them to its texture cache.
pub struct DemandPlan {
    pub cache_capacity: usize,
    pub pinned_indices: Vec<usize>,
    /// Cache entries outside the keep window; already forgotten by the loader.
    pub evicted_indices: Vec<usize>,
}

/// Work products waiting to be consumed by the UI thread.
pub struct Taken {
    pub decoded: Vec<DecodedImage>,
    pub dimension_updates: Vec<usize>,
}

/// Pending counts main.rs polls for repaint scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backlog {
    pub loads: usize,
    pub decoded: usize,
    pub dimension_results: usize,
    pub dimension_probes: usize,
}

/// Repairs a visible-retry may carry besides a plain re-request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryRepair {
    None,
    /// Quality upgrade: drop loaded bookkeeping but keep in-flight requests.
    ForceReload,
    /// Stall heal: drop in-flight bookkeeping too so the placeholder can re-request.
    ResetState,
}

/// A visible-item repair request; the retry params every caller used to pass by hand.
pub struct VisibleRetry<'a> {
    pub image_list: &'a [PathBuf],
    pub index: usize,
    pub max_texture_side: u32,
    pub target_texture_side: u32,
    pub downscale_filter: FilterType,
    pub gif_filter: FilterType,
    pub force_triangle_filters: bool,
    pub repair: RetryRepair,
}

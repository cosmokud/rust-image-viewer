//! `MangaLoader` and its internal fields, channels and probes.

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

/// High-performance manga image loader with parallel decoding.
pub struct MangaLoader {
    /// Channel to send load requests to worker threads
    pub(crate) request_tx: Sender<LoadRequest>,
    /// Channel for visible-item retries that should bypass normal preload backlog.
    pub(crate) urgent_request_tx: Sender<LoadRequest>,
    /// Channel to receive decoded images from worker threads
    pub(crate) result_rx: Receiver<DecodedImage>,
    /// Indices currently being loaded for the active generation.
    /// Maps index -> generation to prevent stale completions from clearing newer in-flight work.
    pub(crate) loading_indices: Arc<RwLock<HashMap<usize, usize>>>,
    /// Highest loaded texture side per index for the active generation.
    /// Used for LOD-aware skip logic to avoid redundant reload churn.
    pub(crate) loaded_levels: Arc<RwLock<HashMap<usize, u32>>>,
    /// Retry state for indices whose decode failed or produced no usable pixels.
    /// Failed items are retried with backoff, and only aggressively retried when visible.
    pub(crate) retry_state: Arc<RwLock<HashMap<usize, RetryState>>>,
    /// Indices whose last video probe could not run because the gate was busy. Separate from
    /// `retry_state`: a probe miss paces the next attempt and never exhausts the decode budget.
    pub(crate) probe_blocked: HashMap<usize, ProbeBlockedState>,
    /// Videos whose spent thumbnail budget re-opens on a widening schedule. Images never appear
    /// here: their ceiling is final until the file itself changes.
    pub(crate) video_revive: HashMap<usize, VideoReviveState>,
    /// Cached original dimensions and media type (from file headers) for stable layout
    /// Maps index -> (width, height, media_type)
    pub dimension_cache: HashMap<usize, (u32, u32, MangaMediaType)>,

    /// Async dimension-probe request channel (main thread -> worker).
    pub(crate) dim_request_tx: Sender<DimRequest>,
    /// Low-priority async dimension-probe request channel for background warm-up work.
    pub(crate) dim_background_request_tx: Sender<DimRequest>,
    /// Async dimension-probe result channel (worker -> main thread).
    pub(crate) dim_result_rx: Receiver<DimResult>,
    /// Indices currently queued for async dimension probing (main thread only).
    pub(crate) dim_pending: HashSet<usize>,
    /// Flag to signal shutdown to worker threads
    pub(crate) shutdown: Arc<AtomicBool>,
    /// Current scroll direction: positive = scrolling down, negative = scrolling up
    pub(crate) scroll_direction: i32,
    /// Last known visible index for priority calculation
    pub(crate) last_visible_index: usize,
    /// Generation counter to invalidate stale requests on image list change
    pub(crate) generation: Arc<AtomicUsize>,
    /// Current generation for filtering results
    pub(crate) current_generation: usize,
    /// Statistics for debugging
    pub stats: LoaderStats,
    /// Estimated number of visible items on screen (for adaptive preloading)
    pub(crate) visible_page_count: usize,
    /// Long-strip-only viewport coverage equivalent (e.g. 1.5 visible pages).
    pub(crate) strip_visible_item_equivalent: Option<f32>,
    /// Last demand's cache capacity, kept for the retain floor.
    pub(crate) cache_capacity_last: usize,
    /// Layout and raw on-screen count from the last demand; prime windows are derived from them.
    pub(crate) last_layout: MangaLayoutMode,
    pub(crate) last_visible_indices_count: usize,
    pub(crate) last_masonry_rows: usize,
    /// Test seam: the worker-facing halves of the channels, so tests can observe/inject.
    #[cfg(test)]
    pub(crate) probe: Option<LoaderProbe>,
}

#[cfg(test)]
pub(crate) struct LoaderProbe {
    pub(crate) requests: Receiver<LoadRequest>,
    pub(crate) urgent: Receiver<LoadRequest>,
    pub(crate) dim_requests: Receiver<DimRequest>,
    pub(crate) dim_background_requests: Receiver<DimRequest>,
    pub(crate) dim_results: Sender<DimResult>,
}

/// Statistics for monitoring loader performance.
#[derive(Default, Clone)]
pub struct LoaderStats {
    pub images_loaded: usize,
    pub images_pending: usize,
}

/// Channels a loader owns or keeps private to its workers.
pub(crate) struct LoaderChannels {
    pub(crate) request_tx: Sender<LoadRequest>,
    pub(crate) request_rx: Receiver<LoadRequest>,
    pub(crate) urgent_request_tx: Sender<LoadRequest>,
    pub(crate) urgent_request_rx: Receiver<LoadRequest>,
    pub(crate) result_tx: Sender<DecodedImage>,
    pub(crate) result_rx: Receiver<DecodedImage>,
    pub(crate) dim_request_tx: Sender<DimRequest>,
    pub(crate) dim_request_rx: Receiver<DimRequest>,
    pub(crate) dim_background_request_tx: Sender<DimRequest>,
    pub(crate) dim_background_request_rx: Receiver<DimRequest>,
    pub(crate) dim_result_tx: Sender<DimResult>,
    pub(crate) dim_result_rx: Receiver<DimResult>,
}

impl LoaderChannels {
    pub(crate) fn bounded() -> Self {
        // Bounded channels prevent unbounded memory growth under scroll bursts.
        let (request_tx, request_rx) = crossbeam_channel::bounded::<LoadRequest>(256);
        let (urgent_request_tx, urgent_request_rx) =
            crossbeam_channel::bounded::<LoadRequest>(URGENT_REQUEST_QUEUE_CAPACITY);
        let (result_tx, result_rx) =
            crossbeam_channel::bounded::<DecodedImage>(MAX_PENDING_UPLOADS);
        let (dim_request_tx, dim_request_rx) = crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_background_request_tx, dim_background_request_rx) =
            crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_result_tx, dim_result_rx) = crossbeam_channel::bounded::<DimResult>(64);

        Self {
            request_tx,
            request_rx,
            urgent_request_tx,
            urgent_request_rx,
            result_tx,
            result_rx,
            dim_request_tx,
            dim_request_rx,
            dim_background_request_tx,
            dim_background_request_rx,
            dim_result_tx,
            dim_result_rx,
        }
    }
}

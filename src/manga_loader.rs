//! High-performance parallel image loader for Manga Reading Mode.
//!
//! This module implements a sophisticated multi-threaded image loading system
//! optimized for seamless scrolling through hundreds of images. Key features:
//!
//! - **Lock-free communication**: Uses crossbeam channels for zero-contention
//!   message passing between the main thread and worker threads.
//!
//! - **Parallel decoding**: Uses Rayon's thread pool for parallel image decoding,
//!   utilizing all available CPU cores for maximum throughput.
//!
//! - **Priority-based loading**: Images closer to the viewport are loaded first,
//!   with scroll direction awareness for predictive prefetching.
//!
//! - **Batch texture uploads**: Decoded images are batched for GPU upload to
//!   minimize driver overhead and prevent frame drops.
//!
//! - **Memory-efficient caching**: LRU-style eviction keeps memory bounded
//!   while maximizing cache hit rate.

use std::borrow::Cow;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError};
use fast_image_resize as fir;
use hashbrown::{HashMap, HashSet};
use image::imageops::FilterType;
use lru::LruCache;
use parking_lot::RwLock;
use rayon::prelude::*;

use crate::image_loader::{
    get_media_type, is_supported_image, is_supported_video, probe_image_dimensions, LoadedImage,
    MediaType,
};
use crate::metadata_cache::{
    lookup_cached_dimensions, lookup_cached_dimensions_batch, lookup_cached_static_thumbnail,
    lookup_cached_video_thumbnail, store_cached_dimensions, store_cached_static_thumbnail,
    store_cached_video_thumbnail, CachedImageThumbnail, CachedMediaKind, CachedVideoThumbnail,
};
use crate::video_player::gstreamer_runtime_available;
use crate::video_thumbnail::{
    extract_video_first_frame_without_gstreamer, probe_video_dimensions_without_gstreamer,
};

/// Maximum number of decoded images to hold in memory awaiting GPU upload.
/// This bounds memory usage even if the main thread is slow to consume results.
const MAX_PENDING_UPLOADS: usize = 128;

/// Maximum number of images to keep in the texture cache.
/// Beyond this, the oldest entries are evicted to control VRAM usage.
const DEFAULT_CACHED_TEXTURES: usize = 128;

/// Small dedicated queue for visible-item retries that should not wait behind preload churn.
const URGENT_REQUEST_QUEUE_CAPACITY: usize = 128;

/// Directional preload window multipliers derived from the actual number of visible items.
/// The scroll direction gets 2x visible items ahead; the reverse direction gets 1x behind.
const PRELOAD_LOOK_AHEAD_MULTIPLIER: usize = 2;
const PRELOAD_LOOK_BEHIND_MULTIPLIER: usize = 1;

/// Strip mode uses viewport coverage instead of whole-item counts so partial pages do not
/// inflate preload windows. Example: 1.5 visible pages -> 3 ahead, 2 behind.
const STRIP_PRELOAD_LOOK_AHEAD_MULTIPLIER: f32 = PRELOAD_LOOK_AHEAD_MULTIPLIER as f32;
const STRIP_PRELOAD_LOOK_BEHIND_MULTIPLIER: f32 = PRELOAD_LOOK_BEHIND_MULTIPLIER as f32;

/// Clamp the directional preload windows to keep memory usage bounded.
const MIN_PRELOAD_AHEAD: usize = 12;
const MIN_PRELOAD_BEHIND: usize = 6;
const MAX_PRELOAD_AHEAD: usize = 256;
const MAX_PRELOAD_BEHIND: usize = 128;

/// If the visible index jumps by more than this many pages, treat it as a "large jump".
///
/// For large jumps we want latency (load the target page ASAP) over throughput (prefetch neighbors).
const LARGE_JUMP_INDEX_THRESHOLD: usize = 32;

/// Maximum number of dimension probe items to include in a single request.
/// Larger values increase background throughput but can increase burstiness.
const DIM_REQUEST_BATCH_SIZE: usize = 64;

/// Maximum number of dimension results bundled into a single result message.
const DIM_RESULT_CHUNK_SIZE: usize = 64;

/// Base backoff for decode retries after a failed/empty preload.
const PRELOAD_RETRY_BASE_DELAY_MS: u64 = 250;
/// Cap retry backoff so visible items recover quickly once they come into view.
const PRELOAD_RETRY_MAX_DELAY_MS: u64 = 4000;
/// Stop retrying media that repeatedly fails to decode; broken videos/GIFs should not spin forever.
const PRELOAD_RETRY_MAX_ATTEMPTS: u8 = 3;
/// Texture-side buckets used for masonry/strip LOD requests.
/// Requests are rounded up to the next bucket to avoid churn from tiny deltas.
pub const LOD_SIDE_BUCKETS: &[u32] = &[
    96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096,
];

/// Media type for manga items (extended to include videos/animations)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MangaMediaType {
    /// Static image (JPG, PNG, etc.)
    StaticImage,
    /// Animated image (GIF, animated WebP)
    AnimatedImage,
    /// Video file (MP4, MKV, WEBM, etc.)
    Video,
}

/// A decoded image ready for GPU upload.
#[derive(Clone)]
pub struct DecodedImage {
    pub index: usize,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Original dimensions from file header (may differ from texture dims if downscaled)
    pub original_width: u32,
    pub original_height: u32,
    /// Media type of this item
    pub media_type: MangaMediaType,
    /// Requested target side (LOD bucket) for this decode request.
    pub requested_side: u32,
    /// Time spent waiting in the decode queue before a worker started this request.
    pub queue_wait: Duration,
    /// Total worker decode time for this request.
    pub decode_time: Duration,
    /// Time spent in the final resize/downscale step after decode.
    pub resize_time: Duration,
}

/// Request sent to the loader thread pool.
#[derive(Clone)]
pub struct LoadRequest {
    /// Generation for cancellation/coalescing.
    /// Requests from older generations are ignored by the coordinator.
    pub generation: usize,
    pub index: usize,
    pub path: PathBuf,
    pub max_texture_side: u32,
    pub target_texture_side: u32,
    pub downscale_filter: FilterType,
    pub gif_filter: FilterType,
    pub priority: i32, // Lower = higher priority
    pub queued_at: Instant,
}

#[derive(Clone)]
struct DimRequest {
    generation: usize,
    items: Vec<(usize, PathBuf)>,
}

struct DimResult {
    generation: usize,
    items: Vec<(usize, u32, u32, MangaMediaType)>,
}

#[derive(Clone, Copy, Debug)]
struct RetryState {
    attempts: u8,
    next_retry_at: Instant,
}

/// High-performance manga image loader with parallel decoding.
pub struct MangaLoader {
    /// Channel to send load requests to worker threads
    request_tx: Sender<LoadRequest>,
    /// Channel for visible-item retries that should bypass normal preload backlog.
    urgent_request_tx: Sender<LoadRequest>,
    /// Channel to receive decoded images from worker threads
    result_rx: Receiver<DecodedImage>,
    /// Indices currently being loaded for the active generation.
    /// Maps index -> generation to prevent stale completions from clearing newer in-flight work.
    loading_indices: Arc<RwLock<HashMap<usize, usize>>>,
    /// Highest loaded texture side per index for the active generation.
    /// Used for LOD-aware skip logic to avoid redundant reload churn.
    loaded_levels: Arc<RwLock<HashMap<usize, u32>>>,
    /// Retry state for indices whose decode failed or produced no usable pixels.
    /// Failed items are retried with backoff, and only aggressively retried when visible.
    retry_state: Arc<RwLock<HashMap<usize, RetryState>>>,
    /// Cached original dimensions and media type (from file headers) for stable layout
    /// Maps index -> (width, height, media_type)
    pub dimension_cache: HashMap<usize, (u32, u32, MangaMediaType)>,

    /// Async dimension-probe request channel (main thread -> worker).
    dim_request_tx: Sender<DimRequest>,
    /// Low-priority async dimension-probe request channel for background warm-up work.
    dim_background_request_tx: Sender<DimRequest>,
    /// Async dimension-probe result channel (worker -> main thread).
    dim_result_rx: Receiver<DimResult>,
    /// Indices currently queued for async dimension probing (main thread only).
    dim_pending: HashSet<usize>,
    /// Flag to signal shutdown to worker threads
    shutdown: Arc<AtomicBool>,
    /// Current scroll direction: positive = scrolling down, negative = scrolling up
    scroll_direction: i32,
    /// Last known visible index for priority calculation
    last_visible_index: usize,
    /// Generation counter to invalidate stale requests on image list change
    generation: Arc<AtomicUsize>,
    /// Current generation for filtering results
    current_generation: usize,
    /// Statistics for debugging
    pub stats: LoaderStats,
    /// Estimated number of visible items on screen (for adaptive preloading)
    visible_page_count: usize,
    /// Long-strip-only viewport coverage equivalent (e.g. 1.5 visible pages).
    strip_visible_item_equivalent: Option<f32>,
}

/// Statistics for monitoring loader performance.
#[derive(Default, Clone)]
pub struct LoaderStats {
    pub images_loaded: usize,
    pub images_pending: usize,
}

impl MangaLoader {
    /// Create a new manga loader with background thread pool.
    pub fn new() -> Self {
        // Create bounded channels to prevent unbounded memory growth
        let (request_tx, request_rx) = crossbeam_channel::bounded::<LoadRequest>(256);
        let (urgent_request_tx, urgent_request_rx) =
            crossbeam_channel::bounded::<LoadRequest>(URGENT_REQUEST_QUEUE_CAPACITY);
        let (result_tx, result_rx) =
            crossbeam_channel::bounded::<DecodedImage>(MAX_PENDING_UPLOADS);

        let (dim_request_tx, dim_request_rx) = crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_background_request_tx, dim_background_request_rx) =
            crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_result_tx, dim_result_rx) = crossbeam_channel::bounded::<DimResult>(64);

        let loading_indices = Arc::new(RwLock::new(HashMap::new()));
        let loaded_levels = Arc::new(RwLock::new(HashMap::new()));
        let retry_state = Arc::new(RwLock::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicUsize::new(0));

        // Spawn a coordinator thread that processes requests using Rayon
        let loading_clone = Arc::clone(&loading_indices);
        let loaded_clone = Arc::clone(&loaded_levels);
        let retry_clone = Arc::clone(&retry_state);
        let shutdown_clone = Arc::clone(&shutdown);
        let generation_clone = Arc::clone(&generation);

        crate::async_runtime::spawn_blocking_or_thread("manga-loader-coordinator", move || {
            Self::coordinator_loop(
                request_rx,
                urgent_request_rx,
                result_tx,
                loading_clone,
                loaded_clone,
                retry_clone,
                shutdown_clone,
                generation_clone,
            );
        });

        // Spawn a lightweight dimension probe worker.
        // This keeps header-based size probes off the UI thread.
        let shutdown_clone = Arc::clone(&shutdown);
        let generation_clone = Arc::clone(&generation);
        crate::async_runtime::spawn_blocking_or_thread("manga-dimension-worker", move || {
            while !shutdown_clone.load(Ordering::Acquire) {
                let req = loop {
                    if let Ok(request) = dim_request_rx.try_recv() {
                        break Some(request);
                    }

                    if let Ok(request) = dim_background_request_rx.try_recv() {
                        break Some(request);
                    }

                    crossbeam_channel::select! {
                        recv(dim_request_rx) -> request => {
                            if let Ok(request) = request {
                                break Some(request);
                            }
                        }
                        recv(dim_background_request_rx) -> request => {
                            if let Ok(request) = request {
                                break Some(request);
                            }
                        }
                        default(Duration::from_millis(500)) => {
                            if shutdown_clone.load(Ordering::Acquire) {
                                break None;
                            }
                        }
                    }
                };

                let Some(req) = req else {
                    break;
                };

                // Drop stale generation requests immediately so folder switches do not keep
                // the dimension worker busy with obsolete probes.
                if req.generation != generation_clone.load(Ordering::Acquire) {
                    continue;
                }

                let mut out: Vec<(usize, u32, u32, MangaMediaType)> = req
                    .items
                    .into_par_iter()
                    .filter_map(|(idx, path)| {
                        let is_video = is_supported_video(&path);
                        let is_image = is_supported_image(&path);

                        if !is_video && !is_image {
                            return None;
                        }

                        let media_type = if is_video {
                            MangaMediaType::Video
                        } else {
                            MangaMediaType::StaticImage
                        };

                        let dims = if is_video {
                            Self::probe_video_dimensions(&path)
                        } else {
                            Self::probe_image_dimensions_cached(&path)
                        };

                        let (w, h) = if let Some((w, h)) = dims {
                            (w, h)
                        } else if is_video {
                            (1920, 1080)
                        } else {
                            (1200, 1600)
                        };

                        Some((idx, w, h, media_type))
                    })
                    .collect();

                out.par_sort_unstable_by_key(|(idx, _, _, _)| *idx);

                for chunk in out.chunks(DIM_RESULT_CHUNK_SIZE) {
                    // Generation may advance while probing; avoid pushing obsolete results.
                    if req.generation != generation_clone.load(Ordering::Acquire) {
                        break;
                    }

                    if dim_result_tx
                        .send(DimResult {
                            generation: req.generation,
                            items: chunk.to_vec(),
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });

        Self {
            request_tx,
            urgent_request_tx,
            result_rx,
            loading_indices,
            loaded_levels,
            retry_state,
            dimension_cache: HashMap::new(),
            dim_request_tx,
            dim_background_request_tx,
            dim_result_rx,
            dim_pending: HashSet::new(),
            shutdown,
            scroll_direction: 1,
            last_visible_index: 0,
            generation,
            current_generation: 0,
            stats: LoaderStats::default(),
            visible_page_count: 1,
            strip_visible_item_equivalent: None,
        }
    }

    /// Number of indices currently being decoded on background workers.
    ///
    /// This is the authoritative source for "are we still loading?".
    /// `stats.images_pending` is updated opportunistically and may lag.
    pub fn pending_load_count(&self) -> usize {
        self.loading_indices.read().len()
    }

    /// Number of decoded-image messages waiting to be consumed by the UI thread.
    pub fn pending_decoded_count(&self) -> usize {
        self.result_rx.len()
    }

    /// Number of async dimension-probe results waiting to be consumed by the UI thread.
    pub fn pending_dimension_results_count(&self) -> usize {
        self.dim_result_rx.len()
    }

    /// Number of indices currently waiting for dimension probe completion.
    pub fn pending_dimension_probe_count(&self) -> usize {
        self.dim_pending.len()
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

    /// Queue async dimension probes for a range of indices.
    ///
    /// This does not block the UI thread. Results are applied when `poll_dimension_results` is called.
    pub fn request_dimensions_range(&mut self, image_list: &[PathBuf], start: usize, end: usize) {
        self.enqueue_dimension_range(image_list, start, end, self.dim_request_tx.clone());
    }

    /// Queue async background dimension probes for a range of indices.
    ///
    /// This is intended for non-interactive masonry warm-up and should not compete with
    /// immediately visible media.
    pub fn request_dimensions_range_background(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
    ) {
        self.enqueue_dimension_range(
            image_list,
            start,
            end,
            self.dim_background_request_tx.clone(),
        );
    }

    fn enqueue_dimension_range(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
        request_tx: Sender<DimRequest>,
    ) {
        let end = end.min(image_list.len());
        if start >= end {
            return;
        }

        // Build a bounded batch of missing indices.
        let mut items: Vec<(usize, PathBuf)> = Vec::new();
        for idx in start..end {
            if items.len() >= DIM_REQUEST_BATCH_SIZE {
                break;
            }

            if self.dimension_cache.contains_key(&idx) || self.dim_pending.contains(&idx) {
                continue;
            }

            let path = match image_list.get(idx) {
                Some(p) => p.clone(),
                None => continue,
            };

            // Only request for supported media.
            if !is_supported_image(&path) && !is_supported_video(&path) {
                continue;
            }

            items.push((idx, path));
        }

        if items.is_empty() {
            return;
        }

        let indices: Vec<usize> = items.iter().map(|(i, _)| *i).collect();
        match request_tx.try_send(DimRequest {
            generation: self.current_generation,
            items,
        }) {
            Ok(()) => {
                for idx in indices {
                    self.dim_pending.insert(idx);
                }
            }
            Err(TrySendError::Full(_)) => {
                // Backpressure: try again next frame/update.
            }
            Err(TrySendError::Disconnected(_)) => {
                // Worker gone; ignore.
            }
        }
    }

    /// Queue async dimension probes for all uncached indices.
    ///
    /// This is non-blocking and respects channel backpressure.
    /// Returns the number of indices successfully enqueued in this call.
    #[allow(dead_code)]
    pub fn request_all_missing_dimensions(&mut self, image_list: &[PathBuf]) -> usize {
        if image_list.is_empty() {
            return 0;
        }

        let mut enqueued = 0usize;
        let mut batch: Vec<(usize, PathBuf)> = Vec::with_capacity(DIM_REQUEST_BATCH_SIZE);

        for (idx, path) in image_list.iter().enumerate() {
            if self.dimension_cache.contains_key(&idx) || self.dim_pending.contains(&idx) {
                continue;
            }

            if !is_supported_image(path) && !is_supported_video(path) {
                continue;
            }

            batch.push((idx, path.clone()));

            if batch.len() >= DIM_REQUEST_BATCH_SIZE {
                let indices: Vec<usize> = batch.iter().map(|(i, _)| *i).collect();
                let items = std::mem::take(&mut batch);
                match self.dim_background_request_tx.try_send(DimRequest {
                    generation: self.current_generation,
                    items,
                }) {
                    Ok(()) => {
                        for idx in indices {
                            self.dim_pending.insert(idx);
                        }
                        enqueued += DIM_REQUEST_BATCH_SIZE;
                    }
                    Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                        return enqueued;
                    }
                }
            }
        }

        if !batch.is_empty() {
            let indices: Vec<usize> = batch.iter().map(|(i, _)| *i).collect();
            let items = std::mem::take(&mut batch);
            match self.dim_background_request_tx.try_send(DimRequest {
                generation: self.current_generation,
                items,
            }) {
                Ok(()) => {
                    for idx in &indices {
                        self.dim_pending.insert(*idx);
                    }
                    enqueued += indices.len();
                }
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                    return enqueued;
                }
            }
        }

        enqueued
    }

    /// Drain async dimension results and apply them to `dimension_cache`.
    /// Returns indices whose dimensions were updated.
    pub fn poll_dimension_results(&mut self, max_messages: usize) -> Vec<usize> {
        let mut updated: Vec<usize> = Vec::new();

        for _ in 0..max_messages {
            let res = match self.dim_result_rx.try_recv() {
                Ok(r) => r,
                Err(_) => break,
            };

            // Drop stale results (e.g., after cancel/clear).
            if res.generation != self.current_generation {
                continue;
            }

            for (idx, w, h, mt) in res.items {
                let changed = self
                    .dimension_cache
                    .get(&idx)
                    .map_or(true, |(old_w, old_h, old_mt)| {
                        *old_w != w || *old_h != h || *old_mt != mt
                    });

                if changed {
                    self.dimension_cache.insert(idx, (w, h, mt));
                    updated.push(idx);
                }

                self.dim_pending.remove(&idx);
            }
        }

        updated
    }

    /// Coordinator loop that processes requests in parallel using Rayon.
    fn coordinator_loop(
        request_rx: Receiver<LoadRequest>,
        urgent_request_rx: Receiver<LoadRequest>,
        result_tx: Sender<DecodedImage>,
        loading_indices: Arc<RwLock<HashMap<usize, usize>>>,
        loaded_levels: Arc<RwLock<HashMap<usize, u32>>>,
        retry_state: Arc<RwLock<HashMap<usize, RetryState>>>,
        shutdown: Arc<AtomicBool>,
        generation: Arc<AtomicUsize>,
    ) {
        // Collect requests in batches for parallel processing
        let mut batch: Vec<LoadRequest> = Vec::with_capacity(16);

        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Collect available requests (non-blocking after first)
            batch.clear();

            // Prefer urgent visible retries so sharpening upgrades do not wait behind preload work.
            crossbeam_channel::select! {
                recv(urgent_request_rx) -> req => {
                    if let Ok(req) = req {
                        batch.push(req);
                    } else {
                        continue;
                    }
                }
                recv(request_rx) -> req => {
                    if let Ok(req) = req {
                        batch.push(req);
                    } else {
                        continue;
                    }
                }
                default(std::time::Duration::from_millis(500)) => continue,
            }

            while batch.len() < 32 {
                match urgent_request_rx.try_recv() {
                    Ok(req) => batch.push(req),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                }
            }

            // Drain any additional pending requests (non-blocking)
            while batch.len() < 32 {
                match request_rx.try_recv() {
                    Ok(req) => batch.push(req),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Sort by priority (lower = higher priority = process first)
            batch.sort_by_key(|r| r.priority);

            // Get current generation for filtering stale requests
            let current_gen = generation.load(Ordering::Acquire);

            // If the generation changes while we're decoding (e.g., a fast scrollbar jump),
            // we prefer to drop the whole batch's outputs rather than clog the result channel
            // with stale work.
            let generation_changed = || generation.load(Ordering::Acquire) != current_gen;

            enum DecodeOutcome {
                Skipped,
                Failed,
                Decoded(DecodedImage),
            }

            let process_one = |req: &LoadRequest| -> (usize, usize, DecodeOutcome) {
                let req_generation = req.generation;

                // Skip if already loaded or if we've been shut down
                if shutdown.load(Ordering::Relaxed) {
                    return (req.index, req_generation, DecodeOutcome::Skipped);
                }

                // Skip stale requests (e.g., after fast scrollbar jumps / cancel).
                if req_generation != current_gen {
                    return (req.index, req_generation, DecodeOutcome::Skipped);
                }

                // Check if already in loaded set
                {
                    let loaded = loaded_levels.read();
                    if loaded
                        .get(&req.index)
                        .is_some_and(|side| *side >= req.target_texture_side)
                    {
                        return (req.index, req_generation, DecodeOutcome::Skipped);
                    }
                }

                // Load the image
                let queue_wait = req.queued_at.elapsed();
                let decode_started = Instant::now();
                let decoded = Self::load_single_image(req);
                let decode_time = decode_started.elapsed();

                let outcome = match decoded {
                    Some(mut decoded) => {
                        decoded.queue_wait = queue_wait;
                        decoded.decode_time = decode_time;
                        DecodeOutcome::Decoded(decoded)
                    }
                    None => DecodeOutcome::Failed,
                };

                (req.index, req_generation, outcome)
            };

            let publish_one = |idx: usize, req_generation: usize, outcome: DecodeOutcome| {
                // Request has finished one way or another; allow it to be re-requested if needed.
                Self::clear_loading_if_generation(&loading_indices, idx, req_generation);

                if generation_changed() {
                    return true;
                }

                match outcome {
                    DecodeOutcome::Skipped => {}
                    DecodeOutcome::Failed => {
                        Self::register_decode_failure(&loaded_levels, &retry_state, idx);
                    }
                    DecodeOutcome::Decoded(decoded) => {
                        // A decoded payload with empty pixels/dimensions is still useful for metadata
                        // (e.g., video dimension fallback), but does not satisfy texture preloading.
                        let has_usable_pixels =
                            !decoded.pixels.is_empty() && decoded.width > 0 && decoded.height > 0;

                        let loaded_side = decoded.width.max(decoded.height);

                        match result_tx.try_send(decoded) {
                            Ok(_) => {
                                if has_usable_pixels {
                                    let mut loaded = loaded_levels.write();
                                    loaded
                                        .entry(idx)
                                        .and_modify(|side| *side = (*side).max(loaded_side))
                                        .or_insert(loaded_side);
                                    retry_state.write().remove(&idx);
                                } else {
                                    Self::register_decode_failure(
                                        &loaded_levels,
                                        &retry_state,
                                        idx,
                                    );
                                }
                            }
                            Err(TrySendError::Full(_decoded)) => {
                                // Channel full: drop decoded result.
                                // We intentionally do NOT mark as loaded so the main thread can re-request.
                                loaded_levels.write().remove(&idx);
                            }
                            Err(TrySendError::Disconnected(_decoded)) => {
                                return false; // Main thread gone, exit
                            }
                        }
                    }
                }

                true
            };

            // IMPORTANT: for "urgent" requests (negative priority), decode the single highest
            // priority request first (serially) so it is not competing with neighbor prefetch.
            // This is the key to making far jumps feel instant.
            let urgent_head = batch.first().map_or(false, |r| r.priority < 0);
            let mut start_index = 0usize;

            if urgent_head {
                if let Some(first) = batch.first() {
                    let (idx, req_generation, outcome) = process_one(first);
                    if !publish_one(idx, req_generation, outcome) {
                        return;
                    }
                    start_index = 1;
                }
            }

            if start_index >= batch.len() {
                continue;
            }

            let parallel_len = batch.len() - start_index;
            let (outcome_tx, outcome_rx) = crossbeam_channel::unbounded();
            let mut disconnected = false;

            rayon::scope(|scope| {
                for req in batch[start_index..].iter() {
                    let outcome_tx = outcome_tx.clone();
                    scope.spawn(move |_| {
                        let outcome = process_one(req);
                        let _ = outcome_tx.send(outcome);
                    });
                }

                drop(outcome_tx);

                for _ in 0..parallel_len {
                    let Ok((idx, req_generation, outcome)) = outcome_rx.recv() else {
                        break;
                    };

                    if !publish_one(idx, req_generation, outcome) {
                        disconnected = true;
                        break;
                    }
                }
            });

            if disconnected {
                return;
            }
        }
    }

    fn clear_loading_if_generation(
        loading_indices: &Arc<RwLock<HashMap<usize, usize>>>,
        index: usize,
        generation: usize,
    ) {
        let mut loading = loading_indices.write();
        if loading.get(&index).copied() == Some(generation) {
            loading.remove(&index);
        }
    }

    fn retry_backoff_for_attempt(attempts: u8) -> Duration {
        let shift = attempts.saturating_sub(1).min(6) as u32;
        let factor = 1u64 << shift;
        let delay_ms = PRELOAD_RETRY_BASE_DELAY_MS
            .saturating_mul(factor)
            .min(PRELOAD_RETRY_MAX_DELAY_MS);
        Duration::from_millis(delay_ms)
    }

    fn register_decode_failure(
        loaded_levels: &Arc<RwLock<HashMap<usize, u32>>>,
        retry_state: &Arc<RwLock<HashMap<usize, RetryState>>>,
        index: usize,
    ) {
        loaded_levels.write().remove(&index);

        let now = Instant::now();
        let mut retry = retry_state.write();
        let attempts = retry
            .get(&index)
            .map(|state| state.attempts)
            .unwrap_or(0)
            .saturating_add(1);
        let next_retry_at = now + Self::retry_backoff_for_attempt(attempts);
        retry.insert(
            index,
            RetryState {
                attempts,
                next_retry_at,
            },
        );
    }

    fn probe_image_dimensions_cached(path: &std::path::Path) -> Option<(u32, u32)> {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Image) {
            return Some((w, h));
        }

        let dims = probe_image_dimensions(path);
        if let Some((w, h)) = dims {
            store_cached_dimensions(path, CachedMediaKind::Image, w, h);
        }

        dims
    }

    fn probe_video_dimensions_fast(path: &std::path::Path) -> (u32, u32) {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Video) {
            return (w, h);
        }

        if let Some((w, h)) = probe_video_dimensions_without_gstreamer(path) {
            if w > 0 && h > 0 {
                store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                return (w, h);
            }
        }

        Self::fallback_video_dimensions(path).unwrap_or((1920, 1080))
    }

    /// Load a single image on a worker thread.
    /// For video files, this extracts the first frame as a thumbnail placeholder.
    fn load_single_image(req: &LoadRequest) -> Option<DecodedImage> {
        let effective_texture_side = req
            .target_texture_side
            .max(1)
            .min(req.max_texture_side.max(1));

        // Determine media type
        let media_type = get_media_type(&req.path)?;

        match media_type {
            MediaType::Video => {
                // For videos, try to extract the first frame as a thumbnail
                // This provides a visual preview instead of a gray placeholder
                match Self::extract_video_first_frame(&req.path, effective_texture_side) {
                    Some((pixels, width, height, original_width, original_height)) => {
                        let resize_started = Instant::now();
                        let (width, height, pixels) = downscale_rgba_if_needed(
                            width,
                            height,
                            &pixels,
                            effective_texture_side,
                            req.downscale_filter,
                        );
                        let resize_time = resize_started.elapsed();

                        Some(DecodedImage {
                            index: req.index,
                            pixels: pixels.into_owned(),
                            width,
                            height,
                            original_width,
                            original_height,
                            media_type: MangaMediaType::Video,
                            requested_side: effective_texture_side,
                            queue_wait: Duration::ZERO,
                            decode_time: Duration::ZERO,
                            resize_time,
                        })
                    }
                    None => {
                        // Fallback: use cheap dimensions only. Do not call the full probe here,
                        // because that can re-run first-frame extraction for already-failed videos.
                        let (original_width, original_height) =
                            Self::probe_video_dimensions_fast(&req.path);

                        Some(DecodedImage {
                            index: req.index,
                            pixels: Vec::new(),
                            width: 0,
                            height: 0,
                            original_width,
                            original_height,
                            media_type: MangaMediaType::Video,
                            requested_side: effective_texture_side,
                            queue_wait: Duration::ZERO,
                            decode_time: Duration::ZERO,
                            resize_time: Duration::ZERO,
                        })
                    }
                }
            }
            MediaType::Image => {
                // Get original dimensions from file header first (fast, no decode)
                let (original_width, original_height) =
                    Self::probe_image_dimensions_cached(&req.path)?;

                // For definitely-static formats, prefer persistent thumbnail pyramid cache.
                // This avoids repeat decode+resize work across sessions and dense masonry runs.
                let may_be_animated_by_ext = req
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "gif" | "webp"))
                    .unwrap_or(false);
                if !may_be_animated_by_ext {
                    if let Some(cached) =
                        lookup_cached_static_thumbnail(&req.path, effective_texture_side)
                    {
                        return Some(DecodedImage {
                            index: req.index,
                            pixels: cached.pixels,
                            width: cached.width,
                            height: cached.height,
                            original_width: cached.original_width,
                            original_height: cached.original_height,
                            media_type: MangaMediaType::StaticImage,
                            requested_side: effective_texture_side,
                            queue_wait: Duration::ZERO,
                            decode_time: Duration::ZERO,
                            resize_time: Duration::ZERO,
                        });
                    }
                }

                // For animated WebP files we only decode the first frame here so that
                // the manga scroll view isn't blocked by a potentially very expensive
                // full-animation decode.  The full animation will be loaded lazily by
                // `manga_update_animated_textures` when the user actually focuses on it.
                let is_animated_webp = LoadedImage::is_animated_webp(&req.path);

                let downscale_filter = req.downscale_filter;
                let gif_filter = req.gif_filter;

                let img = if is_animated_webp {
                    LoadedImage::load_first_frame_only(
                        &req.path,
                        Some(effective_texture_side),
                        downscale_filter,
                        gif_filter,
                    )
                    .ok()?
                } else {
                    LoadedImage::load_with_max_texture_side(
                        &req.path,
                        Some(effective_texture_side),
                        downscale_filter,
                        gif_filter,
                    )
                    .ok()?
                };

                // Determine if this is an animated image.
                // For animated WebP, we loaded only the first frame, but we still
                // know it's animated from the header check above.
                let manga_media_type = if is_animated_webp || img.is_animated() {
                    MangaMediaType::AnimatedImage
                } else {
                    MangaMediaType::StaticImage
                };

                let frame = img.current_frame_data();

                // Downscale if needed (should already be done by loader, but safety check)
                let resize_started = Instant::now();
                let (width, height, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    effective_texture_side,
                    downscale_filter,
                );
                let resize_time = resize_started.elapsed();
                let pixels = pixels.into_owned();

                if manga_media_type == MangaMediaType::StaticImage {
                    store_cached_static_thumbnail(
                        &req.path,
                        effective_texture_side,
                        &CachedImageThumbnail {
                            pixels: pixels.clone(),
                            width,
                            height,
                            original_width,
                            original_height,
                        },
                    );
                }

                Some(DecodedImage {
                    index: req.index,
                    pixels,
                    width,
                    height,
                    original_width,
                    original_height,
                    media_type: manga_media_type,
                    requested_side: effective_texture_side,
                    queue_wait: Duration::ZERO,
                    decode_time: Duration::ZERO,
                    resize_time,
                })
            }
        }
    }

    fn quantize_target_texture_side(target_texture_side: u32, max_texture_side: u32) -> u32 {
        let max_side = max_texture_side.max(1);
        let target = target_texture_side.max(1).min(max_side);

        for &bucket in LOD_SIDE_BUCKETS {
            if bucket >= target && bucket <= max_side {
                return bucket;
            }
        }

        max_side
    }

    fn limit_target_texture_side_for_index(
        &self,
        index: usize,
        target_texture_side: u32,
        max_texture_side: u32,
    ) -> u32 {
        let quantized = Self::quantize_target_texture_side(target_texture_side, max_texture_side);

        self.dimension_cache
            .get(&index)
            .map(|(width, height, _)| quantized.min((*width).max(*height).max(1)))
            .unwrap_or(quantized)
    }

    /// Probe video dimensions for stable layout sizing.
    ///
    /// Uses a lightweight first-frame probe, then falls back to filename heuristics.
    fn probe_video_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
        if !gstreamer_runtime_available() {
            if let Some((w, h)) = probe_video_dimensions_without_gstreamer(path) {
                store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                return Some((w, h));
            }
        }

        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Video) {
            return Some((w, h));
        }

        let probed = Self::extract_video_first_frame(path, 256)
            .map(|(_, _, _, original_w, original_h)| (original_w, original_h));

        if let Some((w, h)) = probed {
            if w > 0 && h > 0 {
                store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                return Some((w, h));
            }
        }

        let fallback = Self::fallback_video_dimensions(path);
        if let Some((w, h)) = fallback {
            store_cached_dimensions(path, CachedMediaKind::Video, w, h);
        }

        fallback
    }

    fn fallback_video_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
        let filename = path.file_name()?.to_string_lossy().to_lowercase();

        if filename.contains("vertical")
            || filename.contains("portrait")
            || filename.contains("9x16")
            || filename.contains("1080x1920")
            || filename.contains("720x1280")
        {
            return Some((1080, 1920));
        }

        if filename.contains("4k") || filename.contains("2160") {
            Some((3840, 2160))
        } else if filename.contains("1440") || filename.contains("2k") {
            Some((2560, 1440))
        } else if filename.contains("1080") || filename.contains("fhd") {
            Some((1920, 1080))
        } else if filename.contains("720") || filename.contains("hd") {
            Some((1280, 720))
        } else if filename.contains("480") || filename.contains("sd") {
            Some((854, 480))
        } else {
            Some((1920, 1080))
        }
    }

    fn video_thumbnail_output_dimensions(
        source_dimensions: Option<(u32, u32)>,
        max_texture_side: u32,
    ) -> Option<(u32, u32)> {
        let (source_w, source_h) = source_dimensions?;
        if source_w == 0 || source_h == 0 || max_texture_side == 0 {
            return None;
        }

        let scale = (max_texture_side as f64 / source_w as f64)
            .min(max_texture_side as f64 / source_h as f64)
            .min(1.0);
        if scale >= 0.999 {
            return None;
        }

        Some((
            (source_w as f64 * scale).round().max(1.0) as u32,
            (source_h as f64 * scale).round().max(1.0) as u32,
        ))
    }

    /// Extract the first frame from a video file as a thumbnail.
    ///
    /// Uses GStreamer to decode just the first frame without loading the entire video.
    /// This is much faster than full video playback initialization and provides
    /// a visual preview for videos in manga mode.
    ///
    /// Returns: Some((pixels, width, height, original_width, original_height)) or None on failure
    fn extract_video_first_frame(
        path: &std::path::Path,
        max_texture_side: u32,
    ) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
        if let Some(mut cached) = lookup_cached_video_thumbnail(path, max_texture_side) {
            if !gstreamer_runtime_available() {
                if let Some((original_width, original_height)) =
                    probe_video_dimensions_without_gstreamer(path)
                {
                    if cached.original_width != original_width
                        || cached.original_height != original_height
                    {
                        cached.original_width = original_width;
                        cached.original_height = original_height;
                        store_cached_video_thumbnail(path, max_texture_side, &cached);
                    }
                }
            }

            return Some((
                cached.pixels,
                cached.width,
                cached.height,
                cached.original_width,
                cached.original_height,
            ));
        }

        if let Some((pixels, width, height, original_width, original_height)) =
            extract_video_first_frame_without_gstreamer(path, max_texture_side)
        {
            store_cached_video_thumbnail(
                path,
                max_texture_side,
                &CachedVideoThumbnail {
                    pixels: pixels.clone(),
                    width,
                    height,
                    original_width,
                    original_height,
                },
            );

            return Some((pixels, width, height, original_width, original_height));
        }

        if !gstreamer_runtime_available() {
            return None;
        }

        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_app as gst_app;
        use gstreamer_video as gst_video;
        use parking_lot::Mutex;
        use std::sync::Arc;
        use std::time::Duration;

        // Initialize GStreamer if needed (static check to avoid repeated init)
        static GST_INIT: std::sync::OnceLock<Result<(), ()>> = std::sync::OnceLock::new();
        let init_result = GST_INIT.get_or_init(|| gst::init().map_err(|_| ()));
        if init_result.is_err() {
            return None;
        }

        // Build URI from path
        let uri = gst::glib::filename_to_uri(path, None).ok()?.to_string();

        let source_dimensions = Self::probe_video_dimensions_fast(path);
        let output_dimensions =
            Self::video_thumbnail_output_dimensions(Some(source_dimensions), max_texture_side);
        let caps_filter = match output_dimensions {
            Some((width, height)) if width > 0 && height > 0 => {
                format!("video/x-raw,format=RGBA,width={},height={}", width, height)
            }
            _ => "video/x-raw,format=RGBA".to_string(),
        };

        // Create a minimal pipeline for frame extraction
        // Use decodebin for auto-detection and videoscale/videoconvert for format conversion
        let pipeline_str = format!(
            "uridecodebin uri=\"{}\" name=dec ! videoconvert ! videoscale ! \
             {} ! appsink name=sink max-buffers=1 drop=true",
            uri.replace("\"", "\\\""),
            caps_filter
        );

        let pipeline = gst::parse::launch(&pipeline_str).ok()?;
        let pipeline = pipeline.downcast::<gst::Pipeline>().ok()?;

        // Get the appsink element
        let appsink = pipeline
            .by_name("sink")?
            .dynamic_cast::<gst_app::AppSink>()
            .ok()?;

        // Storage for extracted frame
        let frame_data: Arc<Mutex<Option<(Vec<u8>, u32, u32)>>> = Arc::new(Mutex::new(None));
        let frame_data_clone = Arc::clone(&frame_data);

        // Set up preroll callback to capture the first frame
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_preroll(move |sink| {
                    if let Ok(sample) = sink.pull_preroll() {
                        if let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps()) {
                            if let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) {
                                let width = video_info.width();
                                let height = video_info.height();
                                if let Ok(map) = buffer.map_readable() {
                                    let pixels = map.as_slice().to_vec();
                                    let mut data = frame_data_clone.lock();
                                    *data = Some((pixels, width, height));
                                }
                            }
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Set to PAUSED to get the first frame (preroll)
        if pipeline.set_state(gst::State::Paused).is_err() {
            let _ = pipeline.set_state(gst::State::Null);
            return None;
        }

        // Wait for state change or timeout (short for dense masonry responsiveness)
        let bus = pipeline.bus()?;
        let mut got_frame = false;

        // Wait for ASYNC_DONE or ERROR, with timeout
        let deadline = std::time::Instant::now() + Duration::from_millis(250);
        while std::time::Instant::now() < deadline {
            if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(50)) {
                match msg.view() {
                    gst::MessageView::AsyncDone(_) => {
                        got_frame = true;
                        break;
                    }
                    gst::MessageView::Error(_) => {
                        break;
                    }
                    gst::MessageView::Eos(_) => {
                        break;
                    }
                    _ => {}
                }
            }

            // Check if we already got frame data
            if frame_data.lock().is_some() {
                got_frame = true;
                break;
            }
        }

        // Cleanup pipeline
        let _ = pipeline.set_state(gst::State::Null);

        if !got_frame {
            return None;
        }

        // Extract the frame data
        let (pixels, width, height) = {
            let data = frame_data.lock();
            data.clone()?
        };

        if pixels.is_empty() || width == 0 || height == 0 {
            return None;
        }

        let (original_width, original_height) = source_dimensions;

        // Downscale if needed for GPU texture limits
        let (final_width, final_height, final_pixels) = downscale_rgba_if_needed(
            width,
            height,
            &pixels,
            max_texture_side,
            FilterType::Triangle,
        );

        let final_pixels = final_pixels.into_owned();
        store_cached_video_thumbnail(
            path,
            max_texture_side,
            &CachedVideoThumbnail {
                pixels: final_pixels.clone(),
                width: final_width,
                height: final_height,
                original_width,
                original_height,
            },
        );

        Some((
            final_pixels,
            final_width,
            final_height,
            original_width,
            original_height,
        ))
    }

    fn calculate_strip_preload_counts(&self, visible_item_equivalent: f32) -> (usize, usize) {
        let visible_items = visible_item_equivalent.max(1.0);
        let ahead = (visible_items * STRIP_PRELOAD_LOOK_AHEAD_MULTIPLIER)
            .ceil()
            .max(1.0) as usize;
        let behind = (visible_items * STRIP_PRELOAD_LOOK_BEHIND_MULTIPLIER)
            .ceil()
            .max(1.0) as usize;

        (ahead.min(MAX_PRELOAD_AHEAD), behind.min(MAX_PRELOAD_BEHIND))
    }

    /// Calculate preload counts based on the current layout's visible item signal.
    /// Long strip uses fractional viewport coverage; masonry keeps the existing whole-item floor.
    ///
    /// Returns (preload_ahead, preload_behind)
    fn calculate_preload_counts(&self) -> (usize, usize) {
        if let Some(visible_item_equivalent) = self.strip_visible_item_equivalent {
            return self.calculate_strip_preload_counts(visible_item_equivalent);
        }

        let visible_items = self.visible_page_count.max(1);
        let ahead = visible_items
            .saturating_mul(PRELOAD_LOOK_AHEAD_MULTIPLIER)
            .clamp(MIN_PRELOAD_AHEAD, MAX_PRELOAD_AHEAD);
        let behind = visible_items
            .saturating_mul(PRELOAD_LOOK_BEHIND_MULTIPLIER)
            .clamp(MIN_PRELOAD_BEHIND, MAX_PRELOAD_BEHIND);

        (ahead, behind)
    }

    /// Update the visible item count for adaptive preloading.
    /// Call this after calculating how many items are visible on screen.
    /// Long strip can optionally pass fractional viewport coverage to avoid oversized windows.
    pub fn update_visible_page_count(
        &mut self,
        visible_page_count: usize,
        strip_visible_item_equivalent: Option<f32>,
    ) {
        self.visible_page_count = visible_page_count.max(1);
        self.strip_visible_item_equivalent = strip_visible_item_equivalent
            .filter(|value| value.is_finite())
            .map(|value| value.max(1.0));
    }

    /// Get current preload ahead count (useful for cache eviction in main.rs)
    pub fn get_preload_ahead(&self) -> usize {
        self.calculate_preload_counts().0
    }

    /// Get current preload behind count (useful for cache eviction in main.rs)
    pub fn get_preload_behind(&self) -> usize {
        self.calculate_preload_counts().1
    }

    /// Request loading of images around the visible range.
    /// Uses priority-based loading with scroll direction and visibility awareness.
    ///
    /// The algorithm adapts to the number of currently visible items.
    pub fn update_preload_queue(
        &mut self,
        image_list: &[PathBuf],
        visible_index: usize,
        _screen_height: f32,
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
            self.cancel_pending_loads();
        }

        // Update scroll direction based on visible index change
        if visible_index > prev_visible_index {
            self.scroll_direction = 1; // Scrolling down
        } else if visible_index < prev_visible_index {
            self.scroll_direction = -1; // Scrolling up
        }
        self.last_visible_index = visible_index;

        // Calculate preload counts based on visible pages
        let (base_ahead, base_behind) = self.calculate_preload_counts();

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
        requests.sort_by_key(|r| r.priority);
        for req in requests {
            let idx = req.index;
            match self.request_tx.try_send(req) {
                Ok(()) => {
                    self.loading_indices.write().insert(idx, generation);
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

    /// Poll for decoded images ready for GPU upload with a caller-provided limit.
    ///
    /// `max_items` is clamped to `1..=MAX_PENDING_UPLOADS`.
    /// Returns `(decoded_images, dimension_updates)` where `dimension_updates`
    /// includes indices whose cached source dimensions changed.
    pub fn poll_decoded_images_with_limit(
        &mut self,
        max_items: usize,
    ) -> (Vec<DecodedImage>, Vec<usize>) {
        let max_items = max_items.clamp(1, MAX_PENDING_UPLOADS);
        let mut results = Vec::with_capacity(max_items);
        let mut dimension_updates = Vec::with_capacity(max_items);

        for _ in 0..max_items {
            match self.result_rx.try_recv() {
                Ok(decoded) => {
                    // Cache dimensions and media type for stable layout
                    let new_dims = (
                        decoded.original_width,
                        decoded.original_height,
                        decoded.media_type,
                    );
                    let changed = self.dimension_cache.get(&decoded.index).map_or(
                        true,
                        |(old_w, old_h, old_mt)| {
                            if *old_w == new_dims.0 && *old_h == new_dims.1 && *old_mt == new_dims.2
                            {
                                return false;
                            }

                            // Ignore tiny video size jitter to avoid perpetual masonry relayouts.
                            if *old_mt == MangaMediaType::Video
                                && new_dims.2 == MangaMediaType::Video
                            {
                                let width_delta = old_w.abs_diff(new_dims.0);
                                let height_delta = old_h.abs_diff(new_dims.1);
                                if width_delta <= 2 && height_delta <= 2 {
                                    let old_aspect = *old_w as f32 / (*old_h).max(1) as f32;
                                    let new_aspect = new_dims.0 as f32 / new_dims.1.max(1) as f32;
                                    if (old_aspect - new_aspect).abs() <= 0.003 {
                                        return false;
                                    }
                                }
                            }

                            true
                        },
                    );

                    if changed {
                        self.dimension_cache.insert(decoded.index, new_dims);
                        if !dimension_updates.contains(&decoded.index) {
                            dimension_updates.push(decoded.index);
                        }
                    }

                    results.push(decoded);
                    self.stats.images_loaded += 1;
                }
                Err(_) => break,
            }
        }

        (results, dimension_updates)
    }

    /// Start async dimension caching for all images in the list.
    /// This returns immediately and caches dimensions in the background.
    /// The first few visible images are prioritized.
    pub fn cache_all_dimensions(&mut self, image_list: &[PathBuf]) -> usize {
        // For fast startup, only cache the first batch of visible media synchronously.
        // The rest will be cached on-demand or when media is loaded.
        const INITIAL_CACHE_COUNT: usize = 30;

        // Clear existing cache
        self.dimension_cache.clear();

        if image_list.is_empty() {
            return 0;
        }

        let cached_lookup_items: Vec<(usize, CachedMediaKind, PathBuf)> = image_list
            .iter()
            .enumerate()
            .filter_map(|(idx, path)| {
                if is_supported_video(path) {
                    Some((idx, CachedMediaKind::Video, path.clone()))
                } else if is_supported_image(path) {
                    Some((idx, CachedMediaKind::Image, path.clone()))
                } else {
                    None
                }
            })
            .collect();

        if !cached_lookup_items.is_empty() {
            let batch_items: Vec<(PathBuf, CachedMediaKind)> = cached_lookup_items
                .iter()
                .map(|(_, kind, path)| (path.clone(), *kind))
                .collect();
            let cached_results = lookup_cached_dimensions_batch(&batch_items);

            for ((idx, kind, _), cached) in
                cached_lookup_items.iter().zip(cached_results.into_iter())
            {
                if let Some((w, h)) = cached {
                    let media_type = match kind {
                        CachedMediaKind::Image => MangaMediaType::StaticImage,
                        CachedMediaKind::Video => MangaMediaType::Video,
                    };
                    self.dimension_cache.insert(*idx, (w, h, media_type));
                }
            }
        }

        // Cache first batch synchronously for immediate layout
        let initial_probe_items: Vec<(usize, PathBuf)> = image_list
            .iter()
            .enumerate()
            .filter(|(idx, path)| {
                !self.dimension_cache.contains_key(idx)
                    && (is_supported_video(path) || is_supported_image(path))
            })
            .take(INITIAL_CACHE_COUNT)
            .map(|(idx, path)| (idx, path.clone()))
            .collect();
        let initial_batch: Vec<(usize, Option<(u32, u32, MangaMediaType)>)> = initial_probe_items
            .par_iter()
            .map(|(idx, path)| {
                let is_video = is_supported_video(path);
                let is_image = is_supported_image(path);

                if is_video {
                    // For videos, probe dimensions
                    let dims = Self::probe_video_dimensions(path);
                    (*idx, dims.map(|(w, h)| (w, h, MangaMediaType::Video)))
                } else if is_image {
                    // For images, get from file header
                    let dims = Self::probe_image_dimensions_cached(path);
                    // We can't easily determine if an image is animated without loading it
                    // Default to static, will be updated when actually loaded
                    (*idx, dims.map(|(w, h)| (w, h, MangaMediaType::StaticImage)))
                } else {
                    (*idx, None)
                }
            })
            .collect();
        for (idx, opt_dims) in initial_batch {
            if let Some((w, h, media_type)) = opt_dims {
                self.dimension_cache.insert(idx, (w, h, media_type));
            }
        }

        // The rest will be cached on-demand when media is loaded
        // or when manga_get_image_display_height is called
        self.cached_dimensions_count(image_list.len())
    }

    fn clear_with_dimension_policy(&mut self, preserve_dimensions: bool) {
        // Increment generation to invalidate pending requests
        self.current_generation += 1;
        self.generation
            .store(self.current_generation, Ordering::Release);

        // Clear indices
        self.loading_indices.write().clear();
        self.loaded_levels.write().clear();
        self.retry_state.write().clear();

        // Clear dimension cache
        if !preserve_dimensions {
            self.dimension_cache.clear();
        }

        // Clear any queued dimension probes
        self.dim_pending.clear();

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
    pub fn clear_preserving_dimensions(&mut self) {
        self.clear_with_dimension_policy(true);
    }

    /// Mark an index as needing reload (called when cache is evicted).
    pub fn mark_unloaded(&mut self, index: usize) {
        self.loaded_levels.write().remove(&index);
        self.retry_state.write().remove(&index);
    }

    /// Clear all pending/loaded bookkeeping for an index so a visible placeholder can self-heal.
    pub fn reset_index_state(&mut self, index: usize) {
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.retry_state.write().remove(&index);
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Sync loader jump-tracking to an externally handled visible-index change.
    ///
    /// Masonry scrollbar dragging can jump far enough that the UI already knows it has
    /// repositioned to a new destination. In that case we may want to cancel stale work once,
    /// then prevent `update_preload_queue` from treating the same jump as a second fresh jump.
    pub fn sync_external_visible_index(&mut self, visible_index: usize, cancel_pending: bool) {
        let previous_index = self.last_visible_index;

        if visible_index > previous_index {
            self.scroll_direction = 1;
        } else if visible_index < previous_index {
            self.scroll_direction = -1;
        }

        if cancel_pending {
            self.cancel_pending_loads();
        }

        self.last_visible_index = visible_index;
    }

    /// Force a high-priority retry for a visible item that is missing a texture.
    ///
    /// This is used as a self-healing path when UI detects an in-view placeholder
    /// but loader bookkeeping says the item was already loaded.
    pub fn request_visible_retry(
        &mut self,
        image_list: &[PathBuf],
        index: usize,
        max_texture_side: u32,
        target_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        force_triangle_filters: bool,
    ) -> bool {
        let Some(path) = image_list.get(index).cloned() else {
            return false;
        };

        if !is_supported_image(&path) && !is_supported_video(&path) {
            return false;
        }

        if self.loading_indices.read().get(&index).copied() == Some(self.current_generation) {
            return false;
        }

        if self.retry_state.read().get(&index).is_some_and(|state| {
            state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS || Instant::now() < state.next_retry_at
        }) {
            return false;
        }

        let target_texture_side =
            self.limit_target_texture_side_for_index(index, target_texture_side, max_texture_side);

        // Visible placeholder takes precedence over potentially stale loaded bookkeeping.
        // The UI only calls this for in-view placeholders or explicit quality upgrades, so
        // we intentionally bypass loaded-side short-circuits here to avoid wedging tiles
        // after cache eviction/downgrade races.
        self.loaded_levels.write().remove(&index);

        let (request_downscale_filter, request_gif_filter) = if force_triangle_filters {
            (FilterType::Triangle, FilterType::Triangle)
        } else {
            (downscale_filter, gif_filter)
        };

        let req = LoadRequest {
            generation: self.current_generation,
            index,
            path,
            max_texture_side,
            target_texture_side,
            downscale_filter: request_downscale_filter,
            gif_filter: request_gif_filter,
            priority: -200_000,
            queued_at: Instant::now(),
        };

        let send_result = match self.urgent_request_tx.try_send(req.clone()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(req)) | Err(TrySendError::Disconnected(req)) => {
                self.request_tx.try_send(req)
            }
        };

        match send_result {
            Ok(()) => {
                self.loading_indices
                    .write()
                    .insert(index, self.current_generation);
                self.retry_state.write().remove(&index);
                self.stats.images_pending = self.loading_indices.read().len();
                true
            }
            Err(TrySendError::Full(_req)) => false,
            Err(TrySendError::Disconnected(_req)) => false,
        }
    }

    /// Cancel all pending load requests (called on large jumps like Home/End or fast scrollbar drag).
    /// This prevents loading intermediate images when jumping to a distant position.
    pub fn cancel_pending_loads(&mut self) {
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

impl Default for MangaLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MangaLoader {
    fn drop(&mut self) {
        // Signal shutdown to coordinator thread
        self.shutdown.store(true, Ordering::Release);
    }
}

/// Downscale RGBA pixel data if it exceeds the maximum texture size.
/// Uses Cow to avoid unnecessary allocations when no downscaling is needed.
fn image_filter_to_fir(filter: FilterType) -> fir::FilterType {
    match filter {
        FilterType::Nearest => fir::FilterType::Box,
        FilterType::Triangle => fir::FilterType::Bilinear,
        FilterType::CatmullRom => fir::FilterType::CatmullRom,
        FilterType::Gaussian => fir::FilterType::Gaussian,
        FilterType::Lanczos3 => fir::FilterType::Lanczos3,
    }
}

fn resize_rgba_with_fir(
    width: u32,
    height: u32,
    pixels: &[u8],
    new_w: u32,
    new_h: u32,
    filter: FilterType,
) -> Option<Vec<u8>> {
    let src = fir::images::ImageRef::new(width, height, pixels, fir::PixelType::U8x4).ok()?;
    let mut dst = fir::images::Image::new(new_w, new_h, fir::PixelType::U8x4);

    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(image_filter_to_fir(filter)));

    let mut resizer = fir::Resizer::new();
    resizer.resize(&src, &mut dst, Some(&options)).ok()?;

    Some(dst.into_vec())
}

fn downscale_rgba_if_needed<'a>(
    width: u32,
    height: u32,
    pixels: &'a [u8],
    max_texture_side: u32,
    filter: FilterType,
) -> (u32, u32, Cow<'a, [u8]>) {
    if max_texture_side == 0 {
        return (width, height, Cow::Borrowed(pixels));
    }

    if width <= max_texture_side && height <= max_texture_side {
        return (width, height, Cow::Borrowed(pixels));
    }

    // Preserve aspect ratio; clamp to at least 1x1.
    let scale =
        (max_texture_side as f64 / width as f64).min(max_texture_side as f64 / height as f64);
    let new_w = ((width as f64) * scale).round().max(1.0) as u32;
    let new_h = ((height as f64) * scale).round().max(1.0) as u32;

    if let Some(resized) = resize_rgba_with_fir(width, height, pixels, new_w, new_h, filter) {
        return (new_w, new_h, Cow::Owned(resized));
    }

    // Convert to an owned buffer for resizing.
    let Some(img) = image::RgbaImage::from_raw(width, height, pixels.to_vec()) else {
        return (width, height, Cow::Borrowed(pixels));
    };

    let resized = image::imageops::resize(&img, new_w, new_h, filter);
    (new_w, new_h, Cow::Owned(resized.into_raw()))
}

/// LRU-style texture cache for manga mode.
/// Keeps track of usage order for eviction.
pub struct MangaTextureCache {
    /// Non-evictable entries for currently visible indices.
    pinned_entries: HashMap<usize, MangaTextureEntry>,
    /// Evictable entries, ordered by recency.
    unpinned_entries: LruCache<usize, MangaTextureEntry>,
    /// Maximum number of entries to cache
    max_entries: usize,
    /// Indices that should not be evicted while still visible.
    pinned_indices: HashSet<usize>,
}

#[derive(Clone)]
struct MangaTextureEntry {
    texture: egui::TextureHandle,
    width: u32,
    height: u32,
    media_type: MangaMediaType,
}

impl MangaTextureCache {
    pub fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).expect("cache capacity is non-zero");
        Self {
            pinned_entries: HashMap::with_capacity(max_entries),
            unpinned_entries: LruCache::new(capacity),
            max_entries: max_entries.max(1),
            pinned_indices: HashSet::new(),
        }
    }

    fn total_entries(&self) -> usize {
        self.pinned_entries.len() + self.unpinned_entries.len()
    }

    fn evict_to_capacity(&mut self) -> Vec<usize> {
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

    /// Get texture ID and dimensions from cache (avoids borrow issues).
    /// Returns (texture_id, width, height) if found.
    pub fn get_texture_info(&mut self, index: usize) -> Option<(egui::TextureId, u32, u32)> {
        if let Some(entry) = self.pinned_entries.get(&index) {
            return Some((entry.texture.id(), entry.width, entry.height));
        }

        self.unpinned_entries
            .get(&index)
            .map(|entry| (entry.texture.id(), entry.width, entry.height))
    }

    /// Peek texture dimensions without mutating LRU recency state.
    pub fn peek_texture_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.pinned_entries
            .get(&index)
            .map(|entry| (entry.width, entry.height))
            .or_else(|| {
                self.unpinned_entries
                    .peek(&index)
                    .map(|entry| (entry.width, entry.height))
            })
    }

    /// Get a cloned texture handle, dimensions, and media type from cache.
    /// Returns (texture_handle, width, height, media_type) if found.
    pub fn get_texture_handle_info(
        &mut self,
        index: usize,
    ) -> Option<(egui::TextureHandle, u32, u32, MangaMediaType)> {
        if let Some(entry) = self.pinned_entries.get(&index) {
            return Some((
                entry.texture.clone(),
                entry.width,
                entry.height,
                entry.media_type,
            ));
        }

        self.unpinned_entries.get(&index).map(|entry| {
            (
                entry.texture.clone(),
                entry.width,
                entry.height,
                entry.media_type,
            )
        })
    }

    /// Check if an index is in the cache without updating access time.
    pub fn contains(&self, index: usize) -> bool {
        self.pinned_entries.contains_key(&index) || self.unpinned_entries.contains(&index)
    }

    /// Insert a texture into the cache with explicit media type.
    /// Returns evicted indices if cache was full.
    pub fn insert_with_type(
        &mut self,
        index: usize,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
        media_type: MangaMediaType,
    ) -> Vec<usize> {
        let entry = MangaTextureEntry {
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
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) {
        if let Some(entry) = self.pinned_entries.get_mut(&index) {
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            return;
        }

        if let Some(entry) = self.unpinned_entries.get_mut(&index) {
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
        }
    }

    /// Remove an entry from the cache.
    pub fn remove(&mut self, index: usize) {
        self.pinned_entries.remove(&index);
        self.unpinned_entries.pop(&index);
        self.pinned_indices.remove(&index);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.pinned_entries.clear();
        self.unpinned_entries.clear();
        self.pinned_indices.clear();
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

impl Default for MangaTextureCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHED_TEXTURES)
    }
}

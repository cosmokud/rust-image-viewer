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
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use image::imageops::FilterType;
use parking_lot::RwLock;
use rayon::prelude::*;

use crate::image_loader::{is_supported_image, is_supported_video, LoadedImage, MediaType, get_media_type};

/// Maximum number of decoded images to hold in memory awaiting GPU upload.
/// This bounds memory usage even if the main thread is slow to consume results.
const MAX_PENDING_UPLOADS: usize = 32;

/// Maximum number of images to keep in the texture cache.
/// Beyond this, the oldest entries are evicted to control VRAM usage.
const MAX_CACHED_TEXTURES: usize = 64;

/// Number of images to preload ahead/behind the current view.
const PRELOAD_AHEAD: usize = 12;
const PRELOAD_BEHIND: usize = 6;

/// Batch size for GPU texture uploads per frame.
/// Uploading too many textures in one frame can cause stutters.
const UPLOAD_BATCH_SIZE: usize = 4;

/// Maximum number of dimension probe items to include in a single request.
/// Larger values increase background throughput but can increase burstiness.
const DIM_REQUEST_BATCH_SIZE: usize = 64;

/// Maximum number of dimension results bundled into a single result message.
const DIM_RESULT_CHUNK_SIZE: usize = 64;

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
    pub priority: i32, // Lower = higher priority
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

/// High-performance manga image loader with parallel decoding.
pub struct MangaLoader {
    /// Channel to send load requests to worker threads
    request_tx: Sender<LoadRequest>,
    /// Channel to receive decoded images from worker threads
    result_rx: Receiver<DecodedImage>,
    /// Set of indices currently being loaded (to avoid duplicate requests)
    loading_indices: Arc<RwLock<HashSet<usize>>>,
    /// Set of indices that have been loaded (to avoid re-requesting)
    loaded_indices: Arc<RwLock<HashSet<usize>>>,
    /// Cached original dimensions and media type (from file headers) for stable layout
    /// Maps index -> (width, height, media_type)
    pub dimension_cache: HashMap<usize, (u32, u32, MangaMediaType)>,

    /// Async dimension-probe request channel (main thread -> worker).
    dim_request_tx: Sender<DimRequest>,
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
        let (result_tx, result_rx) = crossbeam_channel::bounded::<DecodedImage>(MAX_PENDING_UPLOADS);

        let (dim_request_tx, dim_request_rx) = crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_result_tx, dim_result_rx) = crossbeam_channel::bounded::<DimResult>(64);

        let loading_indices = Arc::new(RwLock::new(HashSet::new()));
        let loaded_indices = Arc::new(RwLock::new(HashSet::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicUsize::new(0));

        // Spawn a coordinator thread that processes requests using Rayon
        let loading_clone = Arc::clone(&loading_indices);
        let loaded_clone = Arc::clone(&loaded_indices);
        let shutdown_clone = Arc::clone(&shutdown);
        let generation_clone = Arc::clone(&generation);

        std::thread::Builder::new()
            .name("manga-loader-coordinator".into())
            .spawn(move || {
                Self::coordinator_loop(
                    request_rx,
                    result_tx,
                    loading_clone,
                    loaded_clone,
                    shutdown_clone,
                    generation_clone,
                );
            })
            .expect("Failed to spawn manga loader coordinator thread");

        // Spawn a lightweight dimension probe worker.
        // This keeps file header reads (image::image_dimensions) off the UI thread.
        let shutdown_clone = Arc::clone(&shutdown);
        std::thread::Builder::new()
            .name("manga-dimension-worker".into())
            .spawn(move || {
                while !shutdown_clone.load(Ordering::Acquire) {
                    let req = match dim_request_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(r) => r,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    };

                    let mut out: Vec<(usize, u32, u32, MangaMediaType)> = Vec::with_capacity(req.items.len());
                    for (idx, path) in req.items {
                        let is_video = is_supported_video(&path);
                        let is_image = is_supported_image(&path);

                        let dims = if is_video {
                            Self::probe_video_dimensions(&path)
                        } else if is_image {
                            image::image_dimensions(&path).ok()
                        } else {
                            None
                        };

                        if let Some((w, h)) = dims {
                            let mt = if is_video { MangaMediaType::Video } else { MangaMediaType::StaticImage };
                            out.push((idx, w, h, mt));
                        }

                        if out.len() >= DIM_RESULT_CHUNK_SIZE {
                            let chunk = std::mem::take(&mut out);
                            if dim_result_tx
                                .send(DimResult {
                                    generation: req.generation,
                                    items: chunk,
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                    }

                    if !out.is_empty() {
                        if dim_result_tx
                            .send(DimResult {
                                generation: req.generation,
                                items: out,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            })
            .expect("Failed to spawn manga dimension worker thread");

        Self {
            request_tx,
            result_rx,
            loading_indices,
            loaded_indices,
            dimension_cache: HashMap::new(),
            dim_request_tx,
            dim_result_rx,
            dim_pending: HashSet::new(),
            shutdown,
            scroll_direction: 1,
            last_visible_index: 0,
            generation,
            current_generation: 0,
            stats: LoaderStats::default(),
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

    /// Queue async dimension probes for a range of indices.
    ///
    /// This does not block the UI thread. Results are applied when `poll_dimension_results` is called.
    pub fn request_dimensions_range(&mut self, image_list: &[PathBuf], start: usize, end: usize) {
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
        match self.dim_request_tx.try_send(DimRequest {
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
                for (idx, _w, _h, _mt) in res.items {
                    self.dim_pending.remove(&idx);
                }
                continue;
            }

            for (idx, w, h, mt) in res.items {
                self.dimension_cache.insert(idx, (w, h, mt));
                self.dim_pending.remove(&idx);
                updated.push(idx);
            }
        }

        updated
    }

    /// Coordinator loop that processes requests in parallel using Rayon.
    fn coordinator_loop(
        request_rx: Receiver<LoadRequest>,
        result_tx: Sender<DecodedImage>,
        loading_indices: Arc<RwLock<HashSet<usize>>>,
        loaded_indices: Arc<RwLock<HashSet<usize>>>,
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

            // Block on first request (saves CPU when idle)
            match request_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(req) => batch.push(req),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }

            // Drain any additional pending requests (non-blocking)
            while let Ok(req) = request_rx.try_recv() {
                batch.push(req);
                if batch.len() >= 32 {
                    break;
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Sort by priority (lower = higher priority = process first)
            batch.sort_by_key(|r| r.priority);

            // Get current generation for filtering stale requests
            let current_gen = generation.load(Ordering::Acquire);

            // Process batch in parallel using Rayon
            let results: Vec<(usize, Option<DecodedImage>)> = batch
                .par_iter()
                .map(|req| {
                    // Skip if already loaded or if we've been shut down
                    if shutdown.load(Ordering::Relaxed) {
                        return (req.index, None);
                    }

                    // Skip stale requests (e.g., after fast scrollbar jumps / cancel).
                    if req.generation != current_gen {
                        return (req.index, None);
                    }

                    // Check if already in loaded set
                    {
                        let loaded = loaded_indices.read();
                        if loaded.contains(&req.index) {
                            return (req.index, None);
                        }
                    }

                    // Load the image
                    let decoded = Self::load_single_image(req);

                    (req.index, decoded)
                })
                .collect();

            // Publish results to main thread.
            // IMPORTANT: only mark an index as loaded after the decoded image is successfully enqueued.
            // Otherwise, a full result channel would cause the image to be permanently considered "loaded"
            // even though the main thread never received it, leaving placeholders stuck forever.
            for (idx, decoded_opt) in results {
                // Request has finished one way or another; allow it to be re-requested if needed.
                loading_indices.write().remove(&idx);

                let Some(decoded) = decoded_opt else {
                    continue;
                };

                match result_tx.try_send(decoded) {
                    Ok(_) => {
                        loaded_indices.write().insert(idx);
                    }
                    Err(TrySendError::Full(_decoded)) => {
                        // Channel full: drop decoded result.
                        // We intentionally do NOT mark as loaded so the main thread can re-request.
                    }
                    Err(TrySendError::Disconnected(_decoded)) => {
                        return; // Main thread gone, exit
                    }
                }
            }
        }
    }

    /// Load a single image on a worker thread.
    /// For video files, this extracts dimensions but doesn't decode - video playback is handled separately.
    fn load_single_image(req: &LoadRequest) -> Option<DecodedImage> {
        // Determine media type
        let media_type = get_media_type(&req.path)?;
        
        match media_type {
            MediaType::Video => {
                // For videos, we need to probe dimensions without full decode
                // Return a placeholder with dimensions - actual video playback is handled by VideoPlayer
                // Try to get video dimensions using GStreamer's discovery or fallback to a default
                let (original_width, original_height) = Self::probe_video_dimensions(&req.path)
                    .unwrap_or((1920, 1080)); // Fallback to 1080p
                
                // Return a minimal decoded image marker for videos
                // The actual video texture will be created by the VideoPlayer
                Some(DecodedImage {
                    index: req.index,
                    pixels: Vec::new(), // Empty - video textures are handled separately
                    width: 0,
                    height: 0,
                    original_width,
                    original_height,
                    media_type: MangaMediaType::Video,
                })
            }
            MediaType::Image => {
                // Get original dimensions from file header first (fast, no decode)
                let (original_width, original_height) = image::image_dimensions(&req.path).ok()?;

                // Load and decode the image
                let downscale_filter = FilterType::Triangle; // Fast filter for manga
                let gif_filter = FilterType::Triangle;

                let img = LoadedImage::load_with_max_texture_side(
                    &req.path,
                    Some(req.max_texture_side),
                    downscale_filter,
                    gif_filter,
                )
                .ok()?;

                // Determine if this is an animated image
                let is_animated = img.is_animated();
                let manga_media_type = if is_animated {
                    MangaMediaType::AnimatedImage
                } else {
                    MangaMediaType::StaticImage
                };

                let frame = img.current_frame_data();

                // Downscale if needed (should already be done by loader, but safety check)
                let (width, height, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    req.max_texture_side,
                    downscale_filter,
                );

                Some(DecodedImage {
                    index: req.index,
                    pixels: pixels.into_owned(),
                    width,
                    height,
                    original_width,
                    original_height,
                    media_type: manga_media_type,
                })
            }
        }
    }

    /// Probe video dimensions without full decode (using file metadata if available)
    fn probe_video_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
        // For now, we use a simple heuristic based on common video resolutions
        // In a production system, you'd use GStreamer's discoverer or ffprobe
        // But since GStreamer init is expensive and should happen on main thread,
        // we return a reasonable default and update dimensions when video is played
        
        // Try to infer from filename patterns (e.g., "video_1080p.mp4")
        let filename = path.file_name()?.to_string_lossy().to_lowercase();
        
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
            // Default to 1080p for unknown videos
            Some((1920, 1080))
        }
    }

    /// Request loading of images around the visible range.
    /// Uses priority-based loading with scroll direction awareness.
    pub fn update_preload_queue(
        &mut self,
        image_list: &[PathBuf],
        visible_index: usize,
        _screen_height: f32,
        max_texture_side: u32,
    ) {
        if image_list.is_empty() {
            return;
        }

        // Update scroll direction based on visible index change
        if visible_index > self.last_visible_index {
            self.scroll_direction = 1; // Scrolling down
        } else if visible_index < self.last_visible_index {
            self.scroll_direction = -1; // Scrolling up
        }
        self.last_visible_index = visible_index;

        // Calculate the range of indices to preload
        let ahead = if self.scroll_direction > 0 {
            PRELOAD_AHEAD
        } else {
            PRELOAD_BEHIND
        };
        let behind = if self.scroll_direction > 0 {
            PRELOAD_BEHIND
        } else {
            PRELOAD_AHEAD
        };

        let start_idx = visible_index.saturating_sub(behind);
        let end_idx = (visible_index + ahead + 1).min(image_list.len());

        // Collect indices that need loading
        let mut requests: Vec<LoadRequest> = Vec::new();
        let generation = self.current_generation;

        {
            let loading = self.loading_indices.read();
            let loaded = self.loaded_indices.read();

            for idx in start_idx..end_idx {
                // Check if file is a supported media type (image or video)
                let is_image = is_supported_image(&image_list[idx]);
                let is_video = is_supported_video(&image_list[idx]);
                if !is_image && !is_video {
                    continue;
                }

                // Skip if already loading or loaded
                if loading.contains(&idx) || loaded.contains(&idx) {
                    continue;
                }

                // Calculate priority based on distance from visible index
                // and scroll direction
                let distance = (idx as i32 - visible_index as i32).abs();
                let direction_bonus = if self.scroll_direction > 0 {
                    if idx > visible_index { 0 } else { 10 }
                } else {
                    if idx < visible_index { 0 } else { 10 }
                };

                let priority = distance + direction_bonus;

                requests.push(LoadRequest {
                    generation,
                    index: idx,
                    path: image_list[idx].clone(),
                    max_texture_side,
                    priority,
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
                    self.loading_indices.write().insert(idx);
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

    /// Poll for decoded images ready for GPU upload.
    /// Returns up to UPLOAD_BATCH_SIZE images per call to avoid frame drops.
    pub fn poll_decoded_images(&mut self) -> Vec<DecodedImage> {
        let mut results = Vec::with_capacity(UPLOAD_BATCH_SIZE);

        for _ in 0..UPLOAD_BATCH_SIZE {
            match self.result_rx.try_recv() {
                Ok(decoded) => {
                    // Cache dimensions and media type for stable layout
                    self.dimension_cache.insert(
                        decoded.index,
                        (decoded.original_width, decoded.original_height, decoded.media_type),
                    );
                    results.push(decoded);
                    self.stats.images_loaded += 1;
                }
                Err(_) => break,
            }
        }

        results
    }

    /// Start async dimension caching for all images in the list.
    /// This returns immediately and caches dimensions in the background.
    /// The first few visible images are prioritized.
    pub fn cache_all_dimensions(&mut self, image_list: &[PathBuf]) {
        // For fast startup, only cache the first batch of visible media synchronously.
        // The rest will be cached on-demand or when media is loaded.
        const INITIAL_CACHE_COUNT: usize = 30;

        // Clear existing cache
        self.dimension_cache.clear();

        // Cache first batch synchronously for immediate layout
        let initial_batch: Vec<(usize, Option<(u32, u32, MangaMediaType)>)> = image_list
            .par_iter()
            .take(INITIAL_CACHE_COUNT)
            .enumerate()
            .map(|(idx, path)| {
                let is_video = is_supported_video(path);
                let is_image = is_supported_image(path);
                
                if is_video {
                    // For videos, probe dimensions
                    let dims = Self::probe_video_dimensions(path);
                    (idx, dims.map(|(w, h)| (w, h, MangaMediaType::Video)))
                } else if is_image {
                    // For images, get from file header
                    let dims = image::image_dimensions(path).ok();
                    // We can't easily determine if an image is animated without loading it
                    // Default to static, will be updated when actually loaded
                    (idx, dims.map(|(w, h)| (w, h, MangaMediaType::StaticImage)))
                } else {
                    (idx, None)
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
    }

    /// Cache dimensions for a specific range of indices (called on-demand).
    pub fn cache_dimensions_range(&mut self, image_list: &[PathBuf], start: usize, end: usize) {
        let end = end.min(image_list.len());
        if start >= end {
            return;
        }

        // Only cache indices we don't have yet
        let indices_to_cache: Vec<usize> = (start..end)
            .filter(|&idx| !self.dimension_cache.contains_key(&idx))
            .collect();

        if indices_to_cache.is_empty() {
            return;
        }

        // Cache in parallel
        let dims: Vec<(usize, Option<(u32, u32, MangaMediaType)>)> = indices_to_cache
            .par_iter()
            .map(|&idx| {
                let path = &image_list[idx];
                let is_video = is_supported_video(path);
                let is_image = is_supported_image(path);
                
                if is_video {
                    let dims = Self::probe_video_dimensions(path);
                    (idx, dims.map(|(w, h)| (w, h, MangaMediaType::Video)))
                } else if is_image {
                    let dims = image::image_dimensions(path).ok();
                    (idx, dims.map(|(w, h)| (w, h, MangaMediaType::StaticImage)))
                } else {
                    (idx, None)
                }
            })
            .collect();

        for (idx, opt_dims) in dims {
            if let Some((w, h, media_type)) = opt_dims {
                self.dimension_cache.insert(idx, (w, h, media_type));
            }
        }
    }

    /// Clear all caches and reset state (called when exiting manga mode).
    pub fn clear(&mut self) {
        // Increment generation to invalidate pending requests
        self.current_generation += 1;
        self.generation.store(self.current_generation, Ordering::Release);

        // Clear indices
        self.loading_indices.write().clear();
        self.loaded_indices.write().clear();

        // Clear dimension cache
        self.dimension_cache.clear();

        // Clear any queued dimension probes
        self.dim_pending.clear();

        // Drain dimension result channel
        while self.dim_result_rx.try_recv().is_ok() {}

        // Drain result channel
        while self.result_rx.try_recv().is_ok() {}

        // Note: we can't directly drain the request channel from here because we only own
        // the Sender; cancellation is handled via generation checks in the coordinator.

        // Reset stats
        self.stats = LoaderStats::default();
    }

    /// Mark an index as needing reload (called when cache is evicted).
    pub fn mark_unloaded(&mut self, index: usize) {
        self.loaded_indices.write().remove(&index);
    }

    /// Cancel all pending load requests (called on large jumps like Home/End or fast scrollbar drag).
    /// This prevents loading intermediate images when jumping to a distant position.
    pub fn cancel_pending_loads(&mut self) {
        // Increment generation to invalidate in-flight requests
        // The coordinator thread will check this and skip stale requests
        self.current_generation += 1;
        self.generation.store(self.current_generation, Ordering::Release);

        // Clear loading indices (they'll be re-requested around the new position)
        self.loading_indices.write().clear();

        // Drain result channel to clear stale decoded images
        while self.result_rx.try_recv().is_ok() {}

        // Clear any queued dimension probes and drain stale results
        self.dim_pending.clear();
        while self.dim_result_rx.try_recv().is_ok() {}

        self.stats.images_pending = 0;
    }

    /// Check if an index is currently being loaded.
    #[allow(dead_code)]
    pub fn is_loading(&self, index: usize) -> bool {
        self.loading_indices.read().contains(&index)
    }

    /// Check if an index has been loaded.
    #[allow(dead_code)]
    pub fn is_loaded(&self, index: usize) -> bool {
        self.loaded_indices.read().contains(&index)
    }

    /// Get cached dimensions for an index (width, height only).
    pub fn get_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.dimension_cache.get(&index).map(|(w, h, _)| (*w, *h))
    }

    /// Get cached media info for an index (width, height, media_type).
    #[allow(dead_code)]
    pub fn get_media_info(&self, index: usize) -> Option<(u32, u32, MangaMediaType)> {
        self.dimension_cache.get(&index).copied()
    }

    /// Get media type for an index.
    pub fn get_media_type(&self, index: usize) -> Option<MangaMediaType> {
        self.dimension_cache.get(&index).map(|(_, _, mt)| *mt)
    }

    /// Update dimensions for a video (called when actual dimensions are known from playback).
    pub fn update_video_dimensions(&mut self, index: usize, width: u32, height: u32) {
        if let Some(entry) = self.dimension_cache.get_mut(&index) {
            entry.0 = width;
            entry.1 = height;
        } else {
            self.dimension_cache.insert(index, (width, height, MangaMediaType::Video));
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
    let scale = (max_texture_side as f64 / width as f64).min(max_texture_side as f64 / height as f64);
    let new_w = ((width as f64) * scale).round().max(1.0) as u32;
    let new_h = ((height as f64) * scale).round().max(1.0) as u32;

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
    /// Maps index to (texture_handle, width, height, media_type, last_access_frame)
    entries: HashMap<usize, (egui::TextureHandle, u32, u32, MangaMediaType, u64)>,
    /// Current frame counter for LRU tracking
    frame_counter: u64,
    /// Maximum number of entries to cache
    max_entries: usize,
}

impl MangaTextureCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(max_entries),
            frame_counter: 0,
            max_entries,
        }
    }

    /// Increment frame counter (call once per frame).
    pub fn tick(&mut self) {
        self.frame_counter += 1;
    }

    /// Get a texture from cache, updating its access time.
    #[allow(dead_code)]
    pub fn get(&mut self, index: usize) -> Option<&egui::TextureHandle> {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.4 = self.frame_counter;
            Some(&entry.0)
        } else {
            None
        }
    }

    /// Get texture ID and dimensions from cache (avoids borrow issues).
    /// Returns (texture_id, width, height) if found.
    pub fn get_texture_info(&mut self, index: usize) -> Option<(egui::TextureId, u32, u32)> {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.4 = self.frame_counter;
            Some((entry.0.id(), entry.1, entry.2))
        } else {
            None
        }
    }

    /// Get texture ID, dimensions, and media type from cache.
    /// Returns (texture_id, width, height, media_type) if found.
    #[allow(dead_code)]
    pub fn get_texture_info_with_type(&mut self, index: usize) -> Option<(egui::TextureId, u32, u32, MangaMediaType)> {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.4 = self.frame_counter;
            Some((entry.0.id(), entry.1, entry.2, entry.3))
        } else {
            None
        }
    }

    /// Get texture and dimensions from cache.
    #[allow(dead_code)]
    pub fn get_with_dims(&mut self, index: usize) -> Option<(&egui::TextureHandle, u32, u32)> {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.4 = self.frame_counter;
            Some((&entry.0, entry.1, entry.2))
        } else {
            None
        }
    }

    /// Check if an index is in the cache without updating access time.
    pub fn contains(&self, index: usize) -> bool {
        self.entries.contains_key(&index)
    }

    /// Insert a texture into the cache.
    /// Returns evicted indices if cache was full.
    #[allow(dead_code)]
    pub fn insert(
        &mut self,
        index: usize,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) -> Vec<usize> {
        self.insert_with_type(index, texture, width, height, MangaMediaType::StaticImage)
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
        let mut evicted = Vec::new();

        // Evict oldest entries if at capacity
        while self.entries.len() >= self.max_entries {
            // Find oldest entry
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, _, _, frame))| *frame)
                .map(|(&idx, _)| idx);

            if let Some(oldest_idx) = oldest {
                self.entries.remove(&oldest_idx);
                evicted.push(oldest_idx);
            } else {
                break;
            }
        }

        self.entries
            .insert(index, (texture, width, height, media_type, self.frame_counter));

        evicted
    }

    /// Update an existing texture in the cache (for video frame updates).
    /// Does not evict anything, just replaces the existing entry.
    #[allow(dead_code)]
    pub fn update_texture(
        &mut self,
        index: usize,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.0 = texture;
            entry.1 = width;
            entry.2 = height;
            entry.4 = self.frame_counter;
        }
    }

    /// Remove an entry from the cache.
    pub fn remove(&mut self, index: usize) {
        self.entries.remove(&index);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get the number of cached textures.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if cache is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get indices of all cached textures.
    pub fn cached_indices(&self) -> Vec<usize> {
        self.entries.keys().copied().collect()
    }
}

impl Default for MangaTextureCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_TEXTURES)
    }
}

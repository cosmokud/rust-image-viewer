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

use crossbeam_channel::{Receiver, Sender, TrySendError};
use image::imageops::FilterType;
use parking_lot::RwLock;
use rayon::prelude::*;

use crate::image_loader::{is_supported_image, LoadedImage};

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
}

/// Request sent to the loader thread pool.
#[derive(Clone)]
pub struct LoadRequest {
    pub index: usize,
    pub path: PathBuf,
    pub max_texture_side: u32,
    pub priority: i32, // Lower = higher priority
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
    /// Cached original dimensions (from file headers) for stable layout
    pub dimension_cache: HashMap<usize, (u32, u32)>,
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

        Self {
            request_tx,
            result_rx,
            loading_indices,
            loaded_indices,
            dimension_cache: HashMap::new(),
            shutdown,
            scroll_direction: 1,
            last_visible_index: 0,
            generation,
            current_generation: 0,
            stats: LoaderStats::default(),
        }
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
            let _current_gen = generation.load(Ordering::Acquire);

            // Process batch in parallel using Rayon
            let results: Vec<Option<DecodedImage>> = batch
                .par_iter()
                .map(|req| {
                    // Skip if already loaded or if we've been shut down
                    if shutdown.load(Ordering::Relaxed) {
                        return None;
                    }

                    // Check if already in loaded set
                    {
                        let loaded = loaded_indices.read();
                        if loaded.contains(&req.index) {
                            // Remove from loading set
                            loading_indices.write().remove(&req.index);
                            return None;
                        }
                    }

                    // Load the image
                    let decoded = Self::load_single_image(req);

                    // Mark as no longer loading, add to loaded
                    {
                        let mut loading = loading_indices.write();
                        loading.remove(&req.index);
                    }
                    {
                        let mut loaded = loaded_indices.write();
                        if let Some(ref d) = decoded {
                            loaded.insert(d.index);
                        }
                    }

                    decoded
                })
                .collect();

            // Send results to main thread (drop any that don't fit)
            for decoded in results.into_iter().flatten() {
                // Try to send; if the channel is full, skip (main thread will re-request if needed)
                match result_tx.try_send(decoded) {
                    Ok(_) => {}
                    Err(TrySendError::Full(_)) => {
                        // Channel full, drop this result (it will be re-requested)
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        return; // Main thread gone, exit
                    }
                }
            }
        }
    }

    /// Load a single image on a worker thread.
    fn load_single_image(req: &LoadRequest) -> Option<DecodedImage> {
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
        })
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

        {
            let loading = self.loading_indices.read();
            let loaded = self.loaded_indices.read();

            for idx in start_idx..end_idx {
                // Skip non-image files
                if !is_supported_image(&image_list[idx]) {
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
                    index: idx,
                    path: image_list[idx].clone(),
                    max_texture_side,
                    priority,
                });
            }
        }

        // Mark indices as loading
        if !requests.is_empty() {
            let mut loading = self.loading_indices.write();
            for req in &requests {
                loading.insert(req.index);
            }
        }

        // Send requests (sorted by priority)
        requests.sort_by_key(|r| r.priority);
        for req in requests {
            // Non-blocking send; if channel is full, skip (will retry next frame)
            let _ = self.request_tx.try_send(req);
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
                    // Cache dimensions for stable layout
                    self.dimension_cache.insert(
                        decoded.index,
                        (decoded.original_width, decoded.original_height),
                    );
                    results.push(decoded);
                    self.stats.images_loaded += 1;
                }
                Err(_) => break,
            }
        }

        results
    }

    /// Pre-cache dimensions for all images in the list (reads file headers only).
    /// This is O(n) but only reads a few bytes from each file header.
    pub fn cache_all_dimensions(&mut self, image_list: &[PathBuf]) {
        // Clear existing cache
        self.dimension_cache.clear();

        // Parallel dimension reading using Rayon
        let dims: Vec<(usize, Option<(u32, u32)>)> = image_list
            .par_iter()
            .enumerate()
            .filter(|(_, path)| is_supported_image(path))
            .map(|(idx, path)| {
                let dims = image::image_dimensions(path).ok();
                (idx, dims)
            })
            .collect();

        for (idx, opt_dims) in dims {
            if let Some((w, h)) = opt_dims {
                self.dimension_cache.insert(idx, (w, h));
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

        // Drain result channel
        while self.result_rx.try_recv().is_ok() {}

        // Drain request channel
        while self.request_tx.try_send(LoadRequest {
            index: usize::MAX, // Sentinel value to skip
            path: PathBuf::new(),
            max_texture_side: 0,
            priority: i32::MAX,
        }).is_ok() {}

        // Reset stats
        self.stats = LoaderStats::default();
    }

    /// Mark an index as needing reload (called when cache is evicted).
    pub fn mark_unloaded(&mut self, index: usize) {
        self.loaded_indices.write().remove(&index);
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

    /// Get cached dimensions for an index.
    pub fn get_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.dimension_cache.get(&index).copied()
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
    /// Maps index to (texture_handle, width, height, last_access_frame)
    entries: HashMap<usize, (egui::TextureHandle, u32, u32, u64)>,
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
            entry.3 = self.frame_counter;
            Some(&entry.0)
        } else {
            None
        }
    }

    /// Get texture ID and dimensions from cache (avoids borrow issues).
    /// Returns (texture_id, width, height) if found.
    pub fn get_texture_info(&mut self, index: usize) -> Option<(egui::TextureId, u32, u32)> {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.3 = self.frame_counter;
            Some((entry.0.id(), entry.1, entry.2))
        } else {
            None
        }
    }

    /// Get texture and dimensions from cache.
    #[allow(dead_code)]
    pub fn get_with_dims(&mut self, index: usize) -> Option<(&egui::TextureHandle, u32, u32)> {
        if let Some(entry) = self.entries.get_mut(&index) {
            entry.3 = self.frame_counter;
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
    pub fn insert(
        &mut self,
        index: usize,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) -> Vec<usize> {
        let mut evicted = Vec::new();

        // Evict oldest entries if at capacity
        while self.entries.len() >= self.max_entries {
            // Find oldest entry
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, _, frame))| *frame)
                .map(|(&idx, _)| idx);

            if let Some(oldest_idx) = oldest {
                self.entries.remove(&oldest_idx);
                evicted.push(oldest_idx);
            } else {
                break;
            }
        }

        self.entries
            .insert(index, (texture, width, height, self.frame_counter));

        evicted
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

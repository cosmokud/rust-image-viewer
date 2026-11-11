//! Constructors and `Default`/`Drop` for the loader and the texture cache.

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

impl VideoProbeGate {
    pub(crate) fn new(permits: usize) -> Self {
        Self {
            available: parking_lot::Mutex::new(permits),
            released: parking_lot::Condvar::new(),
        }
    }
}

impl Drop for VideoProbePermit {
    fn drop(&mut self) {
        PROBE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        *self.gate.available.lock() += 1;
        self.gate.released.notify_one();
    }
}

impl MangaLoader {
    /// Create a new manga loader with background thread pool.
    pub fn new() -> Self {
        let LoaderChannels {
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
        } = LoaderChannels::bounded();

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
        // Header-based size probes stay off the UI thread.
        let shutdown_clone = Arc::clone(&shutdown);
        let generation_clone = Arc::clone(&generation);
        crate::async_runtime::spawn_blocking_or_thread("manga-dimension-worker", move || {
            // Chunks the result channel refused. Re-emitted on the next pass: dropping them made
            // the tile fall back to 16:9 and re-probe, which is what stalled the gallery.
            let mut parked: std::collections::VecDeque<DimResult> =
                std::collections::VecDeque::new();

            while !shutdown_clone.load(Ordering::Acquire) {
                // Parked results go out before new work, with a short poll instead of a 500ms
                // block so a drained channel is refilled on the next tick.
                if !parked.is_empty() {
                    match Self::flush_parked_dim_chunks(&dim_result_tx, &mut parked) {
                        DimSend::Disconnected => return,
                        DimSend::Flushed => {}
                        DimSend::Parked => {
                            if let Ok(request) = dim_request_rx
                                .try_recv()
                                .or_else(|_| dim_background_request_rx.try_recv())
                            {
                                if let Some(req) =
                                    Self::dim_request_ready(request, &generation_clone)
                                {
                                    Self::run_dim_request(
                                        req,
                                        &dim_result_tx,
                                        &generation_clone,
                                        &mut parked,
                                    );
                                }
                                continue;
                            }

                            std::thread::sleep(Duration::from_millis(
                                DIM_RESULT_SEND_RETRY_SLEEP_MS,
                            ));
                            continue;
                        }
                    }
                }

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

                if let Some(req) = Self::dim_request_ready(req, &generation_clone) {
                    Self::run_dim_request(req, &dim_result_tx, &generation_clone, &mut parked);
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
            probe_blocked: HashMap::new(),
            video_revive: HashMap::new(),
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
            cache_capacity_last: MANGA_CACHE_MIN_ENTRIES,
            last_layout: MangaLayoutMode::LongStrip,
            last_visible_indices_count: 1,
            last_masonry_rows: 2,
            #[cfg(test)]
            probe: None,
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
}

impl Default for MangaTextureCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHED_TEXTURES)
    }
}

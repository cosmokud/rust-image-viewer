//! The loader's coordinator thread loop and its per-message handling.

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
    /// Coordinator loop that processes requests in parallel using Rayon.
    pub(crate) fn coordinator_loop(
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
                /// Decode returned nothing; carries the file shape it failed at.
                Failed(Option<FileStamp>),
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
                if queue_wait >= DECODE_QUEUE_WAIT_TRACE {
                    crate::diag::trace(
                        "decode-queue",
                        &format!(
                            "idx={} wait={}ms file={}",
                            req.index,
                            queue_wait.as_millis(),
                            diag_file_label(&req.path)
                        ),
                    );
                }
                let decode_started = Instant::now();
                let decoded = Self::load_single_image(req);
                let decode_time = decode_started.elapsed();

                let outcome = match decoded {
                    Some(mut decoded) => {
                        decoded.queue_wait = queue_wait;
                        decoded.decode_time = decode_time;
                        DecodeOutcome::Decoded(decoded)
                    }
                    None => DecodeOutcome::Failed(FileStamp::of_path(&req.path)),
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
                    DecodeOutcome::Failed(stamp) => {
                        Self::register_decode_failure(&loaded_levels, &retry_state, idx, stamp);
                    }
                    DecodeOutcome::Decoded(decoded) => {
                        let stamp = FileStamp::of_path(decoded.path.as_path());
                        // A decoded payload with empty pixels/dimensions is still useful for metadata
                        // (e.g., video dimension fallback), but does not satisfy texture preloading.
                        let has_usable_pixels =
                            !decoded.pixels.is_empty() && decoded.width > 0 && decoded.height > 0;
                        // A gated-probe miss is not a file failure: counting it spent the whole
                        // video retry budget in one or two frame passes and the tile then never
                        // re-requested itself for the session.
                        let deferred = decoded.deferred;

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
                                } else if deferred {
                                    // The probe cooldown already paces the next attempt.
                                    loaded_levels.write().remove(&idx);
                                } else {
                                    Self::register_decode_failure(
                                        &loaded_levels,
                                        &retry_state,
                                        idx,
                                        stamp,
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

    pub(crate) fn clear_loading_if_generation(
        loading_indices: &Arc<RwLock<HashMap<usize, usize>>>,
        index: usize,
        generation: usize,
    ) {
        let mut loading = loading_indices.write();
        if loading.get(&index).copied() == Some(generation) {
            loading.remove(&index);
        }
    }
}

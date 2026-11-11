//! Dimension probing inside the loader: queues, chunks, parking, warm-up.

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
    /// `None` for a request whose generation has passed: folder switches must not keep the
    /// dimension worker busy with obsolete probes.
    pub(crate) fn dim_request_ready(
        req: DimRequest,
        generation: &AtomicUsize,
    ) -> Option<DimRequest> {
        (req.generation == generation.load(Ordering::Acquire)).then_some(req)
    }

    /// Probe one batch and hand every chunk to the result channel, parking what it refuses.
    pub(crate) fn run_dim_request(
        req: DimRequest,
        dim_result_tx: &Sender<DimResult>,
        generation: &AtomicUsize,
        parked: &mut std::collections::VecDeque<DimResult>,
    ) {
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

                // Breadcrumb before a probe: same reason as the decode one.
                crate::diag::note_file(idx, "dim-probe", || {
                    format!(
                        "idx={} file={} bytes={} kind={}",
                        idx,
                        diag_file_label(&path),
                        std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                        if is_video { "video" } else { "image" }
                    )
                });

                let dims = if is_video {
                    Self::probe_video_dimensions(idx, &path)
                } else {
                    Self::probe_image_dimensions_cached(&path)
                };

                let (w, h) = if let Some((w, h)) = dims {
                    (w, h)
                } else if is_video {
                    FALLBACK_VIDEO_DIMENSIONS
                } else {
                    FALLBACK_IMAGE_DIMENSIONS
                };

                Some((idx, w, h, media_type))
            })
            .collect();

        out.par_sort_unstable_by_key(|(idx, _, _, _)| *idx);

        for chunk in out.chunks(DIM_RESULT_CHUNK_SIZE) {
            // Generation may advance while probing; avoid pushing obsolete results. The
            // dropped indices stay in `dim_pending`; the caller's stale-probe reaper
            // releases them so they can be probed again.
            if req.generation != generation.load(Ordering::Acquire) {
                break;
            }

            let chunk = DimResult {
                generation: req.generation,
                items: chunk.to_vec(),
            };
            if Self::queue_dim_chunk(dim_result_tx, chunk, parked) == DimSend::Disconnected {
                return;
            }
        }
    }

    /// Hand one chunk over, or park it in `parked` for the next pass. Never discards: the
    /// dropped indices would leave `dim_pending` set until the stale-probe reaper fires, and
    /// the tile would re-probe the file through gstreamer.
    pub(crate) fn queue_dim_chunk(
        dim_result_tx: &Sender<DimResult>,
        mut chunk: DimResult,
        parked: &mut std::collections::VecDeque<DimResult>,
    ) -> DimSend {
        // Keep order: a later chunk must not overtake one still parked.
        if !parked.is_empty() {
            parked.push_back(chunk);
            return DimSend::Parked;
        }

        for _ in 0..DIM_RESULT_SEND_MAX_ATTEMPTS {
            crate::diag::count(crate::diag::Counter::DimAttempts, 1);
            match dim_result_tx.try_send(chunk) {
                Ok(()) => {
                    crate::diag::count(crate::diag::Counter::DimChunks, 1);
                    return DimSend::Flushed;
                }
                Err(TrySendError::Full(returned)) => {
                    chunk = returned;
                    std::thread::sleep(Duration::from_millis(DIM_RESULT_SEND_RETRY_SLEEP_MS));
                }
                Err(TrySendError::Disconnected(_)) => return DimSend::Disconnected,
            }
        }

        crate::diag::count(crate::diag::Counter::DimParked, 1);
        parked.push_back(chunk);
        DimSend::Parked
    }

    /// Re-emit parked chunks until the channel is full again.
    pub(crate) fn flush_parked_dim_chunks(
        dim_result_tx: &Sender<DimResult>,
        parked: &mut std::collections::VecDeque<DimResult>,
    ) -> DimSend {
        while let Some(chunk) = parked.pop_front() {
            match dim_result_tx.try_send(chunk) {
                Ok(()) => crate::diag::count(crate::diag::Counter::DimFlushed, 1),
                Err(TrySendError::Full(returned)) => {
                    parked.push_front(returned);
                    return DimSend::Parked;
                }
                Err(TrySendError::Disconnected(_)) => {
                    crate::diag::count(crate::diag::Counter::DimDropped, parked.len() as u64 + 1);
                    parked.clear();
                    return DimSend::Disconnected;
                }
            }
        }

        DimSend::Flushed
    }

    /// Handle-only loader: no worker threads. Paired channels let tests inject dimension
    /// results and read what `demand`/`prime`/`retry_visible` enqueued.
    #[cfg(test)]
    pub(crate) fn without_workers() -> Self {
        let channels = LoaderChannels::bounded();

        Self {
            request_tx: channels.request_tx,
            urgent_request_tx: channels.urgent_request_tx,
            result_rx: channels.result_rx,
            loading_indices: Arc::new(RwLock::new(HashMap::new())),
            loaded_levels: Arc::new(RwLock::new(HashMap::new())),
            retry_state: Arc::new(RwLock::new(HashMap::new())),
            probe_blocked: HashMap::new(),
            video_revive: HashMap::new(),
            dimension_cache: HashMap::new(),
            dim_request_tx: channels.dim_request_tx,
            dim_background_request_tx: channels.dim_background_request_tx,
            dim_result_rx: channels.dim_result_rx,
            dim_pending: HashSet::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            scroll_direction: 1,
            last_visible_index: 0,
            generation: Arc::new(AtomicUsize::new(0)),
            current_generation: 0,
            stats: LoaderStats::default(),
            visible_page_count: 1,
            strip_visible_item_equivalent: None,
            cache_capacity_last: MANGA_CACHE_MIN_ENTRIES,
            last_layout: MangaLayoutMode::LongStrip,
            last_visible_indices_count: 1,
            last_masonry_rows: 2,
            probe: Some(LoaderProbe {
                requests: channels.request_rx,
                urgent: channels.urgent_request_rx,
                dim_requests: channels.dim_request_rx,
                dim_background_requests: channels.dim_background_request_rx,
                dim_results: channels.dim_result_tx,
            }),
        }
    }

    /// Queue async dimension probes for a range of indices.
    ///
    /// This does not block the UI thread. Results are applied when `take` runs.
    pub(crate) fn queue_dimension_range(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
    ) {
        self.enqueue_dimension_range(image_list, start, end, self.dim_request_tx.clone());
    }

    /// Queue async background dimension probes for a range of indices.
    ///
    /// For non-interactive masonry warm-up: must not compete with visible media.
    pub(crate) fn queue_background_dimension_range(
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

    pub(crate) fn enqueue_dimension_range(
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

    /// Background metadata warm-up for `[start, end)`: queues probes and, when
    /// `fallback_seed_max > 0`, seeds the stall-recovery fallback dimensions.
    /// Returns how many fallback entries were seeded.
    pub fn warm_dimensions_background(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
        fallback_seed_max: usize,
    ) -> usize {
        self.queue_background_dimension_range(image_list, start, end);
        if fallback_seed_max == 0 {
            0
        } else {
            self.seed_fallback_dimensions_for_range(image_list, start, end, fallback_seed_max)
        }
    }

    /// Drain async dimension results and apply them to `dimension_cache`.
    /// Returns indices whose dimensions were updated.
    pub(crate) fn apply_dimension_results(&mut self, max_messages: usize) -> Vec<usize> {
        let mut updated: Vec<usize> = Vec::new();

        for _ in 0..max_messages {
            let res = match self.dim_result_rx.try_recv() {
                Ok(r) => r,
                Err(_) => break,
            };

            // Drop stale results (e.g., after cancel/clear) but release the probe bookkeeping so
            // those indices are not stranded in `dim_pending` for the whole next generation.
            if res.generation != self.current_generation {
                for (idx, _, _, _) in res.items {
                    self.dim_pending.remove(&idx);
                }
                continue;
            }

            for (idx, raw_w, raw_h, mt) in res.items {
                // Unsanitized header dims poison layout and can clamp the request target to 1.
                let (w, h) = probed_or_fallback_dimensions(Some((raw_w, raw_h)), mt);
                let changed =
                    self.dimension_cache
                        .get(&idx)
                        .map_or(true, |(old_w, old_h, old_mt)| {
                            if *old_w == w && *old_h == h && *old_mt == mt {
                                return false;
                            }

                            // Ignore tiny video-size jitter from probes to avoid
                            // perpetual masonry relayout churn.
                            if *old_mt == MangaMediaType::Video && mt == MangaMediaType::Video {
                                let width_delta = old_w.abs_diff(w);
                                let height_delta = old_h.abs_diff(h);
                                if width_delta <= 2 && height_delta <= 2 {
                                    let old_aspect = *old_w as f32 / (*old_h).max(1) as f32;
                                    let new_aspect = w as f32 / h.max(1) as f32;
                                    if (old_aspect - new_aspect).abs() <= 0.003 {
                                        return false;
                                    }
                                }
                            }

                            true
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

    /// Seed conservative fallback dimensions for uncached items in `[start, end)`.
    ///
    /// This is a recovery path for rare probe stalls: it keeps masonry warm-up moving
    /// while higher-fidelity dimensions can still arrive later from decode/probe results.
    pub(crate) fn seed_fallback_dimensions_for_range(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
        max_items: usize,
    ) -> usize {
        if max_items == 0 || image_list.is_empty() {
            return 0;
        }

        let end = end.min(image_list.len());
        if start >= end {
            return 0;
        }

        let mut seeded = 0usize;

        for idx in start..end {
            if seeded >= max_items {
                break;
            }

            let Some(path) = image_list.get(idx) else {
                self.dim_pending.remove(&idx);
                continue;
            };

            let is_video = is_supported_video(path);
            let is_image = is_supported_image(path);

            if !is_video && !is_image {
                self.dim_pending.remove(&idx);
                continue;
            }

            if self.dimension_cache.contains_key(&idx) {
                self.dim_pending.remove(&idx);
                continue;
            }

            let fallback = if is_video {
                (
                    FALLBACK_VIDEO_DIMENSIONS.0,
                    FALLBACK_VIDEO_DIMENSIONS.1,
                    MangaMediaType::Video,
                )
            } else {
                (
                    FALLBACK_IMAGE_DIMENSIONS.0,
                    FALLBACK_IMAGE_DIMENSIONS.1,
                    MangaMediaType::StaticImage,
                )
            };

            self.dimension_cache.insert(idx, fallback);
            self.dim_pending.remove(&idx);
            seeded = seeded.saturating_add(1);
        }

        seeded
    }
}

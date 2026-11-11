//! Draining the decode queue into the UI, including the dimension cache prime.

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
    /// Drain queued work products: decoded images and dimension results, both bounded.
    /// The only way the caller asks for work products; counts come from `backlog`.
    pub fn take(&mut self, max_decoded: usize, max_dimension_results: usize) -> Taken {
        let (decoded, decoded_dimension_updates) = self.take_decoded(max_decoded);
        let mut dimension_updates = self.apply_dimension_results(max_dimension_results);
        dimension_updates.extend(decoded_dimension_updates);
        dimension_updates.sort_unstable();
        dimension_updates.dedup();

        Taken {
            decoded,
            dimension_updates,
        }
    }

    /// Decoded images ready for GPU upload, bounded by `max_items` (0 takes none).
    /// Also returns indices whose cached source dimensions changed.
    pub(crate) fn take_decoded(&mut self, max_items: usize) -> (Vec<DecodedImage>, Vec<usize>) {
        let max_items = max_items.min(MAX_PENDING_UPLOADS);
        let mut results = Vec::with_capacity(max_items);
        let mut dimension_updates = Vec::with_capacity(max_items);

        for _ in 0..max_items {
            match self.result_rx.try_recv() {
                Ok(decoded) => {
                    // Cache dimensions and media type for stable layout. A corrupt header must not
                    // land in `dimension_cache`; prefer the decoded texture size, else keep the
                    // dimensions the layout already has.
                    let new_dims =
                        sanitize_probed_dimensions(decoded.original_width, decoded.original_height)
                            .or_else(|| sanitize_probed_dimensions(decoded.width, decoded.height))
                            .map(|(w, h)| (w, h, decoded.media_type));
                    let changed = new_dims.is_some_and(|new_dims| {
                        self.dimension_cache.get(&decoded.index).map_or(
                            true,
                            |(old_w, old_h, old_mt)| {
                                if *old_w == new_dims.0
                                    && *old_h == new_dims.1
                                    && *old_mt == new_dims.2
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
                                        let new_aspect =
                                            new_dims.0 as f32 / new_dims.1.max(1) as f32;
                                        if (old_aspect - new_aspect).abs() <= 0.003 {
                                            return false;
                                        }
                                    }
                                }

                                true
                            },
                        )
                    });

                    if let (true, Some(new_dims)) = (changed, new_dims) {
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
    pub fn prime_dimension_cache(&mut self, image_list: &[PathBuf]) -> usize {
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
                if let Some((w, h)) = cached.and_then(|(w, h)| sanitize_probed_dimensions(w, h)) {
                    let media_type = match kind {
                        CachedMediaKind::Image => MangaMediaType::StaticImage,
                        CachedMediaKind::Video => MangaMediaType::Video,
                    };
                    self.dimension_cache.insert(*idx, (w, h, media_type));
                }
            }
        }

        // Cache first batch synchronously for immediate layout. WebP stays async:
        // animated WebP folders can contain many costly headers, and folder travel
        // must paint the destination before masonry warm-up does heavier work.
        let initial_probe_items: Vec<(usize, PathBuf)> = image_list
            .iter()
            .enumerate()
            .filter(|(idx, path)| {
                !self.dimension_cache.contains_key(idx)
                    && (is_supported_video(path) || is_supported_image(path))
                    && !Self::path_is_webp(path)
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
                    let dims = Self::probe_video_dimensions(*idx, path);
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
            // Skip unusable headers: a later probe/decode fills the entry instead of poisoning it.
            let Some((w, h, media_type)) = opt_dims else {
                continue;
            };
            if let Some((w, h)) = sanitize_probed_dimensions(w, h) {
                self.dimension_cache.insert(idx, (w, h, media_type));
            }
        }

        // The rest will be cached on-demand when media is loaded
        // or when manga_get_image_display_height is called
        self.cached_dimensions_count(image_list.len())
    }
}

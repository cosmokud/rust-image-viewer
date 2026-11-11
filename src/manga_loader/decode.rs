//! The decode path for one tile plus video dimension probing.

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
    pub(crate) fn probe_image_dimensions_cached(path: &std::path::Path) -> Option<(u32, u32)> {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Image) {
            return Some((w, h));
        }

        let dims = probe_image_dimensions(path);
        if let Some((w, h)) = dims {
            store_cached_dimensions(path, CachedMediaKind::Image, w, h);
        }

        dims
    }

    pub(crate) fn path_is_webp(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("webp"))
    }

    /// Dimensions without touching the gstreamer gate: cache, shell metadata, filename hint.
    /// Counted once per gstreamer pipeline start: the summary line reports probes/s.
    pub(crate) fn note_gst_probe() {
        crate::diag::count(crate::diag::Counter::GstProbes, 1);
    }

    pub(crate) fn cheap_video_dimensions(path: &std::path::Path) -> (u32, u32) {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Video) {
            return (w, h);
        }

        if let Some((w, h)) = probe_video_dimensions_without_gstreamer(path) {
            if w > 0 && h > 0 {
                store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                return (w, h);
            }
        }

        Self::fallback_video_dimensions(path).unwrap_or(FALLBACK_VIDEO_DIMENSIONS)
    }

    /// Load a single image on a worker thread.
    /// For video files, this extracts the first frame as a thumbnail placeholder.
    pub(crate) fn load_single_image(req: &LoadRequest) -> Option<DecodedImage> {
        // Diag forensics wrap: a failed decode returns `None` from several depths, and the log has
        // to name the file, so wrap the whole thing instead of instrumenting each exit.
        let started = Instant::now();
        let declared = Self::probe_image_dimensions_cached(&req.path);
        // Breadcrumb: a hard kill leaves the offending file as the last line in the log.
        crate::diag::note_file(req.index, "decode-start", || {
            format!(
                "idx={} file={} bytes={} declared={}",
                req.index,
                diag_file_label(&req.path),
                std::fs::metadata(&req.path)
                    .map(|meta| meta.len())
                    .unwrap_or(0),
                declared.map_or_else(|| "?".to_string(), |(w, h)| format!("{w}x{h}"))
            )
        });
        let decoded = Self::load_single_image_inner(req);
        let elapsed = started.elapsed();

        let slow = elapsed >= DECODE_FORENSICS_SLOW;
        if slow || decoded.is_none() {
            crate::diag::trace_file("decode-file", || {
                let (width, height, rgba_bytes) = decoded
                    .as_ref()
                    .map(|image| (image.width, image.height, image.pixels.len()))
                    .unwrap_or((0, 0, 0));
                let declared = declared
                    .map(|(w, h)| format!("{}x{}", w, h))
                    .unwrap_or_else(|| "?".to_string());
                format!(
                    "idx={} file={} declared={} decoded={}x{} rgba={}B elapsed={}ms outcome={}",
                    req.index,
                    diag_file_label(&req.path),
                    declared,
                    width,
                    height,
                    rgba_bytes,
                    elapsed.as_millis(),
                    if decoded.is_some() { "ok" } else { "failed" }
                )
            });
        }

        decoded
    }

    pub(crate) fn load_single_image_inner(req: &LoadRequest) -> Option<DecodedImage> {
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
                // One permit for the whole tile: extraction and its inner dimension probe share it.
                // A gate timeout starts this index's cooldown instead of retrying next pass.
                let tile_permit = probe_permit_for_index(req.index);
                // No permit (gate timeout or cooldown) is a transient miss, not a bad file: the
                // payload still carries cheap dimensions, but the tile must stay re-requestable.
                let deferred = tile_permit.is_none();
                match tile_permit.as_ref().and_then(|permit| {
                    Self::extract_video_first_frame(&req.path, effective_texture_side, Some(permit))
                }) {
                    Some(frame) => {
                        clear_probe_cooldown(req.index);
                        let resize_started = Instant::now();
                        let (width, height, pixels) = downscale_rgba_if_needed(
                            frame.width,
                            frame.height,
                            &frame.pixels,
                            effective_texture_side,
                            req.downscale_filter,
                        );
                        let resize_time = resize_started.elapsed();

                        Some(DecodedImage {
                            index: req.index,
                            path: req.path.clone(),
                            pixels: pixels.into_owned(),
                            width,
                            height,
                            original_width: frame.original_width,
                            original_height: frame.original_height,
                            media_type: MangaMediaType::Video,
                            requested_side: effective_texture_side,
                            queue_wait: Duration::ZERO,
                            decode_time: Duration::ZERO,
                            resize_time,
                            deferred: false,
                        })
                    }
                    None => {
                        // Fallback: use cheap dimensions only. Do not call the full probe here,
                        // because that can re-run first-frame extraction for already-failed videos.
                        // A gate timeout also lands here: no decode worker may wait twice.
                        let (original_width, original_height) =
                            Self::cheap_video_dimensions(&req.path);

                        Some(DecodedImage {
                            index: req.index,
                            path: req.path.clone(),
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
                            deferred,
                        })
                    }
                }
            }
            MediaType::Image => {
                // Get original dimensions from file header first (fast, no decode)
                let (original_width, original_height) =
                    Self::probe_image_dimensions_cached(&req.path)?;

                // Animated media (GIF and animated WebP) only decode their first frame here:
                // a tile needs one frame, and the full animation is loaded lazily by
                // `manga_update_animated_textures` when the user actually focuses on it.
                // Loading the full animation in a scroll pass made one long GIF pay a whole
                // stream count plus window composite for a thumbnail nobody sees animate.
                let is_animated_webp = LoadedImage::is_animated_webp(&req.path);
                let is_gif = req
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"));

                let downscale_filter = req.downscale_filter;
                let gif_filter = req.gif_filter;

                let mut img = if is_animated_webp || is_gif {
                    LoadedImage::load_first_frame_thumbnail(
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
                img.reset_animation_to_first_frame();

                // Determine if this is an animated image. For the first-frame-only loads the
                // frame count is 1 by construction, so the header check above decides.
                let manga_media_type = if is_animated_webp || is_gif || img.is_animated() {
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

                Some(DecodedImage {
                    index: req.index,
                    path: req.path.clone(),
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
                    deferred: false,
                })
            }
        }
    }

    pub(crate) fn quantize_target_texture_side(
        target_texture_side: u32,
        max_texture_side: u32,
    ) -> u32 {
        let max_side = max_texture_side.max(1);
        let target = target_texture_side.max(1).min(max_side);

        for &bucket in LOD_SIDE_BUCKETS {
            if bucket >= target && bucket <= max_side {
                return bucket;
            }
        }

        max_side
    }

    pub(crate) fn limit_target_texture_side_for_index(
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
    /// Uses a lightweight first-frame probe, then falls back to filename heuristics. One permit
    /// covers the whole chain: the first-frame fallback inside must not wait on the gate again.
    pub(crate) fn probe_video_dimensions(
        index: usize,
        path: &std::path::Path,
    ) -> Option<(u32, u32)> {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Video) {
            return Some((w, h));
        }

        // Cooling down after a gate timeout: keep the cheap size layout already has instead of
        // re-probing (and re-laying-out) on every pass.
        if probe_cooldown_active(index) {
            return Some(Self::cheap_video_dimensions(path));
        }

        let mut owned = None;
        let permit = probe_permit(None, &mut owned);
        if permit.is_none() {
            start_probe_cooldown(index);
        }
        if let Some(permit) = permit {
            if gstreamer_runtime_available() {
                Self::note_gst_probe();
                if let Some((w, h)) = probe_video_dimensions_with_gstreamer(path) {
                    if w > 0 && h > 0 {
                        store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                        clear_probe_cooldown(index);
                        return Some((w, h));
                    }
                }
            }

            if let Some((w, h)) = probe_video_dimensions_without_gstreamer(path) {
                if w > 0 && h > 0 {
                    store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                    clear_probe_cooldown(index);
                    return Some((w, h));
                }
            }

            let probed = Self::extract_video_first_frame(path, 256, Some(permit))
                .map(|frame| (frame.original_width, frame.original_height));

            if let Some((w, h)) = probed {
                if w > 0 && h > 0 {
                    store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                    clear_probe_cooldown(index);
                    return Some((w, h));
                }
            }
        }

        // The filename heuristic is a guess, not a probe: caching it pinned a loadable video at
        // the wrong size for the whole session.
        Self::fallback_video_dimensions(path)
    }

    pub(crate) fn fallback_video_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
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

    /// First frame of a video tile, entirely delegated to the app's one alpha-aware extractor.
    ///
    /// The permit is only claimed here so a tile's whole probe chain (dimensions, then the frame)
    /// shares one gate slot; the decode runs inside `extract_video_first_frame_thumbnail`, which
    /// demotes the alpha decodebin and retries opaque when the stream carries a VP8/VP9 alpha
    /// plane. Keeping a second, alpha-unaware copy of that pipeline here is what left every such
    /// webm on a permanent placeholder: this copy aborted on the alphacombine bus error ("Cannot
    /// handle streams without an initial alpha buffer") instead of falling back, while playback
    /// of the very same file worked.
    pub(crate) fn extract_video_first_frame(
        path: &std::path::Path,
        max_texture_side: u32,
        permit: Option<&VideoProbePermit>,
    ) -> Option<FirstFrame> {
        // Released by the guard on every path below, including the early returns.
        let mut owned = None;
        let _probe_permit = probe_permit(permit, &mut owned)?;
        crate::extract_video_first_frame_thumbnail(path, max_texture_side)
    }
}

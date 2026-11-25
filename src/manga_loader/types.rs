//! Plain data types exchanged with the UI: media type, decoded image, load/dim requests.

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

/// A decoded image ready for GPU upload (pre-converted & premultiplied off the UI thread).
#[derive(Clone)]
pub struct DecodedImage {
    pub index: usize,
    pub path: PathBuf,
    pub color_image: egui::ColorImage,
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
    /// Payload carries no texture because a gated probe could not run (permit timeout or cooldown),
    /// not because the file failed: it must not spend the index's retry budget.
    pub deferred: bool,
}

impl DecodedImage {
    #[inline]
    pub fn pixel_bytes(&self) -> usize {
        self.color_image
            .pixels
            .len()
            .saturating_mul(std::mem::size_of::<egui::Color32>())
    }

    #[inline]
    pub fn has_pixels(&self) -> bool {
        !self.color_image.pixels.is_empty()
    }
}

/// Minimum allocation size routed to the background free thread (~512 KiB).
pub(crate) const LARGE_DROP_MIN_BYTES: usize = 512 * 1024;

/// Lowers the current worker thread priority so background decode/probe/free work never
/// starves the UI, OpenGL, or DWM compositor threads at high refresh rates.
pub(crate) fn set_current_thread_below_normal() {
    #[cfg(windows)]
    unsafe {
        winapi::um::processthreadsapi::SetThreadPriority(
            winapi::um::processthreadsapi::GetCurrentThread(),
            winapi::um::winbase::THREAD_PRIORITY_BELOW_NORMAL as winapi::ctypes::c_int,
        );
    }
}

/// Worker count for background decode pools, leaving cores headroom for UI + OpenGL + DWM.
pub(crate) fn decode_worker_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .saturating_sub(2)
        .clamp(1, 6)
}

/// Drops an owned large buffer on a dedicated below-normal-priority background thread so the
/// UI thread never stalls inside synchronous OS deallocation (`VirtualFree`).
pub(crate) fn defer_large_drop<T: Send + 'static>(item: T) {
    use std::any::Any;
    use std::sync::mpsc::{self, Sender};
    use std::sync::OnceLock;

    static DROP_TX: OnceLock<Sender<Box<dyn Any + Send>>> = OnceLock::new();
    let tx = DROP_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn Any + Send>>();
        let _ = std::thread::Builder::new()
            .name("bg-large-dropper".to_string())
            .spawn(move || {
                set_current_thread_below_normal();
                while let Ok(boxed) = rx.recv() {
                    drop(boxed);
                }
            });
        tx
    });
    let _ = tx.send(Box::new(item));
}

pub(crate) fn drop_color_image_off_thread(image: egui::ColorImage) {
    let bytes = image
        .pixels
        .len()
        .saturating_mul(std::mem::size_of::<egui::Color32>());
    if bytes >= LARGE_DROP_MIN_BYTES {
        defer_large_drop(image);
    }
}

pub(crate) fn drop_decoded_batch_off_thread(items: Vec<DecodedImage>) {
    let total_bytes: usize = items.iter().map(DecodedImage::pixel_bytes).sum();
    if total_bytes >= LARGE_DROP_MIN_BYTES {
        defer_large_drop(items);
    }
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
pub(crate) struct DimRequest {
    pub(crate) generation: usize,
    pub(crate) items: Vec<(usize, PathBuf)>,
}

pub(crate) struct DimResult {
    pub(crate) generation: usize,
    pub(crate) items: Vec<(usize, u32, u32, MangaMediaType)>,
}

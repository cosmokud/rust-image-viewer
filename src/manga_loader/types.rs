//! Plain data types exchanged with the UI: media type, decoded image, load/dim requests.

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
    pub path: PathBuf,
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
    /// Payload carries no texture because a gated probe could not run (permit timeout or cooldown),
    /// not because the file failed: it must not spend the index's retry budget.
    pub deferred: bool,
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

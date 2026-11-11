//! Loader constants and the LOD side buckets.

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

/// Texture-side buckets used for masonry/strip LOD requests.
/// Requests are rounded up to the next bucket to avoid churn from tiny deltas.
pub const LOD_SIDE_BUCKETS: &[u32] = &[
    96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096,
];

/// Layout fallbacks used when a header probe yields nothing usable.
pub(crate) const FALLBACK_IMAGE_DIMENSIONS: (u32, u32) = (1200, 1600);

pub(crate) const FALLBACK_VIDEO_DIMENSIONS: (u32, u32) = (1920, 1080);

/// Sanity ceiling for a probed pixel count and side length; real pages stay far below.
pub(crate) const MAX_PROBED_PIXELS: u64 = 400_000_000;

pub(crate) const MAX_PROBED_SIDE: u32 = 65535;

/// Aspect ceiling (long:short); a corrupt header outside it poisons masonry layout.
pub(crate) const MAX_PROBED_ASPECT: u32 = 100;

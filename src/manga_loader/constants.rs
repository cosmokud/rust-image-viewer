//! Loader constants and the LOD side buckets.

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

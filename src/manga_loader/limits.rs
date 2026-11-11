//! Constants and capacity limits for the loader (queue sizes, preload multipliers, revive delays).

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

/// Maximum number of decoded images to hold in memory awaiting GPU upload.
/// This bounds memory usage even if the main thread is slow to consume results.
pub(crate) const MAX_PENDING_UPLOADS: usize = 128;

/// Maximum number of images to keep in the texture cache.
/// Beyond this, the oldest entries are evicted to control VRAM usage.
pub(crate) const DEFAULT_CACHED_TEXTURES: usize = 128;

/// Small dedicated queue for visible-item retries that should not wait behind preload churn.
pub(crate) const URGENT_REQUEST_QUEUE_CAPACITY: usize = 128;

/// Directional preload window multipliers derived from the actual number of visible items.
/// The scroll direction gets 2x visible items ahead; the reverse direction gets 1x behind.
pub(crate) const PRELOAD_LOOK_AHEAD_MULTIPLIER: usize = 2;

pub(crate) const PRELOAD_LOOK_BEHIND_MULTIPLIER: usize = 1;

/// Strip mode uses viewport coverage instead of whole-item counts so partial pages do not
/// inflate preload windows. Example: 1.5 visible pages -> 3 ahead, 2 behind.
pub(crate) const STRIP_PRELOAD_LOOK_AHEAD_MULTIPLIER: f32 = PRELOAD_LOOK_AHEAD_MULTIPLIER as f32;

pub(crate) const STRIP_PRELOAD_LOOK_BEHIND_MULTIPLIER: f32 = PRELOAD_LOOK_BEHIND_MULTIPLIER as f32;

/// Clamp the directional preload windows to keep memory usage bounded.
pub(crate) const MIN_PRELOAD_AHEAD: usize = 12;

pub(crate) const MIN_PRELOAD_BEHIND: usize = 6;

pub(crate) const MAX_PRELOAD_AHEAD: usize = 256;

pub(crate) const MAX_PRELOAD_BEHIND: usize = 128;

/// If the visible index jumps by more than this many pages, treat it as a "large jump".
///
/// For large jumps we want latency (load the target page ASAP) over throughput (prefetch neighbors).
pub(crate) const LARGE_JUMP_INDEX_THRESHOLD: usize = 32;

/// Maximum number of dimension probe items to include in a single request.
/// Larger values increase background throughput but can increase burstiness.
pub(crate) const DIM_REQUEST_BATCH_SIZE: usize = 64;

/// Maximum number of dimension results bundled into a single result message.
pub(crate) const DIM_RESULT_CHUNK_SIZE: usize = 64;

/// Bounded tries for handing a dimension chunk to the (64-slot) result channel before the
/// chunk is parked for the next pass: a blocking `send` here parks the only dim worker and
/// stalls every later probe behind it until the main thread happens to drain.
pub(crate) const DIM_RESULT_SEND_MAX_ATTEMPTS: u32 = 100;

pub(crate) const DIM_RESULT_SEND_RETRY_SLEEP_MS: u64 = 2;

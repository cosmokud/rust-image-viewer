//! Texture-cache byte budget constants.

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

/// Cache entry limits shared with the byte-aware budget clamps.
pub(crate) const MANGA_CACHE_MIN_ENTRIES: usize = 64;

pub(crate) const MANGA_CACHE_MAX_ENTRIES: usize = 1024;

/// Byte-aware cache budgets: oversized textures must not inflate entry-count targets.
pub(crate) const MASONRY_CACHE_BUDGET_BYTES_IDLE: usize = 768 * 1024 * 1024;

pub(crate) const MASONRY_CACHE_BUDGET_BYTES_NAV: usize = 512 * 1024 * 1024;

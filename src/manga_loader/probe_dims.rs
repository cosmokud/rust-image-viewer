//! Dimension probing: sanitising probed sizes and the fallback policy.

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

/// Reject probed dimensions a corrupt/garbage header can produce.
///
/// A 0 or absurd side clamps the texture request target to ~1, which the loader then treats as
/// "satisfied" and never re-requests; the tile stays a placeholder for good.
pub(crate) fn sanitize_probed_dimensions(width: u32, height: u32) -> Option<(u32, u32)> {
    if width < 1 || height < 1 || width > MAX_PROBED_SIDE || height > MAX_PROBED_SIDE {
        return None;
    }
    if u64::from(width) * u64::from(height) > MAX_PROBED_PIXELS {
        return None;
    }
    let (long, short) = if width >= height {
        (width, height)
    } else {
        (height, width)
    };
    if long > short.saturating_mul(MAX_PROBED_ASPECT) {
        return None;
    }
    Some((width, height))
}

/// Probed dimensions, or the media-type placeholder when the probe was unusable.
pub(crate) fn probed_or_fallback_dimensions(
    dims: Option<(u32, u32)>,
    media_type: MangaMediaType,
) -> (u32, u32) {
    dims.and_then(|(width, height)| sanitize_probed_dimensions(width, height))
        .unwrap_or(if media_type == MangaMediaType::Video {
            FALLBACK_VIDEO_DIMENSIONS
        } else {
            FALLBACK_IMAGE_DIMENSIONS
        })
}

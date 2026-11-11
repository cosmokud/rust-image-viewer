//! Diag tracing hooks used by the loader.

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

/// `RIV_MANGA_TRACE=1` diagnostics: name the repair rule that fired, at most once per second.
/// Off by default; the call sites are already slow paths, never per-frame loops.
pub fn trace_rule(rule: &str, index: usize) {
    use std::sync::atomic::AtomicU64;
    use std::sync::OnceLock;

    pub(crate) static ENABLED: OnceLock<bool> = OnceLock::new();
    pub(crate) static ORIGIN: OnceLock<Instant> = OnceLock::new();
    /// Sentinel `u64::MAX` = never printed, so the first rule is not swallowed by the window.
    pub(crate) static LAST_MS: AtomicU64 = AtomicU64::new(u64::MAX);

    if !*ENABLED.get_or_init(|| crate::diag::flag("RIV_MANGA_TRACE")) {
        return;
    }

    let elapsed_ms = ORIGIN.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let mut last = LAST_MS.load(Ordering::Relaxed);
    loop {
        if last != u64::MAX && elapsed_ms.saturating_sub(last) < 1000 {
            return;
        }
        match LAST_MS.compare_exchange_weak(last, elapsed_ms, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(actual) => last = actual,
        }
    }

    if !crate::diag::trace("manga-trace", &format!("{rule} idx={index}")) {
        eprintln!("[manga-trace] {rule} idx={index}");
    }
}

/// One-line file identity for diag forensics: name + on-disk size (one stat, diag paths only).
pub(crate) fn diag_file_label(path: &std::path::Path) -> String {
    let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    format!(
        "{} ({}B)",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("?"),
        bytes
    )
}

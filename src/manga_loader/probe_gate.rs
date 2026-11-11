//! The video-probe permit gate: how many first-frame probes run at once, and the per-index cooldown that keeps a failing probe from being retried.

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

/// Caps concurrent gstreamer first-frame probes. Each probe opens the video, drives a pipeline
/// to `Paused` and waits up to 1.5s on the bus; several at once saturate the disk and starve
/// every other read, which is what froze the gallery/masonry/long-strip modes. Scaled off the
/// core count because the old fixed 2 serialized a whole gallery behind two pipelines.
pub(crate) const MIN_CONCURRENT_VIDEO_PROBES: usize = 2;

pub(crate) const MAX_CONCURRENT_VIDEO_PROBES: usize = 8;

/// Bounded wait: a rayon worker must never park indefinitely behind the gate.
pub(crate) const VIDEO_PROBE_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(400);

/// Cooldown after a gate timeout: the index keeps its cheap dimensions and no gated probe is
/// retried until this elapses. Retrying on the next pass saturated the gate and re-ran a
/// masonry/strip relayout every pass (20 timeouts in one session).
pub(crate) const VIDEO_PROBE_COOLDOWN: Duration = Duration::from_secs(5);

/// Index -> cooldown deadline. Global because the decode workers and the single dimension worker
/// probe the same indices; cleared wholesale when the media list changes.
pub(crate) static VIDEO_PROBE_COOLDOWN_UNTIL: std::sync::OnceLock<
    parking_lot::Mutex<HashMap<usize, Instant>>,
> = std::sync::OnceLock::new();

pub(crate) fn probe_cooldown_map() -> &'static parking_lot::Mutex<HashMap<usize, Instant>> {
    VIDEO_PROBE_COOLDOWN_UNTIL.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// `true` while this index must skip gated video probes and keep its cheap dimensions.
pub(crate) fn probe_cooldown_active(index: usize) -> bool {
    let mut map = probe_cooldown_map().lock();
    match map.get(&index).copied() {
        Some(deadline) if deadline > Instant::now() => true,
        Some(_) => {
            map.remove(&index);
            false
        }
        None => false,
    }
}

pub(crate) fn start_probe_cooldown(index: usize) {
    probe_cooldown_map()
        .lock()
        .insert(index, Instant::now() + VIDEO_PROBE_COOLDOWN);
}

pub(crate) fn clear_probe_cooldown(index: usize) {
    probe_cooldown_map().lock().remove(&index);
}

/// Drop cooldowns for indices outside the prefetch window: a video scrolled back into view gets a
/// fresh chance at a real probe even though the cooldown has not expired yet.
pub(crate) fn clear_probe_cooldowns_outside(start: usize, end: usize) {
    let mut map = probe_cooldown_map().lock();
    if map.is_empty() {
        return;
    }
    map.retain(|index, _| *index >= start && *index < end);
}

pub(crate) fn clear_all_probe_cooldowns() {
    probe_cooldown_map().lock().clear();
}

pub(crate) fn video_probe_permit_count() -> usize {
    std::thread::available_parallelism()
        .map_or(MIN_CONCURRENT_VIDEO_PROBES, |cores| cores.get() / 4)
        .clamp(MIN_CONCURRENT_VIDEO_PROBES, MAX_CONCURRENT_VIDEO_PROBES)
}

// Probe permits held by the calling thread; the single-permit-per-tile rule means 1.
thread_local! {
    pub(crate) static PROBE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct VideoProbeGate {
    pub(crate) available: parking_lot::Mutex<usize>,
    pub(crate) released: parking_lot::Condvar,
}

impl VideoProbeGate {
    /// Parks on a condvar instead of the old `try_acquire` + 4ms sleep loop. `None` (gate still
    /// busy after the bounded wait) makes the caller degrade to cheap dimensions, never block.
    pub(crate) fn acquire_timeout(
        self: &std::sync::Arc<Self>,
        timeout: Duration,
    ) -> Option<VideoProbePermit> {
        let started = Instant::now();
        let deadline = started + timeout;
        let mut available = self.available.lock();
        loop {
            if *available > 0 {
                *available -= 1;
                let depth = PROBE_DEPTH.with(|depth| {
                    depth.set(depth.get() + 1);
                    depth.get()
                });
                let waited = started.elapsed();
                crate::diag::count(crate::diag::Counter::ProbeWaits, 1);
                if crate::diag::enabled() {
                    crate::diag::write_line(&format!(
                        "probe permit wait={}ms depth={depth}",
                        waited.as_millis()
                    ));
                }
                return Some(VideoProbePermit {
                    gate: std::sync::Arc::clone(self),
                });
            }
            let now = Instant::now();
            if now >= deadline {
                crate::diag::count(crate::diag::Counter::ProbeTimeouts, 1);
                if crate::diag::enabled() {
                    crate::diag::write_line(&format!(
                        "probe permit timeout after {}ms: degrading to cheap dims",
                        started.elapsed().as_millis()
                    ));
                }
                return None;
            }
            self.released.wait_for(&mut available, deadline - now);
        }
    }

    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        *self.available.lock()
    }
}

/// A held video-probe permit. Deliberately lifetime-free: one tile hands the same permit down
/// `probe_video_dimensions` -> `extract_video_first_frame` instead of queueing twice.
/// Holds its own gate handle so a test gate gets its permits back, not the global one.
pub(crate) struct VideoProbePermit {
    pub(crate) gate: std::sync::Arc<VideoProbeGate>,
}

pub(crate) type VideoProbeGateCell = std::sync::OnceLock<std::sync::Arc<VideoProbeGate>>;

pub(crate) static VIDEO_PROBE_GATE: VideoProbeGateCell = std::sync::OnceLock::new();

pub(crate) fn video_probe_gate() -> &'static std::sync::Arc<VideoProbeGate> {
    VIDEO_PROBE_GATE
        .get_or_init(|| std::sync::Arc::new(VideoProbeGate::new(video_probe_permit_count())))
}

/// Permit for one probe chain, bounded by `VIDEO_PROBE_ACQUIRE_TIMEOUT`.
pub(crate) fn acquire_probe_permit() -> Option<VideoProbePermit> {
    video_probe_gate().acquire_timeout(VIDEO_PROBE_ACQUIRE_TIMEOUT)
}

/// Reuse the caller's permit, else take one. `None` = gate busy past the bounded wait.
pub(crate) fn probe_permit<'a>(
    ticket: Option<&'a VideoProbePermit>,
    owned: &'a mut Option<VideoProbePermit>,
) -> Option<&'a VideoProbePermit> {
    if ticket.is_some() {
        return ticket;
    }

    *owned = acquire_probe_permit();
    owned.as_ref()
}

/// Gate permit for one tile. A timeout starts the index cooldown, so the next pass reuses the
/// cheap dimensions instead of queueing behind the same saturated gate.
pub(crate) fn probe_permit_for_index(index: usize) -> Option<VideoProbePermit> {
    if probe_cooldown_active(index) {
        return None;
    }

    let permit = acquire_probe_permit();
    if permit.is_none() {
        start_probe_cooldown(index);
    }
    permit
}

/// Runs `probe` under the shared video-probe gate. `None` when the gate stayed busy past the
/// bounded wait, so an off-frame caller degrades instead of adding more disk load.
///
/// Worker threads only: the wait is bounded but not short, so never call this from the frame
/// loop — a UI-thread caller must schedule the probe instead.
pub fn with_video_probe_gate<T>(probe: impl FnOnce() -> T) -> Option<T> {
    let _permit = acquire_probe_permit()?;
    Some(probe())
}

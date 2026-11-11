//! High-performance parallel image loader for Manga Reading Mode.
//!
//! This module implements a sophisticated multi-threaded image loading system
//! optimized for seamless scrolling through hundreds of images. Key features:
//!
//! - **Lock-free communication**: Uses crossbeam channels for zero-contention
//!   message passing between the main thread and worker threads.
//!
//! - **Parallel decoding**: Uses Rayon's thread pool for parallel image decoding,
//!   utilizing all available CPU cores for maximum throughput.
//!
//! - **Priority-based loading**: Images closer to the viewport are loaded first,
//!   with scroll direction awareness for predictive prefetching.
//!
//! - **Batch texture uploads**: Decoded images are batched for GPU upload to
//!   minimize driver overhead and prevent frame drops.
//!
//! - **Memory-efficient caching**: LRU-style eviction keeps memory bounded
//!   while maximizing cache hit rate.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError};
use hashbrown::{HashMap, HashSet};
use image::imageops::FilterType;
use lru::LruCache;
use parking_lot::RwLock;
use rayon::prelude::*;

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
    extract_video_first_frame_without_gstreamer, probe_video_dimensions_with_gstreamer,
    probe_video_dimensions_without_gstreamer,
};
use crate::viewer_mode::MangaLayoutMode;
use crate::FileStamp;

/// Caps concurrent gstreamer first-frame probes. Each probe opens the video, drives a pipeline
/// to `Paused` and waits up to 1.5s on the bus; several at once saturate the disk and starve
/// every other read, which is what froze the gallery/masonry/long-strip modes. Scaled off the
/// core count because the old fixed 2 serialized a whole gallery behind two pipelines.
const MIN_CONCURRENT_VIDEO_PROBES: usize = 2;
const MAX_CONCURRENT_VIDEO_PROBES: usize = 8;
/// Bounded wait: a rayon worker must never park indefinitely behind the gate.
const VIDEO_PROBE_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(400);

/// Cooldown after a gate timeout: the index keeps its cheap dimensions and no gated probe is
/// retried until this elapses. Retrying on the next pass saturated the gate and re-ran a
/// masonry/strip relayout every pass (20 timeouts in one session).
const VIDEO_PROBE_COOLDOWN: Duration = Duration::from_secs(5);

/// Index -> cooldown deadline. Global because the decode workers and the single dimension worker
/// probe the same indices; cleared wholesale when the media list changes.
static VIDEO_PROBE_COOLDOWN_UNTIL: std::sync::OnceLock<
    parking_lot::Mutex<HashMap<usize, Instant>>,
> = std::sync::OnceLock::new();

fn probe_cooldown_map() -> &'static parking_lot::Mutex<HashMap<usize, Instant>> {
    VIDEO_PROBE_COOLDOWN_UNTIL.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// `true` while this index must skip gated video probes and keep its cheap dimensions.
fn probe_cooldown_active(index: usize) -> bool {
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

fn start_probe_cooldown(index: usize) {
    probe_cooldown_map()
        .lock()
        .insert(index, Instant::now() + VIDEO_PROBE_COOLDOWN);
}

fn clear_probe_cooldown(index: usize) {
    probe_cooldown_map().lock().remove(&index);
}

/// Drop cooldowns for indices outside the prefetch window: a video scrolled back into view gets a
/// fresh chance at a real probe even though the cooldown has not expired yet.
fn clear_probe_cooldowns_outside(start: usize, end: usize) {
    let mut map = probe_cooldown_map().lock();
    if map.is_empty() {
        return;
    }
    map.retain(|index, _| *index >= start && *index < end);
}

fn clear_all_probe_cooldowns() {
    probe_cooldown_map().lock().clear();
}

fn video_probe_permit_count() -> usize {
    std::thread::available_parallelism()
        .map_or(MIN_CONCURRENT_VIDEO_PROBES, |cores| cores.get() / 4)
        .clamp(MIN_CONCURRENT_VIDEO_PROBES, MAX_CONCURRENT_VIDEO_PROBES)
}

// Probe permits held by the calling thread; the single-permit-per-tile rule means 1.
thread_local! {
    static PROBE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct VideoProbeGate {
    available: parking_lot::Mutex<usize>,
    released: parking_lot::Condvar,
}

impl VideoProbeGate {
    fn new(permits: usize) -> Self {
        Self {
            available: parking_lot::Mutex::new(permits),
            released: parking_lot::Condvar::new(),
        }
    }

    /// Parks on a condvar instead of the old `try_acquire` + 4ms sleep loop. `None` (gate still
    /// busy after the bounded wait) makes the caller degrade to cheap dimensions, never block.
    fn acquire_timeout(self: &std::sync::Arc<Self>, timeout: Duration) -> Option<VideoProbePermit> {
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
    fn available(&self) -> usize {
        *self.available.lock()
    }
}

/// A held video-probe permit. Deliberately lifetime-free: one tile hands the same permit down
/// `extract_video_first_frame` -> `probe_video_dimensions_fast` instead of queueing twice.
/// Holds its own gate handle so a test gate gets its permits back, not the global one.
struct VideoProbePermit {
    gate: std::sync::Arc<VideoProbeGate>,
}

impl Drop for VideoProbePermit {
    fn drop(&mut self) {
        PROBE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        *self.gate.available.lock() += 1;
        self.gate.released.notify_one();
    }
}

type VideoProbeGateCell = std::sync::OnceLock<std::sync::Arc<VideoProbeGate>>;
static VIDEO_PROBE_GATE: VideoProbeGateCell = std::sync::OnceLock::new();

fn video_probe_gate() -> &'static std::sync::Arc<VideoProbeGate> {
    VIDEO_PROBE_GATE
        .get_or_init(|| std::sync::Arc::new(VideoProbeGate::new(video_probe_permit_count())))
}

/// Permit for one probe chain, bounded by `VIDEO_PROBE_ACQUIRE_TIMEOUT`.
fn acquire_probe_permit() -> Option<VideoProbePermit> {
    video_probe_gate().acquire_timeout(VIDEO_PROBE_ACQUIRE_TIMEOUT)
}

/// Reuse the caller's permit, else take one. `None` = gate busy past the bounded wait.
fn probe_permit<'a>(
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
fn probe_permit_for_index(index: usize) -> Option<VideoProbePermit> {
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

/// Maximum number of decoded images to hold in memory awaiting GPU upload.
/// This bounds memory usage even if the main thread is slow to consume results.
const MAX_PENDING_UPLOADS: usize = 128;

/// Maximum number of images to keep in the texture cache.
/// Beyond this, the oldest entries are evicted to control VRAM usage.
const DEFAULT_CACHED_TEXTURES: usize = 128;

/// Small dedicated queue for visible-item retries that should not wait behind preload churn.
const URGENT_REQUEST_QUEUE_CAPACITY: usize = 128;

/// Directional preload window multipliers derived from the actual number of visible items.
/// The scroll direction gets 2x visible items ahead; the reverse direction gets 1x behind.
const PRELOAD_LOOK_AHEAD_MULTIPLIER: usize = 2;
const PRELOAD_LOOK_BEHIND_MULTIPLIER: usize = 1;

/// Strip mode uses viewport coverage instead of whole-item counts so partial pages do not
/// inflate preload windows. Example: 1.5 visible pages -> 3 ahead, 2 behind.
const STRIP_PRELOAD_LOOK_AHEAD_MULTIPLIER: f32 = PRELOAD_LOOK_AHEAD_MULTIPLIER as f32;
const STRIP_PRELOAD_LOOK_BEHIND_MULTIPLIER: f32 = PRELOAD_LOOK_BEHIND_MULTIPLIER as f32;

/// Clamp the directional preload windows to keep memory usage bounded.
const MIN_PRELOAD_AHEAD: usize = 12;
const MIN_PRELOAD_BEHIND: usize = 6;
const MAX_PRELOAD_AHEAD: usize = 256;
const MAX_PRELOAD_BEHIND: usize = 128;

/// If the visible index jumps by more than this many pages, treat it as a "large jump".
///
/// For large jumps we want latency (load the target page ASAP) over throughput (prefetch neighbors).
const LARGE_JUMP_INDEX_THRESHOLD: usize = 32;

/// Maximum number of dimension probe items to include in a single request.
/// Larger values increase background throughput but can increase burstiness.
const DIM_REQUEST_BATCH_SIZE: usize = 64;

/// Maximum number of dimension results bundled into a single result message.
const DIM_RESULT_CHUNK_SIZE: usize = 64;

/// Bounded tries for handing a dimension chunk to the (64-slot) result channel before the
/// chunk is parked for the next pass: a blocking `send` here parks the only dim worker and
/// stalls every later probe behind it until the main thread happens to drain.
const DIM_RESULT_SEND_MAX_ATTEMPTS: u32 = 100;
const DIM_RESULT_SEND_RETRY_SLEEP_MS: u64 = 2;

/// What happened to a dimension chunk hand-off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DimSend {
    /// Accepted by the channel.
    Flushed,
    /// Channel still full: the chunk is queued for the next pass, never dropped.
    Parked,
    Disconnected,
}

/// Decode-queue waits above this are worth a diag line: a fast probe should never queue long.
const DECODE_QUEUE_WAIT_TRACE: Duration = Duration::from_millis(100);
/// Decode wall time above which the diag forensics line names the file.
const DECODE_FORENSICS_SLOW: Duration = Duration::from_millis(250);
/// Base backoff for decode retries after a failed/empty preload.
const PRELOAD_RETRY_BASE_DELAY_MS: u64 = 250;
/// Cap retry backoff so visible items recover quickly once they come into view.
const PRELOAD_RETRY_MAX_DELAY_MS: u64 = 4000;
/// Stop retrying media that repeatedly fails to decode; broken GIFs should not spin forever.
const PRELOAD_RETRY_MAX_ATTEMPTS: u8 = 3;
/// Video thumbnails are a preview, not a requirement: a video whose first frame cannot be decoded
/// still plays, so the ceiling above only *paces* re-attempts for it instead of ending them. The
/// delay doubles per revive and the count is bounded, so a genuinely broken video settles while a
/// one-off decoder hiccup no longer pins the tile as a placeholder for the session.
const VIDEO_THUMBNAIL_REVIVE_BASE_DELAY: Duration = Duration::from_secs(1);
const VIDEO_THUMBNAIL_REVIVE_MAX_DELAY: Duration = Duration::from_secs(10);
const VIDEO_THUMBNAIL_REVIVE_MAX: u32 = 4;
/// Verification re-reads allowed for a file that keeps failing at the *same* stamp.
/// A GIF that decodes but cannot be rendered keeps failing until it is rewritten, and the
/// file stamp is the only re-arm signal there is: one re-read per this interval, only for
/// failed indices, keeps the tile dead on every frame in between.
const FAILED_STAMP_RECHECK: Duration = Duration::from_secs(5);
/// Texture-side buckets used for masonry/strip LOD requests.
/// Requests are rounded up to the next bucket to avoid churn from tiny deltas.
pub const LOD_SIDE_BUCKETS: &[u32] = &[
    96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096,
];

/// Layout fallbacks used when a header probe yields nothing usable.
const FALLBACK_IMAGE_DIMENSIONS: (u32, u32) = (1200, 1600);
const FALLBACK_VIDEO_DIMENSIONS: (u32, u32) = (1920, 1080);
/// Sanity ceiling for a probed pixel count and side length; real pages stay far below.
const MAX_PROBED_PIXELS: u64 = 400_000_000;
const MAX_PROBED_SIDE: u32 = 65535;
/// Aspect ceiling (long:short); a corrupt header outside it poisons masonry layout.
const MAX_PROBED_ASPECT: u32 = 100;

/// Reject probed dimensions a corrupt/garbage header can produce.
///
/// A 0 or absurd side clamps the texture request target to ~1, which the loader then treats as
/// "satisfied" and never re-requests; the tile stays a placeholder for good.
fn sanitize_probed_dimensions(width: u32, height: u32) -> Option<(u32, u32)> {
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
fn probed_or_fallback_dimensions(
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

/// `RIV_MANGA_TRACE=1` diagnostics: name the repair rule that fired, at most once per second.
/// Off by default; the call sites are already slow paths, never per-frame loops.
pub fn trace_rule(rule: &str, index: usize) {
    use std::sync::atomic::AtomicU64;
    use std::sync::OnceLock;

    static ENABLED: OnceLock<bool> = OnceLock::new();
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    /// Sentinel `u64::MAX` = never printed, so the first rule is not swallowed by the window.
    static LAST_MS: AtomicU64 = AtomicU64::new(u64::MAX);

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
fn diag_file_label(path: &std::path::Path) -> String {
    let bytes = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    format!(
        "{} ({}B)",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("?"),
        bytes
    )
}

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
struct DimRequest {
    generation: usize,
    items: Vec<(usize, PathBuf)>,
}

struct DimResult {
    generation: usize,
    items: Vec<(usize, u32, u32, MangaMediaType)>,
}

#[derive(Clone, Copy, Debug)]
struct RetryState {
    attempts: u8,
    next_retry_at: Instant,
    /// Shape of the file when it failed. A rewritten file re-arms the index immediately;
    /// while the stamp is unchanged the tile stays dead (no frame-by-frame re-decode).
    stamp: Option<FileStamp>,
    /// Last time the stamp of a stamp-blocked index was re-read.
    stamp_checked_at: Instant,
}

/// Per-index state for "a gated video probe could not run". Kept apart from [`RetryState`] so a
/// degenerate media set (every visible video blocked by the gate) can never drain the decode budget
/// and settle every tile on a placeholder for the session.
#[derive(Clone, Copy, Debug)]
struct ProbeBlockedState {
    /// Passes that could not run the probe; paces `next_probe_at`.
    misses: u32,
    /// No gated probe is attempted for this index before this instant.
    next_probe_at: Instant,
}

impl ProbeBlockedState {
    /// Consecutive misses that only slow the probe down; there is deliberately no ceiling.
    const MISS_LIMIT: u32 = 16;
    const BASE_DELAY: Duration = Duration::from_millis(250);
    const MAX_DELAY: Duration = Duration::from_millis(2500);

    fn note_miss(&mut self, now: Instant) {
        self.next_probe_at = now + Self::backoff_for_misses(self.misses);
        self.misses = self.misses.saturating_add(1).min(Self::MISS_LIMIT);
    }

    fn backoff_for_misses(misses: u32) -> Duration {
        let factor = 1u64 << misses.min(4);
        Self::BASE_DELAY
            .saturating_mul(factor as u32)
            .min(Self::MAX_DELAY)
    }
}

/// Per-index pacing for a video whose spent thumbnail budget is re-opened. [`RetryState`] keeps the
/// ceiling semantics (an image stays dead); a video widens the interval per revive instead, so the
/// tile cannot sit on a static placeholder until the cursor happens to touch it.
#[derive(Clone, Copy, Debug)]
struct VideoReviveState {
    /// Re-opens already granted; [`VIDEO_THUMBNAIL_REVIVE_MAX`] ends them.
    revives: u32,
    /// No re-open before this instant.
    next_revive_at: Instant,
}

impl VideoReviveState {
    fn note_revive(&mut self, now: Instant) {
        self.revives = self
            .revives
            .saturating_add(1)
            .min(VIDEO_THUMBNAIL_REVIVE_MAX);
        self.next_revive_at = now + Self::delay_for_revives(self.revives.saturating_sub(1));
    }

    fn delay_for_revives(revives: u32) -> Duration {
        let factor = 1u64 << revives.min(3);
        VIDEO_THUMBNAIL_REVIVE_BASE_DELAY
            .saturating_mul(factor as u32)
            .min(VIDEO_THUMBNAIL_REVIVE_MAX_DELAY)
    }
}

/// Cache entry limits shared with the byte-aware budget clamps.
const MANGA_CACHE_MIN_ENTRIES: usize = 64;
const MANGA_CACHE_MAX_ENTRIES: usize = 1024;
/// Byte-aware cache budgets: oversized textures must not inflate entry-count targets.
const MASONRY_CACHE_BUDGET_BYTES_IDLE: usize = 768 * 1024 * 1024;
const MASONRY_CACHE_BUDGET_BYTES_NAV: usize = 512 * 1024 * 1024;

/// What the viewer currently shows. One `demand` call replaces the hand-sequenced
/// visible-count + dimension-probe + preload + cache-budget sequence that used to live in main.rs.
pub struct Demand<'a> {
    pub image_list: &'a [PathBuf],
    pub layout: MangaLayoutMode,
    /// Index at the viewport top; anchors probe, preload and keep windows.
    pub anchor_index: usize,
    pub visible_indices: &'a [usize],
    /// Whole-page count for the viewport when it differs from `visible_indices` (strip fallback).
    pub visible_page_count: usize,
    /// Long-strip fractional viewport coverage; None for grid layouts.
    pub strip_visible_item_equivalent: Option<f32>,
    pub max_texture_side: u32,
    /// Display-derived side before LOD quantization; quantized inside the loader.
    pub target_texture_side: u32,
    pub downscale_filter: FilterType,
    pub gif_filter: FilterType,
    pub force_triangle_filters: bool,
    pub masonry_rows: usize,
    pub zoom: f32,
    /// Scroll offset moved up this frame (keep window bias).
    pub scrolling_up: bool,
    /// Scrolling up or backward velocity (probe window bias).
    pub moving_up: bool,
    pub navigation_active: bool,
    pub preserve_recent_cache: bool,
    /// Capacity from the previous demand; used by the retain floor.
    pub previous_cache_capacity: usize,
    /// Indices the texture cache currently holds; the keep window evicts from this set.
    pub cached_indices: &'a [usize],
}

/// Loader-side answers for `Demand`. main.rs applies them to its texture cache.
pub struct DemandPlan {
    pub cache_capacity: usize,
    pub pinned_indices: Vec<usize>,
    /// Cache entries outside the keep window; already forgotten by the loader.
    pub evicted_indices: Vec<usize>,
}

/// Work products waiting to be consumed by the UI thread.
pub struct Taken {
    pub decoded: Vec<DecodedImage>,
    pub dimension_updates: Vec<usize>,
}

/// Pending counts main.rs polls for repaint scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backlog {
    pub loads: usize,
    pub decoded: usize,
    pub dimension_results: usize,
    pub dimension_probes: usize,
}

/// Repairs a visible-retry may carry besides a plain re-request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryRepair {
    None,
    /// Quality upgrade: drop loaded bookkeeping but keep in-flight requests.
    ForceReload,
    /// Stall heal: drop in-flight bookkeeping too so the placeholder can re-request.
    ResetState,
}

/// A visible-item repair request; the retry params every caller used to pass by hand.
pub struct VisibleRetry<'a> {
    pub image_list: &'a [PathBuf],
    pub index: usize,
    pub max_texture_side: u32,
    pub target_texture_side: u32,
    pub downscale_filter: FilterType,
    pub gif_filter: FilterType,
    pub force_triangle_filters: bool,
    pub repair: RetryRepair,
}

/// High-performance manga image loader with parallel decoding.
pub struct MangaLoader {
    /// Channel to send load requests to worker threads
    request_tx: Sender<LoadRequest>,
    /// Channel for visible-item retries that should bypass normal preload backlog.
    urgent_request_tx: Sender<LoadRequest>,
    /// Channel to receive decoded images from worker threads
    result_rx: Receiver<DecodedImage>,
    /// Indices currently being loaded for the active generation.
    /// Maps index -> generation to prevent stale completions from clearing newer in-flight work.
    loading_indices: Arc<RwLock<HashMap<usize, usize>>>,
    /// Highest loaded texture side per index for the active generation.
    /// Used for LOD-aware skip logic to avoid redundant reload churn.
    loaded_levels: Arc<RwLock<HashMap<usize, u32>>>,
    /// Retry state for indices whose decode failed or produced no usable pixels.
    /// Failed items are retried with backoff, and only aggressively retried when visible.
    retry_state: Arc<RwLock<HashMap<usize, RetryState>>>,
    /// Indices whose last video probe could not run because the gate was busy. Separate from
    /// `retry_state`: a probe miss paces the next attempt and never exhausts the decode budget.
    probe_blocked: HashMap<usize, ProbeBlockedState>,
    /// Videos whose spent thumbnail budget re-opens on a widening schedule. Images never appear
    /// here: their ceiling is final until the file itself changes.
    video_revive: HashMap<usize, VideoReviveState>,
    /// Cached original dimensions and media type (from file headers) for stable layout
    /// Maps index -> (width, height, media_type)
    pub dimension_cache: HashMap<usize, (u32, u32, MangaMediaType)>,

    /// Async dimension-probe request channel (main thread -> worker).
    dim_request_tx: Sender<DimRequest>,
    /// Low-priority async dimension-probe request channel for background warm-up work.
    dim_background_request_tx: Sender<DimRequest>,
    /// Async dimension-probe result channel (worker -> main thread).
    dim_result_rx: Receiver<DimResult>,
    /// Indices currently queued for async dimension probing (main thread only).
    dim_pending: HashSet<usize>,
    /// Flag to signal shutdown to worker threads
    shutdown: Arc<AtomicBool>,
    /// Current scroll direction: positive = scrolling down, negative = scrolling up
    scroll_direction: i32,
    /// Last known visible index for priority calculation
    last_visible_index: usize,
    /// Generation counter to invalidate stale requests on image list change
    generation: Arc<AtomicUsize>,
    /// Current generation for filtering results
    current_generation: usize,
    /// Statistics for debugging
    pub stats: LoaderStats,
    /// Estimated number of visible items on screen (for adaptive preloading)
    visible_page_count: usize,
    /// Long-strip-only viewport coverage equivalent (e.g. 1.5 visible pages).
    strip_visible_item_equivalent: Option<f32>,
    /// Last demand's cache capacity, kept for the retain floor.
    cache_capacity_last: usize,
    /// Layout and raw on-screen count from the last demand; prime windows are derived from them.
    last_layout: MangaLayoutMode,
    last_visible_indices_count: usize,
    last_masonry_rows: usize,
    /// Test seam: the worker-facing halves of the channels, so tests can observe/inject.
    #[cfg(test)]
    probe: Option<LoaderProbe>,
}

#[cfg(test)]
struct LoaderProbe {
    requests: Receiver<LoadRequest>,
    urgent: Receiver<LoadRequest>,
    dim_requests: Receiver<DimRequest>,
    dim_background_requests: Receiver<DimRequest>,
    dim_results: Sender<DimResult>,
}

/// Statistics for monitoring loader performance.
#[derive(Default, Clone)]
pub struct LoaderStats {
    pub images_loaded: usize,
    pub images_pending: usize,
}

/// Channels a loader owns or keeps private to its workers.
struct LoaderChannels {
    request_tx: Sender<LoadRequest>,
    request_rx: Receiver<LoadRequest>,
    urgent_request_tx: Sender<LoadRequest>,
    urgent_request_rx: Receiver<LoadRequest>,
    result_tx: Sender<DecodedImage>,
    result_rx: Receiver<DecodedImage>,
    dim_request_tx: Sender<DimRequest>,
    dim_request_rx: Receiver<DimRequest>,
    dim_background_request_tx: Sender<DimRequest>,
    dim_background_request_rx: Receiver<DimRequest>,
    dim_result_tx: Sender<DimResult>,
    dim_result_rx: Receiver<DimResult>,
}

impl LoaderChannels {
    fn bounded() -> Self {
        // Bounded channels prevent unbounded memory growth under scroll bursts.
        let (request_tx, request_rx) = crossbeam_channel::bounded::<LoadRequest>(256);
        let (urgent_request_tx, urgent_request_rx) =
            crossbeam_channel::bounded::<LoadRequest>(URGENT_REQUEST_QUEUE_CAPACITY);
        let (result_tx, result_rx) =
            crossbeam_channel::bounded::<DecodedImage>(MAX_PENDING_UPLOADS);
        let (dim_request_tx, dim_request_rx) = crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_background_request_tx, dim_background_request_rx) =
            crossbeam_channel::bounded::<DimRequest>(64);
        let (dim_result_tx, dim_result_rx) = crossbeam_channel::bounded::<DimResult>(64);

        Self {
            request_tx,
            request_rx,
            urgent_request_tx,
            urgent_request_rx,
            result_tx,
            result_rx,
            dim_request_tx,
            dim_request_rx,
            dim_background_request_tx,
            dim_background_request_rx,
            dim_result_tx,
            dim_result_rx,
        }
    }
}

impl MangaLoader {
    /// Create a new manga loader with background thread pool.
    pub fn new() -> Self {
        let LoaderChannels {
            request_tx,
            request_rx,
            urgent_request_tx,
            urgent_request_rx,
            result_tx,
            result_rx,
            dim_request_tx,
            dim_request_rx,
            dim_background_request_tx,
            dim_background_request_rx,
            dim_result_tx,
            dim_result_rx,
        } = LoaderChannels::bounded();

        let loading_indices = Arc::new(RwLock::new(HashMap::new()));
        let loaded_levels = Arc::new(RwLock::new(HashMap::new()));
        let retry_state = Arc::new(RwLock::new(HashMap::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicUsize::new(0));

        // Spawn a coordinator thread that processes requests using Rayon
        let loading_clone = Arc::clone(&loading_indices);
        let loaded_clone = Arc::clone(&loaded_levels);
        let retry_clone = Arc::clone(&retry_state);
        let shutdown_clone = Arc::clone(&shutdown);
        let generation_clone = Arc::clone(&generation);

        crate::async_runtime::spawn_blocking_or_thread("manga-loader-coordinator", move || {
            Self::coordinator_loop(
                request_rx,
                urgent_request_rx,
                result_tx,
                loading_clone,
                loaded_clone,
                retry_clone,
                shutdown_clone,
                generation_clone,
            );
        });

        // Spawn a lightweight dimension probe worker.
        // Header-based size probes stay off the UI thread.
        let shutdown_clone = Arc::clone(&shutdown);
        let generation_clone = Arc::clone(&generation);
        crate::async_runtime::spawn_blocking_or_thread("manga-dimension-worker", move || {
            // Chunks the result channel refused. Re-emitted on the next pass: dropping them made
            // the tile fall back to 16:9 and re-probe, which is what stalled the gallery.
            let mut parked: std::collections::VecDeque<DimResult> =
                std::collections::VecDeque::new();

            while !shutdown_clone.load(Ordering::Acquire) {
                // Parked results go out before new work, with a short poll instead of a 500ms
                // block so a drained channel is refilled on the next tick.
                if !parked.is_empty() {
                    match Self::flush_parked_dim_chunks(&dim_result_tx, &mut parked) {
                        DimSend::Disconnected => return,
                        DimSend::Flushed => {}
                        DimSend::Parked => {
                            if let Ok(request) = dim_request_rx
                                .try_recv()
                                .or_else(|_| dim_background_request_rx.try_recv())
                            {
                                if let Some(req) =
                                    Self::dim_request_ready(request, &generation_clone)
                                {
                                    Self::run_dim_request(
                                        req,
                                        &dim_result_tx,
                                        &generation_clone,
                                        &mut parked,
                                    );
                                }
                                continue;
                            }

                            std::thread::sleep(Duration::from_millis(
                                DIM_RESULT_SEND_RETRY_SLEEP_MS,
                            ));
                            continue;
                        }
                    }
                }

                let req = loop {
                    if let Ok(request) = dim_request_rx.try_recv() {
                        break Some(request);
                    }

                    if let Ok(request) = dim_background_request_rx.try_recv() {
                        break Some(request);
                    }

                    crossbeam_channel::select! {
                        recv(dim_request_rx) -> request => {
                            if let Ok(request) = request {
                                break Some(request);
                            }
                        }
                        recv(dim_background_request_rx) -> request => {
                            if let Ok(request) = request {
                                break Some(request);
                            }
                        }
                        default(Duration::from_millis(500)) => {
                            if shutdown_clone.load(Ordering::Acquire) {
                                break None;
                            }
                        }
                    }
                };

                let Some(req) = req else {
                    break;
                };

                if let Some(req) = Self::dim_request_ready(req, &generation_clone) {
                    Self::run_dim_request(req, &dim_result_tx, &generation_clone, &mut parked);
                }
            }
        });

        Self {
            request_tx,
            urgent_request_tx,
            result_rx,
            loading_indices,
            loaded_levels,
            retry_state,
            probe_blocked: HashMap::new(),
            video_revive: HashMap::new(),
            dimension_cache: HashMap::new(),
            dim_request_tx,
            dim_background_request_tx,
            dim_result_rx,
            dim_pending: HashSet::new(),
            shutdown,
            scroll_direction: 1,
            last_visible_index: 0,
            generation,
            current_generation: 0,
            stats: LoaderStats::default(),
            visible_page_count: 1,
            strip_visible_item_equivalent: None,
            cache_capacity_last: MANGA_CACHE_MIN_ENTRIES,
            last_layout: MangaLayoutMode::LongStrip,
            last_visible_indices_count: 1,
            last_masonry_rows: 2,
            #[cfg(test)]
            probe: None,
        }
    }

    /// `None` for a request whose generation has passed: folder switches must not keep the
    /// dimension worker busy with obsolete probes.
    fn dim_request_ready(req: DimRequest, generation: &AtomicUsize) -> Option<DimRequest> {
        (req.generation == generation.load(Ordering::Acquire)).then_some(req)
    }

    /// Probe one batch and hand every chunk to the result channel, parking what it refuses.
    fn run_dim_request(
        req: DimRequest,
        dim_result_tx: &Sender<DimResult>,
        generation: &AtomicUsize,
        parked: &mut std::collections::VecDeque<DimResult>,
    ) {
        let mut out: Vec<(usize, u32, u32, MangaMediaType)> = req
            .items
            .into_par_iter()
            .filter_map(|(idx, path)| {
                let is_video = is_supported_video(&path);
                let is_image = is_supported_image(&path);

                if !is_video && !is_image {
                    return None;
                }

                let media_type = if is_video {
                    MangaMediaType::Video
                } else {
                    MangaMediaType::StaticImage
                };

                // Breadcrumb before a probe: same reason as the decode one.
                crate::diag::note_file(idx, "dim-probe", || {
                    format!(
                        "idx={} file={} bytes={} kind={}",
                        idx,
                        diag_file_label(&path),
                        std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                        if is_video { "video" } else { "image" }
                    )
                });

                let dims = if is_video {
                    Self::probe_video_dimensions(idx, &path)
                } else {
                    Self::probe_image_dimensions_cached(&path)
                };

                let (w, h) = if let Some((w, h)) = dims {
                    (w, h)
                } else if is_video {
                    FALLBACK_VIDEO_DIMENSIONS
                } else {
                    FALLBACK_IMAGE_DIMENSIONS
                };

                Some((idx, w, h, media_type))
            })
            .collect();

        out.par_sort_unstable_by_key(|(idx, _, _, _)| *idx);

        for chunk in out.chunks(DIM_RESULT_CHUNK_SIZE) {
            // Generation may advance while probing; avoid pushing obsolete results. The
            // dropped indices stay in `dim_pending`; the caller's stale-probe reaper
            // releases them so they can be probed again.
            if req.generation != generation.load(Ordering::Acquire) {
                break;
            }

            let chunk = DimResult {
                generation: req.generation,
                items: chunk.to_vec(),
            };
            if Self::queue_dim_chunk(dim_result_tx, chunk, parked) == DimSend::Disconnected {
                return;
            }
        }
    }

    /// Hand one chunk over, or park it in `parked` for the next pass. Never discards: the
    /// dropped indices would leave `dim_pending` set until the stale-probe reaper fires, and
    /// the tile would re-probe the file through gstreamer.
    fn queue_dim_chunk(
        dim_result_tx: &Sender<DimResult>,
        mut chunk: DimResult,
        parked: &mut std::collections::VecDeque<DimResult>,
    ) -> DimSend {
        // Keep order: a later chunk must not overtake one still parked.
        if !parked.is_empty() {
            parked.push_back(chunk);
            return DimSend::Parked;
        }

        for _ in 0..DIM_RESULT_SEND_MAX_ATTEMPTS {
            crate::diag::count(crate::diag::Counter::DimAttempts, 1);
            match dim_result_tx.try_send(chunk) {
                Ok(()) => {
                    crate::diag::count(crate::diag::Counter::DimChunks, 1);
                    return DimSend::Flushed;
                }
                Err(TrySendError::Full(returned)) => {
                    chunk = returned;
                    std::thread::sleep(Duration::from_millis(DIM_RESULT_SEND_RETRY_SLEEP_MS));
                }
                Err(TrySendError::Disconnected(_)) => return DimSend::Disconnected,
            }
        }

        crate::diag::count(crate::diag::Counter::DimParked, 1);
        parked.push_back(chunk);
        DimSend::Parked
    }

    /// Re-emit parked chunks until the channel is full again.
    fn flush_parked_dim_chunks(
        dim_result_tx: &Sender<DimResult>,
        parked: &mut std::collections::VecDeque<DimResult>,
    ) -> DimSend {
        while let Some(chunk) = parked.pop_front() {
            match dim_result_tx.try_send(chunk) {
                Ok(()) => crate::diag::count(crate::diag::Counter::DimFlushed, 1),
                Err(TrySendError::Full(returned)) => {
                    parked.push_front(returned);
                    return DimSend::Parked;
                }
                Err(TrySendError::Disconnected(_)) => {
                    crate::diag::count(crate::diag::Counter::DimDropped, parked.len() as u64 + 1);
                    parked.clear();
                    return DimSend::Disconnected;
                }
            }
        }

        DimSend::Flushed
    }

    /// Handle-only loader: no worker threads. Paired channels let tests inject dimension
    /// results and read what `demand`/`prime`/`retry_visible` enqueued.
    #[cfg(test)]
    fn without_workers() -> Self {
        let channels = LoaderChannels::bounded();

        Self {
            request_tx: channels.request_tx,
            urgent_request_tx: channels.urgent_request_tx,
            result_rx: channels.result_rx,
            loading_indices: Arc::new(RwLock::new(HashMap::new())),
            loaded_levels: Arc::new(RwLock::new(HashMap::new())),
            retry_state: Arc::new(RwLock::new(HashMap::new())),
            probe_blocked: HashMap::new(),
            video_revive: HashMap::new(),
            dimension_cache: HashMap::new(),
            dim_request_tx: channels.dim_request_tx,
            dim_background_request_tx: channels.dim_background_request_tx,
            dim_result_rx: channels.dim_result_rx,
            dim_pending: HashSet::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            scroll_direction: 1,
            last_visible_index: 0,
            generation: Arc::new(AtomicUsize::new(0)),
            current_generation: 0,
            stats: LoaderStats::default(),
            visible_page_count: 1,
            strip_visible_item_equivalent: None,
            cache_capacity_last: MANGA_CACHE_MIN_ENTRIES,
            last_layout: MangaLayoutMode::LongStrip,
            last_visible_indices_count: 1,
            last_masonry_rows: 2,
            probe: Some(LoaderProbe {
                requests: channels.request_rx,
                urgent: channels.urgent_request_rx,
                dim_requests: channels.dim_request_rx,
                dim_background_requests: channels.dim_background_request_rx,
                dim_results: channels.dim_result_tx,
            }),
        }
    }

    /// Pending counts for repaint scheduling. Authoritative source for "are we still loading?".
    pub fn backlog(&self) -> Backlog {
        Backlog {
            loads: self.loading_indices.read().len(),
            decoded: self.result_rx.len(),
            dimension_results: self.dim_result_rx.len(),
            dimension_probes: self.dim_pending.len(),
        }
    }

    /// (in-flight decode loads, retry-state entries) for the diag frame line.
    pub fn diag_counts(&self) -> (usize, usize) {
        (
            self.loading_indices.read().len(),
            self.retry_state.read().len(),
        )
    }

    /// True when a visible index still needs a request: not in flight and no loaded level
    /// reaching the target side. `backlog` only counts in-flight work, so a caller that pumps
    /// the preload queue off `backlog` alone stalls once batches are dropped (generation
    /// change, full result channel) and needs pointer input to recover. Level-triggered signal.
    pub fn demand_outstanding(
        &self,
        image_list: &[PathBuf],
        visible_indices: &[usize],
        max_texture_side: u32,
        target_texture_side: u32,
    ) -> bool {
        let loading = self.loading_indices.read();
        let loaded = self.loaded_levels.read();
        let retry = self.retry_state.read();

        visible_indices.iter().any(|&idx| {
            let Some(path) = image_list.get(idx) else {
                return false;
            };
            if !is_supported_image(path) && !is_supported_video(path) {
                return false;
            }

            // Same skip gates as `prefetch`, so this is true only while a request is issuable.
            if loading.get(&idx).copied() == Some(self.current_generation) {
                return false;
            }
            let request_target = self.limit_target_texture_side_for_index(
                idx,
                target_texture_side,
                max_texture_side,
            );
            if let Some(side) = loaded.get(&idx) {
                // Unknown source dims can put `request_target` above what the file can reach;
                // any stored level then counts, else the pump would never go idle.
                if *side >= request_target || !self.dimension_cache.contains_key(&idx) {
                    return false;
                }
            }
            let spent = retry
                .get(&idx)
                .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS);
            if !spent {
                return true;
            }
            // A video thumbnail budget re-opens on a schedule, so its visible tile is still
            // outstanding work; an image at the ceiling lets the app go idle.
            self.video_thumbnail_revive_pending(idx, path)
        })
    }

    /// Indices in `visible_indices` still waiting on a gated video probe that the gate has already
    /// blocked at least once. They are outstanding work (the tile shows a placeholder) but must
    /// keep repainting until the backoff window opens, so the UI cannot settle on a static
    /// placeholder with no wake-up scheduled.
    pub fn probe_blocked_visible_indices(&self, visible_indices: &[usize]) -> Vec<usize> {
        visible_indices
            .iter()
            .copied()
            .filter(|idx| self.probe_blocked.contains_key(idx))
            .collect()
    }

    /// Number of cached dimensions for indices in `[0, total_len)`.
    pub fn cached_dimensions_count(&self, total_len: usize) -> usize {
        if total_len == 0 {
            return 0;
        }

        self.dimension_cache
            .keys()
            .filter(|&&idx| idx < total_len)
            .count()
    }

    /// Queue async dimension probes for a range of indices.
    ///
    /// This does not block the UI thread. Results are applied when `take` runs.
    fn queue_dimension_range(&mut self, image_list: &[PathBuf], start: usize, end: usize) {
        self.enqueue_dimension_range(image_list, start, end, self.dim_request_tx.clone());
    }

    /// Queue async background dimension probes for a range of indices.
    ///
    /// For non-interactive masonry warm-up: must not compete with visible media.
    fn queue_background_dimension_range(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
    ) {
        self.enqueue_dimension_range(
            image_list,
            start,
            end,
            self.dim_background_request_tx.clone(),
        );
    }

    fn enqueue_dimension_range(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
        request_tx: Sender<DimRequest>,
    ) {
        let end = end.min(image_list.len());
        if start >= end {
            return;
        }

        // Build a bounded batch of missing indices.
        let mut items: Vec<(usize, PathBuf)> = Vec::new();
        for idx in start..end {
            if items.len() >= DIM_REQUEST_BATCH_SIZE {
                break;
            }

            if self.dimension_cache.contains_key(&idx) || self.dim_pending.contains(&idx) {
                continue;
            }

            let path = match image_list.get(idx) {
                Some(p) => p.clone(),
                None => continue,
            };

            // Only request for supported media.
            if !is_supported_image(&path) && !is_supported_video(&path) {
                continue;
            }

            items.push((idx, path));
        }

        if items.is_empty() {
            return;
        }

        let indices: Vec<usize> = items.iter().map(|(i, _)| *i).collect();
        match request_tx.try_send(DimRequest {
            generation: self.current_generation,
            items,
        }) {
            Ok(()) => {
                for idx in indices {
                    self.dim_pending.insert(idx);
                }
            }
            Err(TrySendError::Full(_)) => {
                // Backpressure: try again next frame/update.
            }
            Err(TrySendError::Disconnected(_)) => {
                // Worker gone; ignore.
            }
        }
    }

    /// Background metadata warm-up for `[start, end)`: queues probes and, when
    /// `fallback_seed_max > 0`, seeds the stall-recovery fallback dimensions.
    /// Returns how many fallback entries were seeded.
    pub fn warm_dimensions_background(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
        fallback_seed_max: usize,
    ) -> usize {
        self.queue_background_dimension_range(image_list, start, end);
        if fallback_seed_max == 0 {
            0
        } else {
            self.seed_fallback_dimensions_for_range(image_list, start, end, fallback_seed_max)
        }
    }

    /// Drain async dimension results and apply them to `dimension_cache`.
    /// Returns indices whose dimensions were updated.
    fn apply_dimension_results(&mut self, max_messages: usize) -> Vec<usize> {
        let mut updated: Vec<usize> = Vec::new();

        for _ in 0..max_messages {
            let res = match self.dim_result_rx.try_recv() {
                Ok(r) => r,
                Err(_) => break,
            };

            // Drop stale results (e.g., after cancel/clear) but release the probe bookkeeping so
            // those indices are not stranded in `dim_pending` for the whole next generation.
            if res.generation != self.current_generation {
                for (idx, _, _, _) in res.items {
                    self.dim_pending.remove(&idx);
                }
                continue;
            }

            for (idx, raw_w, raw_h, mt) in res.items {
                // Unsanitized header dims poison layout and can clamp the request target to 1.
                let (w, h) = probed_or_fallback_dimensions(Some((raw_w, raw_h)), mt);
                let changed =
                    self.dimension_cache
                        .get(&idx)
                        .map_or(true, |(old_w, old_h, old_mt)| {
                            if *old_w == w && *old_h == h && *old_mt == mt {
                                return false;
                            }

                            // Ignore tiny video-size jitter from probes to avoid
                            // perpetual masonry relayout churn.
                            if *old_mt == MangaMediaType::Video && mt == MangaMediaType::Video {
                                let width_delta = old_w.abs_diff(w);
                                let height_delta = old_h.abs_diff(h);
                                if width_delta <= 2 && height_delta <= 2 {
                                    let old_aspect = *old_w as f32 / (*old_h).max(1) as f32;
                                    let new_aspect = w as f32 / h.max(1) as f32;
                                    if (old_aspect - new_aspect).abs() <= 0.003 {
                                        return false;
                                    }
                                }
                            }

                            true
                        });

                if changed {
                    self.dimension_cache.insert(idx, (w, h, mt));
                    updated.push(idx);
                }

                self.dim_pending.remove(&idx);
            }
        }

        updated
    }

    /// Seed conservative fallback dimensions for uncached items in `[start, end)`.
    ///
    /// This is a recovery path for rare probe stalls: it keeps masonry warm-up moving
    /// while higher-fidelity dimensions can still arrive later from decode/probe results.
    fn seed_fallback_dimensions_for_range(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
        max_items: usize,
    ) -> usize {
        if max_items == 0 || image_list.is_empty() {
            return 0;
        }

        let end = end.min(image_list.len());
        if start >= end {
            return 0;
        }

        let mut seeded = 0usize;

        for idx in start..end {
            if seeded >= max_items {
                break;
            }

            let Some(path) = image_list.get(idx) else {
                self.dim_pending.remove(&idx);
                continue;
            };

            let is_video = is_supported_video(path);
            let is_image = is_supported_image(path);

            if !is_video && !is_image {
                self.dim_pending.remove(&idx);
                continue;
            }

            if self.dimension_cache.contains_key(&idx) {
                self.dim_pending.remove(&idx);
                continue;
            }

            let fallback = if is_video {
                (
                    FALLBACK_VIDEO_DIMENSIONS.0,
                    FALLBACK_VIDEO_DIMENSIONS.1,
                    MangaMediaType::Video,
                )
            } else {
                (
                    FALLBACK_IMAGE_DIMENSIONS.0,
                    FALLBACK_IMAGE_DIMENSIONS.1,
                    MangaMediaType::StaticImage,
                )
            };

            self.dimension_cache.insert(idx, fallback);
            self.dim_pending.remove(&idx);
            seeded = seeded.saturating_add(1);
        }

        seeded
    }

    /// Coordinator loop that processes requests in parallel using Rayon.
    fn coordinator_loop(
        request_rx: Receiver<LoadRequest>,
        urgent_request_rx: Receiver<LoadRequest>,
        result_tx: Sender<DecodedImage>,
        loading_indices: Arc<RwLock<HashMap<usize, usize>>>,
        loaded_levels: Arc<RwLock<HashMap<usize, u32>>>,
        retry_state: Arc<RwLock<HashMap<usize, RetryState>>>,
        shutdown: Arc<AtomicBool>,
        generation: Arc<AtomicUsize>,
    ) {
        // Collect requests in batches for parallel processing
        let mut batch: Vec<LoadRequest> = Vec::with_capacity(16);

        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Collect available requests (non-blocking after first)
            batch.clear();

            // Prefer urgent visible retries so sharpening upgrades do not wait behind preload work.
            crossbeam_channel::select! {
                recv(urgent_request_rx) -> req => {
                    if let Ok(req) = req {
                        batch.push(req);
                    } else {
                        continue;
                    }
                }
                recv(request_rx) -> req => {
                    if let Ok(req) = req {
                        batch.push(req);
                    } else {
                        continue;
                    }
                }
                default(std::time::Duration::from_millis(500)) => continue,
            }

            while batch.len() < 32 {
                match urgent_request_rx.try_recv() {
                    Ok(req) => batch.push(req),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                }
            }

            // Drain any additional pending requests (non-blocking)
            while batch.len() < 32 {
                match request_rx.try_recv() {
                    Ok(req) => batch.push(req),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Sort by priority (lower = higher priority = process first)
            batch.sort_by_key(|r| r.priority);

            // Get current generation for filtering stale requests
            let current_gen = generation.load(Ordering::Acquire);

            // If the generation changes while we're decoding (e.g., a fast scrollbar jump),
            // we prefer to drop the whole batch's outputs rather than clog the result channel
            // with stale work.
            let generation_changed = || generation.load(Ordering::Acquire) != current_gen;

            enum DecodeOutcome {
                Skipped,
                /// Decode returned nothing; carries the file shape it failed at.
                Failed(Option<FileStamp>),
                Decoded(DecodedImage),
            }

            let process_one = |req: &LoadRequest| -> (usize, usize, DecodeOutcome) {
                let req_generation = req.generation;

                // Skip if already loaded or if we've been shut down
                if shutdown.load(Ordering::Relaxed) {
                    return (req.index, req_generation, DecodeOutcome::Skipped);
                }

                // Skip stale requests (e.g., after fast scrollbar jumps / cancel).
                if req_generation != current_gen {
                    return (req.index, req_generation, DecodeOutcome::Skipped);
                }

                // Check if already in loaded set
                {
                    let loaded = loaded_levels.read();
                    if loaded
                        .get(&req.index)
                        .is_some_and(|side| *side >= req.target_texture_side)
                    {
                        return (req.index, req_generation, DecodeOutcome::Skipped);
                    }
                }

                // Load the image
                let queue_wait = req.queued_at.elapsed();
                if queue_wait >= DECODE_QUEUE_WAIT_TRACE {
                    crate::diag::trace(
                        "decode-queue",
                        &format!(
                            "idx={} wait={}ms file={}",
                            req.index,
                            queue_wait.as_millis(),
                            diag_file_label(&req.path)
                        ),
                    );
                }
                let decode_started = Instant::now();
                let decoded = Self::load_single_image(req);
                let decode_time = decode_started.elapsed();

                let outcome = match decoded {
                    Some(mut decoded) => {
                        decoded.queue_wait = queue_wait;
                        decoded.decode_time = decode_time;
                        DecodeOutcome::Decoded(decoded)
                    }
                    None => DecodeOutcome::Failed(FileStamp::of_path(&req.path)),
                };

                (req.index, req_generation, outcome)
            };

            let publish_one = |idx: usize, req_generation: usize, outcome: DecodeOutcome| {
                // Request has finished one way or another; allow it to be re-requested if needed.
                Self::clear_loading_if_generation(&loading_indices, idx, req_generation);

                if generation_changed() {
                    return true;
                }

                match outcome {
                    DecodeOutcome::Skipped => {}
                    DecodeOutcome::Failed(stamp) => {
                        Self::register_decode_failure(&loaded_levels, &retry_state, idx, stamp);
                    }
                    DecodeOutcome::Decoded(decoded) => {
                        let stamp = FileStamp::of_path(decoded.path.as_path());
                        // A decoded payload with empty pixels/dimensions is still useful for metadata
                        // (e.g., video dimension fallback), but does not satisfy texture preloading.
                        let has_usable_pixels =
                            !decoded.pixels.is_empty() && decoded.width > 0 && decoded.height > 0;
                        // A gated-probe miss is not a file failure: counting it spent the whole
                        // video retry budget in one or two frame passes and the tile then never
                        // re-requested itself for the session.
                        let deferred = decoded.deferred;

                        let loaded_side = decoded.width.max(decoded.height);

                        match result_tx.try_send(decoded) {
                            Ok(_) => {
                                if has_usable_pixels {
                                    let mut loaded = loaded_levels.write();
                                    loaded
                                        .entry(idx)
                                        .and_modify(|side| *side = (*side).max(loaded_side))
                                        .or_insert(loaded_side);
                                    retry_state.write().remove(&idx);
                                } else if deferred {
                                    // The probe cooldown already paces the next attempt.
                                    loaded_levels.write().remove(&idx);
                                } else {
                                    Self::register_decode_failure(
                                        &loaded_levels,
                                        &retry_state,
                                        idx,
                                        stamp,
                                    );
                                }
                            }
                            Err(TrySendError::Full(_decoded)) => {
                                // Channel full: drop decoded result.
                                // We intentionally do NOT mark as loaded so the main thread can re-request.
                                loaded_levels.write().remove(&idx);
                            }
                            Err(TrySendError::Disconnected(_decoded)) => {
                                return false; // Main thread gone, exit
                            }
                        }
                    }
                }

                true
            };

            // IMPORTANT: for "urgent" requests (negative priority), decode the single highest
            // priority request first (serially) so it is not competing with neighbor prefetch.
            // This is the key to making far jumps feel instant.
            let urgent_head = batch.first().map_or(false, |r| r.priority < 0);
            let mut start_index = 0usize;

            if urgent_head {
                if let Some(first) = batch.first() {
                    let (idx, req_generation, outcome) = process_one(first);
                    if !publish_one(idx, req_generation, outcome) {
                        return;
                    }
                    start_index = 1;
                }
            }

            if start_index >= batch.len() {
                continue;
            }

            let parallel_len = batch.len() - start_index;
            let (outcome_tx, outcome_rx) = crossbeam_channel::unbounded();
            let mut disconnected = false;

            rayon::scope(|scope| {
                for req in batch[start_index..].iter() {
                    let outcome_tx = outcome_tx.clone();
                    scope.spawn(move |_| {
                        let outcome = process_one(req);
                        let _ = outcome_tx.send(outcome);
                    });
                }

                drop(outcome_tx);

                for _ in 0..parallel_len {
                    let Ok((idx, req_generation, outcome)) = outcome_rx.recv() else {
                        break;
                    };

                    if !publish_one(idx, req_generation, outcome) {
                        disconnected = true;
                        break;
                    }
                }
            });

            if disconnected {
                return;
            }
        }
    }

    fn clear_loading_if_generation(
        loading_indices: &Arc<RwLock<HashMap<usize, usize>>>,
        index: usize,
        generation: usize,
    ) {
        let mut loading = loading_indices.write();
        if loading.get(&index).copied() == Some(generation) {
            loading.remove(&index);
        }
    }

    fn retry_backoff_for_attempt(attempts: u8) -> Duration {
        let shift = attempts.saturating_sub(1).min(6) as u32;
        let factor = 1u64 << shift;
        let delay_ms = PRELOAD_RETRY_BASE_DELAY_MS
            .saturating_mul(factor)
            .min(PRELOAD_RETRY_MAX_DELAY_MS);
        Duration::from_millis(delay_ms)
    }

    fn register_decode_failure(
        loaded_levels: &Arc<RwLock<HashMap<usize, u32>>>,
        retry_state: &Arc<RwLock<HashMap<usize, RetryState>>>,
        index: usize,
        stamp: Option<FileStamp>,
    ) {
        loaded_levels.write().remove(&index);

        let now = Instant::now();
        let mut retry = retry_state.write();
        // The stamp this failure happened at outlives the attempt count: once the budget is
        // spent, an unchanged file must stay dead, and any change to the file must revive it.
        let stamp = stamp.or_else(|| retry.get(&index).and_then(|state| state.stamp));
        let attempts = retry
            .get(&index)
            .map(|state| state.attempts)
            .unwrap_or(0)
            .saturating_add(1);
        let next_retry_at = now + Self::retry_backoff_for_attempt(attempts);
        retry.insert(
            index,
            RetryState {
                attempts,
                next_retry_at,
                stamp,
                stamp_checked_at: now,
            },
        );
    }

    /// `rearm_failed_indices` over whatever is still blocked in this window, rate-limited per
    /// index. Every prefetch pass goes through here, so a rewritten file comes back to life on
    /// the next pass instead of waiting for the tile to be drawn.
    fn rearm_failed_in_window(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
    ) -> usize {
        let blocked: Vec<usize> = {
            let retry = self.retry_state.read();
            (start..end.min(image_list.len()))
                .filter(|idx| {
                    retry
                        .get(idx)
                        .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS)
                })
                .collect()
        };

        self.rearm_failed_indices(image_list, blocked).len()
    }

    /// A file rewritten since it failed is a different file: drop everything that memoizes the
    /// failure so the next prefetch/draw request can decode it again.
    ///
    /// The stamp is re-read only for indices at the retry ceiling and at most once per
    /// `FAILED_STAMP_RECHECK`; a broken but unchanged file therefore costs one `stat` per
    /// interval and is never re-decoded, which is what kept corrupt GIFs pegging the CPU.
    pub fn rearm_failed_indices<I>(&mut self, image_list: &[PathBuf], indices: I) -> Vec<usize>
    where
        I: IntoIterator<Item = usize>,
    {
        let now = Instant::now();
        let mut rearmed = Vec::new();

        {
            let mut retry = self.retry_state.write();
            for index in indices {
                let Some(state) = retry.get_mut(&index) else {
                    continue;
                };
                if state.attempts < PRELOAD_RETRY_MAX_ATTEMPTS {
                    // Backoff retries already cover this index; nothing is blocked yet.
                    continue;
                }
                if now < state.stamp_checked_at + FAILED_STAMP_RECHECK {
                    continue;
                }
                state.stamp_checked_at = now;

                let Some(path) = image_list.get(index) else {
                    continue;
                };
                let current = FileStamp::of_path(path);
                // Same shape (or a file that vanished): still the broken file, stay blocked.
                if current.is_none() || current == state.stamp {
                    continue;
                }

                rearmed.push(index);
            }

            for index in &rearmed {
                retry.remove(index);
            }
        }

        if rearmed.is_empty() {
            return Vec::new();
        }

        let mut loaded = self.loaded_levels.write();
        let mut loading = self.loading_indices.write();
        for &index in &rearmed {
            loaded.remove(&index);
            loading.remove(&index);
        }
        self.stats.images_pending = loading.len();
        drop(loading);
        drop(loaded);

        rearmed
    }

    fn probe_image_dimensions_cached(path: &std::path::Path) -> Option<(u32, u32)> {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Image) {
            return Some((w, h));
        }

        let dims = probe_image_dimensions(path);
        if let Some((w, h)) = dims {
            store_cached_dimensions(path, CachedMediaKind::Image, w, h);
        }

        dims
    }

    fn path_is_webp(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("webp"))
    }

    /// Dimensions without touching the gstreamer gate: cache, shell metadata, filename hint.
    /// Counted once per gstreamer pipeline start: the summary line reports probes/s.
    fn note_gst_probe() {
        crate::diag::count(crate::diag::Counter::GstProbes, 1);
    }

    fn cheap_video_dimensions(path: &std::path::Path) -> (u32, u32) {
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

    fn probe_video_dimensions_fast(
        path: &std::path::Path,
        permit: Option<&VideoProbePermit>,
    ) -> (u32, u32) {
        if let Some((w, h)) = lookup_cached_dimensions(path, CachedMediaKind::Video) {
            return (w, h);
        }

        if gstreamer_runtime_available() {
            let mut owned = None;
            if probe_permit(permit, &mut owned).is_some() {
                Self::note_gst_probe();
                if let Some((w, h)) = probe_video_dimensions_with_gstreamer(path) {
                    if w > 0 && h > 0 {
                        store_cached_dimensions(path, CachedMediaKind::Video, w, h);
                        return (w, h);
                    }
                }
            }
        }

        Self::cheap_video_dimensions(path)
    }

    /// Load a single image on a worker thread.
    /// For video files, this extracts the first frame as a thumbnail placeholder.
    fn load_single_image(req: &LoadRequest) -> Option<DecodedImage> {
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

    fn load_single_image_inner(req: &LoadRequest) -> Option<DecodedImage> {
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

    fn quantize_target_texture_side(target_texture_side: u32, max_texture_side: u32) -> u32 {
        let max_side = max_texture_side.max(1);
        let target = target_texture_side.max(1).min(max_side);

        for &bucket in LOD_SIDE_BUCKETS {
            if bucket >= target && bucket <= max_side {
                return bucket;
            }
        }

        max_side
    }

    fn limit_target_texture_side_for_index(
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
    fn probe_video_dimensions(index: usize, path: &std::path::Path) -> Option<(u32, u32)> {
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

    fn fallback_video_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
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

    fn video_thumbnail_output_dimensions(
        source_dimensions: Option<(u32, u32)>,
        max_texture_side: u32,
    ) -> Option<(u32, u32)> {
        let (source_w, source_h) = source_dimensions?;
        if source_w == 0 || source_h == 0 || max_texture_side == 0 {
            return None;
        }

        let scale = (max_texture_side as f64 / source_w as f64)
            .min(max_texture_side as f64 / source_h as f64)
            .min(1.0);
        if scale >= 0.999 {
            return None;
        }

        Some((
            (source_w as f64 * scale).round().max(1.0) as u32,
            (source_h as f64 * scale).round().max(1.0) as u32,
        ))
    }

    /// Extract the first frame from a video file as a thumbnail.
    ///
    /// Uses GStreamer to decode just the first frame without loading the entire video.
    /// This is much faster than full video playback initialization and provides
    /// a visual preview for videos in manga mode.
    ///
    fn extract_video_first_frame(
        path: &std::path::Path,
        max_texture_side: u32,
        permit: Option<&VideoProbePermit>,
    ) -> Option<FirstFrame> {
        // Released by the guard on every path below, including the early returns.
        let mut owned = None;
        let _probe_permit = probe_permit(permit, &mut owned)?;
        let gstreamer_available = gstreamer_runtime_available();

        if !gstreamer_available {
            if let Some((pixels, width, height, original_width, original_height)) =
                extract_video_first_frame_without_gstreamer(path, max_texture_side)
            {
                return Some(FirstFrame::new(
                    pixels,
                    width,
                    height,
                    original_width,
                    original_height,
                ));
            }
            return None;
        }

        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_app as gst_app;
        use gstreamer_video as gst_video;
        use parking_lot::Mutex;
        use std::sync::Arc;
        use std::time::Duration;

        use crate::video_player::run_gst_with_timeout;

        // Initialize GStreamer if needed (static check to avoid repeated init)
        static GST_INIT: std::sync::OnceLock<Result<(), ()>> = std::sync::OnceLock::new();
        let init_result = GST_INIT.get_or_init(|| {
            let result = gst::init().map_err(|_| ());
            if result.is_ok() {
                crate::video_player::apply_av1_decoder_stability_ranks();
            }
            result
        });
        if init_result.is_err() {
            return None;
        }

        // Build URI from path
        let uri = gst::glib::filename_to_uri(path, None).ok()?.to_string();

        let source_dimensions = Some(Self::probe_video_dimensions_fast(path, Some(_probe_permit)));
        let output_dimensions =
            Self::video_thumbnail_output_dimensions(source_dimensions, max_texture_side);
        let caps_filter = match output_dimensions {
            Some((width, height)) if width > 0 && height > 0 => {
                format!("video/x-raw,format=RGBA,width={},height={}", width, height)
            }
            _ => "video/x-raw,format=RGBA".to_string(),
        };

        // Create a minimal pipeline for frame extraction
        // Use decodebin for auto-detection and videoscale/videoconvert for format conversion
        let pipeline_str = format!(
            "uridecodebin uri=\"{}\" name=dec ! videoconvert ! videoscale ! \
             {} ! appsink name=sink max-buffers=1 drop=true",
            uri.replace("\"", "\\\""),
            caps_filter
        );

        Self::note_gst_probe();
        let pipeline = gst::parse::launch(&pipeline_str).ok()?;
        let pipeline = pipeline.downcast::<gst::Pipeline>().ok()?;

        // Get the appsink element
        let appsink = pipeline
            .by_name("sink")?
            .dynamic_cast::<gst_app::AppSink>()
            .ok()?;

        // Storage for extracted frame
        let frame_data: Arc<Mutex<Option<(Vec<u8>, u32, u32)>>> = Arc::new(Mutex::new(None));
        let frame_data_clone = Arc::clone(&frame_data);

        // Set up preroll callback to capture the first frame
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_preroll(move |sink| {
                    if let Ok(sample) = sink.pull_preroll() {
                        if let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps()) {
                            if let Ok(video_info) = gst_video::VideoInfo::from_caps(caps) {
                                let width = video_info.width();
                                let height = video_info.height();
                                if let Ok(map) = buffer.map_readable() {
                                    let pixels = map.as_slice().to_vec();
                                    let mut data = frame_data_clone.lock();
                                    *data = Some((pixels, width, height));
                                }
                            }
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Set to PAUSED to get the first frame (preroll)
        let pipeline_paused = pipeline.clone();
        if run_gst_with_timeout("manga-thumb-paused", Duration::from_secs(8), move || {
            pipeline_paused.set_state(gst::State::Paused)
        })
        .map_or(true, |result| result.is_err())
        {
            let pipeline_null = pipeline.clone();
            let _ = run_gst_with_timeout("manga-thumb-null", Duration::from_secs(1), move || {
                pipeline_null.set_state(gst::State::Null)
            });
            return None;
        }

        let bus = pipeline.bus()?;
        let wait_for_frame = |timeout_ms: u64| -> bool {
            let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
            while std::time::Instant::now() < deadline {
                if frame_data.lock().is_some() {
                    return true;
                }

                if let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(50)) {
                    match msg.view() {
                        gst::MessageView::Error(_) => return false,
                        gst::MessageView::AsyncDone(_) | gst::MessageView::Eos(_) => {
                            if frame_data.lock().is_some() {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
            }

            frame_data.lock().is_some()
        };

        let _ = wait_for_frame(300);
        let pre_seek_frame = frame_data.lock().clone();
        *frame_data.lock() = None;
        let pipeline_seek = pipeline.clone();
        let seeked_to_zero =
            run_gst_with_timeout("manga-thumb-seek", Duration::from_secs(2), move || {
                pipeline_seek
                    .seek_simple(
                        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                        gst::ClockTime::ZERO,
                    )
                    .is_ok()
            })
            .unwrap_or(false);
        let extracted_frame = if seeked_to_zero {
            if wait_for_frame(3000) {
                frame_data.lock().clone()
            } else {
                None
            }
        } else {
            pre_seek_frame
        };

        // Cleanup pipeline
        let pipeline_null = pipeline.clone();
        let _ = run_gst_with_timeout("manga-thumb-null", Duration::from_secs(1), move || {
            pipeline_null.set_state(gst::State::Null)
        });

        // Extract the frame data
        let (pixels, width, height) = extracted_frame?;

        if pixels.is_empty() || width == 0 || height == 0 {
            return None;
        }

        let (original_width, original_height) = source_dimensions.unwrap_or((width, height));

        // Downscale if needed for GPU texture limits
        let (final_width, final_height, final_pixels) = downscale_rgba_if_needed(
            width,
            height,
            &pixels,
            max_texture_side,
            FilterType::Triangle,
        );

        Some(FirstFrame::new(
            final_pixels.into_owned(),
            final_width,
            final_height,
            original_width,
            original_height,
        ))
    }

    /// Preload counts (ahead, behind) from the applied visible-item signal.
    fn calculate_preload_counts(&self) -> (usize, usize) {
        let visible_items = self.visible_page_count.max(1);
        let ahead = visible_items
            .saturating_mul(PRELOAD_LOOK_AHEAD_MULTIPLIER)
            .clamp(MIN_PRELOAD_AHEAD, MAX_PRELOAD_AHEAD);
        let behind = visible_items
            .saturating_mul(PRELOAD_LOOK_BEHIND_MULTIPLIER)
            .clamp(MIN_PRELOAD_BEHIND, MAX_PRELOAD_BEHIND);

        (ahead, behind)
    }

    fn strip_window_counts(visible_item_equivalent: f32) -> (usize, usize) {
        let visible = visible_item_equivalent.max(1.0);
        let behind = (visible * STRIP_PRELOAD_LOOK_BEHIND_MULTIPLIER)
            .ceil()
            .max(1.0) as usize;
        let ahead = (visible * STRIP_PRELOAD_LOOK_AHEAD_MULTIPLIER)
            .ceil()
            .max(1.0) as usize;

        (behind, ahead)
    }

    /// (ahead, behind) preload counts for the current layout signal, direction-agnostic.
    fn effective_preload_counts(&self) -> (usize, usize) {
        match self.strip_visible_item_equivalent {
            Some(visible_item_equivalent) => {
                let (behind, ahead) = Self::strip_window_counts(visible_item_equivalent);
                (ahead, behind)
            }
            None => self.calculate_preload_counts(),
        }
    }

    /// Dimension-probe window around the anchor, biased by scroll direction.
    /// Grid layouts get row-scaled 384/768 caps; strip gets its fractional look window.
    fn probe_window(demand: &Demand<'_>) -> (usize, usize) {
        if demand.layout.is_grid() {
            let rows = demand.masonry_rows.clamp(2, 10);
            let directional_visible = demand.visible_indices.len().max(1);
            let forward = directional_visible
                .saturating_mul(2)
                .max(rows.saturating_mul(8));
            let backward = directional_visible.max(rows.saturating_mul(4));
            let max_behind = 384usize.saturating_mul(rows);
            let max_ahead = 768usize.saturating_mul(rows);

            if demand.moving_up {
                (forward.min(max_behind), backward.min(max_ahead))
            } else {
                (backward.min(max_behind), forward.min(max_ahead))
            }
        } else {
            let visible_item_equivalent = demand
                .strip_visible_item_equivalent
                .unwrap_or(demand.visible_indices.len() as f32);
            let (look_backward, look_forward) = Self::strip_window_counts(visible_item_equivalent);

            if demand.moving_up {
                (look_forward, look_backward)
            } else {
                (look_backward, look_forward)
            }
        }
    }

    /// Entry-count capacity target: visible density × zoom tier, byte-clamped for grid layouts.
    fn cache_capacity_target(visible_indices_count: usize, demand: &Demand<'_>) -> usize {
        let visible = demand.visible_page_count.max(visible_indices_count).max(1);
        let mut target = if demand.layout.is_grid() {
            let rows = demand.masonry_rows.clamp(2, 10);
            let zoom_factor = if demand.zoom <= 0.35 {
                4
            } else if demand.zoom <= 0.55 {
                3
            } else if demand.zoom <= 0.80 {
                2
            } else {
                1
            };
            let density_factor = 3 + rows;

            visible
                .saturating_mul(density_factor)
                .saturating_mul(zoom_factor)
                .clamp(MANGA_CACHE_MIN_ENTRIES, MANGA_CACHE_MAX_ENTRIES)
        } else {
            let zoom_factor = if demand.zoom <= 0.45 {
                3
            } else if demand.zoom <= 0.75 {
                2
            } else {
                1
            };

            visible
                .saturating_mul(5)
                .saturating_mul(zoom_factor)
                .clamp(MANGA_CACHE_MIN_ENTRIES, 512)
        };

        if demand.layout.is_grid() {
            // Byte-aware clamp: oversized textures must not inflate the entry-count target.
            // 1.5 x RGBA is the decode+texture peak per entry.
            let est_side = demand.target_texture_side.max(1) as usize;
            let est_bytes_per_texture = est_side
                .saturating_mul(est_side)
                .saturating_mul(4)
                .saturating_mul(3)
                / 2;
            let budget_bytes = if demand.navigation_active {
                MASONRY_CACHE_BUDGET_BYTES_NAV
            } else {
                MASONRY_CACHE_BUDGET_BYTES_IDLE
            };
            let byte_limited_capacity = (budget_bytes / est_bytes_per_texture.max(1))
                .clamp(MANGA_CACHE_MIN_ENTRIES, MANGA_CACHE_MAX_ENTRIES);
            let keep_floor = demand
                .visible_indices
                .len()
                .max(1)
                .max(demand.masonry_rows.clamp(2, 10).saturating_mul(4));
            target = target.min(byte_limited_capacity.max(keep_floor));
        }

        if demand.preserve_recent_cache {
            let retain_floor = demand
                .previous_cache_capacity
                .saturating_mul(7)
                .saturating_div(8)
                .max(demand.visible_indices.len().max(1));
            target = target.max(retain_floor);
        }

        target
    }

    /// Symmetric keep window around the anchor, scaled by the preload bias and the cache budget.
    fn keep_window(
        target_cache_capacity: usize,
        demand: &Demand<'_>,
        counts: (usize, usize),
    ) -> (usize, usize) {
        let (keep_ahead, keep_behind) = counts;

        let (mut final_keep_behind, mut final_keep_ahead) = if demand.layout.is_grid() {
            let rows = demand.masonry_rows.clamp(2, 10);
            let symmetric = keep_ahead
                .max(keep_behind)
                .saturating_mul((rows + 1) / 2)
                .max(12usize.saturating_mul(rows));
            (symmetric, symmetric)
        } else if demand.scrolling_up {
            (keep_ahead, keep_behind)
        } else {
            (keep_behind, keep_ahead)
        };

        let visible_budget = demand
            .visible_indices
            .len()
            .max(1)
            .min(target_cache_capacity.max(1));
        let keep_budget_total = target_cache_capacity.saturating_sub(visible_budget);
        if keep_budget_total == 0 {
            final_keep_behind = 0;
            final_keep_ahead = 0;
        } else {
            let requested_total = final_keep_behind.saturating_add(final_keep_ahead);
            if requested_total > keep_budget_total {
                let mut scaled_behind =
                    final_keep_behind.saturating_mul(keep_budget_total) / requested_total.max(1);
                let mut scaled_ahead = keep_budget_total.saturating_sub(scaled_behind);

                if final_keep_behind > 0 && scaled_behind == 0 {
                    scaled_behind = 1.min(keep_budget_total);
                    scaled_ahead = keep_budget_total.saturating_sub(scaled_behind);
                }
                if final_keep_ahead > 0 && scaled_ahead == 0 && keep_budget_total > 1 {
                    scaled_ahead = 1;
                    scaled_behind = keep_budget_total.saturating_sub(scaled_ahead);
                }

                final_keep_behind = scaled_behind;
                final_keep_ahead = scaled_ahead;
            }
        }

        (final_keep_behind, final_keep_ahead)
    }

    /// The only way the viewer says "this is what is on screen".
    ///
    /// Owns the whole derived sequence: LOD side, dimension-probe window, preload queue,
    /// cache capacity target, pinned set and keep-window eviction. The texture cache stays
    /// with the caller (it holds egui handles), so the plan comes back rather than gets applied.
    pub fn demand(&mut self, demand: &Demand<'_>) -> DemandPlan {
        let image_list = demand.image_list;
        let visible_indices_count = demand.visible_indices.len().max(1);
        self.last_layout = demand.layout;
        self.last_visible_indices_count = visible_indices_count;
        self.last_masonry_rows = demand.masonry_rows.clamp(2, 10);
        self.strip_visible_item_equivalent = demand
            .strip_visible_item_equivalent
            .filter(|value| value.is_finite())
            .map(|value| value.max(1.0));

        let target_texture_side =
            Self::quantize_target_texture_side(demand.target_texture_side, demand.max_texture_side);
        let cache_capacity = Self::cache_capacity_target(visible_indices_count, demand);
        self.cache_capacity_last = cache_capacity;
        // Cache-bounded visible count: capacity is derived from the raw count, then feeds back.
        self.visible_page_count = visible_indices_count.min(cache_capacity.max(1)).max(1);

        let (behind, ahead) = Self::probe_window(demand);
        let cache_start = demand.anchor_index.saturating_sub(behind);
        let cache_end = (demand.anchor_index + ahead).min(image_list.len());
        self.queue_dimension_range(image_list, cache_start, cache_end);
        self.rearm_failed_in_window(image_list, cache_start, cache_end);

        self.prefetch(
            image_list,
            demand.anchor_index,
            demand.max_texture_side,
            target_texture_side,
            demand.downscale_filter,
            demand.gif_filter,
            demand.force_triangle_filters,
        );

        let (keep_behind, keep_ahead) =
            Self::keep_window(cache_capacity, demand, self.effective_preload_counts());
        let keep_start = demand.anchor_index.saturating_sub(keep_behind);
        let keep_end = (demand
            .anchor_index
            .saturating_add(keep_ahead)
            .saturating_add(1))
        .min(image_list.len());
        let visible: HashSet<usize> = demand.visible_indices.iter().copied().collect();
        let mut evicted_indices = Vec::new();
        for &idx in demand.cached_indices {
            if !(keep_start..keep_end).contains(&idx) && !visible.contains(&idx) {
                self.forget_indices(std::iter::once(idx));
                evicted_indices.push(idx);
            }
        }

        // Wanted texture-cache state; main.rs owns the cache (egui handles) and applies it.
        DemandPlan {
            cache_capacity,
            pinned_indices: demand.visible_indices.to_vec(),
            evicted_indices,
        }
    }

    /// Strip-window (behind, ahead) for a fractional viewport coverage signal.
    pub fn strip_window_for_visible_equivalent(visible_item_equivalent: f32) -> (usize, usize) {
        Self::strip_window_counts(visible_item_equivalent)
    }

    /// Jump-prime window (behind, ahead) for a destination far from the current demand.
    fn prime_window(&self) -> (usize, usize) {
        if self.last_layout.is_grid() {
            let rows = self.last_masonry_rows.clamp(2, 10);
            let visible = self.last_visible_indices_count.max(1);
            (
                visible.max(rows.saturating_mul(4)),
                visible.saturating_mul(2).max(rows.saturating_mul(8)),
            )
        } else {
            Self::strip_window_counts(self.strip_visible_item_equivalent.unwrap_or(1.0))
        }
    }

    /// Load a navigation destination before the viewport actually arrives there.
    /// Same windows and priority path as a `demand` at that index, minus the cache plan.
    pub fn prime(
        &mut self,
        image_list: &[PathBuf],
        index: usize,
        max_texture_side: u32,
        target_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        force_triangle_filters: bool,
    ) {
        let index = index.min(image_list.len().saturating_sub(1));
        let (behind, ahead) = self.prime_window();
        let start = index.saturating_sub(behind);
        let end = index.saturating_add(ahead).min(image_list.len());
        self.queue_dimension_range(image_list, start, end);
        let target_texture_side =
            Self::quantize_target_texture_side(target_texture_side, max_texture_side);
        self.rearm_failed_in_window(image_list, start, end);
        self.prefetch(
            image_list,
            index,
            max_texture_side,
            target_texture_side,
            downscale_filter,
            gif_filter,
            force_triangle_filters,
        );
    }

    fn prefetch(
        &mut self,
        image_list: &[PathBuf],
        visible_index: usize,
        max_texture_side: u32,
        target_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        force_triangle_filters: bool,
    ) {
        if image_list.is_empty() {
            return;
        }

        let target_texture_side =
            Self::quantize_target_texture_side(target_texture_side, max_texture_side);
        let (request_downscale_filter, request_gif_filter) = if force_triangle_filters {
            (FilterType::Triangle, FilterType::Triangle)
        } else {
            (downscale_filter, gif_filter)
        };

        // Detect a far jump (e.g., dragging the scrollbar, Home/End, or any other big reposition).
        // On a far jump we cancel older work and make the target page the only "urgent" item.
        let prev_visible_index = self.last_visible_index;
        let index_delta = visible_index.abs_diff(prev_visible_index);
        let is_large_jump = index_delta > LARGE_JUMP_INDEX_THRESHOLD;
        if is_large_jump {
            self.cancel_all();
        }

        // Update scroll direction based on visible index change
        if visible_index > prev_visible_index {
            self.scroll_direction = 1; // Scrolling down
        } else if visible_index < prev_visible_index {
            self.scroll_direction = -1; // Scrolling up
        }
        self.last_visible_index = visible_index;

        // Apply scroll direction bias to the counts set by `demand`
        let (base_ahead, base_behind) = self.effective_preload_counts();

        // Apply scroll direction bias
        let ahead = if self.scroll_direction > 0 {
            base_ahead
        } else {
            base_behind
        };
        let behind = if self.scroll_direction > 0 {
            base_behind
        } else {
            base_ahead
        };

        let start_idx = visible_index.saturating_sub(behind);
        let end_idx = (visible_index + ahead + 1).min(image_list.len());

        // A video that scrolled out of the window gets a fresh probe chance on the way back:
        // both its gate cooldown and (for videos only) the download budget spent on blocked
        // probes are dropped once the index leaves the window.
        clear_probe_cooldowns_outside(start_idx, end_idx);
        let mut reentered: Vec<usize> = Vec::new();
        if !self.probe_blocked.is_empty() {
            let (start, end) = (start_idx, end_idx);
            reentered = self
                .probe_blocked
                .keys()
                .copied()
                .filter(|index| *index < start || *index >= end)
                .collect();
            self.probe_blocked
                .retain(|index, _| *index >= start && *index < end);
        }
        // An image with a spent budget is genuinely broken (corrupt header), so only videos are
        // revived here: their budget was burnt by gate timeouts rather than by the file.
        reentered.retain(|index| {
            image_list
                .get(*index)
                .is_some_and(|path| is_supported_video(path))
        });
        self.rearm_indices(reentered);
        // A video tile inside the window whose budget is spent waits for its revive window; the
        // ceiling has to pace re-attempts here or nothing would ever re-request it again.
        self.revive_spent_video_thumbnails(image_list, start_idx, end_idx);

        // Collect indices that need loading
        let mut requests: Vec<LoadRequest> = Vec::new();
        let generation = self.current_generation;
        let now = Instant::now();

        {
            let loading = self.loading_indices.read();
            let loaded = self.loaded_levels.read();
            let retry = self.retry_state.read();

            for idx in start_idx..end_idx {
                let request_target_texture_side = self.limit_target_texture_side_for_index(
                    idx,
                    target_texture_side,
                    max_texture_side,
                );

                // Check if file is a supported media type (image or video)
                let is_image = is_supported_image(&image_list[idx]);
                let is_video = is_supported_video(&image_list[idx]);
                if !is_image && !is_video {
                    continue;
                }

                // Skip if already loading or loaded at sufficient LOD.
                if loading.get(&idx).copied() == Some(generation)
                    || loaded
                        .get(&idx)
                        .is_some_and(|side| *side >= request_target_texture_side)
                {
                    continue;
                }

                if let Some(retry_state) = retry.get(&idx) {
                    let retry_due = now >= retry_state.next_retry_at;
                    if retry_state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS || !retry_due {
                        continue;
                    }
                }

                // Gate-blocked video: wait out the probe backoff instead of burning attempts.
                if self
                    .probe_blocked
                    .get(&idx)
                    .is_some_and(|state| now < state.next_probe_at)
                {
                    continue;
                }

                // Calculate priority based on distance from visible index
                // and scroll direction
                let distance = (idx as i32 - visible_index as i32).abs();
                let direction_bonus = if self.scroll_direction > 0 {
                    if idx > visible_index {
                        0
                    } else {
                        10
                    }
                } else {
                    if idx < visible_index {
                        0
                    } else {
                        10
                    }
                };

                // On a far jump, mark the actual target page as "urgent" so it can preempt
                // any neighbor prefetch and be decoded first.
                let priority = if is_large_jump && idx == visible_index {
                    -100_000
                } else {
                    distance + direction_bonus
                };

                requests.push(LoadRequest {
                    generation,
                    index: idx,
                    path: image_list[idx].clone(),
                    max_texture_side,
                    target_texture_side: request_target_texture_side,
                    downscale_filter: request_downscale_filter,
                    gif_filter: request_gif_filter,
                    priority,
                    queued_at: Instant::now(),
                });
            }
        }

        // Send requests (sorted by priority). IMPORTANT:
        // Only mark an index as "loading" after the request is successfully enqueued.
        // If the channel is full during fast scrollbar drags, dropping a request while still
        // marking it as loading will permanently wedge that item in the UI.
        //
        // In-flight cap: a window that slides every frame used to accumulate one extra decode per
        // step (`loads:57` in the crash log). Bound the in-flight set to the prefetch window or
        // two decodes per worker, whichever is larger. Priorities sort the visible index (and any
        // urgent demand) first, and those are exempt, so the cap can only cut speculative work.
        let window = end_idx.saturating_sub(start_idx);
        let max_inflight = window.max(rayon::current_num_threads().saturating_mul(2));
        let mut inflight = self.loading_indices.read().len();

        requests.sort_by_key(|r| r.priority);
        for req in requests {
            let idx = req.index;
            let exempt = idx == visible_index || req.priority < 0;
            if inflight >= max_inflight && !exempt {
                continue;
            }
            match self.request_tx.try_send(req) {
                Ok(()) => {
                    self.loading_indices.write().insert(idx, generation);
                    inflight = inflight.saturating_add(1);
                }
                Err(TrySendError::Full(_req)) => {
                    // Backpressure: stop here so already-enqueued high priority work runs first.
                    // Remaining items will be retried next frame.
                    break;
                }
                Err(TrySendError::Disconnected(_req)) => {
                    break;
                }
            }
        }

        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Drain queued work products: decoded images and dimension results, both bounded.
    /// The only way the caller asks for work products; counts come from `backlog`.
    pub fn take(&mut self, max_decoded: usize, max_dimension_results: usize) -> Taken {
        let (decoded, decoded_dimension_updates) = self.take_decoded(max_decoded);
        let mut dimension_updates = self.apply_dimension_results(max_dimension_results);
        dimension_updates.extend(decoded_dimension_updates);
        dimension_updates.sort_unstable();
        dimension_updates.dedup();

        Taken {
            decoded,
            dimension_updates,
        }
    }

    /// Decoded images ready for GPU upload, bounded by `max_items` (0 takes none).
    /// Also returns indices whose cached source dimensions changed.
    fn take_decoded(&mut self, max_items: usize) -> (Vec<DecodedImage>, Vec<usize>) {
        let max_items = max_items.min(MAX_PENDING_UPLOADS);
        let mut results = Vec::with_capacity(max_items);
        let mut dimension_updates = Vec::with_capacity(max_items);

        for _ in 0..max_items {
            match self.result_rx.try_recv() {
                Ok(decoded) => {
                    // Cache dimensions and media type for stable layout. A corrupt header must not
                    // land in `dimension_cache`; prefer the decoded texture size, else keep the
                    // dimensions the layout already has.
                    let new_dims =
                        sanitize_probed_dimensions(decoded.original_width, decoded.original_height)
                            .or_else(|| sanitize_probed_dimensions(decoded.width, decoded.height))
                            .map(|(w, h)| (w, h, decoded.media_type));
                    let changed = new_dims.is_some_and(|new_dims| {
                        self.dimension_cache.get(&decoded.index).map_or(
                            true,
                            |(old_w, old_h, old_mt)| {
                                if *old_w == new_dims.0
                                    && *old_h == new_dims.1
                                    && *old_mt == new_dims.2
                                {
                                    return false;
                                }

                                // Ignore tiny video size jitter to avoid perpetual masonry relayouts.
                                if *old_mt == MangaMediaType::Video
                                    && new_dims.2 == MangaMediaType::Video
                                {
                                    let width_delta = old_w.abs_diff(new_dims.0);
                                    let height_delta = old_h.abs_diff(new_dims.1);
                                    if width_delta <= 2 && height_delta <= 2 {
                                        let old_aspect = *old_w as f32 / (*old_h).max(1) as f32;
                                        let new_aspect =
                                            new_dims.0 as f32 / new_dims.1.max(1) as f32;
                                        if (old_aspect - new_aspect).abs() <= 0.003 {
                                            return false;
                                        }
                                    }
                                }

                                true
                            },
                        )
                    });

                    if let (true, Some(new_dims)) = (changed, new_dims) {
                        self.dimension_cache.insert(decoded.index, new_dims);
                        if !dimension_updates.contains(&decoded.index) {
                            dimension_updates.push(decoded.index);
                        }
                    }

                    results.push(decoded);
                    self.stats.images_loaded += 1;
                }
                Err(_) => break,
            }
        }

        (results, dimension_updates)
    }

    /// Start async dimension caching for all images in the list.
    /// This returns immediately and caches dimensions in the background.
    /// The first few visible images are prioritized.
    pub fn prime_dimension_cache(&mut self, image_list: &[PathBuf]) -> usize {
        // For fast startup, only cache the first batch of visible media synchronously.
        // The rest will be cached on-demand or when media is loaded.
        const INITIAL_CACHE_COUNT: usize = 30;

        // Clear existing cache
        self.dimension_cache.clear();

        if image_list.is_empty() {
            return 0;
        }

        let cached_lookup_items: Vec<(usize, CachedMediaKind, PathBuf)> = image_list
            .iter()
            .enumerate()
            .filter_map(|(idx, path)| {
                if is_supported_video(path) {
                    Some((idx, CachedMediaKind::Video, path.clone()))
                } else if is_supported_image(path) {
                    Some((idx, CachedMediaKind::Image, path.clone()))
                } else {
                    None
                }
            })
            .collect();

        if !cached_lookup_items.is_empty() {
            let batch_items: Vec<(PathBuf, CachedMediaKind)> = cached_lookup_items
                .iter()
                .map(|(_, kind, path)| (path.clone(), *kind))
                .collect();
            let cached_results = lookup_cached_dimensions_batch(&batch_items);

            for ((idx, kind, _), cached) in
                cached_lookup_items.iter().zip(cached_results.into_iter())
            {
                if let Some((w, h)) = cached.and_then(|(w, h)| sanitize_probed_dimensions(w, h)) {
                    let media_type = match kind {
                        CachedMediaKind::Image => MangaMediaType::StaticImage,
                        CachedMediaKind::Video => MangaMediaType::Video,
                    };
                    self.dimension_cache.insert(*idx, (w, h, media_type));
                }
            }
        }

        // Cache first batch synchronously for immediate layout. WebP stays async:
        // animated WebP folders can contain many costly headers, and folder travel
        // must paint the destination before masonry warm-up does heavier work.
        let initial_probe_items: Vec<(usize, PathBuf)> = image_list
            .iter()
            .enumerate()
            .filter(|(idx, path)| {
                !self.dimension_cache.contains_key(idx)
                    && (is_supported_video(path) || is_supported_image(path))
                    && !Self::path_is_webp(path)
            })
            .take(INITIAL_CACHE_COUNT)
            .map(|(idx, path)| (idx, path.clone()))
            .collect();
        let initial_batch: Vec<(usize, Option<(u32, u32, MangaMediaType)>)> = initial_probe_items
            .par_iter()
            .map(|(idx, path)| {
                let is_video = is_supported_video(path);
                let is_image = is_supported_image(path);

                if is_video {
                    // For videos, probe dimensions
                    let dims = Self::probe_video_dimensions(*idx, path);
                    (*idx, dims.map(|(w, h)| (w, h, MangaMediaType::Video)))
                } else if is_image {
                    // For images, get from file header
                    let dims = Self::probe_image_dimensions_cached(path);
                    // We can't easily determine if an image is animated without loading it
                    // Default to static, will be updated when actually loaded
                    (*idx, dims.map(|(w, h)| (w, h, MangaMediaType::StaticImage)))
                } else {
                    (*idx, None)
                }
            })
            .collect();
        for (idx, opt_dims) in initial_batch {
            // Skip unusable headers: a later probe/decode fills the entry instead of poisoning it.
            let Some((w, h, media_type)) = opt_dims else {
                continue;
            };
            if let Some((w, h)) = sanitize_probed_dimensions(w, h) {
                self.dimension_cache.insert(idx, (w, h, media_type));
            }
        }

        // The rest will be cached on-demand when media is loaded
        // or when manga_get_image_display_height is called
        self.cached_dimensions_count(image_list.len())
    }

    fn clear_with_dimension_policy(&mut self, preserve_dimensions: bool) {
        // Increment generation to invalidate pending requests
        self.current_generation += 1;
        self.generation
            .store(self.current_generation, Ordering::Release);

        // Clear indices
        self.loading_indices.write().clear();
        self.loaded_levels.write().clear();
        self.retry_state.write().clear();
        self.probe_blocked.clear();
        self.video_revive.clear();

        // Clear dimension cache
        if !preserve_dimensions {
            self.dimension_cache.clear();
        }

        // Clear any queued dimension probes
        self.dim_pending.clear();
        clear_all_probe_cooldowns();

        // Drain dimension result channel
        while self.dim_result_rx.try_recv().is_ok() {}

        // Drain result channel
        while self.result_rx.try_recv().is_ok() {}

        // Note: we can't directly drain the request channel from here because we only own
        // the Sender; cancellation is handled via generation checks in the coordinator.
        self.stats = LoaderStats::default();
    }

    /// Clear all caches and reset state (called when exiting manga mode).
    pub fn clear(&mut self) {
        self.clear_with_dimension_policy(false);
    }

    /// Clear runtime state but keep the current list's cached dimensions warm.
    /// Drop runtime state; keep the cached dimensions warm. Distinct from `cancel_all`:
    /// this drops dimensions, `cancel_all` keeps them and only cancels pending work.
    pub fn clear_preserving_dimensions(&mut self) {
        self.clear_with_dimension_policy(true);
    }

    /// Forget loaded/retry bookkeeping for evicted indices so they can be re-requested.
    /// In-flight requests are untouched; only `reset_index_state` clears those.
    pub fn forget_indices<I>(&mut self, indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        let mut loaded = self.loaded_levels.write();
        let mut retry = self.retry_state.write();
        for index in indices {
            loaded.remove(&index);
            retry.remove(&index);
        }
    }

    /// Release one index's in-flight and loaded bookkeeping but keep its failure history.
    ///
    /// Used for work that has been in flight far too long: the tile must be able to re-request,
    /// but a file that already exhausted its retry budget must stay at the ceiling. Clearing the
    /// retry state here instead would give a corrupt file a fresh budget every stale sweep.
    pub fn release_inflight(&mut self, index: usize) {
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Drop all bookkeeping for one index so a stale/placeholder tile can re-request.
    pub fn reset_index_state_for(&mut self, index: usize) {
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.retry_state.write().remove(&index);
        self.probe_blocked.remove(&index);
        self.clear_probe_cooldown_for(index);
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Drop this index's gate cooldown and probe backoff. Used when the tile comes back into view
    /// or is hovered: a video that scrolled away and returned gets a fresh probed attempt.
    pub fn clear_probe_cooldown_for(&mut self, index: usize) {
        clear_probe_cooldown(index);
        self.probe_blocked.remove(&index);
    }

    /// True while a video index waits out its `ProbeBlockedState` backoff after gate misses.
    /// Paces probe attempts only; the caller still treats the tile as outstanding work.
    fn probe_backoff_active(&self, index: usize, now: Instant) -> bool {
        self.probe_blocked
            .get(&index)
            .is_some_and(|state| now < state.next_probe_at)
    }

    /// Clear the gate-failure history for `index`: a payload actually reached the texture cache.
    ///
    /// Leaving the entry in place kept `demand_outstanding` true for a tile that had already
    /// resolved, so the parked frame loop kept waking every pump interval for the session.
    pub fn note_probe_succeeded(&mut self, index: usize) {
        self.probe_blocked.remove(&index);
        self.video_revive.remove(&index);
    }

    /// Record that the last attempt for `index` could not run its gated probe.
    pub fn note_probe_blocked(&mut self, index: usize) {
        let now = Instant::now();
        self.probe_blocked
            .entry(index)
            .or_insert_with(|| ProbeBlockedState {
                misses: 0,
                next_probe_at: now,
            })
            .note_miss(now);
    }

    /// A video thumbnail's spent budget re-opens on a widening schedule; images stay dead.
    fn video_revive_due(&self, index: usize, now: Instant) -> bool {
        match self.video_revive.get(&index) {
            None => true,
            Some(state) => now >= state.next_revive_at,
        }
    }

    /// True while a visible video tile still has thumbnail work scheduled: a spent budget a
    /// revive will re-open, or a gate backoff that paces the probe. Its placeholder is honest
    /// waiting work, so the pump may keep running; an image at the ceiling has none.
    fn video_thumbnail_revive_pending(&self, index: usize, path: &Path) -> bool {
        if !is_supported_video(path) {
            return false;
        }
        if self.probe_blocked.contains_key(&index) {
            return true;
        }
        let spent = self
            .retry_state
            .read()
            .get(&index)
            .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS);
        spent
            && self
                .video_revive
                .get(&index)
                .is_none_or(|state| state.revives < VIDEO_THUMBNAIL_REVIVE_MAX)
    }

    /// Hover/visible demand on a video tile: drop the pacing so the next pump re-opens the budget
    /// now, and clear the gate backoff so the probe is not refused. Never runs a probe itself — a
    /// first-frame decode on the frame path freezes the window for its whole timeout.
    ///
    /// Callers run this every frame on the hovered tile, so it is a no-op unless the tile is
    /// actually pacing: clearing a fresh backoff each frame would re-queue a probe every pump.
    pub fn accelerate_video_thumbnail_revive(&mut self, index: usize) {
        let now = Instant::now();
        let pacing = self.probe_blocked.contains_key(&index)
            || self
                .video_revive
                .get(&index)
                .is_some_and(|state| now < state.next_revive_at);
        if !pacing {
            return;
        }
        clear_probe_cooldown(index);
        self.probe_blocked.remove(&index);
        if let Some(state) = self.video_revive.get_mut(&index) {
            state.next_revive_at = now;
        }
    }

    /// Re-open the spent thumbnail budget of every video in the window whose revive window has
    /// elapsed. Images are untouched: their ceiling means the file is broken.
    fn revive_spent_video_thumbnails(&mut self, image_list: &[PathBuf], start: usize, end: usize) {
        let now = Instant::now();
        let due: Vec<usize> = {
            let retry = self.retry_state.read();
            (start..end.min(image_list.len()))
                .filter(|&idx| {
                    image_list
                        .get(idx)
                        .is_some_and(|path| is_supported_video(path))
                })
                .filter(|&idx| {
                    retry
                        .get(&idx)
                        .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS)
                })
                .filter(|&idx| self.video_revive_due(idx, now))
                .collect()
        };

        for index in due {
            self.video_revive
                .entry(index)
                .or_insert_with(|| VideoReviveState {
                    revives: 0,
                    next_revive_at: now,
                })
                .note_revive(now);
            self.rearm_index(index);
        }
    }

    /// Drop every memo that makes `index` look settled: retry ceiling, in-flight/loaded levels,
    /// gate cooldown and probe backoff. Used when a dropped payload or a re-entered window means
    /// the tile never actually got a chance to resolve.
    pub fn rearm_index(&mut self, index: usize) {
        self.reset_index_state_for(index);
        self.release_inflight(index);
    }

    /// `rearm_index` for a batch, skipping indices that already have a texture.
    pub fn rearm_indices<I>(&mut self, indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        for index in indices {
            self.rearm_index(index);
        }
    }

    /// Record that `index` cannot be drawn at all: pin it at the retry ceiling (so prefetch
    /// and every visible-retry path skip it) and drop its in-flight/loaded bookkeeping.
    ///
    /// This is the "X card" state. It is cleared only by [`Self::rearm_failed_indices`], i.e.
    /// when the file itself changed, which is what keeps a corrupt GIF off the CPU entirely.
    pub fn mark_unloadable(&mut self, index: usize, path: &Path) {
        let stamp = FileStamp::of_path(path);
        self.loading_indices.write().remove(&index);
        self.loaded_levels.write().remove(&index);
        self.retry_state.write().insert(
            index,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp,
                stamp_checked_at: Instant::now(),
            },
        );
        self.stats.images_pending = self.loading_indices.read().len();
    }

    /// Record that `index` produced no usable texture, keeping the bounded retry backoff intact
    /// so a corrupt file cannot spin the pump.
    pub fn register_upload_failure(&mut self, index: usize, path: &Path) {
        Self::register_decode_failure(
            &self.loaded_levels,
            &self.retry_state,
            index,
            FileStamp::of_path(path),
        );
    }

    /// Release a stranded dimension probe so the index can be probed again.
    pub fn drop_dim_pending(&mut self, index: usize) {
        self.dim_pending.remove(&index);
    }

    /// True once `index` exhausted its retry budget; nothing else will re-request it, so the
    /// caller must stop treating it as outstanding work.
    pub fn retry_blocked(&self, index: usize) -> bool {
        self.retry_state
            .read()
            .get(&index)
            .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS)
    }

    /// Highest level the loader believes is loaded for `index` (may be set with no texture).
    pub fn loaded_level_for(&self, index: usize) -> Option<u32> {
        self.loaded_levels.read().get(&index).copied()
    }

    /// True while a request for `index` is in flight for the active generation.
    pub fn is_index_in_flight(&self, index: usize) -> bool {
        self.loading_indices.read().get(&index).copied() == Some(self.current_generation)
    }

    /// Snapshot of in-flight work for the caller's stall tracking:
    /// `(indices decoding, indices awaiting a dimension probe)`.
    pub fn inflight_index_snapshot(&self) -> (Vec<usize>, Vec<usize>) {
        (
            self.loading_indices.read().keys().copied().collect(),
            self.dim_pending.iter().copied().collect(),
        )
    }

    /// Sync jump-tracking to an externally handled visible-index change.
    ///
    /// Masonry scrollbar dragging can jump far enough that the UI already knows it has
    /// repositioned. Cancel stale work once, then stop `prefetch` from treating the same
    /// jump as a fresh one.
    pub fn sync_visible_index(&mut self, visible_index: usize, cancel_pending: bool) {
        let previous_index = self.last_visible_index;

        if visible_index > previous_index {
            self.scroll_direction = 1;
        } else if visible_index < previous_index {
            self.scroll_direction = -1;
        }

        if cancel_pending {
            self.cancel_all();
        }

        self.last_visible_index = visible_index;
    }

    /// Visible-index repair path: a high-priority retry for an in-view placeholder.
    ///
    /// Refused once the retry ceiling is reached: every repair variant re-arms the index (the
    /// `ForceReload`/`ResetState` repairs even drop its retry state), so without this guard the
    /// UI's heal and quality-upgrade paths would re-queue a corrupt file on every pass. A
    /// rewritten file is re-armed earlier, by `rearm_failed_indices`.
    pub fn retry_visible(&mut self, request: &VisibleRetry<'_>) -> bool {
        if self.retry_blocked(request.index) {
            // A video's spent thumbnail budget paces re-attempts instead of ending them: a visible
            // tile re-opens it as soon as its revive window elapses, without a hover or a file
            // change. Images stay refused — a corrupt image will not decode on the next pass.
            let is_video = request
                .image_list
                .get(request.index)
                .is_some_and(|path| is_supported_video(path));
            if !is_video || !self.video_revive_due(request.index, Instant::now()) {
                return false;
            }
            self.rearm_index(request.index);
        }

        match request.repair {
            RetryRepair::None => {}
            RetryRepair::ForceReload => self.forget_indices(std::iter::once(request.index)),
            RetryRepair::ResetState => self.reset_index_state_for(request.index),
        }

        self.request_visible_retry(
            request.image_list,
            request.index,
            request.max_texture_side,
            request.target_texture_side,
            request.downscale_filter,
            request.gif_filter,
            request.force_triangle_filters,
        )
    }

    /// Force a high-priority retry for a visible item that is missing a texture.
    ///
    /// This is used as a self-healing path when UI detects an in-view placeholder
    /// but loader bookkeeping says the item was already loaded.
    fn request_visible_retry(
        &mut self,
        image_list: &[PathBuf],
        index: usize,
        max_texture_side: u32,
        target_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        force_triangle_filters: bool,
    ) -> bool {
        let Some(path) = image_list.get(index).cloned() else {
            return false;
        };

        if !is_supported_image(&path) && !is_supported_video(&path) {
            return false;
        }

        if self.loading_indices.read().get(&index).copied() == Some(self.current_generation) {
            return false;
        }

        if self.retry_state.read().get(&index).is_some_and(|state| {
            state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS || Instant::now() < state.next_retry_at
        }) {
            return false;
        }

        // A gate-blocked video is outstanding work, just not yet: one attempt per backoff window.
        if self.probe_backoff_active(index, Instant::now()) {
            return false;
        }

        let target_texture_side =
            self.limit_target_texture_side_for_index(index, target_texture_side, max_texture_side);

        // Visible placeholder takes precedence over potentially stale loaded bookkeeping.
        // The UI only calls this for in-view placeholders or explicit quality upgrades, so
        // we intentionally bypass loaded-side short-circuits here to avoid wedging tiles
        // after cache eviction/downgrade races.
        self.loaded_levels.write().remove(&index);

        let (request_downscale_filter, request_gif_filter) = if force_triangle_filters {
            (FilterType::Triangle, FilterType::Triangle)
        } else {
            (downscale_filter, gif_filter)
        };

        let req = LoadRequest {
            generation: self.current_generation,
            index,
            path,
            max_texture_side,
            target_texture_side,
            downscale_filter: request_downscale_filter,
            gif_filter: request_gif_filter,
            priority: -200_000,
            queued_at: Instant::now(),
        };

        let send_result = match self.urgent_request_tx.try_send(req.clone()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(req)) | Err(TrySendError::Disconnected(req)) => {
                self.request_tx.try_send(req)
            }
        };

        match send_result {
            Ok(()) => {
                self.loading_indices
                    .write()
                    .insert(index, self.current_generation);
                self.retry_state.write().remove(&index);
                self.stats.images_pending = self.loading_indices.read().len();
                true
            }
            Err(TrySendError::Full(_req)) => false,
            Err(TrySendError::Disconnected(_req)) => false,
        }
    }

    /// Cancel all pending work (large jumps like Home/End, fast scrollbar drags, exit).
    /// Stale in-flight results are dropped by generation checks in the coordinator.
    pub fn cancel_all(&mut self) {
        // Increment generation to invalidate in-flight requests
        // The coordinator thread will check this and skip stale requests
        self.current_generation += 1;
        self.generation
            .store(self.current_generation, Ordering::Release);

        // Clear loading indices (they'll be re-requested around the new position)
        self.loading_indices.write().clear();

        // Drain result channel to clear stale decoded images.
        // Any drained decoded index may have been marked as loaded by workers,
        // so clear loaded bookkeeping for those entries.
        let mut drained_indices: Vec<usize> = Vec::new();
        while let Ok(decoded) = self.result_rx.try_recv() {
            drained_indices.push(decoded.index);
        }
        if !drained_indices.is_empty() {
            let mut loaded = self.loaded_levels.write();
            let mut retry = self.retry_state.write();
            for idx in drained_indices {
                loaded.remove(&idx);
                retry.remove(&idx);
            }
        }

        // Clear any queued dimension probes and drain stale results
        self.dim_pending.clear();
        while self.dim_result_rx.try_recv().is_ok() {}

        self.stats.images_pending = 0;
    }

    /// Get cached dimensions for an index (width, height only).
    pub fn get_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.dimension_cache.get(&index).map(|(w, h, _)| (*w, *h))
    }

    /// Get cached media info for an index (width, height, media_type).
    /// Get media type for an index.
    pub fn get_media_type(&self, index: usize) -> Option<MangaMediaType> {
        self.dimension_cache.get(&index).map(|(_, _, mt)| *mt)
    }

    /// Update dimensions for a video (called when actual dimensions are known from playback).
    pub fn update_video_dimensions(&mut self, index: usize, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }

        if let Some(entry) = self.dimension_cache.get_mut(&index) {
            if entry.0 == width && entry.1 == height && entry.2 == MangaMediaType::Video {
                return false;
            }

            // Ignore tiny frame-size jitter from some decoders; it should not reshuffle masonry.
            if entry.2 == MangaMediaType::Video {
                let width_delta = entry.0.abs_diff(width);
                let height_delta = entry.1.abs_diff(height);
                if width_delta <= 2 && height_delta <= 2 {
                    let old_aspect = entry.0 as f32 / entry.1.max(1) as f32;
                    let new_aspect = width as f32 / height as f32;
                    if (old_aspect - new_aspect).abs() <= 0.003 {
                        return false;
                    }
                }
            }

            entry.0 = width;
            entry.1 = height;
            entry.2 = MangaMediaType::Video;
            true
        } else {
            self.dimension_cache
                .insert(index, (width, height, MangaMediaType::Video));
            true
        }
    }
}

impl Default for MangaLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MangaLoader {
    fn drop(&mut self) {
        // Signal shutdown to coordinator thread
        self.shutdown.store(true, Ordering::Release);
    }
}

/// LRU-style texture cache for manga mode.
/// Keeps track of usage order for eviction.
pub struct MangaTextureCache {
    /// Non-evictable entries for currently visible indices.
    pinned_entries: HashMap<usize, MangaTextureEntry>,
    /// Evictable entries, ordered by recency.
    unpinned_entries: LruCache<usize, MangaTextureEntry>,
    /// Maximum number of entries to cache
    max_entries: usize,
    /// Indices that should not be evicted while still visible.
    pinned_indices: HashSet<usize>,
}

#[derive(Clone)]
struct MangaTextureEntry {
    path: PathBuf,
    texture: egui::TextureHandle,
    width: u32,
    height: u32,
    media_type: MangaMediaType,
}

impl MangaTextureCache {
    pub fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).expect("cache capacity is non-zero");
        Self {
            pinned_entries: HashMap::with_capacity(max_entries),
            unpinned_entries: LruCache::new(capacity),
            max_entries: max_entries.max(1),
            pinned_indices: HashSet::new(),
        }
    }

    fn total_entries(&self) -> usize {
        self.pinned_entries.len() + self.unpinned_entries.len()
    }

    fn evict_to_capacity(&mut self) -> Vec<usize> {
        let mut evicted = Vec::new();

        while self.total_entries() > self.max_entries {
            let Some((idx, _)) = self.unpinned_entries.pop_lru() else {
                // All remaining entries are pinned; cannot evict further.
                break;
            };

            self.pinned_indices.remove(&idx);
            evicted.push(idx);
        }

        evicted
    }

    pub fn set_max_entries(&mut self, max_entries: usize) -> Vec<usize> {
        self.max_entries = max_entries.max(1);
        let capacity = NonZeroUsize::new(self.max_entries).expect("cache capacity is non-zero");
        self.unpinned_entries.resize(capacity);
        self.evict_to_capacity()
    }

    pub fn set_pinned_indices<I>(&mut self, pinned: I)
    where
        I: IntoIterator<Item = usize>,
    {
        let new_pinned: HashSet<usize> = pinned.into_iter().collect();

        let to_unpin: Vec<usize> = self
            .pinned_indices
            .difference(&new_pinned)
            .copied()
            .collect();
        for idx in to_unpin {
            if let Some(entry) = self.pinned_entries.remove(&idx) {
                self.unpinned_entries.put(idx, entry);
            }
        }

        let to_pin: Vec<usize> = new_pinned
            .difference(&self.pinned_indices)
            .copied()
            .collect();
        for idx in to_pin {
            if let Some(entry) = self.unpinned_entries.pop(&idx) {
                self.pinned_entries.insert(idx, entry);
            }
        }

        self.pinned_indices = new_pinned;
    }

    /// Increment frame counter (call once per frame).
    pub fn tick(&mut self) {
        // LruCache updates recency on access, so no per-frame bookkeeping is needed.
    }

    pub fn get_texture_info_for_path(
        &mut self,
        index: usize,
        path: &Path,
    ) -> Option<(egui::TextureId, u32, u32)> {
        if let Some(entry) = self.pinned_entries.get(&index) {
            return (entry.path.as_path() == path).then_some((
                entry.texture.id(),
                entry.width,
                entry.height,
            ));
        }

        self.unpinned_entries.get(&index).and_then(|entry| {
            (entry.path.as_path() == path).then_some((
                entry.texture.id(),
                entry.width,
                entry.height,
            ))
        })
    }

    pub fn peek_texture_dimensions_for_path(
        &self,
        index: usize,
        path: &Path,
    ) -> Option<(u32, u32)> {
        self.pinned_entries
            .get(&index)
            .and_then(|entry| (entry.path.as_path() == path).then_some((entry.width, entry.height)))
            .or_else(|| {
                self.unpinned_entries.peek(&index).and_then(|entry| {
                    (entry.path.as_path() == path).then_some((entry.width, entry.height))
                })
            })
    }

    pub fn get_texture_handle_info_for_path(
        &mut self,
        index: usize,
        path: &Path,
    ) -> Option<(egui::TextureHandle, u32, u32, MangaMediaType)> {
        if let Some(entry) = self.pinned_entries.get(&index) {
            return (entry.path.as_path() == path).then_some((
                entry.texture.clone(),
                entry.width,
                entry.height,
                entry.media_type,
            ));
        }

        self.unpinned_entries.get(&index).and_then(|entry| {
            (entry.path.as_path() == path).then_some((
                entry.texture.clone(),
                entry.width,
                entry.height,
                entry.media_type,
            ))
        })
    }

    pub fn contains_for_path(&self, index: usize, path: &Path) -> bool {
        self.pinned_entries
            .get(&index)
            .is_some_and(|entry| entry.path.as_path() == path)
            || self
                .unpinned_entries
                .peek(&index)
                .is_some_and(|entry| entry.path.as_path() == path)
    }

    /// Insert a texture into the cache with explicit media type.
    /// Returns evicted indices if cache was full.
    pub fn insert_with_type(
        &mut self,
        index: usize,
        path: PathBuf,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
        media_type: MangaMediaType,
    ) -> Vec<usize> {
        let entry = MangaTextureEntry {
            path,
            texture,
            width,
            height,
            media_type,
        };

        if self.pinned_indices.contains(&index) {
            self.unpinned_entries.pop(&index);
            self.pinned_entries.insert(index, entry);
        } else {
            self.pinned_entries.remove(&index);
            self.unpinned_entries.put(index, entry);
        }

        self.evict_to_capacity()
    }

    /// Update an existing texture in the cache (for video frame updates).
    /// Does not evict anything, just replaces the existing entry.
    pub fn update_texture(
        &mut self,
        index: usize,
        path: &Path,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) -> bool {
        if let Some(entry) = self.pinned_entries.get_mut(&index) {
            if entry.path.as_path() != path {
                return false;
            }
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            return true;
        }

        if let Some(entry) = self.unpinned_entries.get_mut(&index) {
            if entry.path.as_path() != path {
                return false;
            }
            entry.texture = texture;
            entry.width = width;
            entry.height = height;
            return true;
        }

        false
    }

    /// Remove an entry from the cache.
    pub fn remove(&mut self, index: usize) {
        self.pinned_entries.remove(&index);
        self.unpinned_entries.pop(&index);
        self.pinned_indices.remove(&index);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.pinned_entries.clear();
        self.unpinned_entries.clear();
        self.pinned_indices.clear();
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.total_entries() == 0
    }

    /// Get indices of all cached textures.
    pub fn cached_indices(&self) -> Vec<usize> {
        let mut indices = Vec::with_capacity(self.total_entries());
        indices.extend(self.pinned_entries.keys().copied());
        indices.extend(self.unpinned_entries.iter().map(|(idx, _)| *idx));
        indices
    }
}

impl Default for MangaTextureCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHED_TEXTURES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET_SIDE: u32 = 512;
    const MAX_SIDE: u32 = 4096;

    fn paths(n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("img{i:04}.png")))
            .collect()
    }

    fn demand<'a>(
        list: &'a [PathBuf],
        visible: &'a [usize],
        cached: &'a [usize],
        anchor: usize,
        layout: MangaLayoutMode,
    ) -> Demand<'a> {
        Demand {
            image_list: list,
            layout,
            anchor_index: anchor,
            visible_indices: visible,
            visible_page_count: visible.len().max(1),
            strip_visible_item_equivalent: (!layout.is_grid()).then_some(1.5),
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            masonry_rows: 3,
            zoom: 1.0,
            scrolling_up: false,
            moving_up: false,
            navigation_active: false,
            preserve_recent_cache: false,
            previous_cache_capacity: MANGA_CACHE_MIN_ENTRIES,
            cached_indices: cached,
        }
    }

    /// Dimension window `(start, end)` of the last queued probe batch.
    fn dim_window(loader: &MangaLoader) -> (usize, usize) {
        let probe = loader.probe.as_ref().expect("probe channels");
        let mut last: Option<DimRequest> = None;
        while let Ok(req) = probe.dim_requests.try_recv() {
            last = Some(req);
        }
        let req = last.expect("demand must enqueue a dimension probe window");
        let start = req.items.first().map(|(idx, _)| *idx).unwrap();
        let end = req.items.last().map(|(idx, _)| *idx + 1).unwrap();
        (start, end)
    }

    fn preload_requests(loader: &MangaLoader) -> Vec<LoadRequest> {
        loader
            .probe
            .as_ref()
            .expect("probe channels")
            .requests
            .try_iter()
            .collect()
    }

    fn urgent_requests(loader: &MangaLoader) -> Vec<LoadRequest> {
        loader
            .probe
            .as_ref()
            .expect("probe channels")
            .urgent
            .try_iter()
            .collect()
    }

    fn inject_dimensions(loader: &MangaLoader, items: Vec<(usize, u32, u32, MangaMediaType)>) {
        let results = loader
            .probe
            .as_ref()
            .expect("probe channels")
            .dim_results
            .clone();
        results
            .try_send(DimResult {
                generation: loader.current_generation,
                items,
            })
            .expect("dimension result channel has room");
    }

    #[test]
    fn video_probe_gate_releases_permit_on_early_return() {
        fn probe_that_bails(gate: &std::sync::Arc<VideoProbeGate>) -> Option<u32> {
            let _permit = gate.acquire_timeout(Duration::from_millis(1))?;
            None
        }

        let gate = std::sync::Arc::new(VideoProbeGate::new(2));
        assert_eq!(probe_that_bails(&gate), None);
        assert_eq!(gate.available(), 2);

        let first = gate.acquire_timeout(Duration::from_millis(1)).unwrap();
        let second = gate.acquire_timeout(Duration::from_millis(1)).unwrap();
        assert!(gate.acquire_timeout(Duration::from_millis(1)).is_none());

        drop(first);
        let third = gate.acquire_timeout(Duration::from_millis(1));
        assert!(third.is_some());
        drop(third);
        drop(second);
        assert_eq!(gate.available(), 2);
    }

    #[test]
    fn parked_dim_chunks_are_never_dropped() {
        let (tx, rx) = crossbeam_channel::bounded::<DimResult>(1);
        let mut parked = std::collections::VecDeque::new();
        let chunk = |index: usize| DimResult {
            generation: 0,
            items: vec![(index, 640, 480, MangaMediaType::StaticImage)],
        };

        // First chunk fills the only slot, the second cannot follow it.
        assert_eq!(
            MangaLoader::queue_dim_chunk(&tx, chunk(0), &mut parked),
            DimSend::Flushed
        );
        assert_eq!(
            MangaLoader::queue_dim_chunk(&tx, chunk(1), &mut parked),
            DimSend::Parked
        );
        assert_eq!(
            MangaLoader::queue_dim_chunk(&tx, chunk(2), &mut parked),
            DimSend::Parked
        );
        assert_eq!(parked.len(), 2);

        // Draining the channel releases the parked chunks in order, one slot at a time.
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            MangaLoader::flush_parked_dim_chunks(&tx, &mut parked),
            DimSend::Parked
        );
        assert_eq!(parked.len(), 1);
        assert!(rx.try_recv().is_ok());
        assert_eq!(
            MangaLoader::flush_parked_dim_chunks(&tx, &mut parked),
            DimSend::Flushed
        );
        assert!(parked.is_empty());
        let got: Vec<usize> = [rx.try_recv(), rx.try_recv()]
            .into_iter()
            .flatten()
            .map(|res| res.items[0].0)
            .collect();
        assert_eq!(got, vec![2]);
    }

    #[test]
    fn video_probe_permit_count_scales_with_cores_within_bounds() {
        let count = video_probe_permit_count();
        assert!((MIN_CONCURRENT_VIDEO_PROBES..=MAX_CONCURRENT_VIDEO_PROBES).contains(&count));
    }

    #[test]
    fn probe_cooldown_blocks_until_cleared_or_window_moves() {
        // Indices unique to this test: the map is shared with every real worker.
        let cooled = 99_001;
        start_probe_cooldown(cooled);
        assert!(probe_cooldown_active(cooled));

        clear_probe_cooldown(cooled);
        assert!(!probe_cooldown_active(cooled));

        for index in [99_002, 99_003, 99_004] {
            start_probe_cooldown(index);
        }
        clear_probe_cooldowns_outside(99_003, 99_004);
        assert!(!probe_cooldown_active(99_002), "left the window");
        assert!(probe_cooldown_active(99_003), "still visible");
        assert!(!probe_cooldown_active(99_004), "window is half-open");

        clear_all_probe_cooldowns();
        assert!(!probe_cooldown_active(99_003));
    }

    #[test]
    fn tile_probe_chain_takes_one_permit() {
        // A tile hands its permit down the probe chain: no second acquisition while holding one.
        let gate = std::sync::Arc::new(VideoProbeGate::new(1));
        let held = gate.acquire_timeout(Duration::from_millis(1)).unwrap();
        let mut owned = None;
        assert!(probe_permit(Some(&held), &mut owned).is_some());
        assert!(owned.is_none(), "nested acquire would deadlock the gate");
        assert_eq!(gate.available(), 0);

        // Without a ticket the chain takes its own permit from the shared gate.
        let mut owned = None;
        drop(held);
        assert!(probe_permit(None, &mut owned).is_some());
        assert!(owned.is_some());
    }

    #[test]
    fn demand_probe_window_matches_grid_rows_arithmetic() {
        let list = paths(400);
        let visible = [40usize];

        // rows 3, 1 visible: forward max(2, 24) = 24, backward max(1, 12) = 12.
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 40, MangaLayoutMode::Masonry));
        assert_eq!(dim_window(&loader), (28, 64));

        let mut up = demand(&list, &visible, &[], 40, MangaLayoutMode::Masonry);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (16, 52));
    }

    #[test]
    fn demand_probe_window_scales_with_visible_count() {
        let list = paths(400);
        let visible = [19usize, 20, 21, 22, 23, 24];

        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 21, MangaLayoutMode::Masonry));
        assert_eq!(dim_window(&loader), (9, 45));

        let mut up = demand(&list, &visible, &[], 21, MangaLayoutMode::Masonry);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (0, 33));
    }

    #[test]
    fn demand_probe_window_clamps_to_list_edges() {
        let list = paths(50);

        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Gallery));
        assert_eq!(dim_window(&loader), (0, 24));

        let mut up = demand(&list, &[49], &[], 49, MangaLayoutMode::Gallery);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (25, 50));
    }

    #[test]
    fn demand_out_of_range_anchor_queues_nothing() {
        let list = paths(50);
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &[999], &[], 999, MangaLayoutMode::Masonry));

        assert_eq!(loader.backlog().dimension_probes, 0);
        assert!(preload_requests(&loader).is_empty());
    }

    #[test]
    fn demand_probe_window_uses_strip_fractional_coverage() {
        let list = paths(200);
        let visible = [30usize, 31];

        // 1.5 visible pages -> behind ceil(1.5) = 2, ahead ceil(3.0) = 3.
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(
            &list,
            &visible,
            &[],
            30,
            MangaLayoutMode::LongStrip,
        ));
        assert_eq!(dim_window(&loader), (28, 33));

        let mut up = demand(&list, &visible, &[], 30, MangaLayoutMode::LongStrip);
        up.moving_up = true;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&up);
        assert_eq!(dim_window(&loader), (27, 32));
    }

    #[test]
    fn demand_preload_window_biases_towards_scroll_direction() {
        let list = paths(200);
        let mut loader = MangaLoader::without_workers();

        // Anchor moves forward: 3 ahead, 2 behind around 60 -> 58..=63.
        loader.demand(&demand(
            &list,
            &[60usize],
            &[],
            60,
            MangaLayoutMode::LongStrip,
        ));
        let indices = preload_requests(&loader)
            .iter()
            .map(|req| req.index)
            .collect::<Vec<_>>();
        assert_eq!(indices.len(), 6);
        assert!(indices.iter().all(|idx| (58..=63).contains(idx)));

        // Anchor moves back: the bias flips -> 3 behind, 2 ahead around 50 = 47..=52.
        loader.demand(&demand(
            &list,
            &[50usize],
            &[],
            50,
            MangaLayoutMode::LongStrip,
        ));
        let indices = preload_requests(&loader)
            .iter()
            .map(|req| req.index)
            .collect::<Vec<_>>();
        assert!(indices.iter().all(|idx| (47..=52).contains(idx)));
        assert!(indices.contains(&47));
        assert!(indices.contains(&52));
    }

    #[test]
    fn demand_lod_side_quantizes_up_and_caps_by_source_box() {
        let list = paths(10);
        let visible = [0usize];

        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip));
        // 300 (512 draws off, use 300) -> next bucket; 512 is already a bucket.
        assert_eq!(
            preload_requests(&loader)
                .iter()
                .find(|req| req.index == 0)
                .map(|req| req.target_texture_side),
            Some(512)
        );

        let mut unquantized = demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip);
        unquantized.target_texture_side = 300;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&unquantized);
        assert_eq!(
            preload_requests(&loader)
                .iter()
                .find(|req| req.index == 0)
                .map(|req| req.target_texture_side),
            Some(384)
        );

        let mut loader = MangaLoader::without_workers();
        loader
            .dimension_cache
            .insert(0, (120, 180, MangaMediaType::StaticImage));
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip));
        // Quantized 512 capped by the 180px source long side.
        assert_eq!(
            preload_requests(&loader)
                .iter()
                .find(|req| req.index == 0)
                .map(|req| req.target_texture_side),
            Some(180)
        );
    }

    #[test]
    fn demand_lod_side_clamps_zero_and_one() {
        let list = paths(4);
        let visible = [0usize];

        let mut zero_target = demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip);
        zero_target.target_texture_side = 0;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&zero_target);
        assert_eq!(
            preload_requests(&loader)
                .first()
                .map(|req| req.target_texture_side),
            Some(96)
        );

        let mut tiny_max = demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip);
        tiny_max.target_texture_side = 0;
        tiny_max.max_texture_side = 1;
        let mut loader = MangaLoader::without_workers();
        loader.demand(&tiny_max);
        let requests = preload_requests(&loader);
        assert_eq!(requests.first().map(|req| req.target_texture_side), Some(1));
        assert!(requests.iter().all(|req| req.max_texture_side == 1));
    }

    #[test]
    fn demand_capacity_grid_and_strip_zoom_tiers() {
        let list = paths(10);
        let visible = [0usize, 1, 2];

        // Grid rows 3, zoom 0.8 -> visible 3 * 6 * 2 = 36 -> clamped up to 64.
        let mut loader = MangaLoader::without_workers();
        let mut grid = demand(&list, &visible, &[], 1, MangaLayoutMode::Masonry);
        grid.zoom = 0.8;
        let plan = loader.demand(&grid);
        assert_eq!(plan.cache_capacity, 64);

        // Strip zoom 0.8 -> visible 3 * 5 * 1 = 15 -> clamped up to 64.
        let mut loader = MangaLoader::without_workers();
        let mut strip = demand(&list, &visible, &[], 1, MangaLayoutMode::LongStrip);
        strip.zoom = 0.8;
        let plan = loader.demand(&strip);
        assert_eq!(plan.cache_capacity, 64);

        // Strip zoom 0.4 -> factor 3: 6 visible * 5 * 3 = 90.
        let mut loader = MangaLoader::without_workers();
        let six: Vec<usize> = (0..6).collect();
        let mut strip = demand(&list, &six, &[], 1, MangaLayoutMode::LongStrip);
        strip.zoom = 0.4;
        let plan = loader.demand(&strip);
        assert_eq!(plan.cache_capacity, 90);
    }

    #[test]
    fn demand_capacity_clamps_to_byte_budget_for_huge_folders() {
        let list = paths(400);
        let visible: Vec<usize> = (0..40).collect();
        let mut dense = demand(&list, &visible, &[], 10, MangaLayoutMode::Masonry);
        dense.masonry_rows = 10;
        dense.zoom = 0.3;

        // Side 512: density target clamps to 1024, byte budget allows 512.
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&dense);
        assert_eq!(plan.cache_capacity, 512);

        // Side 4096: byte budget collapses to the 64-entry floor.
        dense.target_texture_side = 4096;
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&dense);
        assert_eq!(plan.cache_capacity, 64);
    }

    #[test]
    fn demand_keeps_preserved_capacity_floor_while_navigating() {
        let list = paths(10);
        let visible = [0usize, 1];
        let mut preserve = demand(&list, &visible, &[], 1, MangaLayoutMode::LongStrip);
        preserve.preserve_recent_cache = true;
        preserve.previous_cache_capacity = 200;

        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&preserve);
        // Retain floor = 200 * 7/8 = 175, above the 64-entry computed target.
        assert_eq!(plan.cache_capacity, 175);
        assert_eq!(plan.pinned_indices, visible.to_vec());
    }

    #[test]
    fn demand_evicts_outside_keep_window_but_never_visible_items() {
        let list = paths(300);
        let visible = [40usize, 41];
        let cached = [0usize, 41, 90, 200];
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&demand(
            &list,
            &visible,
            &cached,
            40,
            MangaLayoutMode::Masonry,
        ));

        assert_eq!(plan.evicted_indices.len(), 3);
        assert!(plan.evicted_indices.contains(&0));
        assert!(plan.evicted_indices.contains(&90));
        assert!(plan.evicted_indices.contains(&200));
        assert!(!plan.evicted_indices.contains(&41));
        // Evicted indices lose their loaded bookkeeping so they can be re-requested.
        assert!(loader.loaded_levels.read().is_empty());
    }

    #[test]
    fn demand_eviction_keeps_pinned_visible_set_intact() {
        let list = paths(400);
        let visible: Vec<usize> = (100..170).collect();
        let cached = [0usize, 299, 300];
        let mut loader = MangaLoader::without_workers();
        let plan = loader.demand(&demand(
            &list,
            &visible,
            &cached,
            100,
            MangaLayoutMode::Masonry,
        ));

        assert!(visible
            .iter()
            .all(|idx| !plan.evicted_indices.contains(idx)));
        assert!(plan.evicted_indices.contains(&299));
        assert!(plan.evicted_indices.contains(&300));
        assert!(!plan.evicted_indices.contains(&0));
    }

    #[test]
    fn cancel_all_drops_pending_work_but_keeps_dimensions() {
        let list = paths(120);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(
            &list,
            &visible,
            &[],
            10,
            MangaLayoutMode::LongStrip,
        ));
        inject_dimensions(
            &mut loader,
            vec![(12, 400, 600, MangaMediaType::StaticImage)],
        );
        loader.take(0, 4);
        assert_eq!(loader.get_dimensions(12), Some((400, 600)));

        let backlog = loader.backlog();
        assert!(backlog.loads > 0);
        assert!(backlog.dimension_probes > 0);

        loader.cancel_all();
        let backlog = loader.backlog();
        assert_eq!(backlog.loads, 0);
        assert_eq!(backlog.decoded, 0);
        assert_eq!(backlog.dimension_results, 0);
        assert_eq!(backlog.dimension_probes, 0);
        assert_eq!(loader.get_dimensions(12), Some((400, 600)));
    }

    #[test]
    fn clear_and_cancel_all_are_separate_behaviours() {
        let mut loader = MangaLoader::without_workers();
        loader
            .dimension_cache
            .insert(3, (500, 700, MangaMediaType::StaticImage));

        loader.cancel_all();
        assert_eq!(loader.get_dimensions(3), Some((500, 700)));

        loader.clear_preserving_dimensions();
        assert_eq!(loader.get_dimensions(3), Some((500, 700)));

        loader.clear();
        assert_eq!(loader.get_dimensions(3), None);
    }

    #[test]
    fn inflight_cap_bounds_prefetch_but_never_starves_visible_index() {
        let list = paths(400);
        let mut loader = MangaLoader::without_workers();

        // First demand absorbs the initial far-jump cancel; the second must not.
        loader.last_visible_index = 200;
        loader.demand(&demand(&list, &[200], &[], 200, MangaLayoutMode::Masonry));
        let _ = preload_requests(&loader);

        // Simulate a scroll storm: 500 decodes already in flight, all far from the next viewport.
        {
            let generation = loader.current_generation;
            let mut loading = loader.loading_indices.write();
            loading.clear();
            for idx in 1000..1500 {
                loading.insert(idx, generation);
            }
        }

        loader.demand(&demand(&list, &[201], &[], 201, MangaLayoutMode::Masonry));

        assert!(
            loader.loading_indices.read().contains_key(&201),
            "visible index must bypass the in-flight cap"
        );
        assert_eq!(
            loader.loading_indices.read().len(),
            501,
            "cap spilled: only the visible index may be added while 500 are in flight"
        );
        let queued: Vec<usize> = preload_requests(&loader)
            .iter()
            .map(|req| req.index)
            .collect();
        assert_eq!(queued, vec![201]);
    }

    #[test]
    fn retry_visible_requeues_urgently_after_forget() {
        let list = paths(40);
        let mut loader = MangaLoader::without_workers();
        let retry = VisibleRetry {
            image_list: &list,
            index: 7,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };
        assert!(loader.retry_visible(&retry));

        let urgent = urgent_requests(&loader);
        assert_eq!(urgent.len(), 1);
        assert_eq!(urgent[0].index, 7);
        assert_eq!(urgent[0].priority, -200_000);

        // Already in flight: a second retry must not stack another request.
        assert!(!loader.retry_visible(&retry));
        assert!(urgent_requests(&loader).is_empty());
    }

    #[test]
    fn retry_visible_after_mark_unloaded_reloads_a_satisfied_index() {
        let list = paths(40);
        let mut loader = MangaLoader::without_workers();
        // Pretend the index is loaded at full LOD and not in flight.
        loader.loaded_levels.write().insert(7, MAX_SIDE);

        let mut retry = VisibleRetry {
            image_list: &list,
            index: 7,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };
        assert!(loader.retry_visible(&retry));
        assert_eq!(urgent_requests(&loader).len(), 1);

        loader.forget_indices([7]);
        (retry.repair, retry.index) = (RetryRepair::ResetState, 8);
        loader
            .loading_indices
            .write()
            .insert(8, loader.current_generation);
        assert!(loader.retry_visible(&retry));
        assert!(loader.loading_indices.read().get(&8) == Some(&loader.current_generation));
    }

    #[test]
    fn reset_index_state_forgets_one_index_only() {
        let list = paths(40);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(
            &list,
            &visible,
            &[],
            10,
            MangaLayoutMode::LongStrip,
        ));

        let before = loader.backlog().loads;
        assert!(before > 1);
        loader.reset_index_state_for(10);

        assert_eq!(loader.backlog().loads, before - 1);
        assert!(!loader.loading_indices.read().contains_key(&10));
        assert!(loader.loading_indices.read().contains_key(&11));
    }

    #[test]
    fn sync_visible_index_cancels_on_far_jump_and_tracks_direction() {
        let list = paths(300);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 10, MangaLayoutMode::Masonry));
        assert!(loader.backlog().loads > 0);

        loader.sync_visible_index(280, true);
        assert_eq!(loader.backlog().loads, 0);
        assert_eq!(loader.scroll_direction, 1);
        assert_eq!(loader.last_visible_index, 280);

        loader.sync_visible_index(3, false);
        assert_eq!(loader.scroll_direction, -1);
        assert_eq!(loader.last_visible_index, 3);
    }

    #[test]
    fn take_applies_dimension_results_once_per_change() {
        let mut loader = MangaLoader::without_workers();
        inject_dimensions(
            &mut loader,
            vec![(5, 800, 1200, MangaMediaType::StaticImage)],
        );

        let taken = loader.take(0, 4);
        assert!(taken.decoded.is_empty());
        assert_eq!(taken.dimension_updates, vec![5]);
        assert_eq!(loader.get_dimensions(5), Some((800, 1200)));
        assert_eq!(loader.get_media_type(5), Some(MangaMediaType::StaticImage));

        // Same dimensions again: already satisfied, so no update and no new request.
        inject_dimensions(
            &mut loader,
            vec![(5, 800, 1200, MangaMediaType::StaticImage)],
        );
        let taken = loader.take(0, 4);
        assert!(taken.dimension_updates.is_empty());
        assert!(preload_requests(&loader).is_empty());
    }

    #[test]
    fn unloadable_index_is_skipped_by_every_request_path() {
        let dir = std::env::temp_dir().join(format!(
            "riv_unloadable_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.gif");
        std::fs::write(&path, b"GIF89a truncated").unwrap();
        let list = vec![path.clone()];

        let mut loader = MangaLoader::without_workers();
        loader.mark_unloadable(0, path.as_path());

        // Prefetch window, visible retry and the pump's idle check must all leave it alone.
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(
            preload_requests(&loader).is_empty(),
            "an unloadable index must not be queued by prefetch"
        );
        assert!(urgent_requests(&loader).is_empty());

        let retry = VisibleRetry {
            image_list: &list,
            index: 0,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };
        assert!(!loader.retry_visible(&retry));
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));
        assert!(loader.retry_blocked(0));

        // A rewritten file revives it.
        std::fs::write(&path, b"still corrupt, but a different file").unwrap();
        loader
            .retry_state
            .write()
            .get_mut(&0)
            .unwrap()
            .stamp_checked_at = Instant::now() - FAILED_STAMP_RECHECK;
        let rearmed = loader.rearm_failed_in_window(&list, 0, 1);
        assert_eq!(rearmed, 1);
        assert!(!loader.retry_blocked(0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn visible_retry_refuses_an_index_past_the_retry_ceiling() {
        let list = paths(40);
        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            7,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: None,
                stamp_checked_at: Instant::now(),
            },
        );

        let retry = VisibleRetry {
            image_list: &list,
            index: 7,
            max_texture_side: MAX_SIDE,
            target_texture_side: TARGET_SIDE,
            downscale_filter: FilterType::Triangle,
            gif_filter: FilterType::Triangle,
            force_triangle_filters: false,
            repair: RetryRepair::ForceReload,
        };

        // Without this the tile heal/upgrade paths keep clearing the level of a corrupt file
        // and re-queue it, which is the churn that pegged the CPU.
        assert!(!loader.retry_visible(&retry));
        assert!(urgent_requests(&loader).is_empty());
    }

    #[test]
    fn demand_skips_indices_already_loaded_at_sufficient_lod() {
        let list = paths(30);
        let visible = [10usize];
        let mut loader = MangaLoader::without_workers();
        loader.loaded_levels.write().insert(10, MAX_SIDE);

        loader.demand(&demand(
            &list,
            &visible,
            &[],
            10,
            MangaLayoutMode::LongStrip,
        ));
        let requests = preload_requests(&loader);
        assert!(!requests.is_empty());
        assert!(requests.iter().all(|req| req.index != 10));
    }

    #[test]
    fn demand_outstanding_tracks_pump_need_not_in_flight_work() {
        let list = paths(3);
        let mut loader = MangaLoader::without_workers();
        // Source box known, so the LOD target is reachable by the file.
        loader
            .dimension_cache
            .insert(0, (4000, 4000, MangaMediaType::StaticImage));
        loader
            .dimension_cache
            .insert(1, (4000, 4000, MangaMediaType::StaticImage));

        // Nothing in flight, nothing loaded.
        assert!(loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // In flight for the active generation.
        loader
            .loading_indices
            .write()
            .insert(0, loader.current_generation);
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // Loaded below target -> still outstanding; at target -> satisfied.
        loader.loading_indices.write().clear();
        loader.loaded_levels.write().insert(0, TARGET_SIDE / 2);
        assert!(loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));
        loader.loaded_levels.write().insert(0, TARGET_SIDE);
        assert!(!loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // Retry ceiling reached -> satisfied, so a failed item lets the app go idle.
        loader.loaded_levels.write().clear();
        loader.retry_state.write().insert(
            1,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: None,
                stamp_checked_at: Instant::now(),
            },
        );
        assert!(!loader.demand_outstanding(&list, &[1], MAX_SIDE, TARGET_SIDE));
    }

    #[test]
    fn take_honours_the_dimension_result_limit() {
        let items = (0..8)
            .map(|idx| (idx, 100 + idx as u32, 200, MangaMediaType::StaticImage))
            .collect();
        let mut loader = MangaLoader::without_workers();
        inject_dimensions(&mut loader, items);

        let taken = loader.take(0, 1);
        // One message may carry several indices; the limit bounds messages drained.
        assert!(!taken.dimension_updates.is_empty());
        assert!(taken.dimension_updates.len() <= 8);
        assert_eq!(loader.backlog().dimension_results, 0);
    }

    #[test]
    fn prime_clamps_window_to_list_and_destination() {
        let list = paths(60);
        let visible = [0usize, 1, 2, 3, 4, 5];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::Masonry));

        // rows 3: behind max(6, 12) = 12, ahead max(12, 24) = 24 around index 55.
        loader.prime(
            &list,
            55,
            MAX_SIDE,
            TARGET_SIDE,
            FilterType::Triangle,
            FilterType::Triangle,
            true,
        );
        assert_eq!(dim_window(&loader), (43, 60));
        assert!(preload_requests(&loader)
            .iter()
            .all(|req| matches!(req.downscale_filter, FilterType::Triangle)));
    }

    #[test]
    fn warm_dimensions_background_queues_only_the_missing_range() {
        let list = paths(200);
        let mut loader = MangaLoader::without_workers();
        loader.warm_dimensions_background(&list, 90, 120, 0);

        let probe = loader
            .probe
            .as_ref()
            .expect("probe channels")
            .dim_background_requests
            .clone();
        let req = probe.try_recv().expect("background probe batch");
        assert_eq!(req.items.first().map(|(idx, _)| *idx), Some(90));
        assert_eq!(req.items.last().map(|(idx, _)| *idx), Some(119));

        // Everything in that window is now pending, so a second request adds nothing.
        loader.warm_dimensions_background(&list, 90, 120, 0);
        assert!(probe.try_recv().is_err());
    }

    #[test]
    fn prime_forces_triangle_filters_for_neighbours() {
        let list = paths(30);
        let visible = [0usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 0, MangaLayoutMode::LongStrip));

        loader.prime(
            &list,
            20,
            MAX_SIDE,
            TARGET_SIDE,
            FilterType::Nearest,
            FilterType::CatmullRom,
            true,
        );
        let requests = preload_requests(&loader);
        assert!(!requests.is_empty());
        assert!(requests
            .iter()
            .all(|req| matches!(req.downscale_filter, FilterType::Triangle)));
        assert!(requests
            .iter()
            .all(|req| matches!(req.gif_filter, FilterType::Triangle)));
    }

    #[test]
    fn probed_dimensions_reject_unusable_headers() {
        // Zero/absurd sides clamp the texture target to 1 and wedge the tile as "satisfied".
        assert_eq!(sanitize_probed_dimensions(0, 1600), None);
        assert_eq!(sanitize_probed_dimensions(1200, 0), None);
        assert_eq!(sanitize_probed_dimensions(70_000, 10), None);
        assert_eq!(sanitize_probed_dimensions(40_000, 40_000), None);
        assert_eq!(sanitize_probed_dimensions(10_000, 10), None);

        assert_eq!(sanitize_probed_dimensions(1, 100), Some((1, 100)));
        assert_eq!(sanitize_probed_dimensions(1200, 1600), Some((1200, 1600)));
        assert_eq!(sanitize_probed_dimensions(4000, 6000), Some((4000, 6000)));

        assert_eq!(
            probed_or_fallback_dimensions(Some((0, 0)), MangaMediaType::StaticImage),
            FALLBACK_IMAGE_DIMENSIONS
        );
        assert_eq!(
            probed_or_fallback_dimensions(Some((0, 0)), MangaMediaType::Video),
            FALLBACK_VIDEO_DIMENSIONS
        );
        assert_eq!(
            probed_or_fallback_dimensions(None, MangaMediaType::StaticImage),
            FALLBACK_IMAGE_DIMENSIONS
        );
        assert_eq!(
            probed_or_fallback_dimensions(Some((800, 1200)), MangaMediaType::StaticImage),
            (800, 1200)
        );
    }

    #[test]
    fn stale_dimension_results_release_their_pending_probes() {
        let mut loader = MangaLoader::without_workers();
        loader.dim_pending.insert(4);
        loader.current_generation = 1;

        let results = loader
            .probe
            .as_ref()
            .expect("probe channels")
            .dim_results
            .clone();
        results
            .try_send(DimResult {
                generation: 0,
                items: vec![(4, 100, 200, MangaMediaType::StaticImage)],
            })
            .expect("dimension result channel has room");

        assert!(loader.take(0, 4).dimension_updates.is_empty());
        assert_eq!(loader.backlog().dimension_probes, 0, "stranded probe");
        assert_eq!(loader.get_dimensions(4), None);
    }

    #[test]
    fn inflight_snapshot_covers_loads_and_pending_probes() {
        let list = paths(20);
        let visible = [3usize];
        let mut loader = MangaLoader::without_workers();
        loader.demand(&demand(&list, &visible, &[], 3, MangaLayoutMode::Masonry));

        let (loading, probing) = loader.inflight_index_snapshot();
        assert!(loading.contains(&3));
        assert!(!probing.is_empty());

        loader.reset_index_state_for(3);
        loader.drop_dim_pending(3);
        let (loading, probing) = loader.inflight_index_snapshot();
        assert!(!loading.contains(&3));
        assert!(!probing.contains(&3));
    }

    #[test]
    fn rewritten_file_is_rearmed_outside_the_prefetch_window_block() {
        let dir = std::env::temp_dir().join(format!(
            "riv_rearm_window_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.png");
        std::fs::write(&path, b"broken").unwrap();
        let stamp = FileStamp::of_path(&path).unwrap();
        let list = vec![path.clone()];

        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            0,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: Some(stamp),
                stamp_checked_at: Instant::now() - FAILED_STAMP_RECHECK,
            },
        );
        loader.loaded_levels.write().insert(0, TARGET_SIDE);

        // Rewritten: the window pass clears the failure and its loaded level together, so the
        // decode can be queued again.
        std::fs::write(&path, b"a different file").unwrap();
        let rearmed = loader.rearm_failed_in_window(&list, 0, 1);
        assert_eq!(rearmed, 1, "the rewritten file must be re-armed");
        assert!(!loader.retry_blocked(0));
        assert_eq!(loader.loaded_level_for(0), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blocked_index_stays_dead_while_the_file_is_unchanged() {
        let dir = std::env::temp_dir().join(format!(
            "riv_rearm_unchanged_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.gif");
        std::fs::write(&path, b"not a gif").unwrap();
        let stamp = FileStamp::of_path(&path).unwrap();

        let list = vec![path.clone()];
        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            0,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                stamp: Some(stamp),
                // Long enough ago that the recheck interval does not gate this call.
                stamp_checked_at: Instant::now() - FAILED_STAMP_RECHECK,
            },
        );

        loader.rearm_failed_indices(&list, [0]);
        assert!(
            loader.retry_blocked(0),
            "an unchanged corrupt file must stay blocked"
        );

        // Rewriting the file changes the stamp: the index is re-armed on the next recheck.
        std::fs::write(&path, b"not a gif either, but a new file").unwrap();
        // Filesystem mtime granularity can land in the same second; the size is what differs
        // here, and a real rewrite changes shape either way.
        loader
            .retry_state
            .write()
            .get_mut(&0)
            .unwrap()
            .stamp_checked_at = Instant::now() - FAILED_STAMP_RECHECK;
        loader.rearm_failed_indices(&list, [0]);
        assert!(
            !loader.retry_blocked(0),
            "a rewritten file must be re-armed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rearm_recheck_is_rate_limited_per_index() {
        let list = paths(4);
        let mut loader = MangaLoader::without_workers();
        loader.retry_state.write().insert(
            1,
            RetryState {
                attempts: PRELOAD_RETRY_MAX_ATTEMPTS,
                next_retry_at: Instant::now(),
                // No stamp recorded: the first recheck of a missing path clears nothing, and
                // the interval then keeps every following frame off that file.
                stamp: None,
                stamp_checked_at: Instant::now(),
            },
        );

        loader.rearm_failed_indices(&list, [1]);
        assert!(
            loader.retry_blocked(1),
            "a recheck inside the interval must not re-stat the file"
        );
    }

    #[test]
    fn upload_failure_clears_the_level_and_settles_after_the_retry_ceiling() {
        let list = paths(4);
        let mut loader = MangaLoader::without_workers();
        loader.loaded_levels.write().insert(1, TARGET_SIDE);

        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            // Bogus path: the stamp is `None`, which still records the failure shape.
            loader.register_upload_failure(1, Path::new("img0001.png"));
        }

        // A level set without a texture must never keep the tile "satisfied" for good.
        assert_eq!(loader.loaded_level_for(1), None);
        assert!(loader.retry_blocked(1));
        assert!(!loader.demand_outstanding(&list, &[1], MAX_SIDE, TARGET_SIDE));
    }

    #[test]
    fn gate_blocked_probe_never_spends_the_retry_budget() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();

        // Many passes blocked by the gate: no attempt may be recorded, so the tile stays
        // requestable forever instead of settling on a placeholder for the session.
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS * 4 {
            loader.note_probe_blocked(0);
        }

        assert!(
            !loader.retry_blocked(0),
            "gate misses are not the file's fault"
        );
        assert_eq!(loader.probe_blocked_visible_indices(&[0]), vec![0]);
        assert!(
            loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE),
            "a gate-blocked tile must keep the pump pumping"
        );

        // The backoff paces attempts without closing them: the window pass queues it again.
        loader.probe_blocked.clear();
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(
            preload_requests(&loader).iter().any(|req| req.index == 0),
            "an unblocked video must be queued by prefetch"
        );
    }

    #[test]
    fn leaving_the_window_rearms_a_gate_blocked_video() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();

        // A video that burnt its budget on blocked probes: still blocked, as before the fix.
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        loader.note_probe_blocked(0);
        assert!(loader.retry_blocked(0));

        // Scroll away: the window no longer contains the index, so it is re-armed.
        loader.demand(&demand(&list, &[], &[], 9, MangaLayoutMode::Masonry));
        assert!(
            !loader.retry_blocked(0),
            "a video that left the window must get a fresh chance"
        );
        assert!(!loader.probe_blocked.contains_key(&0));
        // Leaving the window is not a revive grant, so no revive history is burnt on the way out.
        assert!(!loader.video_revive.contains_key(&0));

        // An image at the ceiling is still a broken file, not a gate victim.
        let images = vec![PathBuf::from("img0001.png")];
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("img0001.png"));
        }
        loader.note_probe_blocked(0);
        loader.demand(&demand(&images, &[], &[], 9, MangaLayoutMode::Masonry));
        assert!(
            loader.retry_blocked(0),
            "images stay dead without a file change"
        );
        assert!(!loader.video_revive.contains_key(&0));
    }

    #[test]
    fn spent_video_thumbnail_budget_reopens_without_a_hover() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();

        // The decoder hiccup spent the whole budget: exactly the state that used to leave the
        // tile on a static placeholder until the cursor happened to touch it.
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        assert!(loader.retry_blocked(0));
        assert!(
            loader.video_thumbnail_revive_pending(0, Path::new("clip0001.mp4")),
            "a spent video budget is waiting work, not a settled failure"
        );
        // The visible tile keeps the pump running while it waits.
        assert!(loader.demand_outstanding(&list, &[0], MAX_SIDE, TARGET_SIDE));

        // The next window pass re-opens it once the revive window has elapsed, with no hover and
        // no retry_visible: the frame loop alone heals the tile.
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: 0,
                next_revive_at: Instant::now(),
            },
        );
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(!loader.retry_blocked(0), "the budget must re-open");
        assert!(
            preload_requests(&loader).iter().any(|req| req.index == 0),
            "the revived video must be queued by the same pass"
        );

        // Revives are bounded and widen: a genuinely broken video settles instead of spinning.
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: VIDEO_THUMBNAIL_REVIVE_MAX,
                next_revive_at: Instant::now() + Duration::from_secs(3600),
            },
        );
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        assert!(!loader.video_thumbnail_revive_pending(0, Path::new("clip0001.mp4")));
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(loader.retry_blocked(0), "exhausted revives settle the tile");
    }

    #[test]
    fn hover_demand_accelerates_a_pacing_video() {
        let list = vec![PathBuf::from("clip0001.mp4")];
        let mut loader = MangaLoader::without_workers();
        for _ in 0..PRELOAD_RETRY_MAX_ATTEMPTS {
            loader.register_upload_failure(0, Path::new("clip0001.mp4"));
        }
        loader.video_revive.insert(
            0,
            VideoReviveState {
                revives: 0,
                next_revive_at: Instant::now() + Duration::from_secs(60),
            },
        );

        // Hover only drops the pacing: the frame loop still owns the decode, so a first-frame
        // pipeline can never run on the UI thread.
        loader.accelerate_video_thumbnail_revive(0);
        loader.demand(&demand(&list, &[0], &[], 0, MangaLayoutMode::Masonry));
        assert!(!loader.retry_blocked(0));
        assert!(loader.probe_blocked.is_empty());
    }
}

//! Small state machines: retry state, probe-blocked state, video revive state.

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

/// What happened to a dimension chunk hand-off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DimSend {
    /// Accepted by the channel.
    Flushed,
    /// Channel still full: the chunk is queued for the next pass, never dropped.
    Parked,
    Disconnected,
}

/// Decode-queue waits above this are worth a diag line: a fast probe should never queue long.
pub(crate) const DECODE_QUEUE_WAIT_TRACE: Duration = Duration::from_millis(100);

/// Decode wall time above which the diag forensics line names the file.
pub(crate) const DECODE_FORENSICS_SLOW: Duration = Duration::from_millis(250);

/// Base backoff for decode retries after a failed/empty preload.
pub(crate) const PRELOAD_RETRY_BASE_DELAY_MS: u64 = 250;

/// Cap retry backoff so visible items recover quickly once they come into view.
pub(crate) const PRELOAD_RETRY_MAX_DELAY_MS: u64 = 4000;

/// Stop retrying media that repeatedly fails to decode; broken GIFs should not spin forever.
pub(crate) const PRELOAD_RETRY_MAX_ATTEMPTS: u8 = 3;

/// Video thumbnails are a preview, not a requirement: a video whose first frame cannot be decoded
/// still plays, so the ceiling above only *paces* re-attempts for it instead of ending them. The
/// delay doubles per revive and the count is bounded, so a genuinely broken video settles while a
/// one-off decoder hiccup no longer pins the tile as a placeholder for the session.
pub(crate) const VIDEO_THUMBNAIL_REVIVE_BASE_DELAY: Duration = Duration::from_secs(1);

pub(crate) const VIDEO_THUMBNAIL_REVIVE_MAX_DELAY: Duration = Duration::from_secs(10);

pub(crate) const VIDEO_THUMBNAIL_REVIVE_MAX: u32 = 4;

/// Verification re-reads allowed for a file that keeps failing at the *same* stamp.
/// A GIF that decodes but cannot be rendered keeps failing until it is rewritten, and the
/// file stamp is the only re-arm signal there is: one re-read per this interval, only for
/// failed indices, keeps the tile dead on every frame in between.
pub(crate) const FAILED_STAMP_RECHECK: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
pub(crate) struct RetryState {
    pub(crate) attempts: u8,
    pub(crate) next_retry_at: Instant,
    /// Shape of the file when it failed. A rewritten file re-arms the index immediately;
    /// while the stamp is unchanged the tile stays dead (no frame-by-frame re-decode).
    pub(crate) stamp: Option<FileStamp>,
    /// Last time the stamp of a stamp-blocked index was re-read.
    pub(crate) stamp_checked_at: Instant,
}

/// Per-index state for "a gated video probe could not run". Kept apart from [`RetryState`] so a
/// degenerate media set (every visible video blocked by the gate) can never drain the decode budget
/// and settle every tile on a placeholder for the session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProbeBlockedState {
    /// Passes that could not run the probe; paces `next_probe_at`.
    pub(crate) misses: u32,
    /// No gated probe is attempted for this index before this instant.
    pub(crate) next_probe_at: Instant,
}

impl ProbeBlockedState {
    /// Consecutive misses that only slow the probe down; there is deliberately no ceiling.
    pub(crate) const MISS_LIMIT: u32 = 16;

    pub(crate) const BASE_DELAY: Duration = Duration::from_millis(250);

    pub(crate) const MAX_DELAY: Duration = Duration::from_millis(2500);

    pub(crate) fn note_miss(&mut self, now: Instant) {
        self.next_probe_at = now + Self::backoff_for_misses(self.misses);
        self.misses = self.misses.saturating_add(1).min(Self::MISS_LIMIT);
    }

    pub(crate) fn backoff_for_misses(misses: u32) -> Duration {
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
pub(crate) struct VideoReviveState {
    /// Re-opens already granted; [`VIDEO_THUMBNAIL_REVIVE_MAX`] ends them.
    pub(crate) revives: u32,
    /// No re-open before this instant.
    pub(crate) next_revive_at: Instant,
}

impl VideoReviveState {
    pub(crate) fn note_revive(&mut self, now: Instant) {
        self.revives = self
            .revives
            .saturating_add(1)
            .min(VIDEO_THUMBNAIL_REVIVE_MAX);
        self.next_revive_at = now + Self::delay_for_revives(self.revives.saturating_sub(1));
    }

    pub(crate) fn delay_for_revives(revives: u32) -> Duration {
        let factor = 1u64 << revives.min(3);
        VIDEO_THUMBNAIL_REVIVE_BASE_DELAY
            .saturating_mul(factor as u32)
            .min(VIDEO_THUMBNAIL_REVIVE_MAX_DELAY)
    }
}

//! The video-thumbnail revive scheduler (spent budget, parked wake-ups).

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

impl MangaLoader {
    /// A video thumbnail's spent budget re-opens on a widening schedule; images stay dead.
    pub(crate) fn video_revive_due(&self, index: usize, now: Instant) -> bool {
        match self.video_revive.get(&index) {
            None => true,
            // Bounded: once every revive is granted the tile settles, so a video that genuinely
            // has no decodable first frame stops costing a decode every interval.
            Some(state) => {
                state.revives < VIDEO_THUMBNAIL_REVIVE_MAX && now >= state.next_revive_at
            }
        }
    }

    /// Cheap "is anything waiting on the revive schedule at all" probe for the idle frame path.
    ///
    /// Only an open window counts: a spent video with no state yet has its first revive due
    /// immediately, which `demand_outstanding` already reports, so nothing needs scheduling.
    pub fn video_revive_waiting(&self, now: Instant) -> bool {
        self.video_revive
            .values()
            .any(|state| state.revives < VIDEO_THUMBNAIL_REVIVE_MAX && now < state.next_revive_at)
    }

    /// Remaining wait of the soonest-usable visible video whose spent thumbnail budget the revive
    /// schedule has not re-opened yet, `ZERO` when one is due right now. `None` when every visible
    /// video is satisfied, out of revives, or not waiting at all.
    ///
    /// Paired with `demand_outstanding`: the parked frame loop schedules this wait instead of
    /// pumping a preload pass every interval that the schedule would refuse anyway.
    pub fn video_revive_visible_wake(
        &self,
        image_list: &[PathBuf],
        visible_indices: &[usize],
        now: Instant,
    ) -> Option<Duration> {
        if self.video_revive.is_empty() && self.retry_state.read().is_empty() {
            return None;
        }
        let retry = self.retry_state.read();
        visible_indices
            .iter()
            .filter(|&&idx| {
                image_list
                    .get(idx)
                    .is_some_and(|path| is_supported_video(path))
            })
            .filter_map(|&idx| {
                let spent = retry
                    .get(&idx)
                    .is_some_and(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS);
                if !spent {
                    return None;
                }
                // No state yet means the first revive is due immediately, so it is issuable now.
                let Some(state) = self.video_revive.get(&idx) else {
                    return Some(Duration::ZERO);
                };
                (state.revives < VIDEO_THUMBNAIL_REVIVE_MAX)
                    .then(|| state.next_revive_at.saturating_duration_since(now))
            })
            .min()
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
    ///
    /// This is the only path that needs a schedule at all — a visible video tile is therefore
    /// re-opened by the frame loop alone, with no hover, no click and no file change.
    pub(crate) fn revive_spent_video_thumbnails(
        &mut self,
        image_list: &[PathBuf],
        start: usize,
        end: usize,
    ) {
        // One lock over the retry map, which only holds failed indices: the common healthy pass
        // must not walk a window of a few hundred entries for a state that cannot exist.
        if !self
            .retry_state
            .read()
            .values()
            .any(|state| state.attempts >= PRELOAD_RETRY_MAX_ATTEMPTS)
        {
            return;
        }

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
            self.grant_video_revive(index, now);
        }
    }

    /// Grant one revive: count it (widening the next window) and re-open the budget. Returns
    /// `false` when the tile has already spent every revive, so callers cannot re-open it again.
    pub(crate) fn grant_video_revive(&mut self, index: usize, now: Instant) -> bool {
        if !self.video_revive_due(index, now) {
            return false;
        }
        // A live decode owns the index: re-opening here would queue a second one behind it, so
        // the revive waits for the in-flight result (or the stale-in-flight sweep) to land.
        if self.is_index_in_flight(index) {
            return false;
        }
        self.video_revive
            .entry(index)
            .or_insert_with(|| VideoReviveState {
                revives: 0,
                next_revive_at: now,
            })
            .note_revive(now);
        self.rearm_index(index);
        true
    }
}

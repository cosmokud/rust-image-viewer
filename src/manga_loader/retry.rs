//! Decode-failure retry bookkeeping and re-arming.

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
    pub(crate) fn retry_backoff_for_attempt(attempts: u8) -> Duration {
        let shift = attempts.saturating_sub(1).min(6) as u32;
        let factor = 1u64 << shift;
        let delay_ms = PRELOAD_RETRY_BASE_DELAY_MS
            .saturating_mul(factor)
            .min(PRELOAD_RETRY_MAX_DELAY_MS);
        Duration::from_millis(delay_ms)
    }

    pub(crate) fn register_decode_failure(
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
    pub(crate) fn rearm_failed_in_window(
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
}

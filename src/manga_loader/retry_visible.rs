//! Visible-tile retry requests.

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
            if !is_video || !self.grant_video_revive(request.index, Instant::now()) {
                return false;
            }
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
    pub(crate) fn request_visible_retry(
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
}

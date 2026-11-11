//! Solo-mode neighbour probing (schedule + poll).

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn solo_wrapped_index_with_offset(&self, offset: isize) -> Option<usize> {
        let len = self.image_list.len();
        if len == 0 {
            return None;
        }

        Some((self.current_index as isize + offset).rem_euclid(len as isize) as usize)
    }

    pub(crate) fn set_solo_preload_momentum(&mut self, momentum: SoloPreloadMomentum) {
        self.solo_preload_momentum = momentum;
        if momentum == SoloPreloadMomentum::Neutral {
            self.solo_preload_momentum_until = None;
            return;
        }

        self.solo_preload_momentum_until =
            Some(Instant::now() + Self::SOLO_PRELOAD_MOMENTUM_LINGER);
    }

    pub(crate) fn current_solo_preload_momentum(&mut self) -> SoloPreloadMomentum {
        if !self.mode().is_fullscreen() {
            self.solo_preload_momentum = SoloPreloadMomentum::Neutral;
            self.solo_preload_momentum_until = None;
            return SoloPreloadMomentum::Neutral;
        }

        let Some(until) = self.solo_preload_momentum_until else {
            return SoloPreloadMomentum::Neutral;
        };

        if Instant::now() > until {
            self.solo_preload_momentum = SoloPreloadMomentum::Neutral;
            self.solo_preload_momentum_until = None;
            return SoloPreloadMomentum::Neutral;
        }

        self.solo_preload_momentum
    }

    pub(crate) fn build_solo_probe_offsets(
        momentum: SoloPreloadMomentum,
        probe_ahead_count: usize,
        probe_behind_count: usize,
    ) -> SoloProbeOffsets {
        let mut offsets = SoloProbeOffsets::new();
        match momentum {
            SoloPreloadMomentum::Neutral => {
                let max_depth = probe_ahead_count.max(probe_behind_count);
                for step in 1..=max_depth {
                    if step <= probe_ahead_count {
                        offsets.push(step as isize);
                    }
                    if step <= probe_behind_count {
                        offsets.push(-(step as isize));
                    }
                }
            }
            SoloPreloadMomentum::Forward => {
                let mut forward_step = 1;
                let mut backward_step = 1;
                while forward_step <= probe_ahead_count || backward_step <= probe_behind_count {
                    for _ in 0..2 {
                        if forward_step <= probe_ahead_count {
                            offsets.push(forward_step as isize);
                            forward_step += 1;
                        }
                    }
                    if backward_step <= probe_behind_count {
                        offsets.push(-(backward_step as isize));
                        backward_step += 1;
                    }
                }
            }
            SoloPreloadMomentum::Backward => {
                let mut backward_step = 1;
                let mut forward_step = 1;
                while backward_step <= probe_behind_count || forward_step <= probe_ahead_count {
                    for _ in 0..2 {
                        if backward_step <= probe_behind_count {
                            offsets.push(-(backward_step as isize));
                            backward_step += 1;
                        }
                    }
                    if forward_step <= probe_ahead_count {
                        offsets.push(forward_step as isize);
                        forward_step += 1;
                    }
                }
            }
        }

        offsets
    }

    pub(crate) fn solo_fullscreen_decode_depths(
        momentum: SoloPreloadMomentum,
        base_behind_count: usize,
        base_ahead_count: usize,
        max_neighbor_count: usize,
    ) -> (usize, usize) {
        let target_depth = match momentum {
            SoloPreloadMomentum::Neutral => Self::SOLO_FULLSCREEN_PRELOAD_NEUTRAL_DEPTH,
            SoloPreloadMomentum::Forward | SoloPreloadMomentum::Backward => {
                Self::SOLO_FULLSCREEN_PRELOAD_MOMENTUM_DEPTH
            }
        };

        (
            base_behind_count.max(target_depth).min(max_neighbor_count),
            base_ahead_count.max(target_depth).min(max_neighbor_count),
        )
    }

    pub(crate) fn solo_image_texture_ready_depths(
        momentum: SoloPreloadMomentum,
        max_neighbor_count: usize,
    ) -> (usize, usize) {
        let target_depth = match momentum {
            SoloPreloadMomentum::Neutral => Self::SOLO_FULLSCREEN_TEXTURE_READY_NEUTRAL_DEPTH,
            SoloPreloadMomentum::Forward | SoloPreloadMomentum::Backward => {
                Self::SOLO_FULLSCREEN_TEXTURE_READY_MOMENTUM_DEPTH
            }
        };
        let target_depth = target_depth.min(max_neighbor_count);

        (target_depth, target_depth)
    }

    pub(crate) fn solo_offset_within_depths(
        offset: isize,
        behind_count: usize,
        ahead_count: usize,
    ) -> bool {
        if offset > 0 {
            (offset as usize) <= ahead_count
        } else if offset < 0 {
            offset.unsigned_abs() <= behind_count
        } else {
            false
        }
    }

    pub(crate) fn schedule_solo_probe_window(
        &mut self,
        current_path: &PathBuf,
        current_media_type: Option<MediaType>,
    ) {
        if self.mode().is_manga() || self.image_list.len() <= 1 {
            return;
        }

        let downscale_filter = self.config.downscale_filter.to_image_filter();
        let gif_filter = self.config.gif_resize_filter.to_image_filter();
        let (base_probe_behind_count, base_probe_ahead_count) =
            self.solo_probe_window_counts_for_path(current_path, current_media_type);
        let momentum = self.current_solo_preload_momentum();
        let max_neighbor_count = self.image_list.len().saturating_sub(1);

        let (mut probe_behind_count, mut probe_ahead_count) = if self.mode().is_fullscreen() {
            Self::solo_fullscreen_decode_depths(
                momentum,
                base_probe_behind_count,
                base_probe_ahead_count,
                max_neighbor_count,
            )
        } else {
            (base_probe_behind_count, base_probe_ahead_count)
        };

        probe_behind_count = probe_behind_count.min(max_neighbor_count);
        probe_ahead_count = probe_ahead_count.min(max_neighbor_count);
        let (texture_ready_behind_count, texture_ready_ahead_count) =
            Self::solo_image_texture_ready_depths(momentum, max_neighbor_count);

        let offsets = if self.mode().is_fullscreen() {
            Self::build_solo_probe_offsets(momentum, probe_ahead_count, probe_behind_count)
        } else {
            let mut legacy_offsets = SoloProbeOffsets::new();
            for offset in 1..=probe_ahead_count as isize {
                legacy_offsets.push(offset);
            }
            for offset in 1..=probe_behind_count as isize {
                legacy_offsets.push(-offset);
            }
            legacy_offsets
        };

        let mut queued_indices = HashSet::new();
        let mut requests = Vec::with_capacity(probe_ahead_count + probe_behind_count + 1);

        if current_media_type == Some(MediaType::Video) {
            let current_target_side =
                self.solo_target_texture_side_for_path(current_path, MediaType::Video, false);
            requests.push(SoloProbeRequest::Video {
                path: current_path.clone(),
                max_texture_side: current_target_side,
            });
            queued_indices.insert(self.current_index);
        }

        for offset in offsets {
            let Some(index) = self.solo_wrapped_index_with_offset(offset) else {
                continue;
            };
            if !queued_indices.insert(index) {
                continue;
            }

            let Some(path) = self.image_list.get(index).cloned() else {
                continue;
            };
            let Some(media_type) = get_media_type(&path) else {
                continue;
            };

            // A neighbour already diagnosed as undrawable must not be re-probed: that decode
            // ran off the UI thread but the request still poured a full decode of the same
            // corrupt file into the queue on every window pass.
            if let Some(stamp) = self.unloadable_files.get(&path).copied() {
                let current = FileStamp::of_path(path.as_path());
                if current.is_none() || current == stamp {
                    continue;
                }
                self.unloadable_files.remove(&path);
            }

            match media_type {
                MediaType::Image => {
                    let extension = path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.to_ascii_lowercase());
                    if extension.as_deref() == Some("gif") {
                        continue;
                    }

                    let target_side =
                        self.solo_target_texture_side_for_path(&path, media_type, true);
                    if self.has_valid_decoded_image_cache_entry(&path, target_side) {
                        continue;
                    }

                    requests.push(SoloProbeRequest::Image {
                        path,
                        max_texture_side: target_side,
                        downscale_filter,
                        gif_filter,
                        texture_preload: Self::solo_offset_within_depths(
                            offset,
                            texture_ready_behind_count,
                            texture_ready_ahead_count,
                        ),
                    });
                }
                MediaType::Video => {
                    let target_side =
                        self.solo_target_texture_side_for_path(&path, media_type, false);
                    requests.push(SoloProbeRequest::Video {
                        path,
                        max_texture_side: target_side,
                    });
                }
            }
        }

        if !requests.is_empty() {
            self.solo_probe_coordinator.submit_batch(requests);
        }
    }

    pub(crate) fn poll_pending_solo_probe(&mut self, ctx: &egui::Context) {
        let mut request_repaint = false;

        loop {
            let result = match self.solo_probe_coordinator.try_recv() {
                Ok(result) => result,
                Err(crossbeam_channel::TryRecvError::Empty)
                | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            };

            match result {
                SoloProbeResult::Image {
                    path,
                    max_texture_side,
                    texture_preload,
                    cached,
                } => {
                    let Some(cached) = cached else {
                        // Media file that cannot be decoded: remember it so the window probe
                        // stops queueing the same corrupt bytes. Files that are simply gone
                        // (thumbnails of deleted neighbours) are not pinned this way.
                        if get_media_type(&path).is_some() {
                            let stamp = FileStamp::of_path(path.as_path());
                            self.unloadable_files.insert(path, stamp);
                        }
                        continue;
                    };

                    let cache_key = crate::cache_key::key(
                        &path,
                        crate::cache_key::KeyScope::TextureSide(max_texture_side),
                    )
                    .unwrap_or_default();
                    let cached = Arc::new(cached);
                    self.decoded_image_cache
                        .insert(cache_key, Arc::clone(&cached));
                    if texture_preload {
                        self.preload_solo_image_texture(
                            ctx,
                            &path,
                            max_texture_side,
                            cached.as_ref(),
                        );
                    }

                    let current_matches = self.current_media_type == Some(MediaType::Image)
                        && self
                            .image_list
                            .get(self.current_index)
                            .is_some_and(|current| current == &path)
                        && self.image.is_none();
                    if current_matches {
                        let gif_filter = self.config.gif_resize_filter.to_image_filter();
                        if self.try_load_image_from_decoded_cache(
                            &path,
                            max_texture_side,
                            gif_filter,
                            false,
                        ) {
                            if self.pending_media_load.as_ref().is_some_and(|pending| {
                                pending.kind == PendingMediaLoadKind::Image && pending.path == path
                            }) {
                                self.pending_media_load = None;
                            }
                            if !self.defer_directory_work_for_fast_startup() {
                                self.schedule_solo_probe_window(&path, Some(MediaType::Image));
                            }
                            request_repaint = true;
                        }
                    }
                }
                SoloProbeResult::Video { path, thumbnail } => {
                    let Some(thumbnail) = thumbnail else {
                        continue;
                    };

                    // FIX: Check if we are seamlessly transitioning or resuming
                    let is_retaining = self.retained_media_placeholder_visible
                        || self.pending_mode_switch_placeholder.is_some();
                    let is_resuming = self.manga_video_preview_resume_by_path.contains_key(&path);

                    let current_matches = self.current_media_type == Some(MediaType::Video)
                        && self
                            .image_list
                            .get(self.current_index)
                            .is_some_and(|current| current == &path)
                        && self.video_texture.is_none()
                        && !is_retaining // Don't draw 1st frame over our seamless handoff!
                        && !is_resuming; // Don't draw 1st frame if we are jumping to a saved time!
                    if current_matches {
                        self.pending_video_thumbnail_placeholder =
                            Some(PendingVideoThumbnailPlaceholder {
                                path: path.clone(),
                                thumbnail,
                            });
                        request_repaint = true;
                    }
                }
            }
        }

        if request_repaint {
            ctx.request_repaint();
        }
    }

    pub(crate) fn preload_cached_solo_image_textures_for_current_neighbors(
        &mut self,
        ctx: &egui::Context,
    ) {
        const MAX_TEXTURE_UPLOADS_PER_FRAME: usize = 2;

        if self.mode().is_manga() || self.image_list.len() <= 1 {
            return;
        }

        let max_neighbor_count = self.image_list.len().saturating_sub(1);
        let (behind_count, ahead_count) = Self::solo_image_texture_ready_depths(
            self.current_solo_preload_momentum(),
            max_neighbor_count,
        );
        let mut uploads = 0usize;

        for offset in Self::build_solo_probe_offsets(
            self.current_solo_preload_momentum(),
            ahead_count,
            behind_count,
        ) {
            if uploads >= MAX_TEXTURE_UPLOADS_PER_FRAME {
                break;
            }

            let Some(index) = self.solo_wrapped_index_with_offset(offset) else {
                continue;
            };
            let Some(path) = self.image_list.get(index).cloned() else {
                continue;
            };
            if !matches!(get_media_type(&path), Some(MediaType::Image)) {
                continue;
            }

            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase());
            if extension.as_deref() == Some("gif") {
                continue;
            }

            let target_side = self.solo_target_texture_side_for_path(&path, MediaType::Image, true);
            if !self.has_valid_decoded_image_cache_entry(&path, target_side) {
                continue;
            }

            let key =
                crate::cache_key::key(&path, crate::cache_key::KeyScope::TextureSide(target_side))
                    .unwrap_or_default();
            let Some(cached) = self.decoded_image_cache.get(&key) else {
                continue;
            };
            if self
                .cached_solo_image_texture_entry_for_frame(&path, &key, &cached.first_frame)
                .is_some()
            {
                continue;
            }

            self.preload_solo_image_texture(ctx, &path, target_side, cached.as_ref());
            uploads = uploads.saturating_add(1);
        }

        if uploads > 0 {
            ctx.request_repaint();
        }
    }
}

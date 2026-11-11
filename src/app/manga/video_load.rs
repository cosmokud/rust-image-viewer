//! Focused manga video-tile load start/gate/poll.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn clear_pending_manga_video_load(&mut self) {
        self.pending_manga_video_load = None;
    }

    /// Drops failed-video memos that aged out or left the window: a re-entered tile retries
    /// instead of failing for the whole session.
    pub(crate) fn prune_manga_video_failed(&mut self, visible: Option<&HashSet<usize>>) {
        if self.manga_video_failed.is_empty() {
            return;
        }
        let now = Instant::now();
        self.manga_video_failed.retain(|&index, failed_at| {
            !Self::manga_video_failure_expired(*failed_at, now)
                && visible.map_or(true, |set| set.contains(&index))
        });
    }

    pub(crate) fn manga_video_failure_expired(failed_at: Instant, now: Instant) -> bool {
        now.saturating_duration_since(failed_at) >= Self::MANGA_VIDEO_FAILED_COOLDOWN
    }

    /// A refused play must not latch the focus index, or the tile never retries.
    pub(crate) fn manga_video_focus_should_latch(play_refused: bool) -> bool {
        !play_refused
    }

    pub(crate) fn manga_video_load_expired(started_at: Instant, now: Instant) -> bool {
        now.saturating_duration_since(started_at) >= Self::MANGA_VIDEO_LOAD_STALE_AFTER
    }

    pub(crate) fn manga_video_load_pending_for_index(&self, index: usize) -> bool {
        self.pending_manga_video_load
            .as_ref()
            .is_some_and(|pending| {
                pending.index == index
                    && self
                        .image_list
                        .get(index)
                        .is_some_and(|current_path| current_path == &pending.path)
            })
    }

    /// "Stuck" = no cached texture while the loader either refuses every request
    /// (`retry_blocked`) or has waited out `MANGA_PLACEHOLDER_STALL_RETRY_MS` on this tile
    /// without a texture.
    pub(crate) fn manga_placeholder_is_stuck(&self, index: usize) -> bool {
        let Some(loader) = self.manga_loader.as_ref() else {
            return false;
        };
        loader.retry_blocked(index) || self.manga_should_heal_placeholder_retry(index)
    }

    /// Hover demand on a video tile whose loader gave up on its thumbnail: drop the pacing so the
    /// next preload pass re-opens the budget, and clear the probe backoff so the probe is not
    /// refused.
    ///
    /// Deliberately schedules instead of probing. A first-frame decode costs seconds of GStreamer
    /// pipeline work, and the shared probe gate waits up to `VIDEO_PROBE_ACQUIRE_TIMEOUT` before
    /// refusing: doing that on the frame path froze the window for as long as one tile took to
    /// resolve, and the decode loop that follows a spin only made it worse.
    pub(crate) fn manga_hover_accelerate_video_thumbnail(&mut self, index: usize) -> bool {
        if self.mode().is_solo() || self.manga_media_is_unloadable(index) {
            return false;
        }
        let Some(path) = self.image_list.get(index) else {
            return false;
        };
        // Cheap gates first: the frame loop calls this for every hovered tile.
        if !is_supported_video(path.as_path()) {
            return false;
        }
        if self
            .manga_texture_cache
            .contains_for_path(index, path.as_path())
        {
            return false;
        }
        if !self.manga_placeholder_is_stuck(index) {
            return false;
        }

        if let Some(loader) = self.manga_loader.as_mut() {
            loader.accelerate_video_thumbnail_revive(index);
        }
        true
    }

    pub(crate) fn start_async_manga_focused_video_load(
        &mut self,
        index: usize,
        path: PathBuf,
        muted: bool,
        initial_volume: f64,
        autoplay: bool,
        seamless_lod_refresh: bool,
    ) {
        if !gstreamer_runtime_available() {
            self.clear_pending_manga_video_load();
            self.remove_manga_video_player(index);
            self.remove_manga_video_texture(index);
            self.manga_video_preview_resume_secs.remove(&index);
            if self.manga_focused_video_index == Some(index) {
                self.manga_focused_video_index = None;
            }
            self.video_playback_unavailable_reason =
                Some(Self::gstreamer_missing_video_error_text().to_string());
            return;
        }

        let request_id = self.next_manga_video_load_request_id;
        self.next_manga_video_load_request_id = self
            .next_manga_video_load_request_id
            .saturating_add(1)
            .max(1);
        let output_bounds = if self.mode().is_masonry() {
            self.manga_video_output_bounds_for_index(index)
        } else {
            // Long-strip focused playback stays at source quality.
            None
        };

        self.pending_manga_video_load = Some(PendingMangaFocusedVideoLoad {
            request_id,
            index,
            path: path.clone(),
            started_at: Instant::now(),
        });

        let saved_position = self.manga_video_preview_resume_by_path.get(&path).copied();

        let (
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
        ) = self.effective_video_decoder_preferences();
        self.manga_video_load_coordinator
            .submit(MangaFocusedVideoLoadRequest {
                request_id,
                index,
                path,
                muted,
                initial_volume,
                prefer_hardware_decode,
                disable_hardware_decode,
                enable_cuda_decode,
                enable_d3d12_decode,
                output_bounds,
                autoplay,
                seamless_lod_refresh,
                resume_position_secs: saved_position,
            });
    }

    pub(crate) fn poll_pending_manga_video_load(&mut self, ctx: &egui::Context) {
        let mut applied_any = false;
        let mut pending_dimension_updates = Vec::new();

        loop {
            let result = match self.manga_video_load_coordinator.try_recv() {
                Ok(result) => result,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.clear_pending_manga_video_load();
                    break;
                }
            };

            let (result_request_id, result_index, result_path, worker_elapsed) = (
                result.request_id,
                result.index,
                &result.path,
                result.worker_elapsed,
            );

            let Some(pending) = self.pending_manga_video_load.as_ref() else {
                // Late result for an expired/cleared load: drop it, but never silently.
                self.perf_metrics.count(Metric::MangaVideoAsyncStale, 1);
                continue;
            };

            if result_request_id != pending.request_id
                || result_index != pending.index
                || result_path != &pending.path
            {
                self.perf_metrics.count(Metric::MangaVideoAsyncStale, 1);
                continue;
            }

            let Some(pending) = self.pending_manga_video_load.take() else {
                continue;
            };

            let total_elapsed = pending.started_at.elapsed();
            self.perf_metrics
                .record(Metric::MangaVideoAsyncMs, total_elapsed);
            self.perf_metrics
                .record(Metric::MangaVideoAsyncWorkerMs, worker_elapsed);
            self.perf_metrics.record(
                Metric::MangaVideoAsyncQueueMs,
                total_elapsed.saturating_sub(worker_elapsed),
            );

            let still_targeted = self.mode().is_manga()
                && self.manga_focused_video_index == Some(result_index)
                && self
                    .image_list
                    .get(result_index)
                    .is_some_and(|current_path| current_path == result_path);

            if !still_targeted {
                self.perf_metrics.count(Metric::MangaVideoAsyncStale, 1);
                continue;
            }

            match result {
                MangaFocusedVideoLoadResult {
                    index,
                    path,
                    autoplay,
                    seamless_lod_refresh,
                    result: Ok(mut player),
                    ..
                } => {
                    if self.manga_video_players.contains_key(&index)
                        && !self.manga_video_player_matches(index)
                    {
                        self.remove_manga_video_player(index);
                        self.remove_manga_video_texture(index);
                    }

                    let mut synchronized_state = false;
                    if seamless_lod_refresh && self.manga_video_player_matches(index) {
                        if let Some(current_player) = self.manga_video_players.get_mut(&index) {
                            let current_position = current_player.displayed_position();
                            let current_was_playing = current_player.is_playing();
                            let current_muted = current_player.is_muted();
                            let current_volume = current_player.volume();

                            if let Some(position) = current_position {
                                let _ = player.seek_to_time_with_mode(
                                    position.as_secs_f64(),
                                    VideoSeekMode::Accurate,
                                );
                            }

                            if current_was_playing {
                                if !player.is_playing() {
                                    player.play_async();
                                }
                            } else if player.is_playing() {
                                player.pause_async();
                            }

                            player.set_muted(current_muted);
                            player.set_volume(current_volume);
                            synchronized_state = true;
                        }
                    }

                    if !synchronized_state {
                        Self::apply_video_audio_overrides(
                            &mut player,
                            self.manga_video_user_muted,
                            self.manga_video_user_volume,
                        );

                        if autoplay && !player.is_playing() {
                            // Enqueues only; a failure surfaces via `take_control_error` later.
                            player.play_async();
                        }
                    }

                    // Re-check resume position at apply-time to cover races where
                    // fullscreen/preview position was recorded after this async load started.
                    let resume_position = self.manga_resume_position_for_index(index);
                    Self::seek_video_player_to_resume_position(&mut player, resume_position);
                    if let Some(position) = player.displayed_position() {
                        self.manga_record_video_preview_resume_secs(index, position);
                    }

                    let dims = player.dimensions();
                    if dims.0 > 0 && dims.1 > 0 {
                        if !self.masonry_authoritative_dimension_lock_active() {
                            if let Some(ref mut loader) = self.manga_loader {
                                if loader.update_video_dimensions(index, dims.0, dims.1) {
                                    pending_dimension_updates.push(index);
                                }
                            }
                        }
                    }

                    if !self.mode().is_masonry() {
                        if let Some(frame) = player.get_frame() {
                            let displayed_position = frame.pts;
                            let target_side = self.manga_target_texture_side_for_dynamic_media(
                                index,
                                MangaMediaType::Video,
                            );
                            let no_downscale =
                                frame.width <= target_side && frame.height <= target_side;
                            let policy = base_upload_policy_inputs(self)
                                .policy(crate::texture_upload::UploadKind::VideoFrame);
                            let frame = frame;
                            let downscaled = (!no_downscale).then(|| {
                                downscale_rgba_if_needed(
                                    frame.width,
                                    frame.height,
                                    &frame.pixels,
                                    target_side,
                                    self.config.downscale_filter.to_image_filter(),
                                )
                            });
                            let (w, h) = downscaled
                                .as_ref()
                                .map(|(w, h, _)| (*w, *h))
                                .unwrap_or((frame.width, frame.height));
                            let pixels = match downscaled.as_ref() {
                                Some((_, _, pixels)) => {
                                    crate::texture_upload::PixelSource::Rgba(pixels.as_ref())
                                }
                                None => {
                                    crate::texture_upload::PixelSource::OpaqueBytes(&frame.pixels)
                                }
                            };

                            let uploaded = crate::texture_upload::upload(
                                ctx,
                                &format!("manga_video_{}", index),
                                self.manga_video_textures
                                    .get_mut(&index)
                                    .map(|(texture, _, _)| texture),
                                pixels,
                                (w, h),
                                &policy,
                            );
                            if let Some((_, stored_w, stored_h)) =
                                self.manga_video_textures.get_mut(&index)
                            {
                                *stored_w = w;
                                *stored_h = h;
                            } else {
                                self.manga_video_textures
                                    .insert(index, (uploaded.handle, w, h));
                            }
                            if let Some(path) = self.image_list.get(index).cloned() {
                                self.manga_video_texture_paths.insert(index, path);
                            }
                            if let Some(position) = displayed_position {
                                self.manga_record_video_preview_resume_secs(index, position);
                            }
                        }
                    }

                    self.manga_video_player_paths.insert(index, path);
                    self.manga_video_players.insert(index, player);
                    self.error_message = None;
                    self.manga_evict_distant_video_players(index, None);
                    applied_any = true;
                }
                MangaFocusedVideoLoadResult {
                    index,
                    path,
                    result: Err(err),
                    ..
                } => {
                    self.manga_video_failed.insert(index, Instant::now());
                    self.video_playback_unavailable_reason =
                        Some(format!("Failed to load video: {}", err));
                    eprintln!(
                        "Failed to create video player for manga index {} ({}): {}",
                        index,
                        path.display(),
                        err
                    );

                    if self.manga_focused_video_index == Some(index)
                        && !self.manga_video_players.contains_key(&index)
                    {
                        self.manga_focused_video_index = None;
                    }
                }
            }
        }

        if self.mode().is_masonry()
            && !self.masonry_authoritative_dimension_lock_active()
            && !pending_dimension_updates.is_empty()
        {
            self.masonry_queue_dimension_updates(pending_dimension_updates);
            if !self.masonry_navigation_active_for_heavy_work() {
                let force_flush = !self.masonry_metadata_preload_active;
                self.masonry_flush_pending_dimension_updates(force_flush);
            }
        }

        if applied_any {
            ctx.request_repaint();
        }
    }

    pub(crate) fn start_async_image_load(
        &mut self,
        path: PathBuf,
        max_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
    ) {
        self.start_async_image_load_with_view_policy(
            path,
            max_texture_side,
            downscale_filter,
            gif_filter,
            false,
        );
    }
}

//! Async image/video loads that feed the solo view.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn start_async_image_load_with_view_policy(
        &mut self,
        path: PathBuf,
        max_texture_side: u32,
        downscale_filter: FilterType,
        gif_filter: FilterType,
        preserve_view_on_complete: bool,
    ) {
        let request_id = self.next_media_load_request_id;
        self.next_media_load_request_id = self.next_media_load_request_id.saturating_add(1).max(1);

        self.pending_media_load = Some(PendingMediaLoad {
            request_id,
            path: path.clone(),
            kind: PendingMediaLoadKind::Image,
            max_texture_side: Some(max_texture_side),
            preserve_view_on_complete,
            started_at: Instant::now(),
        });

        self.media_load_coordinator.submit(MediaLoadRequest::Image {
            request_id,
            path,
            max_texture_side,
            downscale_filter,
            gif_filter,
        });
    }

    pub(crate) fn live_video_output_bounds_for_solo(&self) -> Option<(u32, u32)> {
        let viewport = self.solo_viewport_size_for_lod();
        let max_side = self.max_texture_side.max(1);
        let width = (viewport.x.ceil() as u32).clamp(1, max_side);
        let height = (viewport.y.ceil() as u32).clamp(1, max_side);
        Some((width, height))
    }

    pub(crate) fn async_video_output_bounds_for_solo(&self) -> Option<(u32, u32)> {
        let max_side = self.max_texture_side.max(1);

        // The display the window is on, never the primary: decode output larger than the screen
        // it is shown on is wasted texture memory, smaller is a blurry first frame.
        #[cfg(target_os = "windows")]
        if let Some(display) = self.current_display_from_window_rect() {
            let width = (display.monitor.width() as u32).clamp(1, max_side);
            let height = (display.monitor.height() as u32).clamp(1, max_side);
            return Some((width, height));
        }

        self.live_video_output_bounds_for_solo()
    }

    pub(crate) fn start_async_video_load(&mut self, path: PathBuf) {
        if !gstreamer_runtime_available() {
            self.suppress_video_controls_for_next_video_load = false;
            self.suppress_video_controls_for_request_id = None;
            self.pending_media_load = None;
            self.drop_retained_media_placeholder();
            self.set_video_playback_unavailable_for_path(
                &path,
                Self::gstreamer_missing_video_error_text().to_string(),
            );
            return;
        }

        let request_id = self.next_media_load_request_id;
        self.next_media_load_request_id = self.next_media_load_request_id.saturating_add(1).max(1);

        if self.suppress_video_controls_for_next_video_load {
            self.suppress_video_controls_for_request_id = Some(request_id);
        } else {
            self.suppress_video_controls_for_request_id = None;
        }
        self.suppress_video_controls_for_next_video_load = false;

        let muted = if self.config.video_muted_remember {
            self.config.state_muted
        } else {
            self.config.video_muted_by_default
        };
        let initial_volume = if self.config.video_volume_remember {
            self.config.state_volume
        } else {
            self.config.video_default_volume
        };
        let (
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
        ) = self.effective_video_decoder_preferences();
        let output_bounds = self.async_video_output_bounds_for_solo();

        self.pending_media_load = Some(PendingMediaLoad {
            request_id,
            path: path.clone(),
            kind: PendingMediaLoadKind::Video,
            max_texture_side: output_bounds.map(|(width, height)| width.max(height)),
            preserve_view_on_complete: false,
            started_at: Instant::now(),
        });

        let saved_position = self.manga_video_preview_resume_by_path.get(&path).copied();

        // FIX: Destroy the 1st-frame thumbnail so the UI is forced to use our seamless masonry frame!
        if saved_position.is_some() || self.pending_mode_switch_placeholder.is_some() {
            self.pending_video_thumbnail_placeholder = None;
        }

        self.media_load_coordinator.submit(MediaLoadRequest::Video {
            request_id,
            path,
            muted,
            initial_volume,
            prefer_hardware_decode,
            disable_hardware_decode,
            enable_cuda_decode,
            enable_d3d12_decode,
            output_bounds,
            resume_position_secs: saved_position,
        });
    }

    pub(crate) fn poll_pending_media_load(&mut self, ctx: &egui::Context) {
        let mut applied_any = false;

        loop {
            let result = match self.media_load_coordinator.try_recv() {
                Ok(result) => result,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.drop_retained_media_placeholder();
                    self.clear_pending_media_load();
                    break;
                }
            };

            let (result_request_id, result_path, worker_elapsed) = match &result {
                MediaLoadResult::Image {
                    request_id,
                    path,
                    worker_elapsed,
                    ..
                } => (*request_id, path, *worker_elapsed),
                MediaLoadResult::Video {
                    request_id,
                    path,
                    worker_elapsed,
                    ..
                } => (*request_id, path, *worker_elapsed),
            };

            let Some(pending) = self.pending_media_load.as_ref() else {
                continue;
            };

            if result_request_id != pending.request_id || result_path != &pending.path {
                self.perf_metrics.count(Metric::LoadMediaAsyncStale, 1);
                continue;
            }

            let Some(pending) = self.pending_media_load.take() else {
                continue;
            };

            let total_elapsed = pending.started_at.elapsed();
            self.perf_metrics
                .record(Metric::LoadMediaAsyncMs, total_elapsed);
            self.perf_metrics
                .record(Metric::LoadMediaAsyncWorkerMs, worker_elapsed);
            self.perf_metrics.record(
                Metric::LoadMediaAsyncQueueMs,
                total_elapsed.saturating_sub(worker_elapsed),
            );

            match result {
                MediaLoadResult::Image { path, result, .. } => match result {
                    Ok(loaded) => {
                        if !pending.preserve_view_on_complete {
                            self.consume_deferred_media_view_reset();
                        }
                        self.retained_media_placeholder_visible = false;
                        let (display_w, display_h) = loaded.image.display_dimensions();
                        if display_w > 0 && display_h > 0 {
                            store_cached_dimensions(
                                &path,
                                CachedMediaKind::Image,
                                display_w,
                                display_h,
                            );
                        }

                        self.cache_loaded_image_first_frame(
                            &path,
                            loaded.max_texture_side,
                            &loaded.image,
                            loaded.is_animated_webp,
                        );
                        self.clear_current_image_texture_upload();
                        self.image = Some(loaded.image);
                        self.image_changed = Self::solo_image_load_completion_should_apply_layout(
                            pending.preserve_view_on_complete,
                        );
                        if !pending.preserve_view_on_complete {
                            self.pending_media_layout = false;
                        }
                        self.error_message = None;
                        self.clear_video_playback_unavailable_state();
                        if !self.defer_directory_work_for_fast_startup() {
                            self.schedule_solo_probe_window(&path, Some(MediaType::Image));
                        }

                        if loaded.is_animated_webp {
                            if let Some(rx) = LoadedImage::start_streaming_webp(
                                &path,
                                Some(loaded.max_texture_side),
                                loaded.gif_filter,
                            ) {
                                self.anim_stream_rx = Some(rx);
                                self.anim_stream_path = Some(path);
                                self.anim_stream_done = false;
                                self.anim_seekbar_total_frames = Some(
                                    self.image
                                        .as_ref()
                                        .map(|image| image.frame_count())
                                        .unwrap_or(1),
                                );
                            }
                        }
                    }
                    Err(err) => {
                        self.drop_retained_media_placeholder();
                        self.error_message = Some(err);
                    }
                },
                MediaLoadResult::Video { path, result, .. } => {
                    let suppress_controls_reveal =
                        self.suppress_video_controls_for_request_id == Some(result_request_id);
                    if suppress_controls_reveal {
                        self.suppress_video_controls_for_request_id = None;
                    }

                    match result {
                        Ok(mut player) => {
                            let resume_position_secs = self
                                .manga_video_preview_resume_by_path
                                .get(&path)
                                .copied()
                                .or_else(|| {
                                    self.image_list
                                        .iter()
                                        .position(|candidate| candidate == &path)
                                        .and_then(|idx| {
                                            self.manga_video_preview_resume_secs.get(&idx).copied()
                                        })
                                })
                                .filter(|secs| secs.is_finite() && *secs >= 0.0);
                            let resume_position = resume_position_secs.map(Duration::from_secs_f64);
                            Self::seek_video_player_to_resume_position(
                                &mut player,
                                resume_position,
                            );

                            let dims = player.dimensions();
                            if dims.0 > 0 && dims.1 > 0 {
                                store_cached_dimensions(
                                    &path,
                                    CachedMediaKind::Video,
                                    dims.0,
                                    dims.1,
                                );
                            }

                            self.video_player = Some(player);
                            self.current_video_path = Some(path.clone());
                            self.error_message = None;
                            self.clear_video_playback_unavailable_state();
                            if !suppress_controls_reveal {
                                self.show_video_controls = true;
                                self.touch_bottom_overlays();
                            }

                            if self.defer_media_view_reset {
                                self.pending_media_layout = false;
                            } else {
                                self.retained_media_placeholder_visible = false;
                                self.image_changed = true;
                                self.pending_media_layout = true;
                            }

                            if !self.defer_directory_work_for_fast_startup() {
                                self.schedule_solo_probe_window(&path, Some(MediaType::Video));
                            }
                        }
                        Err(err) => {
                            if self.retained_media_placeholder_visible {
                                self.drop_retained_media_placeholder();
                            }
                            self.error_message = None;
                            self.set_video_playback_unavailable_for_path(
                                &path,
                                format!("Failed to load video: {}", err),
                            );
                            if !suppress_controls_reveal {
                                self.show_video_controls = true;
                                self.touch_bottom_overlays();
                            }
                        }
                    }
                }
            }

            applied_any = true;
        }

        if applied_any {
            ctx.request_repaint();
        }
    }

    pub(crate) fn load_image_retaining_visible_media(&mut self, path: &PathBuf) {
        self.load_media_internal(path, true);
    }
}

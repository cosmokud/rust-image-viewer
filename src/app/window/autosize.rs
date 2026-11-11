//! Floating autosize requests.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Update texture for current frame (handles both images and video)
    /// Returns true if a repaint is needed for animations
    pub(crate) fn update_texture(&mut self, ctx: &egui::Context) -> bool {
        let mut needs_repaint = false;
        let mut activate_deferred_video_swap = false;
        let mut solo_displayed_video_position = None;

        // In manga mode, animated WebPs are handled per-item; don't stream/play the
        // fullscreen animation pipeline.
        if self.mode().is_manga() {
            self.reset_fullscreen_anim_stream_state();
        }

        // ── Drain streamed animation frames (animated WebP) ──
        // The background thread sends individual frames as they are decoded.
        // We append them to `self.image.frames` so the animation plays
        // progressively — no need to wait for the full decode.
        if self.mode().is_solo() {
            if let Some(ref rx) = self.anim_stream_rx {
                const MAX_SOLO_STREAM_FRAMES_PER_TICK: usize = 8;
                let mut got_frames = false;
                let mut drained_this_tick = 0usize;
                loop {
                    if drained_this_tick >= MAX_SOLO_STREAM_FRAMES_PER_TICK {
                        break;
                    }

                    match rx.try_recv() {
                        Ok(frame) => {
                            drained_this_tick += 1;
                            // Only accept if path still matches what we are viewing.
                            let path_ok = self
                                .anim_stream_path
                                .as_ref()
                                .and_then(|p| self.image.as_ref().map(|img| img.path == *p))
                                .unwrap_or(false);
                            if path_ok {
                                if let Some(ref mut img) = self.image {
                                    img.frames.push(frame);
                                    got_frames = true;
                                }
                            }
                        }
                        Err(crossbeam_channel::TryRecvError::Empty) => break,
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            self.reset_fullscreen_anim_stream_state();
                            break;
                        }
                    }
                }
                if got_frames {
                    needs_repaint = true;
                }
            }
            // While still streaming, keep polling at a high rate.
            if !self.anim_stream_done {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }

        if matches!(self.current_media_type, Some(MediaType::Video))
            && (self.video_texture.is_none() || self.retained_media_placeholder_visible)
        {
            let current_path = self.image_list.get(self.current_index).cloned();
            let should_upload_placeholder = self
                .pending_video_thumbnail_placeholder
                .as_ref()
                .is_some_and(|placeholder| Some(&placeholder.path) == current_path.as_ref());
            if should_upload_placeholder {
                if let Some(placeholder) = self.pending_video_thumbnail_placeholder.take() {
                    let placeholder_path = placeholder.path.clone();
                    let policy = self.solo_video_thumbnail_policy();
                    self.video_texture = Some(
                        crate::texture_upload::upload(
                            ctx,
                            "video-thumbnail-placeholder",
                            None,
                            crate::texture_upload::PixelSource::Rgba(&placeholder.thumbnail.pixels),
                            (placeholder.thumbnail.width, placeholder.thumbnail.height),
                            &policy,
                        )
                        .handle,
                    );
                    self.video_texture_source_path = Some(placeholder_path);
                    self.video_texture_dims =
                        Some((placeholder.thumbnail.width, placeholder.thumbnail.height));
                    self.retained_media_placeholder_visible = false;
                    needs_repaint = true;
                }
            }
        }

        let solo_current_display_min_side = self.solo_current_display_min_side();
        let current_video_path = self.image_list.get(self.current_index).cloned();
        let current_video_target_side = current_video_path
            .as_ref()
            .map(|path| self.solo_target_texture_side_for_path(path, MediaType::Video, false))
            .unwrap_or(self.max_texture_side.max(1));
        let solo_video_upload_filter = self.config.downscale_filter.to_image_filter();
        let mut solo_static_inputs = base_upload_policy_inputs(self);
        solo_static_inputs.display_min_side = solo_current_display_min_side;
        let solo_video_policy =
            base_upload_policy_inputs(self).policy(crate::texture_upload::UploadKind::VideoFrame);

        if let Some((path, frame_count, total_duration_ms)) = self.image.as_ref().and_then(|img| {
            if Self::path_uses_animated_fps_override(img.path.as_path()) {
                Some((img.path.clone(), img.frame_count(), img.total_duration_ms()))
            } else {
                None
            }
        }) {
            self.sync_custom_fps_with_current_media_default(
                path.as_path(),
                frame_count,
                total_duration_ms as u64,
            );
        }

        let webp_fps_override = self.webp_effective_fps_override();

        // Handle image texture updates
        let allow_image_animation = self.mode().is_solo();
        if let Some(ref mut img) = self.image {
            // In manga mode, keep the main image static (first frame only).
            let allow_animation = allow_image_animation;
            let webp_override_delay = webp_fps_override
                .filter(|_| Self::path_uses_animated_fps_override(img.path.as_path()))
                .map(Self::frame_delay_for_fps);

            // Only update animation if not paused and we have more than one frame.
            let frame_changed = if allow_animation && !self.gif_paused && img.is_animated() {
                // If we are still streaming and the current frame is the last
                // available one, hold it until the next frame arrives instead of
                // wrapping back to frame 0. Once streaming is done, normal
                // looping resumes.
                if !self.anim_stream_done && img.current_frame_index() + 1 >= img.frame_count() {
                    false // wait for more frames
                } else if let Some(delay) = webp_override_delay {
                    Self::update_animation_with_delay(img, delay)
                } else {
                    img.update_animation()
                }
            } else {
                false
            };

            // Static textures may initially upload without mipmaps (e.g. near 1:1 view).
            // If the user later zooms out far enough, force a one-time texture reupload with
            // mipmaps to suppress halftone/dot-pattern aliasing noise.
            let static_mipmap_upgrade_needed = if !img.is_animated()
                && self.texture.is_some()
                && self.texture_frame == img.current_frame_index()
                && !self.image_texture_mipmap_enabled
            {
                self.image_texture_dims
                    .map(|dims| {
                        solo_static_inputs
                            .policy(crate::texture_upload::UploadKind::StaticImage)
                            .mipmap_enabled(dims)
                    })
                    .unwrap_or(false)
            } else {
                false
            };

            if self.texture.is_none()
                || frame_changed
                || self.texture_frame != img.current_frame_index()
                || static_mipmap_upgrade_needed
            {
                let frame = img.current_frame_data();
                // This should already be constrained in the loader, but keep this guard to
                // avoid backend crashes if a frame slips through.
                let downscale_filter = if img.is_animated() {
                    self.config.gif_resize_filter.to_image_filter()
                } else {
                    self.config.downscale_filter.to_image_filter()
                };

                let (w, h, pixels) = downscale_rgba_if_needed(
                    frame.width,
                    frame.height,
                    &frame.pixels,
                    self.max_texture_side,
                    downscale_filter,
                );
                // Animated frames resolve to the animated filter without mipmaps; static
                // images follow the display-size mipmap policy.
                let kind = if img.is_animated() {
                    crate::texture_upload::UploadKind::AnimatedFrame
                } else {
                    crate::texture_upload::UploadKind::StaticImage
                };
                let policy = solo_static_inputs.policy(kind);
                let uploaded = crate::texture_upload::upload(
                    ctx,
                    "image",
                    self.texture.as_mut(),
                    crate::texture_upload::PixelSource::Rgba(pixels.as_ref()),
                    (w, h),
                    &policy,
                );
                self.image_texture_mipmap_enabled = uploaded.mipmap_enabled;
                self.texture = Some(uploaded.handle);
                self.image_texture_dims = Some((w, h));
                self.texture_frame = img.current_frame_index();
            }

            // Only request repaint for animated images that are not paused
            if allow_animation && img.is_animated() && !self.gif_paused {
                // Calculate time until next frame to avoid unnecessary repaints
                let current_delay = webp_override_delay
                    .unwrap_or_else(|| Duration::from_millis(img.current_delay_ms() as u64));
                let elapsed = img.last_frame_time.elapsed();
                if elapsed < current_delay {
                    // Schedule repaint for when the next frame is due
                    let remaining = current_delay - elapsed;
                    ctx.request_repaint_after(remaining);
                } else {
                    needs_repaint = true;
                }
            }
        }

        // Handle video frame updates
        crate::diag::note_stage("video_poll");
        // Failed queued state changes surface here instead of on the click/call site.
        if let Some(err) = self
            .video_player
            .as_mut()
            .and_then(|player| player.take_control_error())
        {
            self.set_video_playback_unavailable_runtime(err);
        }
        if let Some(ref mut player) = self.video_player {
            // Detect a frozen decoder early and demote the AV1 hardware decoders so the
            // next navigation loads without a state-change timeout delay.
            player.mark_wedged_if_stalled();

            // Update duration cache
            player.update_duration();

            // Check for video end and handle looping
            if player.is_eos() {
                if self.config.video_loop {
                    // Enqueues only: a wedged decoder used to block this frame for 5 s.
                    if player.restart_async() {
                        needs_repaint = true;
                    }
                }
            }

            // Get new frame if available
            if let Some(frame) = player.get_frame() {
                activate_deferred_video_swap = self.defer_media_view_reset;
                solo_displayed_video_position = frame.pts;

                // Live video frames change continuously, so per-frame mipmap generation is wasted
                // work. Thumbnails still use the manga_mipmap_video_thumbnails setting elsewhere.
                let policy = solo_video_policy;

                let no_downscale = frame.width <= current_video_target_side
                    && frame.height <= current_video_target_side;
                let downscaled = (!no_downscale).then(|| {
                    downscale_rgba_if_needed(
                        frame.width,
                        frame.height,
                        &frame.pixels,
                        current_video_target_side,
                        solo_video_upload_filter,
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
                    None => crate::texture_upload::PixelSource::OpaqueBytes(&frame.pixels),
                };

                // Reuse the same GPU texture across frames to avoid per-frame allocations.
                let uploaded = crate::texture_upload::upload(
                    ctx,
                    "video",
                    self.video_texture.as_mut(),
                    pixels,
                    (w, h),
                    &policy,
                );
                self.video_texture = Some(uploaded.handle);
                if self.video_texture_source_path.as_ref() != current_video_path.as_ref() {
                    self.video_texture_source_path = current_video_path.clone();
                }
                self.video_texture_dims = Some((w, h));
                needs_repaint = true;
            }

            // Only request repaint for active video playback or when seeking
            if player.is_playing() {
                // Do not use a 7ms timer here: 7ms mathematically caps the UI below 144 Hz.
                // Immediate repaint lets the platform frame pacing hit high-refresh monitors.
                ctx.request_repaint();
            } else if self.is_seeking {
                needs_repaint = true;
            }
        }

        if let Some(position) = solo_displayed_video_position {
            if matches!(self.current_media_type, Some(MediaType::Video)) {
                if let Some(current_path) = self
                    .current_video_path
                    .clone()
                    .or_else(|| self.current_media_path())
                {
                    if let Some(index) = self
                        .image_list
                        .iter()
                        .position(|path| path == &current_path)
                    {
                        self.manga_record_video_preview_resume_secs(index, position);
                    }
                }
            }
        } else if self.video_player.is_some() {
            self.sync_solo_video_position_to_manga_resume();
        }

        if activate_deferred_video_swap {
            self.consume_deferred_media_view_reset();
            self.retained_media_placeholder_visible = false;
            self.image_changed = true;
            self.pending_media_layout = true;
        }

        // If we are waiting on video dimensions (e.g. right after switching videos),
        // we need a repaint to get the first decoded frame ASAP.
        if self.pending_media_layout {
            needs_repaint = true;
        }

        needs_repaint
    }
}

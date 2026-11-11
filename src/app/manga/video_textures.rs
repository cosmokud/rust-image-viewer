//! Uploading video frames into tile textures.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Update animated GIF/WebP textures in manga mode.
    /// Only the focused animated image is updated to save resources.
    /// Loading of the full animation is done on a background thread to avoid
    /// blocking the UI — the manga texture cache already has a first-frame
    /// thumbnail from the normal manga loader pipeline.
    pub(crate) fn manga_update_animated_textures(&mut self, ctx: &egui::Context) -> bool {
        crate::diag::note_stage("anim_stream");
        if self.mode().is_solo() {
            return false;
        }

        if self.mode().is_masonry() && self.masonry_navigation_active_for_heavy_work() {
            self.manga_anim_streams.clear();
            self.manga_focused_anim_index = None;
            return false;
        }

        let mut needs_repaint = false;
        let mut anim_render_failed: Vec<usize> = Vec::new();
        let active_gif_filter = if self.manga_should_force_triangle_filters() {
            FilterType::Triangle
        } else {
            self.config.gif_resize_filter.to_image_filter()
        };
        let active_downscale_filter = if self.manga_should_force_triangle_filters() {
            FilterType::Triangle
        } else {
            self.config.downscale_filter.to_image_filter()
        };

        let prev_focused = self.manga_focused_anim_index;
        // Determine which animated image should be active (center of viewport).
        let focused_anim_idx = self.manga_get_focused_animated_index();
        self.manga_focused_anim_index = focused_anim_idx;

        // Ensure only one animated WebP stream is active at a time.
        // Any non-focused streams are dropped and their animations reset to the first frame.
        let focused = focused_anim_idx;
        let streams_to_drop: Vec<usize> = self
            .manga_anim_streams
            .keys()
            .copied()
            .filter(|idx| Some(*idx) != focused)
            .collect();
        for idx in &streams_to_drop {
            let stream_done = self
                .manga_anim_stream_done
                .get(idx)
                .copied()
                .unwrap_or(true);

            self.manga_anim_streams.remove(idx);
            self.manga_anim_seekbar_total_frames.remove(idx);
            self.manga_reset_anim_to_first_frame(ctx, *idx, stream_done);
            if !stream_done {
                self.manga_anim_stream_done.insert(*idx, false);
            }
        }

        if prev_focused != focused_anim_idx {
            if let Some(new_idx) = focused_anim_idx {
                let stream_done = self
                    .manga_anim_stream_done
                    .get(&new_idx)
                    .copied()
                    .unwrap_or(true);
                self.manga_reset_anim_to_first_frame(ctx, new_idx, stream_done);
                needs_repaint = true;
            }

            if let Some(prev_idx) = prev_focused {
                if !streams_to_drop.contains(&prev_idx) {
                    let stream_done = self
                        .manga_anim_stream_done
                        .get(&prev_idx)
                        .copied()
                        .unwrap_or(true);
                    self.manga_reset_anim_to_first_frame(ctx, prev_idx, stream_done);
                }
            }
        }

        // ── Determine visible animated-image indices ──
        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_h = self.screen_size.y.max(1.0);
        let viewport_bottom = viewport_top + viewport_h;
        let vis_start = self.manga_index_at_y(viewport_top);
        let vis_end = self.manga_index_at_y(viewport_bottom);
        // ── Start/prepare animation source for the focused animated item only ──
        if let Some(idx) = focused_anim_idx.filter(|&idx| !self.manga_media_is_unloadable(idx)) {
            // Already have the full animation?
            if !(self.manga_animated_images.contains_key(&idx)
                && self
                    .manga_anim_stream_done
                    .get(&idx)
                    .copied()
                    .unwrap_or(true))
                // Already streaming?
                && !self.manga_anim_streams.contains_key(&idx)
                // Already tried and failed?
                && !self.manga_anim_failed.contains(&idx)
            {
                if let Some(path) = self.image_list.get(idx).cloned() {
                    let cached_side_floor = self
                        .manga_texture_cache
                        .peek_texture_dimensions_for_path(idx, path.as_path())
                        .map(|(w, h)| w.max(h).max(1))
                        .unwrap_or(1)
                        .min(self.max_texture_side.max(1));
                    let target_side = self
                        .manga_target_texture_side_for_dynamic_media(
                            idx,
                            MangaMediaType::AnimatedImage,
                        )
                        .max(cached_side_floor);

                    if LoadedImage::is_animated_webp(&path) {
                        // Breadcrumb before the animated decode, as for static/video decodes.
                        crate::diag::note_file(idx, "anim-start", || {
                            format!(
                                "idx={idx} file={} bytes={} side={target_side}",
                                Self::diag_file_label(&path),
                                std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0)
                            )
                        });
                        if let Some(rx) = LoadedImage::start_streaming_webp(
                            &path,
                            Some(target_side),
                            active_gif_filter,
                        ) {
                            self.manga_anim_streams.insert(idx, rx);
                            self.manga_anim_stream_done.insert(idx, false);

                            // Ensure there's a LoadedImage entry with at least the first frame.
                            if !self.manga_animated_images.contains_key(&idx) {
                                match LoadedImage::load_first_frame_only(
                                    &path,
                                    Some(target_side),
                                    active_downscale_filter,
                                    active_gif_filter,
                                ) {
                                    Ok(img) => {
                                        self.manga_animated_images.insert(idx, img);
                                    }
                                    Err(_) => {
                                        anim_render_failed.push(idx);
                                    }
                                }
                            }
                            let base_frames = self
                                .manga_animated_images
                                .get(&idx)
                                .map(|img| img.frame_count())
                                .unwrap_or(1);
                            self.manga_anim_seekbar_total_frames
                                .entry(idx)
                                .or_insert(base_frames);
                        } else {
                            self.manga_anim_failed.insert(idx);
                        }
                    } else {
                        // GIF (and any other non-WebP animated image): load the full animation directly.
                        match LoadedImage::load_with_max_texture_side(
                            &path,
                            Some(target_side),
                            active_downscale_filter,
                            active_gif_filter,
                        ) {
                            Ok(img) if img.is_animated() => {
                                let total_frames = img.frame_count();
                                self.manga_animated_images.insert(idx, img);
                                self.manga_anim_stream_done.insert(idx, true);
                                self.manga_anim_seekbar_total_frames
                                    .insert(idx, total_frames.max(1));
                            }
                            Ok(img) => {
                                // Decoded, but not as animation: a single usable frame is all
                                // this file has. Show it as a still instead of retrying.
                                self.manga_animated_images.insert(idx, img);
                                self.manga_anim_stream_done.insert(idx, true);
                                self.manga_anim_seekbar_total_frames.remove(&idx);
                                self.manga_anim_failed.insert(idx);
                            }
                            Err(_) => {
                                let has_thumbnail = self.image_list.get(idx).is_some_and(|path| {
                                    self.manga_texture_cache.contains_for_path(idx, path)
                                });
                                if has_thumbnail {
                                    // The tile already shows a frame; only the animation is
                                    // unavailable, so it stays a still image.
                                    self.manga_anim_failed.insert(idx);
                                } else {
                                    // Nothing drawable at all: X card, and no further decodes.
                                    self.manga_mark_media_unloadable(idx);
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Drain frames from active streams ──
        let stream_indices: Vec<usize> = self.manga_anim_streams.keys().copied().collect();
        for idx in stream_indices {
            const MAX_MANGA_STREAM_FRAMES_PER_TICK: usize = 8;
            let mut disconnected = false;
            let mut drained_this_tick = 0usize;
            if let Some(rx) = self.manga_anim_streams.get(&idx) {
                loop {
                    if drained_this_tick >= MAX_MANGA_STREAM_FRAMES_PER_TICK {
                        break;
                    }

                    match rx.try_recv() {
                        Ok(frame) => {
                            if let Some(img) = self.manga_animated_images.get_mut(&idx) {
                                img.frames.push(frame);
                            }
                            drained_this_tick += 1;
                            needs_repaint = true;
                        }
                        Err(crossbeam_channel::TryRecvError::Empty) => break,
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
            }
            if disconnected {
                self.manga_anim_streams.remove(&idx);
                self.manga_anim_stream_done.insert(idx, true);
                self.manga_anim_seekbar_total_frames.remove(&idx);
            }
        }

        // Request repaint while any stream is active.
        if !self.manga_anim_streams.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        // ── Update animation frames for the focused animated image only ──
        let anim_upload_policy = base_upload_policy_inputs(self)
            .policy(crate::texture_upload::UploadKind::MangaAnimatedFrame);
        for &idx in focused_anim_idx.iter() {
            let cached_side_floor = self
                .image_list
                .get(idx)
                .and_then(|path| {
                    self.manga_texture_cache
                        .peek_texture_dimensions_for_path(idx, path)
                })
                .map(|(w, h)| w.max(h).max(1))
                .unwrap_or(1)
                .min(self.max_texture_side.max(1));
            let target_side = self
                .manga_target_texture_side_for_dynamic_media(idx, MangaMediaType::AnimatedImage)
                .max(cached_side_floor);
            let stream_done = self
                .manga_anim_stream_done
                .get(&idx)
                .copied()
                .unwrap_or(true);

            let webp_override_delay = if let Some((path, frame_count, total_duration_ms)) =
                self.manga_animated_images.get(&idx).and_then(|img| {
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
                self.webp_effective_fps_override()
                    .map(Self::frame_delay_for_fps)
            } else {
                None
            };

            if let Some(img) = self.manga_animated_images.get_mut(&idx) {
                let frame_changed = if !self.gif_paused && img.frame_count() > 1 {
                    // If still streaming and on the last frame, hold rather than wrap.
                    if !stream_done && img.current_frame_index() + 1 >= img.frame_count() {
                        false
                    } else if let Some(delay) = webp_override_delay {
                        Self::update_animation_with_delay(img, delay)
                    } else {
                        img.update_animation()
                    }
                } else {
                    false
                };

                let cache_entry_matches = self
                    .image_list
                    .get(idx)
                    .is_some_and(|path| self.manga_texture_cache.contains_for_path(idx, path));
                if frame_changed || !cache_entry_matches {
                    let frame = img.current_frame_data();

                    let (w, h, pixels) = downscale_rgba_if_needed(
                        frame.width,
                        frame.height,
                        &frame.pixels,
                        target_side,
                        active_gif_filter,
                    );

                    let entry_path = self.image_list.get(idx).cloned();
                    let mut slot = entry_path.as_ref().and_then(|path| {
                        self.manga_texture_cache
                            .get_texture_handle_info_for_path(idx, path)
                    });
                    let uploaded = crate::texture_upload::upload(
                        ctx,
                        &format!("manga_anim_{}", idx),
                        slot.as_mut().map(|(texture, _, _, _)| texture),
                        crate::texture_upload::PixelSource::Rgba(pixels.as_ref()),
                        (w, h),
                        &anim_upload_policy,
                    );
                    if uploaded.used_placeholder {
                        // The GPU refused the frame; every later frame will be refused too.
                        anim_render_failed.push(idx);
                    }
                    if let Some(path) = entry_path.as_deref().filter(|_| slot.is_some()) {
                        let _ = self.manga_texture_cache.update_texture(
                            idx,
                            path,
                            uploaded.handle,
                            w,
                            h,
                        );
                    } else if let Some(path) = entry_path {
                        let evicted = self.manga_texture_cache.insert_with_type(
                            idx,
                            path,
                            uploaded.handle,
                            w,
                            h,
                            MangaMediaType::AnimatedImage,
                        );
                        if !evicted.is_empty() {
                            if let Some(loader) = self.manga_loader.as_mut() {
                                loader.forget_indices(evicted);
                            }
                        }
                    }

                    needs_repaint = true;
                }

                // Schedule next repaint for animation.
                if img.frame_count() > 1 && !self.gif_paused {
                    let current_delay = webp_override_delay
                        .unwrap_or_else(|| Duration::from_millis(img.current_delay_ms() as u64));
                    let elapsed = img.last_frame_time.elapsed();
                    if elapsed < current_delay {
                        ctx.request_repaint_after(current_delay - elapsed);
                    } else {
                        needs_repaint = true;
                    }
                }
            }
        }

        // A focused animation that decoded but will not go to the GPU is a file that cannot be
        // shown at all: mark it so its tile becomes the X card instead of a spinner loop.
        for idx in anim_render_failed {
            self.manga_mark_media_unloadable(idx);
        }

        // ── Evict animated images that are far from the viewport ──
        let keep_start = vis_start.saturating_sub(5);
        let keep_end = vis_end.saturating_add(5);

        let indices_to_remove: Vec<usize> = self
            .manga_animated_images
            .keys()
            .filter(|&&idx| idx < keep_start || idx > keep_end)
            .copied()
            .collect();

        for idx in indices_to_remove {
            self.manga_animated_images.remove(&idx);
            self.manga_anim_streams.remove(&idx);
            self.manga_anim_stream_done.remove(&idx);
            self.manga_anim_seekbar_total_frames.remove(&idx);
            if self.manga_focused_anim_index == Some(idx) {
                self.manga_focused_anim_index = None;
            }
        }

        needs_repaint
    }
}

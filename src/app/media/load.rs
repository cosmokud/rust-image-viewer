//! The media open path (`load_media` and its internals).

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Load an image from path
    pub(crate) fn load_image(&mut self, path: &PathBuf) {
        self.load_media_internal(path, false);
    }

    /// Load any media (image or video) from path
    pub(crate) fn load_media(&mut self, path: &PathBuf) {
        self.load_media_internal(path, false);
    }

    pub(crate) fn load_media_internal(
        &mut self,
        path: &PathBuf,
        retain_visible_media_until_ready: bool,
    ) {
        let load_media_start = Instant::now();
        if !retain_visible_media_until_ready {
            self.set_solo_preload_momentum(SoloPreloadMomentum::Neutral);
        }

        if self.mode().is_solo()
            && self.mode().layout() == MangaLayoutMode::Masonry
            && self.has_resident_masonry_runtime_cache()
        {
            self.pause_masonry_metadata_preload();
        } else {
            self.reset_masonry_metadata_preload();
        }
        self.clear_pending_media_load();
        self.pending_video_thumbnail_placeholder = None;
        self.clear_video_playback_unavailable_state();

        self.current_file_size_label = None;
        self.current_file_size_label_path = None;
        self.pending_file_size_probe = None;
        self.pending_file_size_probe_path = None;

        // Update the native window title (taskbar title) using Unicode-safe conversion.
        self.pending_window_title = Some(self.compute_window_title_for_path(path));

        // Determine media type up-front so we can decide whether to keep a placeholder frame.
        let is_folder_entry = self.is_folder_navigation_entry_path(path.as_path());
        let media_type = if is_folder_entry {
            Some(MediaType::Image)
        } else {
            get_media_type(path)
        };
        self.current_media_type = media_type;
        self.current_video_path =
            matches!(media_type, Some(MediaType::Video)).then(|| path.clone());

        let mut used_mode_switch_placeholder = false;
        let transition_placeholder = self
            .pending_mode_switch_placeholder
            .take()
            .filter(|placeholder| {
                let matches_target = Some(placeholder.media_type) == media_type;
                if matches_target {
                    used_mode_switch_placeholder = true;
                }
                matches_target
            })
            .or_else(|| {
                if retain_visible_media_until_ready
                    && Self::retain_visible_media_placeholder_for_swap(
                        self.mode().is_fullscreen(),
                        media_type,
                    )
                {
                    self.capture_current_media_placeholder(media_type)
                } else {
                    None
                }
            });
        let keep_current_view_until_swap =
            retain_visible_media_until_ready && transition_placeholder.is_some();

        // Clear previous media state.
        // When a placeholder was captured above we immediately restore it after clearing
        // the current decode state so the visible frame stays on screen during navigation.
        // MEMORY OPTIMIZATION: Explicitly drop textures to release GPU memory immediately.
        // Setting to None allows Rust to drop the TextureHandle, which signals egui to
        // free the underlying GPU texture on the next frame.
        self.stop_fullscreen_video_playback();
        if let Some(texture) = self.video_texture.take() {
            drop(texture);
        }
        self.video_texture_source_path = None;
        self.video_texture_dims = None;
        if let Some(texture) = self.texture.take() {
            drop(texture);
        }
        self.image_texture_dims = None;
        self.image = None;
        self.retained_media_placeholder_visible = transition_placeholder.is_some();

        if let Some(placeholder) = transition_placeholder {
            match placeholder.media_type {
                MediaType::Image => {
                    self.texture = Some(placeholder.texture);
                    self.image_texture_dims = Some(placeholder.dims);
                }
                MediaType::Video => {
                    self.video_texture = Some(placeholder.texture);
                    self.video_texture_source_path = self
                        .current_video_path
                        .clone()
                        .or_else(|| self.current_media_path());
                    self.video_texture_dims = Some(placeholder.dims);
                }
            }
        }

        // Cancel any in-flight background animation stream.
        self.reset_fullscreen_anim_stream_state();

        // Reset GIF playback state for new media
        self.gif_paused = false;
        self.gif_seeking = false;
        self.gif_seek_preview_frame = None;

        if keep_current_view_until_swap {
            self.freeze_current_media_view();
            self.defer_media_view_reset = true;
        } else {
            self.reset_media_view_for_swap();
            self.defer_media_view_reset = false;

            if used_mode_switch_placeholder {
                self.image_changed = true;
            }
        }
        self.error_message = None;

        let defer_directory_work_for_fast_startup = self.defer_directory_work_for_fast_startup();
        if !defer_directory_work_for_fast_startup {
            self.start_async_file_size_probe(path.clone());
        }

        // Reuse cached directory listing when the parent folder is unchanged.
        let index_stats_before = self.media_directory_index.stats();
        let index_lookup_start = Instant::now();

        self.pending_media_directory_scan = None;
        self.pending_media_directory_target = None;
        self.pending_media_directory_scan_kind = None;
        self.pending_media_directory_started_at = None;

        if defer_directory_work_for_fast_startup {
            self.set_image_list(vec![path.clone()]);
        } else {
            if let Some(files) = self.media_directory_index.try_cached_media_for_path(path) {
                self.set_image_list(files);
            } else {
                // Keep current media navigable immediately while the full directory scan runs in background.
                self.set_image_list(vec![path.clone()]);
                let _ = self
                    .begin_media_directory_scan(path, PendingMediaDirectoryScanKind::InitialLoad);
            }
        }

        if self.image_list.is_empty() {
            self.set_image_list(vec![path.clone()]);
        }

        self.perf_metrics
            .record(Metric::MediaIndexLookupMs, index_lookup_start.elapsed());
        let index_stats_after = self.media_directory_index.stats();
        if index_stats_after.hits > index_stats_before.hits {
            self.perf_metrics.count(
                Metric::MediaIndexHits,
                index_stats_after.hits - index_stats_before.hits,
            );
        }
        if index_stats_after.misses > index_stats_before.misses {
            self.perf_metrics.count(
                Metric::MediaIndexMisses,
                index_stats_after.misses - index_stats_before.misses,
            );
        }
        self.set_current_index_clamped(
            self.image_list
                .iter()
                .position(|candidate| candidate == path)
                .unwrap_or(0),
        );

        match media_type {
            Some(MediaType::Video) => {
                if !gstreamer_runtime_available() {
                    self.gstreamer_initialized = false;
                    self.drop_retained_media_placeholder();
                    self.set_video_playback_unavailable_for_path(
                        path,
                        Self::gstreamer_missing_video_error_text().to_string(),
                    );
                    self.show_video_controls = false;
                    self.image_changed = true;
                    self.pending_media_layout = false;
                    self.perf_metrics
                        .record(Metric::LoadMediaPrepareMs, load_media_start.elapsed());
                    return;
                }

                // Mark GStreamer as initialized (it will be lazily initialized on first use)
                self.gstreamer_initialized = true;

                self.start_async_video_load(path.clone());
            }
            Some(MediaType::Image) => {
                if is_folder_entry {
                    self.consume_deferred_media_view_reset();
                    self.drop_retained_media_placeholder();
                    self.image = Some(Self::build_folder_placeholder_image(
                        path.clone(),
                        Self::is_up_navigation_entry_path(path.as_path()),
                    ));
                    self.texture = None;
                    self.image_texture_dims = Some((512, 512));
                    self.show_video_controls = false;
                    self.error_message = None;
                    self.image_changed = true;
                    self.pending_media_layout = false;
                    self.perf_metrics
                        .record(Metric::LoadMediaPrepareMs, load_media_start.elapsed());
                    return;
                }

                // Load as image with configured filters.
                // For animated WebP we only decode the FIRST frame here so the
                // window appears instantly, then start streaming remaining frames
                // in the background so the animation begins playing progressively.
                let downscale_filter = self.config.downscale_filter.to_image_filter();
                let gif_filter = self.config.gif_resize_filter.to_image_filter();
                let target_lod_side =
                    self.solo_target_texture_side_for_path(path, MediaType::Image, true);
                let max_tex =
                    Self::solo_image_load_texture_side(target_lod_side, self.max_texture_side);

                if self.try_load_image_from_decoded_cache(path, max_tex, gif_filter, false) {
                    if !defer_directory_work_for_fast_startup {
                        self.schedule_solo_probe_window(path, media_type);
                    }
                    self.perf_metrics
                        .record(Metric::LoadMediaPrepareMs, load_media_start.elapsed());
                    return;
                }

                if !self.mode().is_fullscreen() {
                    self.pending_media_layout = true;
                }
                self.start_async_image_load(path.clone(), max_tex, downscale_filter, gif_filter);
            }
            None => {
                self.drop_retained_media_placeholder();
                self.error_message = Some(format!("Unsupported file format: {:?}", path));
            }
        }

        if media_type.is_some()
            && !is_folder_entry
            && !defer_directory_work_for_fast_startup
            && self.pending_media_load.is_none()
        {
            self.schedule_solo_probe_window(path, media_type);
        }

        self.perf_metrics
            .record(Metric::LoadMediaPrepareMs, load_media_start.elapsed());
    }

    /// Save the current view state for the current image (fullscreen only).
    /// This allows restoring zoom, pan, and rotation when returning to this image.
    /// The camera the current solo-media view is showing right now.
    pub(crate) fn current_fullscreen_view_state(&self) -> FullscreenViewState {
        FullscreenViewState {
            zoom: self.zoom,
            zoom_target: self.zoom_target,
            offset: self.offset,
            precise_rotation_degrees: self.precise_rotation_degrees,
            precise_rotation_target_degrees: self.precise_rotation_target_degrees,
            rotation_steps: self.current_rotation_steps,
            flip_horizontal: self.flip_horizontal,
            flip_vertical: self.flip_vertical,
        }
    }

    pub(crate) fn save_current_fullscreen_view_state(&mut self) {
        // The state is the solo camera: saving it from the strip or the grid stored a zoom and a
        // pan that belonged to no image on screen, and restoring it later moved their camera.
        if !self.mode().is_fullscreen()
            || !self.mode().is_solo()
            || !self.config.fullscreen_remember_view_state_in_ram
        {
            return;
        }

        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            return;
        };

        let state = self.current_fullscreen_view_state();
        self.fullscreen_view_states.insert(path, state);
    }

    pub(crate) fn clear_pending_media_load(&mut self) {
        self.pending_media_load = None;
        self.retained_media_placeholder_visible = false;
        self.defer_media_view_reset = false;
    }
}

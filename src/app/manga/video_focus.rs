//! Focused video tile selection, LOD refresh and eviction.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_maybe_refresh_focused_video_lod(&mut self) {
        if self.mode().is_solo() || self.mode().is_masonry() {
            return;
        }
        // Long-strip policy: keep focused playback at source quality and do not
        // restart the decoder for zoom-driven LOD swaps.
    }

    /// Evict video players that are far from the current view to conserve resources.
    pub(crate) fn manga_evict_distant_video_players(
        &mut self,
        focused_idx: usize,
        visible_indices_hint: Option<&HashSet<usize>>,
    ) {
        if !self.mode().is_masonry() {
            self.manga_strip_retain_only_focused_video(focused_idx);
            return;
        }

        if self.manga_video_players.len() <= self.manga_max_video_players {
            return;
        }

        let computed_visible_indices = if visible_indices_hint.is_none() {
            Some(self.manga_visible_indices_set_for_preview_tracking())
        } else {
            None
        };
        let visible_indices = visible_indices_hint
            .or(computed_visible_indices.as_ref())
            .expect("visible indices must be available");

        // Calculate distances and sort by distance from focused
        let mut indexed_distances: Vec<(usize, usize)> = self
            .manga_video_players
            .keys()
            .map(|&idx| {
                let dist = idx.abs_diff(focused_idx);
                (idx, dist)
            })
            .filter(|(idx, _)| {
                Some(*idx) != self.manga_focused_video_index && !visible_indices.contains(idx)
            })
            .collect();

        indexed_distances.sort_by_key(|&(_, dist)| dist);

        // Remove the furthest *non-visible* players until we're under the limit.
        // Visible previewed videos must stay alive in RAM for seamless hover resume.
        while self.manga_video_players.len() > self.manga_max_video_players {
            if let Some((idx, _)) = indexed_distances.pop() {
                let record_position = self.manga_video_player_matches(idx);
                if let Some(mut player) = self.remove_manga_video_player(idx) {
                    if record_position {
                        if let Some(position) = Self::latest_video_position_from_player(&mut player)
                        {
                            self.manga_record_video_preview_resume_secs(idx, position);
                        }
                    }
                }
                self.remove_manga_video_texture(idx);
            } else {
                break;
            }
        }
    }

    /// Poll video frames for manga mode and update textures.
    pub(crate) fn manga_update_video_textures(&mut self, ctx: &egui::Context) {
        if self.mode().is_solo() {
            return;
        }

        // Only update the focused video's texture (to save resources)
        if let Some(focused_idx) = self.manga_focused_video_index {
            let upload_side = if self.mode().is_masonry() {
                self.manga_target_texture_side_for_dynamic_media(focused_idx, MangaMediaType::Video)
            } else {
                // Long-strip focused playback keeps highest available quality.
                self.max_texture_side.max(1)
            };
            let mut video_dimensions_changed = false;
            let mut focused_position_to_record = None;

            if let Some(player) = self.manga_video_players.get_mut(&focused_idx) {
                // Detect a frozen decoder early and demote the AV1 hardware decoders so
                // the next load starts without a state-change timeout delay.
                player.mark_wedged_if_stalled();

                // Update duration cache
                player.update_duration();

                // Check for video end and handle looping
                if player.is_eos() {
                    if self.config.video_loop {
                        // Enqueues only: a wedged decoder used to block this frame for 5 s.
                        player.restart_async();
                    }
                }

                // Get new frame if available
                if let Some(frame) = player.get_frame() {
                    focused_position_to_record = frame.pts.or_else(|| player.position());

                    // Update layout dimensions from source/original video size (not decoded frame size).
                    // Decoded frames may be output-bounded/downscaled, which would otherwise shrink
                    // strip layout dimensions and cause cascading reflow/squash at low zoom.
                    let (source_w, source_h) = player.dimensions();
                    let (layout_w, layout_h) = if source_w > 0 && source_h > 0 {
                        (source_w, source_h)
                    } else {
                        (frame.width, frame.height)
                    };
                    if layout_w > 0 && layout_h > 0 {
                        if !self.masonry_authoritative_dimension_lock_active() {
                            if let Some(ref mut loader) = self.manga_loader {
                                video_dimensions_changed =
                                    loader.update_video_dimensions(focused_idx, layout_w, layout_h);
                            }
                        }
                    }

                    let no_downscale = frame.width <= upload_side && frame.height <= upload_side;
                    // Live video textures are updated every frame; the module's VideoFrame
                    // policy keeps mipmaps off because the sink already outputs
                    // near-display resolution.
                    let policy = base_upload_policy_inputs(self)
                        .policy(crate::texture_upload::UploadKind::VideoFrame);
                    let downscaled = (!no_downscale).then(|| {
                        downscale_rgba_if_needed(
                            frame.width,
                            frame.height,
                            &frame.pixels,
                            upload_side,
                            if self.manga_should_force_triangle_filters() {
                                FilterType::Triangle
                            } else {
                                self.config.downscale_filter.to_image_filter()
                            },
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

                    let uploaded = crate::texture_upload::upload(
                        ctx,
                        &format!("manga_video_{}", focused_idx),
                        self.manga_video_textures
                            .get_mut(&focused_idx)
                            .map(|(texture, _, _)| texture),
                        pixels,
                        (w, h),
                        &policy,
                    );
                    if let Some((_, stored_w, stored_h)) =
                        self.manga_video_textures.get_mut(&focused_idx)
                    {
                        *stored_w = w;
                        *stored_h = h;
                    } else {
                        self.manga_video_textures
                            .insert(focused_idx, (uploaded.handle, w, h));
                    }
                    if let Some(path) = self.image_list.get(focused_idx).cloned() {
                        self.manga_video_texture_paths.insert(focused_idx, path);
                    }
                }
            }

            if let Some(position) = focused_position_to_record {
                self.manga_record_video_preview_resume_secs(focused_idx, position);
            }

            if video_dimensions_changed {
                if self.mode().is_masonry() && !self.masonry_authoritative_dimension_lock_active() {
                    self.masonry_queue_dimension_updates(std::iter::once(focused_idx));
                    if !self.masonry_navigation_active_for_heavy_work() {
                        let force_flush = !self.masonry_metadata_preload_active;
                        self.masonry_flush_pending_dimension_updates(force_flush);
                    }
                } else if !self.mode().is_masonry() {
                    // Strip mode uses cached page-height offsets for placement.
                    // If video playback reports better dimensions than the initial probe,
                    // force a layout rebuild so width/height stay in sync (no stretch/squash).
                    self.invalidate_manga_layout_cache();
                }
            }
        }
    }
}

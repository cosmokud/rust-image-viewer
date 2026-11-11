//! Strip/masonry video controls and their seekbars.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Draw seekbar and controls for a video in manga mode
    pub(crate) fn draw_manga_video_seekbar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        video_idx: usize,
    ) {
        let video_seek_policy = self.config.video_seek_policy;
        let commit_seek_mode = self.commit_video_seek_mode();
        let mut play_toggle_requested = false;
        let mut file_navigation_requested: Option<VideoFileNavigation> = None;
        let mut audio_track_requested: Option<i32> = None;
        let mut subtitle_selection_requested: Option<VideoSubtitleSelection> = None;
        let resume_error: Option<String> = None;

        if !self.manga_video_player_matches(video_idx) {
            return;
        }

        let current_video_path = self.image_list.get(video_idx).cloned();
        let Some(player) = self.manga_video_players.get_mut(&video_idx) else {
            return;
        };

        let position_fraction = player.position_fraction() as f32;
        let duration = player.duration();
        let position = player.position();
        let audio_tracks = player.audio_tracks();
        let current_audio_track = player.current_audio_track_index();
        let embedded_subtitle_tracks = player.embedded_subtitle_tracks();
        let current_subtitle_selection = player.current_subtitle_selection();
        let current_audio_button_label =
            Self::current_audio_button_label(&audio_tracks, current_audio_track);
        let current_subtitle_button_label = Self::current_subtitle_button_label(
            &current_subtitle_selection,
            &embedded_subtitle_tracks,
        );

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
            let seek_bar_height = 6.0;
            let available_width = ui.available_width();
            let (seek_rect, seek_response) = ui.allocate_exact_size(
                egui::Vec2::new(available_width, seek_bar_height + 8.0),
                egui::Sense::click_and_drag(),
            );

            let bar_inner = egui::Rect::from_min_size(
                egui::pos2(
                    seek_rect.min.x,
                    seek_rect.center().y - seek_bar_height / 2.0,
                ),
                egui::Vec2::new(seek_rect.width(), seek_bar_height),
            );

            // Background bar
            ui.painter()
                .rect_filled(bar_inner, 3.0, egui::Color32::from_gray(60));

            // Progress bar
            let display_fraction = if self.manga_video_seeking {
                self.manga_video_seek_preview_fraction
                    .unwrap_or(position_fraction)
            } else {
                position_fraction
            };
            let progress_width = bar_inner.width() * display_fraction;
            if progress_width > 0.0 {
                let progress_rect = egui::Rect::from_min_size(
                    bar_inner.min,
                    egui::Vec2::new(progress_width, seek_bar_height),
                );
                ui.painter()
                    .rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(66, 133, 244));
            }

            // Seek handle
            let handle_x = bar_inner.min.x + progress_width;
            let handle_center = egui::pos2(handle_x, bar_inner.center().y);
            let handle_radius = if seek_response.hovered() || seek_response.dragged() {
                8.0
            } else {
                6.0
            };
            ui.painter()
                .circle_filled(handle_center, handle_radius, egui::Color32::WHITE);

            // Handle seeking
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released =
                ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

            if seek_response.is_pointer_button_down_on() && !self.manga_video_seeking {
                if let Some(player) = self.manga_video_players.get(&video_idx) {
                    self.manga_video_seeking = true;
                    self.manga_video_seek_was_playing = player.is_playing();
                    if self.manga_video_seek_was_playing {
                        if let Some(p) = self.manga_video_players.get_mut(&video_idx) {
                            p.pause_async();
                        }
                    }
                    self.manga_video_seek_last_requested_fraction = None;
                    self.manga_video_last_seek_sent = Instant::now() - Duration::from_millis(1000);
                }
            }

            if self.manga_video_seeking && primary_down {
                if let Some(seek_fraction) = Self::active_seek_pointer_fraction(ctx, bar_inner) {
                    self.manga_video_seek_preview_fraction = Some(seek_fraction);

                    if Self::should_emit_drag_seek(
                        duration,
                        seek_fraction,
                        self.manga_video_seek_last_requested_fraction,
                        self.manga_video_last_seek_sent,
                    ) {
                        if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                            let effective_drag_seek_mode =
                                Self::adaptive_drag_seek_mode_for_fraction(
                                    video_seek_policy,
                                    duration,
                                    seek_fraction,
                                    self.manga_video_seek_last_requested_fraction,
                                );
                            let _ = player
                                .seek_with_mode(seek_fraction as f64, effective_drag_seek_mode);
                        }
                        self.manga_video_seek_last_requested_fraction = Some(seek_fraction);
                        self.manga_video_last_seek_sent = Instant::now();
                    }
                }
                ctx.request_repaint();
            }

            if seek_response.clicked() && !self.manga_video_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction =
                        ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                        let _ = player.seek_with_mode(seek_fraction as f64, commit_seek_mode);
                    }
                    ctx.request_repaint();
                }
            }

            if self.manga_video_seeking && (primary_released || !primary_down) {
                let final_fraction = Self::active_seek_pointer_fraction(ctx, bar_inner)
                    .or(self.manga_video_seek_preview_fraction)
                    .or(self.manga_video_seek_last_requested_fraction);
                if let Some(final_fraction) = final_fraction {
                    if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                        let _ = player.seek_with_mode(final_fraction as f64, commit_seek_mode);
                    }
                }
                self.manga_video_seeking = false;
                self.manga_video_seek_preview_fraction = None;
                self.manga_video_seek_last_requested_fraction = None;
                self.manga_video_last_seek_sent = Instant::now();

                if self.manga_video_seek_was_playing {
                    if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                        player.play_async();
                    }
                }
                self.manga_video_seek_was_playing = false;
                ctx.request_repaint();
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                let Some(player) = self.manga_video_players.get_mut(&video_idx) else {
                    return;
                };

                // Play/Pause button
                let is_playing = player.is_playing();
                let play_btn = Self::video_control_icon_button(
                    ui,
                    if is_playing {
                        VideoControlIcon::Pause
                    } else {
                        VideoControlIcon::Play
                    },
                    if is_playing { "Pause" } else { "Play" },
                    None,
                    false,
                );
                if play_btn.clicked() {
                    play_toggle_requested = true;
                }

                ui.add_space(4.0);

                let prev_btn = Self::video_control_icon_button(
                    ui,
                    VideoControlIcon::Previous,
                    self.navigation_tooltip_previous(),
                    None,
                    false,
                );
                if prev_btn.clicked() {
                    file_navigation_requested = Some(VideoFileNavigation::Previous);
                }

                let next_btn = Self::video_control_icon_button(
                    ui,
                    VideoControlIcon::Next,
                    self.navigation_tooltip_next(),
                    None,
                    false,
                );
                if next_btn.clicked() {
                    file_navigation_requested = Some(VideoFileNavigation::Next);
                }

                ui.add_space(8.0);

                // Time display
                let pos_str = position
                    .map(format_duration)
                    .unwrap_or_else(|| "0:00".to_string());
                let dur_str = duration
                    .map(format_duration)
                    .unwrap_or_else(|| "0:00".to_string());
                ui.label(
                    egui::RichText::new(format!("{} / {}", pos_str, dur_str))
                        .color(egui::Color32::WHITE)
                        .size(12.0),
                );

                // Right side: volume controls
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let Some(player) = self.manga_video_players.get_mut(&video_idx) else {
                        return;
                    };

                    // Mute button
                    let is_muted = player.is_muted();
                    let mute_btn = Self::video_control_icon_button(
                        ui,
                        if is_muted {
                            VideoControlIcon::VolumeOff
                        } else {
                            VideoControlIcon::VolumeOn
                        },
                        if is_muted { "Unmute" } else { "Mute" },
                        None,
                        false,
                    );
                    if mute_btn.clicked() {
                        player.toggle_mute();
                        // Persist user's mute choice for all manga videos
                        self.manga_video_user_muted = Some(player.is_muted());
                        self.config
                            .update_video_state(player.is_muted(), player.volume());
                        self.pending_idle_config_sync = true;
                    }

                    // Volume slider
                    let volume = player.volume() as f32;
                    let vol_slider_width = 80.0;
                    let vol_slider_height = 4.0;
                    let (vol_rect, vol_response) = ui.allocate_exact_size(
                        egui::Vec2::new(vol_slider_width, 20.0),
                        egui::Sense::click_and_drag(),
                    );

                    let vol_bar = egui::Rect::from_min_size(
                        egui::pos2(
                            vol_rect.min.x,
                            vol_rect.center().y - vol_slider_height / 2.0,
                        ),
                        egui::Vec2::new(vol_slider_width, vol_slider_height),
                    );

                    ui.painter()
                        .rect_filled(vol_bar, 2.0, egui::Color32::from_gray(60));

                    let vol_width = vol_bar.width() * volume;
                    if vol_width > 0.0 {
                        let vol_progress = egui::Rect::from_min_size(
                            vol_bar.min,
                            egui::Vec2::new(vol_width, vol_slider_height),
                        );
                        ui.painter()
                            .rect_filled(vol_progress, 2.0, egui::Color32::WHITE);
                    }

                    let vol_handle_x = vol_bar.min.x + vol_width;
                    let vol_handle_center = egui::pos2(vol_handle_x, vol_bar.center().y);
                    ui.painter()
                        .circle_filled(vol_handle_center, 5.0, egui::Color32::WHITE);

                    if vol_response.dragged() || vol_response.clicked() {
                        self.manga_video_volume_dragging = true;
                        if let Some(pos) = vol_response.interact_pointer_pos() {
                            let new_vol =
                                ((pos.x - vol_bar.min.x) / vol_bar.width()).clamp(0.0, 1.0);
                            player.set_volume(new_vol as f64);
                            // Persist user's volume choice for all manga videos
                            self.manga_video_user_volume = Some(new_vol as f64);
                            if player.is_muted() && new_vol > 0.0 {
                                player.set_muted(false);
                                // Also persist unmuted state
                                self.manga_video_user_muted = Some(false);
                            }
                            self.config
                                .update_video_state(player.is_muted(), player.volume());
                            self.pending_idle_config_sync = true;
                        }
                    }
                    if vol_response.drag_stopped()
                        || (self.manga_video_volume_dragging && !primary_down)
                    {
                        self.manga_video_volume_dragging = false;
                    }

                    ui.add_space(6.0);

                    let subtitle_popup_id = Self::manga_video_subtitle_popup_id(video_idx);
                    let subtitle_popup_open = ui.memory(|mem| mem.is_popup_open(subtitle_popup_id));
                    let subtitle_btn = Self::video_control_icon_button(
                        ui,
                        VideoControlIcon::SubtitleTracks,
                        "Subtitle track",
                        Some(current_subtitle_button_label.as_str()),
                        subtitle_popup_open,
                    );
                    if subtitle_btn.clicked() {
                        ui.memory_mut(|mem| mem.toggle_popup(subtitle_popup_id));
                    }
                    let external_subtitle_tracks = if subtitle_popup_open || subtitle_btn.clicked()
                    {
                        current_video_path
                            .as_deref()
                            .map(Self::external_subtitle_options_for_video)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    if let Some(selection) = Self::draw_subtitle_track_popup(
                        ui,
                        subtitle_popup_id,
                        &subtitle_btn,
                        &embedded_subtitle_tracks,
                        &external_subtitle_tracks,
                        &current_subtitle_selection,
                    ) {
                        subtitle_selection_requested = Some(selection);
                    }

                    let audio_popup_id = Self::manga_video_audio_popup_id(video_idx);
                    let audio_popup_open = ui.memory(|mem| mem.is_popup_open(audio_popup_id));
                    let audio_btn = Self::video_control_icon_button(
                        ui,
                        VideoControlIcon::AudioTracks,
                        "Audio track",
                        Some(current_audio_button_label.as_str()),
                        audio_popup_open,
                    );
                    if audio_btn.clicked() {
                        ui.memory_mut(|mem| mem.toggle_popup(audio_popup_id));
                    }
                    if let Some(selected_track) = Self::draw_audio_track_popup(
                        ui,
                        audio_popup_id,
                        &audio_btn,
                        &audio_tracks,
                        current_audio_track,
                    ) {
                        audio_track_requested = Some(selected_track);
                    }
                });
            });
        });

        if let Some(err) = resume_error {
            self.remove_manga_video_player(video_idx);
            self.remove_manga_video_texture(video_idx);
            self.manga_video_preview_resume_secs.remove(&video_idx);
            if self.manga_focused_video_index == Some(video_idx) {
                self.manga_focused_video_index = None;
            }
            self.video_playback_unavailable_reason = Some(err);
            self.queue_video_playback_unavailable_popup();
        }

        if play_toggle_requested {
            self.try_toggle_manga_video_play_pause(video_idx);
        }

        if let Some(selected_track) = audio_track_requested {
            self.queue_manga_audio_track_switch(ctx, video_idx, selected_track);
        }

        if let Some(selection) = subtitle_selection_requested {
            if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                if let Err(err) = player.set_subtitle_selection(selection) {
                    tracing::warn!("failed to switch manga subtitle track: {}", err);
                }
            }
        }

        if let Some(navigation) = file_navigation_requested {
            match navigation {
                VideoFileNavigation::Previous => self.navigate_prev_for_video_mode(),
                VideoFileNavigation::Next => self.navigate_next_for_video_mode(),
            }
        }
    }

    /// Draw seekbar for animated GIFs in manga mode
    pub(crate) fn draw_manga_gif_seekbar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        gif_idx: usize,
    ) {
        let Some(img) = self.manga_animated_images.get(&gif_idx) else {
            return;
        };

        if !img.is_animated() {
            return;
        }

        let frame_count = img.frame_count();
        let current_frame = img.current_frame_index();
        let total_duration_ms = img.total_duration_ms();
        let mut display_frame_count = frame_count;
        let is_streaming = self.manga_anim_streams.contains_key(&gif_idx)
            || self
                .manga_anim_stream_done
                .get(&gif_idx)
                .map_or(false, |d| !d);
        if is_streaming {
            let base = self
                .manga_anim_seekbar_total_frames
                .get(&gif_idx)
                .copied()
                .unwrap_or(frame_count.max(1));
            display_frame_count = base.max(current_frame + 1).max(1);
            self.manga_anim_seekbar_total_frames
                .insert(gif_idx, display_frame_count);
        }
        let position_fraction = if is_streaming {
            if display_frame_count > 1 {
                current_frame as f32 / (display_frame_count - 1) as f32
            } else {
                0.0
            }
        } else {
            img.position_fraction() as f32
        };
        let animated_label = Self::animated_image_label_for_path(self.image_list.get(gif_idx));
        let show_fps_controls = self
            .image_list
            .get(gif_idx)
            .is_some_and(|path| Self::path_uses_animated_fps_override(path.as_path()))
            || Self::path_uses_animated_fps_override(img.path.as_path());
        let animated_media_path = img.path.clone();
        let mut file_navigation_requested: Option<VideoFileNavigation> = None;

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
            let seek_bar_height = 6.0;
            let available_width = ui.available_width();
            let (seek_rect, seek_response) = ui.allocate_exact_size(
                egui::Vec2::new(available_width, seek_bar_height + 8.0),
                egui::Sense::click_and_drag(),
            );

            let bar_inner = egui::Rect::from_min_size(
                egui::pos2(
                    seek_rect.min.x,
                    seek_rect.center().y - seek_bar_height / 2.0,
                ),
                egui::Vec2::new(seek_rect.width(), seek_bar_height),
            );

            // Background bar
            ui.painter()
                .rect_filled(bar_inner, 3.0, egui::Color32::from_gray(60));

            // Progress bar
            let display_fraction = if self.gif_seeking {
                self.gif_seek_preview_frame
                    .map(|f| f as f32 / (display_frame_count - 1).max(1) as f32)
                    .unwrap_or(position_fraction)
            } else {
                position_fraction
            };
            let progress_width = bar_inner.width() * display_fraction;
            if progress_width > 0.0 {
                let progress_rect = egui::Rect::from_min_size(
                    bar_inner.min,
                    egui::Vec2::new(progress_width, seek_bar_height),
                );
                ui.painter()
                    .rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(76, 175, 80));
                // Green for GIF
            }

            // Seek handle
            let handle_x = bar_inner.min.x + progress_width;
            let handle_center = egui::pos2(handle_x, bar_inner.center().y);
            let handle_radius = if seek_response.hovered() || seek_response.dragged() {
                8.0
            } else {
                6.0
            };
            ui.painter()
                .circle_filled(handle_center, handle_radius, egui::Color32::WHITE);

            // Handle seeking
            let primary_down = ctx.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let primary_released =
                ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

            if seek_response.is_pointer_button_down_on() && !self.gif_seeking {
                self.gif_seeking = true;
            }

            if self.gif_seeking && primary_down {
                if let Some(pos) = seek_response
                    .interact_pointer_pos()
                    .or_else(|| ctx.input(|i| i.pointer.hover_pos()))
                {
                    let seek_fraction =
                        ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    self.gif_seek_preview_frame = Some(target_frame);

                    // Update the actual frame
                    if let Some(img) = self.manga_animated_images.get_mut(&gif_idx) {
                        img.set_frame(target_frame);
                    }
                    // Force texture update
                    self.manga_texture_cache.remove(gif_idx);
                }
                ctx.request_repaint();
            }

            if seek_response.clicked() && !self.gif_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction =
                        ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let target_frame = ((frame_count - 1) as f32 * seek_fraction).round() as usize;
                    if let Some(img) = self.manga_animated_images.get_mut(&gif_idx) {
                        img.set_frame(target_frame);
                    }
                    self.manga_texture_cache.remove(gif_idx);
                    ctx.request_repaint();
                }
            }

            if self.gif_seeking && (primary_released || !primary_down) {
                self.gif_seeking = false;
                self.gif_seek_preview_frame = None;
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                // Play/Pause button
                let play_btn = ui.add(
                    egui::Button::new(if self.gif_paused { "▶" } else { "⏸" })
                        .min_size(egui::vec2(32.0, 24.0)),
                );

                if play_btn.clicked() {
                    self.gif_paused = !self.gif_paused;
                }

                ui.add_space(4.0);

                let prev_btn = Self::video_control_icon_button(
                    ui,
                    VideoControlIcon::Previous,
                    self.navigation_tooltip_previous(),
                    None,
                    false,
                );
                if prev_btn.clicked() {
                    file_navigation_requested = Some(VideoFileNavigation::Previous);
                }

                let next_btn = Self::video_control_icon_button(
                    ui,
                    VideoControlIcon::Next,
                    self.navigation_tooltip_next(),
                    None,
                    false,
                );
                if next_btn.clicked() {
                    file_navigation_requested = Some(VideoFileNavigation::Next);
                }

                ui.add_space(8.0);

                // Frame display
                let duration_secs = total_duration_ms as f64 / 1000.0;
                let current_time = (position_fraction as f64 * duration_secs).max(0.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Frame {}/{} ({:.1}s / {:.1}s)",
                        current_frame + 1,
                        frame_count,
                        current_time,
                        duration_secs
                    ))
                    .color(egui::Color32::WHITE)
                    .size(12.0),
                );

                // Animated image indicator on right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(animated_label)
                            .color(egui::Color32::from_rgb(76, 175, 80))
                            .size(14.0),
                    );
                    if show_fps_controls {
                        ui.add_space(2.0);
                        self.draw_webp_fps_controls(
                            ui,
                            ctx,
                            Self::manga_webp_fps_popup_id(gif_idx),
                            animated_media_path.as_path(),
                            frame_count,
                            total_duration_ms as u64,
                        );
                    }
                });
            });
        });

        if let Some(navigation) = file_navigation_requested {
            match navigation {
                VideoFileNavigation::Previous => self.navigate_prev_for_video_mode(),
                VideoFileNavigation::Next => self.navigate_next_for_video_mode(),
            }
        }
    }

    /// Determine resize direction based on mouse position
    pub(crate) fn get_resize_direction(
        &self,
        pos: egui::Pos2,
        rect: egui::Rect,
    ) -> ResizeDirection {
        let border = self.config.resize_border_size;
        // Top-right corner competes with the title-bar button cluster; give it a
        // slightly larger hotspot so resize remains reachable in floating mode.
        let top_right_corner_border = border.max(12.0);
        if pos.x > rect.max.x - top_right_corner_border
            && pos.y < rect.min.y + top_right_corner_border
        {
            return ResizeDirection::TopRight;
        }
        let at_left = pos.x < rect.min.x + border;
        let at_right = pos.x > rect.max.x - border;
        let at_top = pos.y < rect.min.y + border;
        let at_bottom = pos.y > rect.max.y - border;

        match (at_left, at_right, at_top, at_bottom) {
            (true, false, true, false) => ResizeDirection::TopLeft,
            (false, true, true, false) => ResizeDirection::TopRight,
            (true, false, false, true) => ResizeDirection::BottomLeft,
            (false, true, false, true) => ResizeDirection::BottomRight,
            (true, false, false, false) => ResizeDirection::Left,
            (false, true, false, false) => ResizeDirection::Right,
            (false, false, true, false) => ResizeDirection::Top,
            (false, false, false, true) => ResizeDirection::Bottom,
            _ => ResizeDirection::None,
        }
    }
}

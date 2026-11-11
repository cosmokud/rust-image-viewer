//! Seek gesture mode selection.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn active_seek_pointer_fraction(
        ctx: &egui::Context,
        bar_inner: egui::Rect,
    ) -> Option<f32> {
        ctx.input(|i| {
            i.pointer
                .interact_pos()
                .or_else(|| i.pointer.latest_pos())
                .map(|pos| ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0))
        })
    }

    pub(crate) fn drag_seek_delta_seconds(
        duration: Option<Duration>,
        current_fraction: f32,
        last_fraction: f32,
    ) -> f64 {
        duration
            .map(|dur| dur.as_secs_f64() * (current_fraction - last_fraction).abs() as f64)
            .unwrap_or_else(|| (current_fraction - last_fraction).abs() as f64)
    }

    pub(crate) fn should_emit_drag_seek(
        duration: Option<Duration>,
        current_fraction: f32,
        last_requested_fraction: Option<f32>,
        last_seek_sent_at: Instant,
    ) -> bool {
        const MIN_DRAG_SEEK_DELTA_FRACTION: f32 = 0.001;
        const DRAG_SEEK_INTERVAL_MS: u64 = 24;
        const FORCE_DRAG_SEEK_DELTA_SECONDS: f64 = 2.0;

        let Some(last_requested_fraction) = last_requested_fraction else {
            return true;
        };

        let fraction_delta = (current_fraction - last_requested_fraction).abs();
        if fraction_delta < MIN_DRAG_SEEK_DELTA_FRACTION {
            return false;
        }

        last_seek_sent_at.elapsed() >= Duration::from_millis(DRAG_SEEK_INTERVAL_MS)
            || Self::drag_seek_delta_seconds(duration, current_fraction, last_requested_fraction)
                >= FORCE_DRAG_SEEK_DELTA_SECONDS
    }

    pub(crate) fn adaptive_drag_seek_mode_for_fraction(
        video_seek_policy: VideoSeekPolicy,
        duration: Option<Duration>,
        current_fraction: f32,
        last_requested_fraction: Option<f32>,
    ) -> VideoSeekMode {
        match video_seek_policy {
            VideoSeekPolicy::Accurate => VideoSeekMode::Accurate,
            VideoSeekPolicy::Keyframe => VideoSeekMode::Keyframe,
            VideoSeekPolicy::Adaptive => {
                const ADAPTIVE_ACCURATE_DELTA_SECONDS: f64 = 1.0;

                let delta_seconds = last_requested_fraction
                    .map(|last| Self::drag_seek_delta_seconds(duration, current_fraction, last))
                    .unwrap_or(f64::INFINITY);

                if delta_seconds <= ADAPTIVE_ACCURATE_DELTA_SECONDS {
                    VideoSeekMode::Accurate
                } else {
                    VideoSeekMode::Keyframe
                }
            }
        }
    }

    /// Draw video seekbar and controls (called from draw_video_controls)
    pub(crate) fn draw_video_seekbar_inner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let video_seek_policy = self.config.video_seek_policy;
        let commit_seek_mode = self.commit_video_seek_mode();
        let mut play_toggle_requested = false;
        let mut file_navigation_requested: Option<VideoFileNavigation> = None;
        let mut audio_track_requested: Option<i32> = None;
        let mut subtitle_selection_requested: Option<VideoSubtitleSelection> = None;
        let resume_error: Option<String> = None;

        ui.vertical(|ui| {
            // === Seek bar (top row) ===
            let current_video_path = self.image_list.get(self.current_index).cloned();
            let Some(player) = self.video_player.as_mut() else {
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

            // Seek bar
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

            // Progress bar (freeze display while dragging to avoid flicker)
            let display_fraction = if self.is_seeking {
                self.seek_preview_fraction.unwrap_or(position_fraction)
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

            // If the pointer is held down on the seek bar, enter seeking mode immediately.
            // This ensures "click-and-hold without moving" pauses playback.
            if seek_response.is_pointer_button_down_on() && !self.is_seeking {
                self.is_seeking = true;
                self.seek_was_playing = player.is_playing();
                if self.seek_was_playing {
                    player.pause_async();
                }
                self.seek_last_requested_fraction = None;
                // Allow an immediate seek on the first frame of interaction.
                self.last_seek_sent_at = Instant::now() - Duration::from_millis(1000);
            }

            // While the button is held and we're in seeking mode, update preview and seek.
            if self.is_seeking && primary_down {
                if let Some(seek_fraction) = Self::active_seek_pointer_fraction(ctx, bar_inner) {
                    self.seek_preview_fraction = Some(seek_fraction);

                    if Self::should_emit_drag_seek(
                        duration,
                        seek_fraction,
                        self.seek_last_requested_fraction,
                        self.last_seek_sent_at,
                    ) {
                        let effective_drag_seek_mode = Self::adaptive_drag_seek_mode_for_fraction(
                            video_seek_policy,
                            duration,
                            seek_fraction,
                            self.seek_last_requested_fraction,
                        );
                        let _ =
                            player.seek_with_mode(seek_fraction as f64, effective_drag_seek_mode);
                        self.seek_last_requested_fraction = Some(seek_fraction);
                        self.last_seek_sent_at = Instant::now();
                    }
                }
                ctx.request_repaint();
            }

            // Single-click seek (no hold): seek immediately, don't change play state.
            if seek_response.clicked() && !self.is_seeking {
                if let Some(pos) = seek_response.interact_pointer_pos() {
                    let seek_fraction =
                        ((pos.x - bar_inner.min.x) / bar_inner.width()).clamp(0.0, 1.0);
                    let _ = player.seek_with_mode(seek_fraction as f64, commit_seek_mode);
                    ctx.request_repaint();
                }
            }

            // On mouse release, finalize seek and restore prior play state.
            if self.is_seeking && (primary_released || !primary_down) {
                let final_fraction = Self::active_seek_pointer_fraction(ctx, bar_inner)
                    .or(self.seek_preview_fraction)
                    .or(self.seek_last_requested_fraction);
                if let Some(final_fraction) = final_fraction {
                    let _ = player.seek_with_mode(final_fraction as f64, commit_seek_mode);
                }
                self.is_seeking = false;
                self.seek_preview_fraction = None;
                self.seek_last_requested_fraction = None;
                self.last_seek_sent_at = Instant::now();

                if self.seek_was_playing {
                    player.play_async();
                }
                self.seek_was_playing = false;
                ctx.request_repaint();
            }

            ui.add_space(4.0);

            // === Bottom row: controls ===
            ui.horizontal(|ui| {
                let Some(player) = self.video_player.as_mut() else {
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

                // Spacer
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let Some(player) = self.video_player.as_mut() else {
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
                        self.config
                            .update_video_state(player.is_muted(), player.volume());
                        self.pending_idle_config_sync = true;
                    }

                    // Volume slider
                    let volume = player.volume() as f32;
                    if !self.volume_slider_visual.is_finite() {
                        self.volume_slider_visual = volume;
                    }
                    if self.is_volume_dragging {
                        self.volume_slider_visual = volume;
                    } else {
                        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
                        let blend = (1.0 - (-14.0 * dt).exp()).clamp(0.0, 1.0);
                        self.volume_slider_visual += (volume - self.volume_slider_visual) * blend;
                        if (volume - self.volume_slider_visual).abs() < 0.0005 {
                            self.volume_slider_visual = volume;
                        } else {
                            ctx.request_repaint();
                        }
                    }
                    let volume_visual = self.volume_slider_visual.clamp(0.0, 1.0);
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

                    // Volume background
                    ui.painter()
                        .rect_filled(vol_bar, 2.0, egui::Color32::from_gray(60));

                    // Volume level
                    let vol_width = vol_bar.width() * volume_visual;
                    if vol_width > 0.0 {
                        let vol_progress = egui::Rect::from_min_size(
                            vol_bar.min,
                            egui::Vec2::new(vol_width, vol_slider_height),
                        );
                        ui.painter()
                            .rect_filled(vol_progress, 2.0, egui::Color32::WHITE);
                    }

                    // Volume handle
                    let vol_handle_x = vol_bar.min.x + vol_width;
                    let vol_handle_center = egui::pos2(vol_handle_x, vol_bar.center().y);
                    ui.painter()
                        .circle_filled(vol_handle_center, 5.0, egui::Color32::WHITE);

                    // Handle volume changes
                    if vol_response.dragged() || vol_response.clicked() {
                        self.is_volume_dragging = true;
                        if let Some(pos) = vol_response.interact_pointer_pos() {
                            let new_vol =
                                ((pos.x - vol_bar.min.x) / vol_bar.width()).clamp(0.0, 1.0);
                            player.set_volume(new_vol as f64);
                            // Unmute when adjusting volume
                            if player.is_muted() && new_vol > 0.0 {
                                player.set_muted(false);
                            }
                            self.config
                                .update_video_state(player.is_muted(), player.volume());
                            self.pending_idle_config_sync = true;
                            self.volume_slider_visual = new_vol;
                        }
                    }
                    if vol_response.hovered() {
                        const VOLUME_WHEEL_POINTS_PER_STEP: f32 = 50.0;
                        const VOLUME_WHEEL_MAX_STEPS_PER_EVENT: f32 = 3.0;
                        let wheel_steps = ctx.input(|i| {
                            i.raw
                                .events
                                .iter()
                                .filter_map(|event| {
                                    let egui::Event::MouseWheel { unit, delta, .. } = event else {
                                        return None;
                                    };
                                    if !delta.y.is_finite() || delta.y == 0.0 {
                                        return None;
                                    }
                                    let steps = match unit {
                                        egui::MouseWheelUnit::Line => delta.y,
                                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                                        egui::MouseWheelUnit::Point => {
                                            delta.y / VOLUME_WHEEL_POINTS_PER_STEP
                                        }
                                    };
                                    Some(steps.clamp(
                                        -VOLUME_WHEEL_MAX_STEPS_PER_EVENT,
                                        VOLUME_WHEEL_MAX_STEPS_PER_EVENT,
                                    ))
                                })
                                .sum::<f32>()
                        });
                        if wheel_steps != 0.0 {
                            let current_volume = player.volume();
                            let step = 0.05f64;
                            let next_volume =
                                (current_volume + wheel_steps as f64 * step).clamp(0.0, 1.0);
                            if (next_volume - current_volume).abs() > f64::EPSILON {
                                player.set_volume(next_volume);
                                if player.is_muted() && next_volume > 0.0 {
                                    player.set_muted(false);
                                }
                                self.config
                                    .update_video_state(player.is_muted(), player.volume());
                                self.pending_idle_config_sync = true;
                                self.arm_media_slider_wheel_guard();
                                ctx.request_repaint();
                            }
                        }
                    }
                    if vol_response.drag_stopped() || (self.is_volume_dragging && !primary_down) {
                        self.is_volume_dragging = false;
                    }

                    ui.add_space(6.0);

                    let subtitle_popup_id = Self::solo_video_subtitle_popup_id();
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

                    let audio_popup_id = Self::solo_video_audio_popup_id();
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
            self.set_video_playback_unavailable_runtime(err);
        }

        if play_toggle_requested {
            self.try_toggle_solo_video_play_pause();
        }

        if let Some(selected_track) = audio_track_requested {
            self.queue_solo_audio_track_switch(ctx, selected_track);
        }

        if let Some(selection) = subtitle_selection_requested {
            if let Some(player) = self.video_player.as_mut() {
                if let Err(err) = player.set_subtitle_selection(selection) {
                    tracing::warn!("failed to switch subtitle track: {}", err);
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
}

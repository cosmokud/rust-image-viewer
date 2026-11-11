//! Video playback availability state and its popup.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn gstreamer_missing_video_error_text() -> &'static str {
        GSTREAMER_MISSING_VIDEO_ERROR_TEXT
    }

    pub(crate) fn is_video_playback_unavailable_active(&self) -> bool {
        if !matches!(self.current_media_type, Some(MediaType::Video)) {
            return false;
        }

        if self.video_player.is_some() || self.video_playback_unavailable_reason.is_none() {
            return false;
        }

        self.pending_media_load
            .as_ref()
            .map_or(true, |pending| pending.kind != PendingMediaLoadKind::Video)
    }

    pub(crate) fn is_video_playback_preview_mode(&self) -> bool {
        self.is_video_playback_unavailable_active() && self.video_texture.is_some()
    }

    pub(crate) fn set_video_playback_unavailable_for_path(
        &mut self,
        path: &PathBuf,
        reason: String,
    ) {
        if let Some(player) = &self.video_player {
            if let Some(path) = &self.current_video_path {
                // Note: Use your actual method for fetching position, e.g., player.position_secs()
                // Assuming it returns an f64 representing seconds:
                if let Some(current_pos) = player.position() {
                    self.manga_video_preview_resume_by_path
                        .insert(path.clone(), current_pos.as_secs_f64());
                }
            }
        }
        if let Some(player) = &self.video_player {
            if let Some(path) = &self.current_video_path {
                // Note: Use your actual method for fetching position, e.g., player.position_secs()
                // Assuming it returns an f64 representing seconds:
                if let Some(current_pos) = player.position() {
                    self.manga_video_preview_resume_by_path
                        .insert(path.clone(), current_pos.as_secs_f64());
                }
            }
        }
        self.video_player = None;
        self.current_video_path = Some(path.clone());
        self.pending_media_layout = false;
        let normalized_reason = if !gstreamer_runtime_available() {
            Self::gstreamer_missing_video_error_text().to_string()
        } else {
            reason
        };
        self.video_playback_unavailable_reason = Some(normalized_reason);

        let target_side = self.solo_target_texture_side_for_path(path, MediaType::Video, false);
        let has_pending_for_path = self
            .pending_video_thumbnail_placeholder
            .as_ref()
            .map_or(false, |pending| pending.path == *path);

        if !has_pending_for_path && self.video_texture.is_none() {
            if let Some(thumbnail) = extract_video_first_frame_thumbnail(path, target_side) {
                self.pending_video_thumbnail_placeholder = Some(PendingVideoThumbnailPlaceholder {
                    path: path.clone(),
                    thumbnail,
                });
            }
        }

        if matches!(self.current_media_type, Some(MediaType::Video))
            && self
                .image_list
                .get(self.current_index)
                .is_some_and(|current| current == path)
        {
            self.queue_video_playback_unavailable_popup();
        }
    }

    pub(crate) fn set_video_playback_unavailable_runtime(&mut self, reason: String) {
        if let Some(path) = self.image_list.get(self.current_index).cloned() {
            self.set_video_playback_unavailable_for_path(&path, reason);
        } else {
            if let Some(player) = &mut self.video_player {
                if let Some(path) = &self.current_video_path {
                    // Grab the exact frame as a Duration, then convert to f64 seconds
                    if let Some(current_pos) = player.position() {
                        self.manga_video_preview_resume_by_path
                            .insert(path.clone(), current_pos.as_secs_f64());
                    }
                }
            }
            self.video_player = None;
            self.pending_media_layout = false;
            self.video_playback_unavailable_reason = Some(reason);
        }

        self.show_video_controls = true;
        self.touch_bottom_overlays();
        self.queue_video_playback_unavailable_popup();
    }

    pub(crate) fn queue_video_playback_unavailable_popup(&mut self) {
        if self.is_video_playback_unavailable_active() {
            self.video_playback_popup_until = Some(Instant::now() + Duration::from_secs(4));
        }
    }

    pub(crate) fn active_video_playback_popup_seconds(&mut self) -> Option<f32> {
        let Some(until) = self.video_playback_popup_until else {
            return None;
        };

        let now = Instant::now();
        if now >= until {
            self.video_playback_popup_until = None;
            return None;
        }

        Some(until.saturating_duration_since(now).as_secs_f32())
    }

    pub(crate) fn video_playback_unavailable_popup_detail(&self) -> String {
        if !gstreamer_runtime_available() {
            return Self::gstreamer_missing_video_error_text().to_string();
        }

        let detail = self
            .video_playback_unavailable_reason
            .as_deref()
            .unwrap_or("GStreamer runtime is unavailable.");
        let first_line = detail.lines().next().unwrap_or(detail).trim();

        const MAX_CHARS: usize = 160;
        if first_line.chars().count() <= MAX_CHARS {
            return first_line.to_string();
        }

        let trimmed: String = first_line.chars().take(MAX_CHARS).collect();
        format!("{}...", trimmed)
    }

    pub(crate) fn paint_video_playback_unavailable_popup(
        &self,
        painter: &egui::Painter,
        frame_rect: egui::Rect,
        remaining_seconds: f32,
    ) {
        let fade = (remaining_seconds / 0.35).clamp(0.0, 1.0);
        let max_rect = frame_rect.shrink2(egui::vec2(16.0, 16.0));
        let panel_width = (frame_rect.width() * 0.82)
            .clamp(340.0, 760.0)
            .min(max_rect.width());
        let text_width = (panel_width - 36.0).max(180.0);

        let title_text = "Playback unavailable";
        let detail_text = self.video_playback_unavailable_popup_detail();
        let footer_text = "Preview mode stays active: zoom, pan, and browsing still work.";

        let title_color =
            egui::Color32::from_rgba_unmultiplied(255, 196, 150, (255.0 * fade) as u8);
        let detail_color =
            egui::Color32::from_rgba_unmultiplied(240, 230, 220, (245.0 * fade) as u8);
        let footer_color =
            egui::Color32::from_rgba_unmultiplied(170, 204, 238, (240.0 * fade) as u8);

        let title_galley = painter.layout_no_wrap(
            title_text.to_owned(),
            egui::FontId::proportional(22.0),
            title_color,
        );
        let detail_galley = painter.layout(
            detail_text,
            egui::FontId::proportional(15.0),
            detail_color,
            text_width,
        );
        let footer_galley = painter.layout(
            footer_text.to_owned(),
            egui::FontId::proportional(13.0),
            footer_color,
            text_width,
        );

        let title_height = title_galley.rect.height();
        let detail_height = detail_galley.rect.height();
        let footer_height = footer_galley.rect.height();

        let panel_height =
            (14.0 + title_height + 8.0 + detail_height + 10.0 + footer_height + 14.0)
                .clamp(108.0, max_rect.height());
        let panel_rect =
            egui::Rect::from_center_size(max_rect.center(), egui::vec2(panel_width, panel_height))
                .intersect(max_rect);

        painter.rect_filled(
            panel_rect,
            14.0,
            egui::Color32::from_rgba_unmultiplied(12, 18, 24, (220.0 * fade) as u8),
        );
        painter.rect_stroke(
            panel_rect,
            14.0,
            egui::Stroke::new(
                1.4_f32,
                egui::Color32::from_rgba_unmultiplied(252, 127, 38, (235.0 * fade) as u8),
            ),
        );

        let text_left = panel_rect.left() + 18.0;
        let mut y = panel_rect.top() + 14.0;
        painter.galley(egui::pos2(text_left, y), title_galley, title_color);
        y += title_height + 8.0;
        painter.galley(egui::pos2(text_left, y), detail_galley, detail_color);
        y += detail_height + 10.0;
        painter.galley(egui::pos2(text_left, y), footer_galley, footer_color);
    }

    pub(crate) fn try_toggle_solo_video_play_pause(&mut self) {
        // Enqueues only: a wedged decoder used to freeze this click for the full 5 s timeout.
        // A real failure arrives through `take_control_error` and still raises the popup.
        if let Some(player) = self.video_player.as_mut() {
            player.toggle_play_pause_async();
        }

        if self.video_player.is_none() && self.is_video_playback_unavailable_active() {
            self.queue_video_playback_unavailable_popup();
        }
    }

    pub(crate) fn try_toggle_manga_video_play_pause(&mut self, index: usize) {
        // Enqueues only; failure recovery happens in `manga_poll_video_control_errors`.
        if let Some(player) = self.manga_video_players.get_mut(&index) {
            player.toggle_play_pause_async();
        }
    }

    /// Async state changes report failures on the control thread. Apply the recovery the
    /// synchronous path used to (drop the player, name the reason) once per frame.
    pub(crate) fn manga_poll_video_control_errors(&mut self) {
        let failed: Vec<(usize, String)> = self
            .manga_video_players
            .iter_mut()
            .filter_map(|(&index, player)| player.take_control_error().map(|err| (index, err)))
            .collect();

        for (index, err) in failed {
            self.remove_manga_video_player(index);
            self.remove_manga_video_texture(index);
            self.manga_video_preview_resume_secs.remove(&index);
            self.manga_video_failed.insert(index, Instant::now());
            if self.manga_focused_video_index == Some(index) {
                self.manga_focused_video_index = None;
            }
            self.video_playback_unavailable_reason = Some(err);
            self.queue_video_playback_unavailable_popup();
        }
    }

    pub(crate) fn queue_solo_audio_track_switch(&mut self, ctx: &egui::Context, track_index: i32) {
        self.pending_solo_audio_track_switch = Some((
            Instant::now() + Self::AUDIO_TRACK_SWITCH_DELAY,
            self.current_index,
            track_index,
        ));
        ctx.request_repaint_after(Self::AUDIO_TRACK_SWITCH_DELAY);
    }

    pub(crate) fn queue_manga_audio_track_switch(
        &mut self,
        ctx: &egui::Context,
        video_idx: usize,
        track_index: i32,
    ) {
        self.pending_manga_audio_track_switches.insert(
            video_idx,
            (Instant::now() + Self::AUDIO_TRACK_SWITCH_DELAY, track_index),
        );
        ctx.request_repaint_after(Self::AUDIO_TRACK_SWITCH_DELAY);
    }

    pub(crate) fn poll_pending_audio_track_switches(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut next_repaint_after: Option<Duration> = None;

        match self.pending_solo_audio_track_switch {
            Some((_, media_index, _)) if media_index != self.current_index => {
                self.pending_solo_audio_track_switch = None;
            }
            Some((apply_at, _, track_index)) if now >= apply_at => {
                self.pending_solo_audio_track_switch = None;
                if let Some(player) = self.video_player.as_mut() {
                    if let Err(err) = player.set_audio_track(track_index) {
                        tracing::warn!("failed to switch audio track: {}", err);
                    }
                }
            }
            Some((apply_at, _, _)) => {
                next_repaint_after = Some(apply_at.saturating_duration_since(now));
            }
            None => {}
        }

        let mut ready_manga_switches = Vec::new();
        for (&video_idx, &(apply_at, track_index)) in &self.pending_manga_audio_track_switches {
            if now >= apply_at {
                ready_manga_switches.push((video_idx, track_index));
            } else {
                let remaining = apply_at.saturating_duration_since(now);
                next_repaint_after = Some(match next_repaint_after {
                    Some(current) => current.min(remaining),
                    None => remaining,
                });
            }
        }

        for (video_idx, track_index) in ready_manga_switches {
            self.pending_manga_audio_track_switches.remove(&video_idx);
            if let Some(player) = self.manga_video_players.get_mut(&video_idx) {
                if let Err(err) = player.set_audio_track(track_index) {
                    tracing::warn!("failed to switch manga audio track: {}", err);
                }
            }
        }

        if let Some(delay) = next_repaint_after {
            ctx.request_repaint_after(delay);
        }
    }
}

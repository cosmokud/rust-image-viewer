//! Preview-tracked visibility and hidden video pruning.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn video_resume_seek_needed(current: Option<Duration>, target: Duration) -> bool {
        let target_secs = target.as_secs_f64();
        if !target_secs.is_finite() || target_secs <= Self::VIDEO_RESUME_MIN_SECONDS {
            return false;
        }

        let Some(current_secs) = current
            .map(|position| position.as_secs_f64())
            .filter(|secs| secs.is_finite())
        else {
            return true;
        };

        (current_secs - target_secs).abs() > Self::VIDEO_RESUME_SEEK_EPSILON_SECONDS
    }

    pub(crate) fn manga_strip_retain_only_focused_video(&mut self, focused_idx: usize) {
        if self.mode().is_masonry() {
            return;
        }

        let stale_indices: Vec<usize> = self
            .manga_video_players
            .keys()
            .copied()
            .filter(|&idx| idx != focused_idx)
            .collect();

        for idx in stale_indices {
            let record_position = self.manga_video_player_matches(idx);
            if let Some(mut player) = self.remove_manga_video_player(idx) {
                if record_position {
                    if let Some(position) = Self::latest_video_position_from_player(&mut player) {
                        self.manga_record_video_preview_resume_secs(idx, position);
                    }
                }
                if player.is_playing() {
                    player.pause_async();
                }
            }
            self.remove_manga_video_texture(idx);
        }
    }

    pub(crate) fn manga_strip_focused_video_playing(&self) -> bool {
        if self.mode().is_solo() || self.mode().is_masonry() {
            return false;
        }

        let Some(focused_idx) = self.manga_focused_video_index else {
            return false;
        };

        self.manga_video_players
            .get(&focused_idx)
            .is_some_and(|player| player.is_playing())
    }

    /// Update manga video playback based on current scroll position.
    /// Ensures only one video plays at a time (the focused one).
    pub(crate) fn manga_update_video_focus(&mut self) {
        self.manga_poll_video_control_errors();
        if self.mode().is_solo() || self.image_list.is_empty() {
            self.clear_pending_manga_video_load();
            return;
        }

        if !gstreamer_runtime_available() {
            if self.manga_focused_video_index.is_some() {
                for player in self.manga_video_players.values_mut() {
                    if player.is_playing() {
                        player.pause_async();
                    }
                }
            }
            self.manga_focused_video_index = None;
            self.clear_pending_manga_video_load();
            self.video_playback_unavailable_reason =
                Some(Self::gstreamer_missing_video_error_text().to_string());
            return;
        }

        let visible_indices_for_preview = self.manga_prune_hidden_video_preview_state();
        self.prune_manga_video_failed(visible_indices_for_preview.as_ref());

        let focused_idx = if self.mode().is_masonry() {
            if let Some(hovered_idx) = self.manga_hovered_media_index {
                hovered_idx
            } else {
                // In masonry mode, autoplay follows hover only.
                // If nothing is hovered, pause any active video.
                if self.manga_focused_video_index.is_some() {
                    for player in self.manga_video_players.values_mut() {
                        if player.is_playing() {
                            player.pause_async();
                        }
                    }
                    self.manga_focused_video_index = None;
                }
                self.clear_pending_manga_video_load();
                return;
            }
        } else {
            self.manga_get_focused_media_index()
        };

        // Check if the focused item is a video
        let focused_is_video = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_media_type(focused_idx))
            .map_or(false, |mt| mt == MangaMediaType::Video);

        // Also check by file extension as a fallback
        let focused_is_video = focused_is_video
            || self
                .image_list
                .get(focused_idx)
                .map_or(false, |p| is_supported_video(p));

        let muted_override = self.manga_video_user_muted;
        let volume_override = self.manga_video_user_volume;
        let muted = muted_override.unwrap_or(if self.config.video_muted_remember {
            self.config.state_muted
        } else {
            self.config.video_muted_by_default
        });
        let volume = volume_override.unwrap_or(if self.config.video_volume_remember {
            self.config.state_volume
        } else {
            self.config.video_default_volume
        });

        if focused_is_video {
            if self.manga_video_failed.contains_key(&focused_idx) {
                self.manga_focused_video_index = None;
                self.clear_pending_manga_video_load();
                return;
            }

            let focus_changed = self.manga_focused_video_index != Some(focused_idx);
            let resume_position = self.manga_resume_position_for_index(focused_idx);
            if self.manga_video_players.contains_key(&focused_idx)
                && !self.manga_video_player_matches(focused_idx)
            {
                self.remove_manga_video_player(focused_idx);
                self.remove_manga_video_texture(focused_idx);
            }

            if focus_changed {
                // Pause all other videos
                for (&idx, player) in self.manga_video_players.iter_mut() {
                    if idx != focused_idx && player.is_playing() {
                        player.pause_async();
                    }
                }
            }
            self.manga_strip_retain_only_focused_video(focused_idx);

            let mut play_refused = false;
            if let Some(player) = self.manga_video_players.get_mut(&focused_idx) {
                if focus_changed {
                    Self::seek_video_player_to_resume_position(player, resume_position);
                }

                // Enqueues only; a failed state change surfaces via the control-error poll.
                if focus_changed && !player.is_playing() && !player.play_async() {
                    // Refused: keep focus unlatched so the next frame retries instead of
                    // leaving this tile unable to play.
                    play_refused = true;
                    if self.last_video_play_refused_log.map_or(true, |at| {
                        at.elapsed() >= Self::MANGA_VIDEO_PLAY_REFUSED_LOG_INTERVAL
                    }) {
                        self.last_video_play_refused_log = Some(Instant::now());
                        crate::diag::trace(
                            "video-control",
                            "manga video control queue refused play; retrying next frame",
                        );
                    }
                }
            }

            if let Some(player) = self.manga_video_players.get_mut(&focused_idx) {
                if focus_changed {
                    // Apply user's persisted mute/volume settings when switching focus.
                    Self::apply_video_audio_overrides(player, muted_override, volume_override);
                }

                if Self::manga_video_focus_should_latch(play_refused) {
                    self.manga_focused_video_index = Some(focused_idx);
                }
                if !self.manga_video_load_pending_for_index(focused_idx) {
                    self.clear_pending_manga_video_load();
                }

                // Evict video players that are far from view.
                self.manga_evict_distant_video_players(
                    focused_idx,
                    visible_indices_for_preview.as_ref(),
                );
                return;
            }

            self.manga_focused_video_index = Some(focused_idx);

            let Some(path) = self.image_list.get(focused_idx).cloned() else {
                self.manga_focused_video_index = None;
                self.clear_pending_manga_video_load();
                return;
            };

            // Stale gate: a coordinator/gst wedge can leave the request pending forever, so an
            // expired one is abandoned here and resubmitted in the same frame. The late result is
            // discarded by the request_id/index/path check in poll_pending_manga_video_load.
            if self
                .pending_manga_video_load
                .as_ref()
                .is_some_and(|pending| {
                    pending.index == focused_idx
                        && Self::manga_video_load_expired(pending.started_at, Instant::now())
                })
            {
                self.clear_pending_manga_video_load();
                self.perf_metrics.count(Metric::MangaVideoAsyncStale, 1);
                crate::diag::trace(
                    "video-control",
                    "stale manga focused video load expired; resubmitting",
                );
            }

            if !self.manga_video_load_pending_for_index(focused_idx) {
                // Ensure GStreamer is initialized before background startup work.
                self.gstreamer_initialized = true;
                self.start_async_manga_focused_video_load(
                    focused_idx,
                    path,
                    muted,
                    volume,
                    true,
                    false,
                );
            }

            return;
        } else {
            // Focused item is not a video - pause all videos
            if self.manga_focused_video_index.is_some() {
                for player in self.manga_video_players.values_mut() {
                    if player.is_playing() {
                        player.pause_async();
                    }
                }
                self.manga_focused_video_index = None;
            }

            if !self.mode().is_masonry() {
                self.clear_manga_video_players();
                self.clear_manga_video_textures();
            }

            self.clear_pending_manga_video_load();
        }
    }
}

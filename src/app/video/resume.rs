//! Video resume positions across modes.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn sync_manga_video_position_to_resume(&mut self, index: usize) {
        if !self.manga_video_player_matches(index) {
            return;
        }

        let position = self
            .manga_video_players
            .get_mut(&index)
            .and_then(Self::latest_video_position_from_player);

        if let Some(position) = position {
            self.manga_record_video_preview_resume_secs(index, position);
        }
    }

    pub(crate) fn sync_solo_video_position_to_manga_resume(&mut self) {
        if !matches!(self.current_media_type, Some(MediaType::Video)) {
            return;
        }

        let Some(current_path) = self
            .current_video_path
            .clone()
            .or_else(|| self.current_media_path())
        else {
            return;
        };
        let Some(index) = self
            .image_list
            .iter()
            .position(|path| path == &current_path)
        else {
            return;
        };

        let position = self
            .video_player
            .as_mut()
            .and_then(Self::latest_video_position_from_player);

        if let Some(position) = position {
            self.manga_record_video_preview_resume_secs(index, position);
        }
    }

    pub(crate) fn capture_solo_video_texture_for_manga_return(
        &self,
    ) -> Option<(PathBuf, egui::TextureHandle, u32, u32)> {
        if !matches!(self.current_media_type, Some(MediaType::Video)) {
            return None;
        }

        let current_path = self
            .current_video_path
            .clone()
            .or_else(|| self.current_media_path())?;
        let source_path = self
            .video_texture_source_path
            .clone()
            .unwrap_or_else(|| current_path.clone());
        if source_path != current_path {
            return None;
        }

        let texture = self.video_texture.as_ref()?.clone();
        let (width, height) = self.video_texture_dims?;
        if width == 0 || height == 0 {
            return None;
        }

        Some((source_path, texture, width, height))
    }

    pub(crate) fn install_manga_video_texture_for_path(
        &mut self,
        path: PathBuf,
        texture: egui::TextureHandle,
        width: u32,
        height: u32,
    ) {
        if width == 0 || height == 0 || !is_supported_video(&path) {
            return;
        }

        let Some(index) = self
            .image_list
            .iter()
            .position(|candidate| candidate == &path)
        else {
            return;
        };

        self.manga_video_textures
            .insert(index, (texture, width, height));
        self.manga_video_texture_paths.insert(index, path);
    }

    pub(crate) fn enter_manga_mode_from_preserved_strip_cache(&mut self) {
        let current_media_dims = self.media_display_dimensions().or(self.video_texture_dims);
        let current_media_type = self.current_media_type;
        let current_image_is_animated =
            self.current_image_is_animated_for_mode_switch(current_media_type);
        let current_path = self.current_media_path();

        self.prepare_enter_manga_mode_state(current_media_type);
        if self.mode().is_grid() {
            if let Some(snapshot) = self.strip_return_masonry_list_snapshot.clone() {
                self.set_image_list_raw(snapshot);
                if let Some(path) = current_path.as_ref() {
                    if let Some(idx) = self
                        .image_list
                        .iter()
                        .position(|candidate| candidate == path)
                    {
                        self.set_current_index_clamped(idx);
                    }
                }
            }
            self.restore_masonry_folder_metadata_snapshot();
            self.mark_manga_dimension_cache_current_if_complete();
        }
        let current_dims_changed = self.cache_current_media_dimensions_for_manga(
            current_media_dims,
            current_media_type,
            current_image_is_animated,
        );

        // The fullscreen traversal may have discovered new dimensions for the currently viewed file.
        // Rebuild strip layout with the latest metadata before restoring viewport position.
        if current_dims_changed {
            self.invalidate_manga_layout_cache();
        }
    }

    pub(crate) fn seek_video_player_to_resume_position(
        player: &mut VideoPlayer,
        resume_position: Option<Duration>,
    ) {
        let Some(position) = resume_position else {
            return;
        };

        if Self::video_resume_seek_needed(player.displayed_position(), position) {
            let _ = player.seek_to_time_with_mode(position.as_secs_f64(), VideoSeekMode::Accurate);
        }
    }

    pub(crate) fn manga_record_video_preview_resume_secs(
        &mut self,
        index: usize,
        position: Duration,
    ) {
        let secs = position.as_secs_f64();
        if !secs.is_finite() || secs < 0.0 {
            return;
        }

        if let Some(previous) = self.manga_video_preview_resume_secs.get(&index).copied() {
            // Guard against transient preroll/query-position drops to the start while preserving
            // normal forward playback progression.
            if secs <= Self::VIDEO_RESUME_MIN_SECONDS && previous > Self::VIDEO_RESUME_MIN_SECONDS {
                return;
            }
        }
        if let Some(previous) = self
            .image_list
            .get(index)
            .and_then(|path| self.manga_video_preview_resume_by_path.get(path))
            .copied()
        {
            if secs <= Self::VIDEO_RESUME_MIN_SECONDS && previous > Self::VIDEO_RESUME_MIN_SECONDS {
                return;
            }
        }

        self.manga_video_preview_resume_secs.insert(index, secs);
        if let Some(path) = self.image_list.get(index).cloned() {
            self.manga_video_preview_resume_by_path.insert(path, secs);
        }
    }

    pub(crate) fn manga_visible_indices_set_for_preview_tracking(&mut self) -> HashSet<usize> {
        if let Some(indices) = self.manga_collect_visible_indices_for_preview_tracking_rtree() {
            return indices;
        }

        self.manga_collect_visible_indices().into_iter().collect()
    }

    pub(crate) fn manga_resume_position_for_index(&self, index: usize) -> Option<Duration> {
        let secs = self
            .image_list
            .get(index)
            .and_then(|path| self.manga_video_preview_resume_by_path.get(path))
            .copied()
            .or_else(|| self.manga_video_preview_resume_secs.get(&index).copied());

        secs.and_then(|secs| {
            if secs.is_finite() && secs >= 0.0 {
                Some(Duration::from_secs_f64(secs))
            } else {
                None
            }
        })
    }

    pub(crate) fn manga_collect_visible_indices_for_preview_tracking_rtree(
        &mut self,
    ) -> Option<HashSet<usize>> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return Some(HashSet::new());
        }

        let viewport_top = self.manga_scroll_offset.max(0.0);
        let viewport_bottom = viewport_top + self.screen_size.y.max(1.0);

        if self.mode().is_masonry() {
            let zoom = self.zoom.max(0.0001);
            let viewport_left = -self.offset.x;
            let viewport_right = viewport_left + self.screen_size.x.max(1.0);
            let min_x = viewport_left / zoom;
            let max_x = viewport_right / zoom;
            let min_y = viewport_top / zoom;
            let max_y = viewport_bottom / zoom;
            let indices = self
                .manga_ensure_masonry_spatial_index()?
                .query_indices(min_x, min_y, max_x, max_y);
            return Some(indices.into_iter().collect());
        }

        let indices = self
            .manga_ensure_strip_spatial_index()?
            .query_vertical_band(viewport_top, viewport_bottom);
        Some(indices.into_iter().collect())
    }

    pub(crate) fn manga_prune_hidden_video_preview_state(&mut self) -> Option<HashSet<usize>> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }
        if self.mode().is_masonry() && self.manga_navigation_active_for_heavy_work() {
            return None;
        }

        let mut tracked_indices: HashSet<usize> = self
            .manga_video_preview_resume_secs
            .keys()
            .copied()
            .collect();
        tracked_indices.extend(self.manga_video_players.keys().copied());
        if tracked_indices.is_empty() {
            return None;
        }

        // Use the spatial index visible-set query (R-tree) so tiny scroll deltas
        // or transient layout jitter do not incorrectly reset preview state.
        let visible_indices = self.manga_visible_indices_set_for_preview_tracking();

        let hidden_indices: Vec<usize> = tracked_indices
            .into_iter()
            .filter(|idx| {
                !visible_indices.contains(idx)
                    || (self.manga_video_players.contains_key(idx)
                        && !self.manga_video_player_matches(*idx))
                    || (self.manga_video_texture_paths.contains_key(idx)
                        && !self.manga_video_texture_matches(*idx))
            })
            .collect();
        if hidden_indices.is_empty() {
            return Some(visible_indices);
        }

        let mut cleared_focused = false;
        for idx in hidden_indices {
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
            self.manga_video_preview_resume_secs.remove(&idx);
            self.remove_manga_video_texture(idx);
            if self
                .pending_manga_video_load
                .as_ref()
                .is_some_and(|pending| pending.index == idx)
            {
                self.clear_pending_manga_video_load();
            }
            if self.manga_focused_video_index == Some(idx) {
                cleared_focused = true;
            }
        }

        if cleared_focused {
            self.manga_focused_video_index = None;
        }

        Some(visible_indices)
    }
}

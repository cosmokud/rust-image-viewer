//! Animated (GIF/WebP) playback timing helpers.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn path_is_gif(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"))
    }

    pub(crate) fn path_uses_animated_fps_override(path: &Path) -> bool {
        Self::path_is_webp(path) || Self::path_is_gif(path)
    }

    pub(crate) fn animated_media_default_custom_fps(
        path: &Path,
        frame_count: usize,
        total_duration_ms: u64,
    ) -> u32 {
        if frame_count > 0 && total_duration_ms > 0 {
            let average_fps = ((frame_count as f64) * 1000.0 / total_duration_ms as f64).round();
            return (average_fps as u32).clamp(1, Self::ANIMATED_IMAGE_CUSTOM_MAX_FPS);
        }

        if Self::path_is_gif(path) {
            Self::ANIMATED_GIF_CUSTOM_DEFAULT_FPS
        } else {
            Self::ANIMATED_IMAGE_CUSTOM_DEFAULT_FPS
        }
    }

    pub(crate) fn sync_custom_fps_with_current_media_default(
        &mut self,
        path: &Path,
        frame_count: usize,
        total_duration_ms: u64,
    ) -> u32 {
        let default_fps =
            Self::animated_media_default_custom_fps(path, frame_count, total_duration_ms);
        let should_reset_for_new_media = self
            .webp_fps_custom_media_path
            .as_ref()
            .is_none_or(|prev| prev != path);

        if should_reset_for_new_media {
            self.webp_fps_custom_media_path = Some(path.to_path_buf());
            self.webp_custom_fps = default_fps;
            self.webp_custom_fps_input = default_fps.to_string();
            self.webp_fps_override = Some(default_fps);
            self.webp_show_custom_fps_slider = true;
        }

        default_fps
    }

    pub(crate) fn is_video_navigation_candidate_path(path: &Path) -> bool {
        if is_supported_video(path) || Self::path_is_gif(path) {
            return true;
        }

        if Self::path_is_webp(path) {
            return LoadedImage::is_animated_webp(path);
        }

        false
    }

    pub(crate) fn is_probably_animated_image_path(&mut self, path: &Path) -> bool {
        if Self::path_is_gif(path) {
            return true;
        }

        if !Self::path_is_webp(path) {
            return false;
        }

        let Some(stamp) = file_stamp_for_path(path) else {
            return false;
        };

        if let Some((cached_stamp, cached_is_animated)) = self.webp_animation_probe_cache.get(path)
        {
            if *cached_stamp == stamp {
                return *cached_is_animated;
            }
        }

        let is_animated = LoadedImage::is_animated_webp(path);
        self.webp_animation_probe_cache
            .insert(path.to_path_buf(), (stamp, is_animated));
        is_animated
    }

    pub(crate) fn current_image_is_animated_for_mode_switch(
        &mut self,
        current_media_type: Option<MediaType>,
    ) -> bool {
        if current_media_type != Some(MediaType::Image) {
            return false;
        }

        if let Some(path) = self.current_media_path() {
            if Self::path_is_gif(path.as_path()) || Self::path_is_webp(path.as_path()) {
                return self.is_probably_animated_image_path(path.as_path());
            }
        }

        self.image.as_ref().is_some_and(|img| img.is_animated())
    }

    pub(crate) fn current_fab_single_action_index(&self) -> Option<usize> {
        if self.mode().is_manga() || self.image_list.is_empty() {
            None
        } else {
            Some(
                self.current_index
                    .min(self.image_list.len().saturating_sub(1)),
            )
        }
    }
}

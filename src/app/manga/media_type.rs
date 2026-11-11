//! Manga media classification and dimension caching.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn cache_current_media_dimensions_for_manga(
        &mut self,
        current_media_dims: Option<(u32, u32)>,
        current_media_type: Option<MediaType>,
        current_image_is_animated: bool,
    ) -> bool {
        if self.mode().is_masonry() && self.masonry_authoritative_dimension_lock_active() {
            return false;
        }

        let (Some((w, h)), Some(media_type)) = (current_media_dims, current_media_type) else {
            return false;
        };

        let manga_media_type =
            Self::manga_media_type_for_current_media(media_type, current_image_is_animated);

        if let Some(ref mut loader) = self.manga_loader {
            let new_entry = (w, h, manga_media_type);

            if media_type == MediaType::Video {
                if let Some((cached_w, cached_h, MangaMediaType::Video)) =
                    loader.dimension_cache.get(&self.current_index).copied()
                {
                    let cached_pixels = cached_w as u64 * cached_h as u64;
                    let new_pixels = w as u64 * h as u64;
                    let cached_aspect = cached_w as f32 / cached_h.max(1) as f32;
                    let new_aspect = w as f32 / h.max(1) as f32;

                    if cached_w > 0
                        && cached_h > 0
                        && new_pixels < cached_pixels
                        && (cached_aspect - new_aspect).abs() <= 0.01
                    {
                        return false;
                    }
                }
            }

            let changed =
                loader.dimension_cache.get(&self.current_index).copied() != Some(new_entry);
            loader.dimension_cache.insert(self.current_index, new_entry);
            return changed;
        }

        false
    }

    pub(crate) fn prepare_enter_manga_mode_state(&mut self, current_media_type: Option<MediaType>) {
        // Strip is fullscreen-only: `Mode::Strip` reports `is_fullscreen()`, so the window must
        // already be fullscreen here. All three callers are gated on it; `ModeState` asserts it.
        self.set_strip_entry_placeholder_from_current_media(current_media_type);
        self.stop_manga_wheel_scroll();
        self.stop_manga_autoscroll();
        self.reset_gif_seek_interaction_state();
        if self.mode().layout() == MangaLayoutMode::Masonry {
            self.pause_masonry_metadata_preload();
        } else {
            self.reset_masonry_metadata_preload();
        }
        self.mode_transition(ModeEvent::EnterStrip);
        set_metadata_cache_enabled(true);
        self.stop_fullscreen_video_playback();
        self.reset_fullscreen_anim_stream_state();
        self.reset_manga_video_user_preferences();
        self.ensure_manga_loader();
    }
}

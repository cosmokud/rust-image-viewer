//! Focused media index queries.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Determine the focused animated image index in manga mode.
    /// Returns Some(idx) only if the focused media is an animated image.
    pub(crate) fn manga_get_focused_animated_index(&mut self) -> Option<usize> {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return None;
        }

        let focused_idx = if self.mode().is_masonry() {
            if Instant::now() < self.manga_hover_autoplay_resume_at {
                return None;
            }

            let Some(hovered_idx) = self.manga_hovered_media_index else {
                return None;
            };
            hovered_idx
        } else {
            self.manga_get_focused_media_index()
        };

        let focused_is_animated = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_media_type(focused_idx))
            .map_or(false, |mt| mt == MangaMediaType::AnimatedImage);

        let focused_path = self.image_list.get(focused_idx).cloned();
        let focused_by_path = focused_path
            .as_ref()
            .is_some_and(|path| self.is_probably_animated_image_path(path.as_path()))
            && !self.manga_anim_failed.contains(&focused_idx);
        let focused_is_animated = focused_is_animated || focused_by_path;

        if focused_is_animated && !self.manga_media_is_unloadable(focused_idx) {
            Some(focused_idx)
        } else {
            None
        }
    }
}

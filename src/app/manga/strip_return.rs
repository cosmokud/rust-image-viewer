//! Returning to strip mode from solo fullscreen.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn should_reuse_masonry_cache_on_return(&self, state: MasonryReturnState) -> bool {
        let current_signature = self
            .strip_return_masonry_list_snapshot
            .as_ref()
            .map(|files| Self::compute_image_list_signature(files))
            .unwrap_or(self.image_list_signature);

        !self.image_list.is_empty()
            && masonry_return_can_reuse_runtime_cache(
                state.list_signature,
                current_signature,
                self.strip_return_preserve_masonry_cache,
                self.manga_loader.is_some(),
            )
    }

    pub(crate) fn scroll_strip_to_current_index(&mut self) {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return;
        }

        let target_index = self
            .current_index
            .min(self.image_list.len().saturating_sub(1));
        self.set_current_index_clamped(target_index);
        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        let target_scroll = if self.mode().is_masonry() {
            self.masonry_scroll_offset_for_index_centered(target_index)
                .unwrap_or_else(|| {
                    self.manga_get_scroll_offset_for_index(target_index)
                        .clamp(0.0, max_scroll)
                })
        } else {
            self.manga_get_scroll_offset_for_index(target_index)
                .clamp(0.0, max_scroll)
        };

        self.manga_scroll_offset = target_scroll;
        self.manga_scroll_target = target_scroll;
        self.manga_scroll_velocity = 0.0;
        self.stop_manga_wheel_scroll();
    }

    pub(crate) fn manga_suspend_runtime_for_solo_fullscreen(&mut self) {
        if let Some(ref mut loader) = self.manga_loader {
            loader.cancel_all();
        }

        // Keep texture/dimension caches alive for fast strip restore,
        // but stop active runtime workloads while in solo fullscreen.
        if self.mode().layout() == MangaLayoutMode::Masonry {
            self.pause_masonry_metadata_preload();
        } else {
            self.reset_masonry_metadata_preload();
        }
        self.clear_manga_runtime_workloads();
        self.clear_manga_video_textures();
        self.manga_animated_images.clear();
        self.manga_anim_failed.clear();
        self.manga_anim_seekbar_total_frames.clear();
    }
}

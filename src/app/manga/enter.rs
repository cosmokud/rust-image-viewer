//! Entering manga mode from the preserved strip cache.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn return_to_strip_mode_from_fullscreen_toggle(&mut self) {
        let Some(layout_mode) = self.mode_state.strip_return_layout() else {
            return;
        };

        if !self.mode().is_fullscreen() || self.mode().is_manga() {
            return;
        }

        self.sync_current_index_to_visible_media();
        self.sync_solo_video_position_to_manga_resume();
        let return_video_texture = self.capture_solo_video_texture_for_manga_return();
        self.reset_gif_seek_interaction_state();
        self.strip_open_force_fit_path = None;

        if (layout_mode).is_grid() {
            if !self.refresh_media_list_before_masonry_entry() {
                self.clear_strip_return_context();
                return;
            }
        }

        let restore_masonry_state = if (layout_mode).is_grid() {
            self.strip_return_masonry_state
        } else {
            None
        };

        let current_viewed_path = self.current_media_path();
        if (layout_mode).is_grid() {
            if let (Some(state), Some(snapshot)) = (
                restore_masonry_state,
                self.strip_return_masonry_list_snapshot.clone(),
            ) {
                let snapshot_signature = Self::compute_image_list_signature(&snapshot);
                if state.list_signature == snapshot_signature {
                    self.set_image_list_raw(snapshot);
                    if let Some(path) = current_viewed_path.as_ref() {
                        if let Some(idx) = self
                            .image_list
                            .iter()
                            .position(|candidate| candidate == path)
                        {
                            self.set_current_index_clamped(idx);
                        }
                    }
                }
            }
        }

        let current_viewed_index = self
            .current_index
            .min(self.image_list.len().saturating_sub(1));
        self.set_current_index_clamped(current_viewed_index);
        let follow_current_viewed_item = restore_masonry_state.is_some_and(|state| {
            !masonry_return_should_restore_saved_scroll(
                state.opened_index,
                state.list_signature,
                current_viewed_index,
                self.image_list_signature,
            )
        });
        let reuse_masonry_cache = restore_masonry_state
            .is_some_and(|state| self.should_reuse_masonry_cache_on_return(state));
        let preserve_resident_masonry_cache =
            (layout_mode).is_grid() && self.has_resident_masonry_runtime_cache();
        let reuse_strip_cache = layout_mode == MangaLayoutMode::LongStrip;

        self.mode_state.set_layout(layout_mode);
        if (layout_mode).is_grid() {
            self.mark_masonry_runtime_cache_resident();
        }
        let reset_long_strip_on_return = self
            .mode()
            .resets_long_strip_on_exit(self.config.manga_long_strip_reset_fit_on_fullscreen_exit);

        if reuse_masonry_cache || preserve_resident_masonry_cache || reuse_strip_cache {
            self.enter_manga_mode_from_preserved_strip_cache();
        } else {
            if (layout_mode).is_grid() {
                self.clear_manga_runtime_cache(true);
            }
            self.toggle_manga_mode();
        }

        if reset_long_strip_on_return && self.mode().is_manga() {
            self.reset_long_strip_view_to_current_media();
            self.manga_update_current_index();
            self.manga_update_preload_queue();
            self.pending_masonry_solo_reentry = None;
        } else if let Some(state) = restore_masonry_state {
            if self.mode().is_masonry() {
                self.zoom = self.clamp_zoom(state.zoom);
                self.zoom_target = self.clamp_zoom(state.zoom_target);
                self.zoom_velocity = 0.0;
                self.offset = state.offset;

                if follow_current_viewed_item {
                    // The current solo item changed, or the list changed underneath it, so
                    // the old absolute masonry scroll position is no longer trustworthy.
                    self.scroll_strip_to_current_index();
                } else {
                    let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
                    self.manga_scroll_offset = state.scroll_offset.clamp(0.0, max_scroll);
                    self.manga_scroll_target = state.scroll_target.clamp(0.0, max_scroll);
                    self.manga_scroll_velocity = 0.0;
                    self.stop_manga_wheel_scroll();
                }

                self.manga_update_current_index();
                self.manga_update_preload_queue();
                self.capture_pending_masonry_solo_reentry(current_viewed_index);
            }
        } else if self.mode().is_manga() {
            // Long-strip return and non-preserved masonry fall back here.
            // Ensure strip position follows the currently viewed file in solo fullscreen.
            self.scroll_strip_to_current_index();
            self.manga_update_current_index();
            self.manga_update_preload_queue();
            self.pending_masonry_solo_reentry = None;
        }

        if let Some((path, texture, width, height)) = return_video_texture {
            self.install_manga_video_texture_for_path(path, texture, width, height);
        }

        self.clear_strip_return_context();
    }

    pub(crate) fn manga_should_force_triangle_filters(&self) -> bool {
        self.mode().is_masonry()
    }
}

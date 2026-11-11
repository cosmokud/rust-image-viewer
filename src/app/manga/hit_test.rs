//! Hit testing inside strip and masonry layouts.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn open_strip_item_in_solo_fullscreen(
        &mut self,
        index: usize,
        button_only_return: bool,
    ) {
        if index >= self.image_list.len() {
            return;
        }

        self.pending_masonry_solo_reentry = None;

        let return_mode = self.mode().layout();
        self.set_current_index_clamped(index);
        let Some(path) = self.image_list.get(index).cloned() else {
            return;
        };

        if self.traverse_folder_entry_path(path.as_path()) {
            return;
        }

        self.strip_open_force_fit_path = Some(path.clone());
        if (return_mode).is_grid() {
            self.mark_masonry_runtime_cache_resident();
            self.capture_masonry_mode_session_snapshot();
        }

        let target_media_type = get_media_type(&path);
        if matches!(target_media_type, Some(MediaType::Video)) {
            self.sync_manga_video_position_to_resume(index);
        }
        self.prepare_mode_switch_placeholder_from_manga_index(index, target_media_type);

        self.activate_strip_return_context(return_mode, index);
        self.stop_manga_wheel_scroll();
        self.stop_manga_autoscroll();
        self.reset_gif_seek_interaction_state();
        if return_mode == MangaLayoutMode::Masonry {
            self.persist_current_masonry_folder_metadata_snapshot();
            self.pause_masonry_metadata_preload();
        } else {
            self.reset_masonry_metadata_preload();
        }
        self.mode_transition(ModeEvent::OpenSolo {
            layout: return_mode,
            button_only: button_only_return,
        });
        set_metadata_cache_enabled(false);
        // Preserve strip runtime caches for fast return from solo fullscreen.
        // Active workloads are still suspended to avoid hidden playback work.
        self.manga_suspend_runtime_for_solo_fullscreen();
        self.load_image(&path);
    }

    /// Toggle manga reading mode on/off
    pub(crate) fn toggle_manga_mode(&mut self) {
        if self.mode().is_solo() {
            self.pending_masonry_solo_reentry = None;
            self.sync_current_index_to_visible_media();
            self.sync_solo_video_position_to_manga_resume();
            let entering_grid = self.mode().is_grid();
            let preferred_masonry_path = if entering_grid {
                self.current_media_path()
            } else {
                None
            };
            let follow_current_viewed_item_on_restore = if entering_grid {
                self.masonry_mode_session_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| {
                        preferred_masonry_path
                            .as_ref()
                            .is_some_and(|path| snapshot.current_path.as_ref() != Some(path))
                    })
            } else {
                false
            };

            let current_media_dims = self.media_display_dimensions().or(self.video_texture_dims);
            let current_media_type = self.current_media_type;
            let current_image_is_animated =
                self.current_image_is_animated_for_mode_switch(current_media_type);

            self.manga_ttv_pending.clear();
            self.manga_ttv_samples_ms.clear();
            self.manga_upload_batch_limit = Self::MANGA_UPLOAD_BATCH_BASE;
            self.manga_visible_indices_last = 0;
            self.manga_visible_strip_equivalent_last = 0.0;
            self.manga_visible_indices_peak = 0;
            self.manga_pending_loads_peak = 0;
            self.manga_pending_decoded_peak = 0;

            self.prepare_enter_manga_mode_state(current_media_type);

            if let Some(anchor_path) = self.current_media_path() {
                self.refresh_media_list_after_path_mutation(Some(anchor_path));
            }

            let restored_masonry_session = if entering_grid {
                self.try_restore_masonry_mode_session_snapshot(preferred_masonry_path.clone())
            } else {
                false
            };
            let mut masonry_authoritative_ready = false;

            if entering_grid {
                self.mark_masonry_runtime_cache_resident();
            }

            let reuse_preserved_masonry_layout =
                restored_masonry_session || self.can_reuse_preserved_masonry_layout();

            // Manga layout cache must be rebuilt for the new mode.
            if !reuse_preserved_masonry_layout {
                self.invalidate_manga_layout_cache();
            }

            // Warm the active list from in-memory state or persistent metadata cache before
            // falling back to background probes for the remaining misses.
            if let Some(ref mut loader) = self.manga_loader {
                if !reuse_preserved_masonry_layout {
                    loader.prime_dimension_cache(&self.image_list);
                }
            }
            if entering_grid {
                self.restore_masonry_folder_metadata_snapshot();
                if restored_masonry_session {
                    self.hydrate_masonry_dimensions_from_session_snapshot();
                }
                masonry_authoritative_ready = self.masonry_authoritative_dimension_lock_active();
            }
            self.mark_manga_dimension_cache_current_if_complete();

            // Ensure the currently viewed item has accurate dimensions immediately.
            // This prevents stretched/overlap artifacts while entering strip mode,
            // especially when the current index is outside the initial cached range.
            let current_dims_changed = self.cache_current_media_dimensions_for_manga(
                current_media_dims,
                current_media_type,
                current_image_is_animated,
            );

            // Dimensions may have changed; rebuild height cache.
            if !reuse_preserved_masonry_layout
                || (current_dims_changed
                    && !(restored_masonry_session && masonry_authoritative_ready))
            {
                self.invalidate_manga_layout_cache();
            }

            if entering_grid {
                if !restored_masonry_session {
                    let new_zoom = self.clamp_zoom(1.0);
                    self.zoom = new_zoom;
                    self.zoom_target = new_zoom;
                    self.zoom_velocity = 0.0;
                }
            } else {
                // Enter long-strip mode: start in a "fit-to-screen by height" zoom (like fullscreen fit).
                // In long-strip mode we apply a per-image `base_scale` (fit tall pages down) and then
                // multiply by `self.zoom`. We want total scale to be `screen_h / img_h`.
                let screen_h = self.screen_size.y.max(1.0);
                if let Some((_w, h)) = self.media_display_dimensions() {
                    let img_h = h as f32;
                    if img_h > 0.0 {
                        let base_scale = if img_h > screen_h {
                            screen_h / img_h
                        } else {
                            1.0
                        };
                        let desired_total_scale = screen_h / img_h;
                        let new_zoom = self.clamp_zoom(desired_total_scale / base_scale);
                        self.zoom = new_zoom;
                        self.zoom_target = new_zoom;
                        self.zoom_velocity = 0.0;
                    }
                }
            }

            if !restored_masonry_session {
                // Reset offset (horizontal pan) and scroll to current image position
                self.offset = egui::Vec2::ZERO;
                let target_index = self
                    .current_index
                    .min(self.image_list.len().saturating_sub(1));
                self.set_current_index_clamped(target_index);
                let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
                let scroll_to = if entering_grid {
                    self.masonry_scroll_offset_for_index_centered(target_index)
                        .unwrap_or_else(|| {
                            self.manga_get_scroll_offset_for_index(target_index)
                                .clamp(0.0, max_scroll)
                        })
                } else {
                    self.manga_get_scroll_offset_for_index(target_index)
                        .clamp(0.0, max_scroll)
                };
                self.manga_scroll_offset = scroll_to;
                self.manga_scroll_target = scroll_to;
                self.manga_scroll_velocity = 0.0;
                self.stop_manga_wheel_scroll();
            } else if follow_current_viewed_item_on_restore {
                self.scroll_strip_to_current_index();
            } else {
                self.manga_scroll_velocity = 0.0;
                self.stop_manga_wheel_scroll();
            }

            let allow_startup_preload = self.mode().layout() == MangaLayoutMode::Masonry
                && (!restored_masonry_session || !masonry_authoritative_ready);
            self.maybe_begin_masonry_metadata_preload(allow_startup_preload);
            if entering_grid {
                self.manga_update_current_index();
                if self.mode().is_true_masonry() {
                    self.persist_current_masonry_folder_metadata_snapshot();
                }
            }

            self.manga_update_preload_queue();
            return;
        }

        // Exiting manga mode: switch fullscreen view to the currently visible page.
        let visible_idx = self.manga_visible_index();
        self.set_current_index_clamped(visible_idx);
        let target_path = self.image_list.get(visible_idx).cloned();
        let target_media_type = target_path.as_ref().and_then(|path| get_media_type(path));

        self.prepare_mode_switch_placeholder_from_manga_index(visible_idx, target_media_type);

        self.stop_manga_wheel_scroll();
        self.stop_manga_autoscroll();
        if self.mode().is_grid() {
            self.capture_masonry_mode_session_snapshot();
        }
        if self.mode().is_true_masonry() || self.has_resident_masonry_runtime_cache() {
            self.persist_current_masonry_folder_metadata_snapshot();
        }
        let keep_resident_masonry_cache =
            self.mode().is_grid() || self.has_resident_masonry_runtime_cache();
        let solo_layout = self.mode().layout();
        self.mode_transition(ModeEvent::LeaveStrip);
        self.mode_state.set_layout(solo_layout);
        set_metadata_cache_enabled(false);
        if keep_resident_masonry_cache {
            self.manga_suspend_runtime_for_solo_fullscreen();
        } else {
            self.clear_manga_runtime_cache(false);
        }

        if let Some(path) = target_path {
            // Load the selected page into normal fullscreen mode.
            self.load_image(&path);
        }
    }
}

//! Strip/Masonry/Gallery toggles.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn toggle_masonry_mode(&mut self) {
        self.toggle_strip_mode(MangaLayoutMode::Masonry);
    }

    pub(crate) fn toggle_gallery_mode(&mut self) {
        self.toggle_strip_mode(MangaLayoutMode::Gallery);
    }

    pub(crate) fn set_masonry_items_per_row(&mut self, items_per_row: usize) {
        let items_per_row = items_per_row.clamp(2, 10);
        if self.masonry_items_per_row == items_per_row {
            return;
        }

        let center_anchor = if self.mode().is_masonry() {
            self.manga_capture_center_anchor()
        } else {
            None
        };

        self.masonry_items_per_row = items_per_row;
        self.config.masonry_items_per_row = items_per_row;
        self.config.save();
        self.invalidate_manga_layout_cache();

        if let Some(anchor) = center_anchor {
            self.manga_apply_center_anchor(anchor);
            self.manga_update_current_index();
            self.manga_update_preload_queue();
        }
    }

    pub(crate) fn toggle_strip_mode(&mut self, layout_mode: MangaLayoutMode) {
        self.pending_masonry_solo_reentry = None;

        if (layout_mode).is_grid() && self.mode().is_solo() {
            if !self.refresh_media_list_before_masonry_entry() {
                return;
            }
        }

        if self.mode().is_solo() {
            self.mode_state.set_layout(layout_mode);
            if (layout_mode).is_grid() {
                self.mark_masonry_runtime_cache_resident();
            }
            self.toggle_manga_mode();
            return;
        }

        if self.mode().layout() == layout_mode {
            self.toggle_manga_mode();
            return;
        }

        // Use the currently viewed file as the single source of truth when switching layouts.
        let target_index = self
            .manga_visible_index()
            .min(self.image_list.len().saturating_sub(1));
        self.set_current_index_clamped(target_index);
        let target_path = self.current_media_path();

        if self.mode().is_grid() {
            self.capture_masonry_mode_session_snapshot();
            if self.mode().is_true_masonry() {
                self.persist_current_masonry_folder_metadata_snapshot();
            }
        }

        let follow_current_viewed_item_on_restore = if (layout_mode).is_grid() {
            self.masonry_mode_session_snapshot
                .as_ref()
                .is_some_and(|snapshot| {
                    target_path
                        .as_ref()
                        .is_some_and(|path| snapshot.current_path.as_ref() != Some(path))
                })
        } else {
            false
        };

        self.mode_state.set_layout(layout_mode);
        set_metadata_cache_enabled(true);
        let restored_masonry_session = if (layout_mode).is_grid() {
            self.mark_masonry_runtime_cache_resident();
            self.restore_masonry_folder_metadata_snapshot();
            self.mark_manga_dimension_cache_current_if_complete();
            let restored = self.try_restore_masonry_mode_session_snapshot(target_path.clone());
            if restored {
                self.hydrate_masonry_dimensions_from_session_snapshot();
            }
            restored
        } else {
            false
        };
        let masonry_authoritative_ready = layout_mode == MangaLayoutMode::Masonry
            && self.masonry_authoritative_dimension_lock_active();

        if !(layout_mode).is_grid() || !restored_masonry_session {
            self.invalidate_manga_layout_cache();
            self.offset = egui::Vec2::ZERO;

            let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
            let scroll_to = if (layout_mode).is_grid() {
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
        }

        if (layout_mode).is_grid() {
            self.manga_update_current_index();
            if self.mode().is_true_masonry() {
                self.persist_current_masonry_folder_metadata_snapshot();
            }
        }

        let allow_startup_preload = layout_mode == MangaLayoutMode::Masonry
            && (!restored_masonry_session || !masonry_authoritative_ready);
        self.maybe_begin_masonry_metadata_preload(allow_startup_preload);

        self.manga_update_preload_queue();
    }

    pub(crate) fn masonry_slot_source_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        self.manga_loader
            .as_ref()
            .and_then(|loader| loader.get_dimensions(index))
            .and_then(|(w, h)| if w > 0 && h > 0 { Some((w, h)) } else { None })
    }
}

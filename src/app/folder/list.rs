//! The media list itself: replace, normalise, append, scan.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn normalize_image_list_for_folder_navigation(
        &self,
        files: Vec<PathBuf>,
    ) -> Vec<PathBuf> {
        if self.folder_navigation_ui_enabled() {
            files
        } else {
            files
                .into_iter()
                .filter(|path| !self.is_folder_navigation_entry_path(path.as_path()))
                .collect()
        }
    }

    pub(crate) fn set_image_list(&mut self, files: Vec<PathBuf>) {
        let files = self.normalize_image_list_for_folder_navigation(files);

        self.folder_placeholder_thumbnail_cache
            .retain(|directory, _| directory.exists());
        self.folder_placeholder_preview_scan_pending
            .retain(|directory| directory.exists());
        self.folder_placeholder_thumbnail_pending
            .retain(|path| path.exists());
        self.folder_placeholder_thumbnail_failures
            .retain(|path, _| path.exists());
        self.unloadable_files.retain(|path, _| path.exists());
        self.folder_placeholder_stamp_cache.clear();

        self.set_image_list_raw(files);
    }

    pub(crate) fn try_append_new_entries_in_strip_mode(
        &mut self,
        scanned_files: &[PathBuf],
    ) -> bool {
        if self.mode().is_solo() || self.image_list.is_empty() {
            return false;
        }

        let scanned_files = self.normalize_image_list_for_folder_navigation(scanned_files.to_vec());
        if scanned_files.is_empty() {
            return false;
        }

        let current_set: HashSet<PathBuf> = self.image_list.iter().cloned().collect();
        let scanned_set: HashSet<PathBuf> = scanned_files.iter().cloned().collect();
        let has_removals = self
            .image_list
            .iter()
            .any(|path| !scanned_set.contains(path));
        if has_removals {
            return false;
        }

        let appended: Vec<PathBuf> = scanned_files
            .into_iter()
            .filter(|path| !current_set.contains(path))
            .collect();

        if appended.is_empty() {
            return true;
        }

        let mut updated = self.image_list.clone();
        updated.extend(appended);
        self.set_image_list_raw(updated);
        self.invalidate_manga_layout_cache();
        self.manga_update_preload_queue();
        true
    }

    pub(crate) fn begin_media_directory_scan(
        &mut self,
        path: &Path,
        kind: PendingMediaDirectoryScanKind,
    ) -> bool {
        let Some(rx) = self.media_directory_index.request_media_scan_for_path(path) else {
            return false;
        };

        self.pending_media_directory_scan = Some(rx);
        self.pending_media_directory_target = Some(path.to_path_buf());
        self.pending_media_directory_scan_kind = Some(kind);
        self.pending_media_directory_started_at = Some(Instant::now());
        true
    }

    pub(crate) fn current_media_path(&self) -> Option<PathBuf> {
        self.image_list.get(self.current_index).cloned()
    }

    pub(crate) fn store_folder_travel_position_for_current_folder(&self) {
        let Some(layout_mode) = self.mode().travel_layout() else {
            return;
        };

        let Some(current_path) = self.current_media_path() else {
            return;
        };

        let Some(current_directory) = current_path.parent().map(Path::to_path_buf) else {
            return;
        };

        let position = FolderTravelPosition {
            current_path,
            current_index: self.current_index,
            scroll_offset: self.manga_scroll_offset.max(0.0),
        };
        store_folder_travel_position(current_directory.as_path(), layout_mode, &position);
    }

    pub(crate) fn restore_folder_travel_position_for_directory(
        &mut self,
        directory: &Path,
    ) -> bool {
        let Some(layout_mode) = self.mode().travel_layout() else {
            return false;
        };

        let Some(position) = lookup_folder_travel_position(directory, layout_mode) else {
            return false;
        };

        if self.image_list.is_empty() {
            return false;
        }

        let mut resolved_index = self.image_list.iter().position(|candidate| {
            candidate.as_path() == position.current_path.as_path()
                && !Self::is_up_navigation_entry_path(candidate.as_path())
        });

        if resolved_index.is_none() {
            let fallback_index = position
                .current_index
                .min(self.image_list.len().saturating_sub(1));
            if self
                .image_list
                .get(fallback_index)
                .is_some_and(|path| !Self::is_up_navigation_entry_path(path.as_path()))
            {
                resolved_index = Some(fallback_index);
            }
        }

        if resolved_index.is_none() {
            resolved_index = self
                .image_list
                .iter()
                .position(|candidate| !Self::is_up_navigation_entry_path(candidate.as_path()));
        }

        let Some(resolved_index) = resolved_index else {
            return false;
        };

        self.set_current_index_clamped(resolved_index);

        match layout_mode {
            FolderTravelLayoutMode::LongStrip => {
                let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
                let scroll_to = self
                    .manga_get_scroll_offset_for_index(resolved_index)
                    .clamp(0.0, max_scroll);
                self.manga_scroll_offset = scroll_to;
                self.manga_scroll_target = scroll_to;
                self.manga_scroll_velocity = 0.0;
            }
            FolderTravelLayoutMode::Masonry => {
                let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
                let restored_offset = if position.scroll_offset.is_finite() {
                    position.scroll_offset.max(0.0)
                } else {
                    0.0
                }
                .clamp(0.0, max_scroll);

                if self.masonry_metadata_preload_active {
                    self.pending_masonry_folder_travel_restore =
                        Some((resolved_index, restored_offset));
                } else {
                    self.manga_scroll_offset = restored_offset;
                    self.manga_scroll_target = restored_offset;
                    self.manga_scroll_velocity = 0.0;
                }
            }
        }

        true
    }

    pub(crate) fn is_up_navigation_entry_name(name: &str) -> bool {
        name == FOLDER_UP_ENTRY_NAME || name == "[...]"
    }
}

//! Folder navigation history (back/forward stacks).

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn folder_navigation_can_go_back(&self) -> bool {
        self.folder_navigation_history_index
            .is_some_and(|index| index > 0 && index < self.folder_navigation_history.len())
    }

    pub(crate) fn folder_navigation_can_go_forward(&self) -> bool {
        self.folder_navigation_history_index
            .is_some_and(|index| index + 1 < self.folder_navigation_history.len())
    }

    pub(crate) fn record_folder_navigation_to_directory(&mut self, directory: &Path) {
        let destination = directory.to_path_buf();
        let current_directory = self
            .current_media_path()
            .and_then(|path| path.parent().map(Path::to_path_buf));

        if current_directory
            .as_ref()
            .is_some_and(|current| current == &destination)
        {
            return;
        }

        if self.folder_navigation_history.is_empty() {
            if let Some(current) = current_directory {
                self.folder_navigation_history.push(current);
                self.folder_navigation_history_index = Some(0);
            }
        } else {
            let mut index = self
                .folder_navigation_history_index
                .unwrap_or_else(|| self.folder_navigation_history.len().saturating_sub(1));
            index = index.min(self.folder_navigation_history.len().saturating_sub(1));
            self.folder_navigation_history_index = Some(index);

            if let Some(current) = current_directory {
                if self.folder_navigation_history.get(index) != Some(&current) {
                    self.folder_navigation_history
                        .truncate(index.saturating_add(1));
                    if self.folder_navigation_history.last() != Some(&current) {
                        self.folder_navigation_history.push(current);
                    }
                    self.folder_navigation_history_index =
                        Some(self.folder_navigation_history.len().saturating_sub(1));
                }
            }

            let index = self
                .folder_navigation_history_index
                .unwrap_or_else(|| self.folder_navigation_history.len().saturating_sub(1));
            self.folder_navigation_history
                .truncate(index.saturating_add(1));
        }

        if self.folder_navigation_history.last() != Some(&destination) {
            self.folder_navigation_history.push(destination);
        }
        if !self.folder_navigation_history.is_empty() {
            self.folder_navigation_history_index =
                Some(self.folder_navigation_history.len().saturating_sub(1));
        }
        self.trim_folder_navigation_history();
    }

    pub(crate) fn navigate_folder_history_back(&mut self) -> bool {
        let Some(current_index) = self.folder_navigation_history_index else {
            return false;
        };
        if current_index == 0 {
            return false;
        }

        self.navigate_folder_history_to_index(current_index - 1)
    }

    pub(crate) fn navigate_folder_history_forward(&mut self) -> bool {
        let Some(current_index) = self.folder_navigation_history_index else {
            return false;
        };
        self.navigate_folder_history_to_index(current_index.saturating_add(1))
    }

    pub(crate) fn navigate_folder_history_to_index(&mut self, target_index: usize) -> bool {
        let Some(target_directory) = self.folder_navigation_history.get(target_index).cloned()
        else {
            return false;
        };

        if self.navigate_to_breadcrumb_directory_internal(
            target_directory.as_path(),
            FolderHistoryNavigationKind::FromHistory,
        ) {
            self.folder_navigation_history_index = Some(target_index);
            true
        } else {
            false
        }
    }

    pub(crate) fn folder_navigation_back_history_items(
        &self,
        max_items: usize,
    ) -> Vec<(usize, PathBuf)> {
        if max_items == 0 {
            return Vec::new();
        }

        let Some(current_index) = self.folder_navigation_history_index else {
            return Vec::new();
        };
        if current_index == 0 || self.folder_navigation_history.is_empty() {
            return Vec::new();
        }

        let start_index = current_index.saturating_sub(max_items);
        let mut entries = Vec::with_capacity(current_index.saturating_sub(start_index));
        for history_index in (start_index..current_index).rev() {
            if let Some(path) = self.folder_navigation_history.get(history_index) {
                entries.push((history_index, path.clone()));
            }
        }

        entries
    }

    pub(crate) fn format_folder_history_entry_label(path: &Path, max_depth: usize) -> String {
        if max_depth == 0 {
            return path.display().to_string();
        }

        let mut segments: Vec<String> = Vec::new();
        let mut cursor = Some(path);
        while let Some(current) = cursor {
            if let Some(name) = current.file_name() {
                let text = name.to_string_lossy();
                if !text.is_empty() {
                    segments.push(text.to_string());
                }
            }
            cursor = current.parent();
        }

        if segments.is_empty() {
            return path.display().to_string();
        }

        let truncated = segments.len() > max_depth;
        segments.truncate(max_depth);
        segments.reverse();

        let separator = std::path::MAIN_SEPARATOR.to_string();
        let joined = segments.join(separator.as_str());
        if truncated {
            format!("..{}{}", separator, joined)
        } else {
            joined
        }
    }

    pub(crate) fn current_breadcrumb_directory(&self) -> Option<PathBuf> {
        self.current_media_path()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.image_list
                    .first()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
    }

    pub(crate) fn navigate_up_from_breadcrumb(&mut self) -> bool {
        let Some(current_directory) = self.current_breadcrumb_directory() else {
            return false;
        };
        let Some(parent_directory) = current_directory.parent().map(Path::to_path_buf) else {
            return false;
        };

        self.navigate_to_breadcrumb_directory(parent_directory.as_path());
        true
    }

    pub(crate) fn navigate_to_breadcrumb_directory(&mut self, directory: &Path) {
        let _ = self.navigate_to_breadcrumb_directory_internal(
            directory,
            FolderHistoryNavigationKind::Record,
        );
    }

    pub(crate) fn navigate_to_breadcrumb_directory_internal(
        &mut self,
        directory: &Path,
        history_navigation: FolderHistoryNavigationKind,
    ) -> bool {
        if !directory.exists() || !directory.is_dir() {
            self.error_message = Some(format!("Folder does not exist: {}", directory.display()));
            return false;
        }

        if self.mode().is_masonry() {
            self.persist_current_masonry_folder_metadata_snapshot();
        }

        // Persist the current folder viewport state before any folder-travel jump.
        self.store_folder_travel_position_for_current_folder();

        let mut files = get_media_in_directory(directory);
        if files.is_empty() {
            self.error_message = Some(format!(
                "No supported media files found in folder: {}",
                directory.display()
            ));
            return false;
        }

        let modified_at = std::fs::metadata(directory)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        files = self
            .media_directory_index
            .apply_directory_scan_result(DirectoryScanResult {
                directory: directory.to_path_buf(),
                files,
                modified_at,
            });

        if files.is_empty() {
            self.error_message = Some(format!(
                "No supported media files found in folder: {}",
                directory.display()
            ));
            return false;
        }

        if history_navigation == FolderHistoryNavigationKind::Record {
            self.record_folder_navigation_to_directory(directory);
        }

        let current_path = self.current_media_path();
        let target_path = current_path
            .filter(|path| {
                path.parent() == Some(directory)
                    && files
                        .iter()
                        .any(|candidate| candidate.as_path() == path.as_path())
            })
            .or_else(|| {
                files
                    .iter()
                    .find(|candidate| !self.is_folder_navigation_entry_path(candidate.as_path()))
                    .cloned()
            })
            .or_else(|| {
                files
                    .iter()
                    .find(|candidate| Self::is_up_navigation_entry_path(candidate.as_path()))
                    .cloned()
            })
            .unwrap_or_else(|| {
                files
                    .first()
                    .cloned()
                    .unwrap_or_else(|| directory.to_path_buf())
            });

        self.error_message = None;
        self.show_controls = true;
        self.controls_show_time = Instant::now();

        if self.mode().is_strip() {
            self.set_image_list(files);
            self.clear_stale_marked_files();
            self.clear_stale_prepared_clipboard_paths();
            self.modal_thumbnail_cache.retain(|path, _| path.exists());

            let resolved_index = self
                .image_list
                .iter()
                .position(|candidate| candidate == &target_path)
                .unwrap_or(0);
            self.set_current_index_clamped(resolved_index);
            self.pending_window_title = Some(self.compute_window_title_for_path(&target_path));

            self.manga_clear_cache();
            self.ensure_manga_loader();

            if self.mode().is_masonry() {
                if let Some(ref mut loader) = self.manga_loader {
                    loader.prime_dimension_cache(&self.image_list);
                }
                self.restore_masonry_folder_metadata_snapshot();
                self.mark_manga_dimension_cache_current_if_complete();
                self.maybe_begin_masonry_metadata_preload(true);
            } else {
                self.reset_masonry_metadata_preload();
            }

            self.manga_update_preload_queue();
            if !self.restore_folder_travel_position_for_directory(directory) {
                self.manga_scroll_offset = 0.0;
                self.manga_scroll_target = 0.0;
                self.manga_scroll_velocity = 0.0;
            }

            if let Some(active_path) = self.current_media_path() {
                self.pending_window_title = Some(self.compute_window_title_for_path(&active_path));
            }
        } else {
            self.load_media(&target_path);
        }

        true
    }

    pub(crate) fn hovered_manga_index_from_pointer(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<usize> {
        if self.mode().is_solo() || !self.mode().is_fullscreen() || self.image_list.is_empty() {
            return None;
        }

        let pointer_pos = ctx.input(|input| input.pointer.hover_pos());
        let Some(pos) = pointer_pos else {
            return None;
        };

        if self.pointer_over_shortcut_blocking_ui(Some(pos), ctx.screen_rect()) {
            return None;
        }

        self.manga_index_at_screen_pos(pos)
    }
}

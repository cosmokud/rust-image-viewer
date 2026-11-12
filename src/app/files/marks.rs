//! Multi-file mark selection and prepared clipboard targets.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn active_mark_shortcuts(&self) -> (Option<egui::Key>, Option<ShortcutModifier>) {
        if self.mode().is_strip() {
            if self.mode().is_masonry() {
                (
                    self.config.masonry_mark_file,
                    self.config.masonry_toggle_mark_file,
                )
            } else {
                (
                    self.config.manga_mark_file,
                    self.config.manga_toggle_mark_file,
                )
            }
        } else {
            (self.config.mark_file, self.config.toggle_mark_file)
        }
    }

    pub(crate) fn shortcut_modifier_matches_input(
        modifier: ShortcutModifier,
        modifiers: egui::Modifiers,
    ) -> bool {
        match modifier {
            ShortcutModifier::Ctrl => modifiers.ctrl && !modifiers.shift && !modifiers.alt,
            ShortcutModifier::Shift => !modifiers.ctrl && modifiers.shift && !modifiers.alt,
            ShortcutModifier::Alt => !modifiers.ctrl && !modifiers.shift && modifiers.alt,
        }
    }

    pub(crate) fn is_markable_index(&self, index: usize) -> bool {
        self.image_list
            .get(index)
            .is_some_and(|path| !Self::is_up_navigation_entry_path(path.as_path()))
    }

    pub(crate) fn mark_target_index_from_pointer(&mut self, ctx: &egui::Context) -> Option<usize> {
        if self.image_list.is_empty() {
            return None;
        }

        let target_index = if let Some(index) = self.hovered_manga_index_from_pointer(ctx) {
            Some(index)
        } else {
            Some(
                self.current_index
                    .min(self.image_list.len().saturating_sub(1)),
            )
        };

        target_index.filter(|index| self.is_markable_index(*index))
    }

    pub(crate) fn is_path_marked(&self, path: &Path) -> bool {
        self.marked_files.contains(path)
    }

    pub(crate) fn clear_prepared_clipboard_for_path(&mut self, path: &Path) -> bool {
        self.prepared_clipboard_paths.remove(path).is_some()
    }

    pub(crate) fn clear_all_marks(&mut self) {
        self.marked_files.clear();
        let had_prepared_clipboard_paths = !self.prepared_clipboard_paths.is_empty();
        self.prepared_clipboard_paths.clear();
        if had_prepared_clipboard_paths {
            self.sync_prepared_clipboard_with_system();
        }
    }

    pub(crate) fn has_marked_files(&self) -> bool {
        !self.marked_files.is_empty()
    }

    /// True while the settings window is open. The window is detached from the viewer, so this
    /// only reports its lifetime: it never gates viewer input.
    pub(crate) fn settings_modal_open(&self) -> bool {
        self.settings.open
    }

    pub(crate) fn any_modal_dialog_open(&self) -> bool {
        self.rename_overlay.is_some()
            || self.pending_single_delete_target.is_some()
            || !self.pending_marked_delete_targets.is_empty()
            || self.pending_exit_confirmation
            || self.stash.open
    }

    pub(crate) fn request_app_exit(&mut self) {
        if self.has_marked_files() {
            self.pending_exit_confirmation = true;
            self.file_action_menu = None;
            self.show_controls = true;
            self.controls_show_time = Instant::now();
        } else {
            self.begin_fast_app_exit();
        }
    }

    pub(crate) fn prepared_clipboard_operation_for_path(
        &self,
        path: &Path,
    ) -> Option<FileClipboardOperation> {
        self.prepared_clipboard_paths.get(path).copied()
    }

    pub(crate) fn is_index_marked(&self, index: usize) -> bool {
        self.image_list
            .get(index)
            .is_some_and(|path| self.is_path_marked(path))
    }

    pub(crate) fn toggle_mark_for_index(&mut self, index: usize) -> bool {
        let Some(path) = self.image_list.get(index).cloned() else {
            return false;
        };
        if Self::is_up_navigation_entry_path(path.as_path()) {
            return false;
        }

        if !self.marked_files.insert(path.clone()) {
            self.marked_files.remove(&path);
            if self.clear_prepared_clipboard_for_path(&path) {
                self.sync_prepared_clipboard_with_system();
            }
            return false;
        }

        true
    }

    pub(crate) fn toggle_marks_for_indices(&mut self, indices: &[usize]) -> usize {
        let mut changed = 0usize;
        let mut prepared_clipboard_changed = false;
        let mut seen_paths = HashSet::new();

        for index in indices {
            if let Some(path) = self.image_list.get(*index).cloned() {
                if Self::is_up_navigation_entry_path(path.as_path()) {
                    continue;
                }
                if !seen_paths.insert(path.clone()) {
                    continue;
                }

                if !self.marked_files.insert(path.clone()) {
                    self.marked_files.remove(&path);
                    if self.clear_prepared_clipboard_for_path(&path) {
                        prepared_clipboard_changed = true;
                    }
                }
                changed = changed.saturating_add(1);
            }
        }

        if prepared_clipboard_changed {
            self.sync_prepared_clipboard_with_system();
        }

        changed
    }

    pub(crate) fn mark_all_files(&mut self) -> usize {
        let mut added = 0usize;
        for path in &self.image_list {
            if Self::is_up_navigation_entry_path(path.as_path()) {
                continue;
            }
            if self.marked_files.insert(path.clone()) {
                added = added.saturating_add(1);
            }
        }

        added
    }

    pub(crate) fn clear_stale_marked_files(&mut self) {
        let prepared_count_before = self.prepared_clipboard_paths.len();
        self.marked_files.retain(|path| path.exists());
        self.prepared_clipboard_paths
            .retain(|path, _| path.exists() && self.marked_files.contains(path));
        if self.prepared_clipboard_paths.len() != prepared_count_before {
            self.sync_prepared_clipboard_with_system();
        }
    }

    pub(crate) fn clear_stale_prepared_clipboard_paths(&mut self) {
        let prepared_count_before = self.prepared_clipboard_paths.len();
        self.prepared_clipboard_paths
            .retain(|path, _| path.exists());
        if self.prepared_clipboard_paths.len() != prepared_count_before {
            self.sync_prepared_clipboard_with_system();
        }
    }

    pub(crate) fn collect_prepared_clipboard_targets(
        &self,
    ) -> Option<(Vec<PathBuf>, FileClipboardOperation)> {
        let operation = self.prepared_clipboard_paths.values().next().copied()?;

        let mut ordered_paths: Vec<PathBuf> = self
            .image_list
            .iter()
            .filter(|path| {
                self.prepared_clipboard_paths
                    .get(*path)
                    .is_some_and(|current_operation| *current_operation == operation)
                    && path.exists()
            })
            .cloned()
            .collect();

        let mut extra_paths: Vec<PathBuf> = self
            .prepared_clipboard_paths
            .iter()
            .filter(|(path, current_operation)| {
                **current_operation == operation && !self.image_list.contains(path) && path.exists()
            })
            .map(|(path, _)| path.clone())
            .collect();
        extra_paths.sort();
        ordered_paths.extend(extra_paths);

        if ordered_paths.is_empty() {
            None
        } else {
            Some((ordered_paths, operation))
        }
    }

    pub(crate) fn sync_prepared_clipboard_with_system(&mut self) {
        let result = match self.collect_prepared_clipboard_targets() {
            Some((paths, operation)) => write_shell_file_list_to_clipboard(&paths, operation),
            None => clear_system_clipboard(),
        };

        if let Err(err) = result {
            self.error_message = Some(err);
        }
    }

    pub(crate) fn refresh_media_list_if_entries_disappeared(&mut self) {
        const MISSING_MEDIA_REFRESH_INTERVAL: Duration = Duration::from_millis(750);

        if self.defer_directory_work_for_fast_startup() {
            return;
        }

        if self.last_missing_media_refresh_check.elapsed() < MISSING_MEDIA_REFRESH_INTERVAL {
            return;
        }
        self.last_missing_media_refresh_check = Instant::now();

        if self.pending_media_directory_scan.is_some() {
            return;
        }

        let refresh_anchor = self
            .current_media_path()
            .or_else(|| self.image_list.first().cloned());

        let Some(anchor_path) = refresh_anchor else {
            return;
        };

        if self
            .media_directory_index
            .try_cached_media_for_path(&anchor_path)
            .is_some()
        {
            return;
        }

        let _ = self.begin_media_directory_scan(
            &anchor_path,
            PendingMediaDirectoryScanKind::ExternalRefresh,
        );
    }

    pub(crate) fn set_prepared_clipboard_targets(
        &mut self,
        paths: &[PathBuf],
        operation: FileClipboardOperation,
    ) {
        self.prepared_clipboard_paths.clear();

        for path in paths {
            if path.exists() {
                self.prepared_clipboard_paths
                    .insert(path.clone(), operation);
            }
        }
    }

    pub(crate) fn mark_visual_for_path(&self, path: &Path) -> Option<FileMarkVisual> {
        match self.prepared_clipboard_operation_for_path(path) {
            Some(FileClipboardOperation::Copy) => Some(FileMarkVisual::Copied),
            Some(FileClipboardOperation::Cut) => Some(FileMarkVisual::Cut),
            None if self.is_path_marked(path) => Some(FileMarkVisual::Marked),
            None => None,
        }
    }

    pub(crate) fn mark_visual_for_index(&self, index: usize) -> Option<FileMarkVisual> {
        self.image_list
            .get(index)
            .and_then(|path| self.mark_visual_for_path(path))
    }

    pub(crate) fn collect_marked_paths_in_current_order(&self) -> Vec<PathBuf> {
        let mut ordered: Vec<PathBuf> = self
            .image_list
            .iter()
            .filter(|path| self.marked_files.contains(*path) && path.exists())
            .cloned()
            .collect();

        let mut extras: Vec<PathBuf> = self
            .marked_files
            .iter()
            .filter(|path| !self.image_list.contains(*path) && path.exists())
            .cloned()
            .collect();
        extras.sort();
        ordered.extend(extras);
        ordered
    }

    pub(crate) fn choose_fallback_path_after_removal(
        &self,
        removed_paths: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        let current_path = self.current_media_path();
        if let Some(path) = current_path.as_ref() {
            if !removed_paths.contains(path) && path.exists() {
                return Some(path.clone());
            }
        }

        for candidate in self
            .image_list
            .iter()
            .skip(self.current_index.saturating_add(1))
        {
            if !removed_paths.contains(candidate) && candidate.exists() {
                return Some(candidate.clone());
            }
        }

        for candidate in self.image_list.iter().take(self.current_index).rev() {
            if !removed_paths.contains(candidate) && candidate.exists() {
                return Some(candidate.clone());
            }
        }

        None
    }

    pub(crate) fn clear_current_media_after_all_files_removed(&mut self) {
        self.clear_pending_media_load();
        self.clear_pending_manga_video_load();
        self.stop_fullscreen_video_playback();
        self.reset_fullscreen_anim_stream_state();

        self.image = None;
        self.texture = None;
        self.image_texture_dims = None;
        self.video_texture = None;
        self.video_texture_source_path = None;
        self.video_texture_dims = None;
        self.current_media_type = None;
        self.current_index = 0;
        self.set_image_list(Vec::new());

        self.current_file_size_label = None;
        self.current_file_size_label_path = None;
        self.pending_file_size_probe = None;
        self.pending_file_size_probe_path = None;

        self.error_message = None;
        self.pending_window_title = Some(env!("CARGO_PKG_NAME").to_string());
        self.clear_all_marks();
        self.prepared_clipboard_paths.clear();
        self.file_action_menu = None;
        self.rename_overlay = None;
        self.pending_single_delete_target = None;
        self.pending_marked_delete_targets.clear();
        self.pending_exit_confirmation = false;
        self.modal_thumbnail_cache.clear();

        if self.mode().is_manga() {
            self.manga_clear_cache();
            self.mode_transition(ModeEvent::LeaveStrip);
            set_metadata_cache_enabled(false);
        }
    }

    pub(crate) fn refresh_media_list_after_path_mutation(
        &mut self,
        preferred_current_path: Option<PathBuf>,
    ) {
        let anchor_path = preferred_current_path
            .clone()
            .or_else(|| self.current_media_path())
            .or_else(|| self.image_list.first().cloned());

        let Some(anchor_path) = anchor_path else {
            self.clear_current_media_after_all_files_removed();
            return;
        };

        if self.mode().is_manga() && self.mode().is_true_masonry() {
            self.persist_current_masonry_folder_metadata_snapshot();
        }

        // Always resolve the actual directory we are viewing, whether the anchor is a file or a subfolder.
        let directory = anchor_path
            .parent()
            .unwrap_or(anchor_path.as_path())
            .to_path_buf();

        self.media_directory_index.invalidate_directory(&directory);

        self.pending_media_directory_scan = None;
        self.pending_media_directory_target = None;
        self.pending_media_directory_scan_kind = None;
        self.pending_media_directory_started_at = None;

        let files = get_media_in_directory(&directory);
        let modified_at = std::fs::metadata(&directory)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        let files = self
            .media_directory_index
            .apply_directory_scan_result(DirectoryScanResult {
                directory,
                files,
                modified_at,
            });
        self.set_image_list(files);
        self.clear_stale_marked_files();
        self.clear_stale_prepared_clipboard_paths();
        self.modal_thumbnail_cache.retain(|path, _| path.exists());

        if self.image_list.is_empty() {
            self.clear_current_media_after_all_files_removed();
            return;
        }

        let resolved_path = preferred_current_path
            .as_ref()
            .and_then(|preferred| {
                self.image_list
                    .iter()
                    .find(|candidate| *candidate == preferred)
                    .cloned()
            })
            .or_else(|| self.current_media_path())
            .or_else(|| self.image_list.first().cloned());

        if let Some(path) = resolved_path {
            let resolved_index = self
                .image_list
                .iter()
                .position(|candidate| candidate == &path)
                .unwrap_or(0);
            self.set_current_index_clamped(resolved_index);
            self.pending_window_title = Some(self.compute_window_title_for_path(&path));
        }

        if self.mode().is_manga() {
            self.manga_clear_cache();
            self.ensure_manga_loader();
            if self.mode().is_grid() {
                self.restore_masonry_folder_metadata_snapshot();
                self.mark_manga_dimension_cache_current_if_complete();
            }
            self.manga_update_preload_queue();
        }
    }

    pub(crate) fn refresh_media_list_before_masonry_entry(&mut self) -> bool {
        let anchor_path = self
            .current_media_path()
            .or_else(|| self.image_list.first().cloned());

        let Some(anchor_path) = anchor_path else {
            return false;
        };

        let current_path_missing = !anchor_path.exists();
        let directory_changed = self
            .media_directory_index
            .cached_directory_changed_for_path(&anchor_path);

        if current_path_missing || directory_changed {
            self.strip_return_masonry_list_snapshot = None;
            self.refresh_media_list_after_path_mutation(Some(anchor_path));
        }

        !self.image_list.is_empty()
    }
}

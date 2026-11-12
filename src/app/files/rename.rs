//! Inline rename and the rename dialog commit path.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn start_inline_rename_for_marked_files(&mut self) {
        let paths = self.collect_marked_paths_in_current_order();
        if paths.is_empty() {
            return;
        }

        self.start_rename_dialog_for_paths(paths);
    }

    pub(crate) fn start_rename_dialog_for_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        let items = paths
            .into_iter()
            .map(|path| RenameDialogItemState {
                draft_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
                original_path: path,
            })
            .collect();

        self.rename_overlay = Some(RenameOverlayState {
            items,
            error_message: None,
            just_opened: true,
        });
        self.file_action_menu = None;
        self.pending_exit_confirmation = false;
        self.show_controls = true;
        self.controls_show_time = Instant::now();
    }

    pub(crate) fn rename_temp_path(original_path: &Path, serial: usize) -> PathBuf {
        let parent = original_path.parent().unwrap_or_else(|| Path::new("."));
        let stem = original_path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("file");
        let extension = original_path.extension().and_then(|ext| ext.to_str());

        for attempt in 0..1024usize {
            let suffix = format!("riv-rename-{}-{}-tmp", std::process::id(), serial + attempt);
            let candidate_name = if let Some(extension) = extension {
                format!("{}.{}.{}", stem, suffix, extension)
            } else {
                format!("{}.{}", stem, suffix)
            };
            let candidate = parent.join(candidate_name);
            if !candidate.exists() {
                return candidate;
            }
        }

        parent.join(format!(
            "riv-rename-fallback-{}-{}",
            std::process::id(),
            serial
        ))
    }

    pub(crate) fn cancel_inline_rename(&mut self) {
        self.rename_overlay = None;
        self.modal_thumbnail_cache.clear();
    }

    pub(crate) fn validate_rename_draft(draft_name: &str) -> Result<(), String> {
        if draft_name.trim().is_empty() {
            return Err("File name cannot be empty".to_string());
        }

        if draft_name == "." || draft_name == ".." {
            return Err("File name is not valid".to_string());
        }

        if draft_name.contains('\\') || draft_name.contains('/') {
            return Err("Use a file name only, not a path".to_string());
        }

        Ok(())
    }

    pub(crate) fn commit_inline_rename(&mut self) {
        let Some(state) = self.rename_overlay.clone() else {
            return;
        };

        if state.items.is_empty() {
            self.rename_overlay = None;
            return;
        }

        let mut planned_paths = Vec::with_capacity(state.items.len());
        let original_paths: HashSet<PathBuf> = state
            .items
            .iter()
            .map(|item| item.original_path.clone())
            .collect();
        let mut seen_targets = HashSet::with_capacity(state.items.len());

        for item in &state.items {
            if let Err(err) = Self::validate_rename_draft(&item.draft_name) {
                if let Some(rename_state) = self.rename_overlay.as_mut() {
                    rename_state.error_message = Some(err);
                }
                return;
            }

            let Some(parent) = item.original_path.parent() else {
                if let Some(rename_state) = self.rename_overlay.as_mut() {
                    rename_state.error_message =
                        Some("Cannot rename a path without a parent folder".to_string());
                }
                return;
            };

            let new_path = parent.join(&item.draft_name);
            if !seen_targets.insert(new_path.clone()) {
                if let Some(rename_state) = self.rename_overlay.as_mut() {
                    rename_state.error_message =
                        Some("Two renamed files would end up with the same name".to_string());
                }
                return;
            }

            if new_path.exists()
                && !original_paths.contains(&new_path)
                && new_path != item.original_path
            {
                if let Some(rename_state) = self.rename_overlay.as_mut() {
                    rename_state.error_message =
                        Some("A file with that name already exists".to_string());
                }
                return;
            }

            planned_paths.push((item.original_path.clone(), new_path));
        }

        let changed_paths: Vec<(PathBuf, PathBuf)> = planned_paths
            .iter()
            .filter(|(original_path, new_path)| original_path != new_path)
            .cloned()
            .collect();
        if changed_paths.is_empty() {
            self.rename_overlay = None;
            return;
        }

        let current_path_before = self.current_media_path();
        let staged_paths: Vec<(PathBuf, PathBuf, PathBuf)> = changed_paths
            .iter()
            .enumerate()
            .map(|(serial, (original_path, new_path))| {
                (
                    original_path.clone(),
                    Self::rename_temp_path(original_path.as_path(), serial),
                    new_path.clone(),
                )
            })
            .collect();

        for (original_path, temp_path, _) in &staged_paths {
            if let Err(err) = fs::rename(original_path, temp_path) {
                for (rollback_original, rollback_temp, _) in &staged_paths {
                    if rollback_temp.exists() {
                        let _ = fs::rename(rollback_temp, rollback_original);
                    }
                }

                if let Some(rename_state) = self.rename_overlay.as_mut() {
                    rename_state.error_message = Some(format!("Rename failed: {err}"));
                }
                return;
            }
        }

        let mut completed_final_paths: Vec<(PathBuf, PathBuf)> =
            Vec::with_capacity(staged_paths.len());
        for (original_path, temp_path, new_path) in &staged_paths {
            if let Err(err) = fs::rename(temp_path, new_path) {
                for (completed_original, completed_new) in completed_final_paths.iter().rev() {
                    let _ = fs::rename(completed_new, completed_original);
                }
                for (rollback_original, rollback_temp, _) in &staged_paths {
                    if rollback_temp.exists() {
                        let _ = fs::rename(rollback_temp, rollback_original);
                    }
                }

                if let Some(rename_state) = self.rename_overlay.as_mut() {
                    rename_state.error_message = Some(format!("Rename failed: {err}"));
                }
                return;
            }

            completed_final_paths.push((original_path.clone(), new_path.clone()));
        }

        let mut prepared_clipboard_changed = false;
        let mut stash_changed = false;
        for (original_path, new_path) in &changed_paths {
            if self.marked_files.remove(original_path) {
                self.marked_files.insert(new_path.clone());
            }
            if self.stash_rename_path(original_path, new_path) {
                stash_changed = true;
            }
            if let Some(operation) = self.prepared_clipboard_paths.remove(original_path) {
                self.prepared_clipboard_paths
                    .insert(new_path.clone(), operation);
                prepared_clipboard_changed = true;
            }
            self.modal_thumbnail_cache.remove(original_path);
        }

        if prepared_clipboard_changed {
            self.sync_prepared_clipboard_with_system();
        }
        if stash_changed {
            self.save_stash();
        }

        self.rename_overlay = None;
        self.modal_thumbnail_cache.clear();

        let renamed_current = current_path_before.as_ref().and_then(|current_path| {
            changed_paths
                .iter()
                .find(|(original_path, _)| original_path == current_path)
                .map(|(_, new_path)| new_path.clone())
        });

        if let Some(new_current_path) = renamed_current {
            if self.mode().is_solo() {
                self.load_media(&new_current_path);
            } else {
                self.refresh_media_list_after_path_mutation(Some(new_current_path));
            }
        } else {
            self.refresh_media_list_after_path_mutation(current_path_before);
        }
    }

    pub(crate) fn collect_keyboard_file_action_targets(&self) -> Vec<PathBuf> {
        let marked_paths = self.collect_marked_paths_in_current_order();
        if !marked_paths.is_empty() {
            return marked_paths;
        }

        self.current_media_path()
            .filter(|path| path.exists())
            .into_iter()
            .collect()
    }
}

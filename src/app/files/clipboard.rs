//! Cut/copy/paste plumbing over the media list.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn collect_keyboard_clipboard_targets(
        &mut self,
        ctx: &egui::Context,
    ) -> Vec<PathBuf> {
        let marked_paths = self.collect_marked_paths_in_current_order();
        if !marked_paths.is_empty() {
            return marked_paths;
        }

        if let Some(path) = self
            .hovered_manga_index_from_pointer(ctx)
            .and_then(|index| self.image_list.get(index))
            .filter(|path| path.exists())
            .cloned()
        {
            return vec![path];
        }

        self.current_media_path()
            .filter(|path| path.exists())
            .into_iter()
            .collect()
    }

    pub(crate) fn apply_clipboard_operation_to_paths(
        &mut self,
        paths: Vec<PathBuf>,
        operation: FileClipboardOperation,
    ) {
        if paths.is_empty() {
            return;
        }

        self.file_action_menu = None;
        if let Err(err) = write_shell_file_list_to_clipboard(&paths, operation) {
            self.error_message = Some(err);
        } else {
            if operation == FileClipboardOperation::Cut {
                self.release_video_resources_for_paths(&paths);
            }
            for path in &paths {
                if path.exists() {
                    self.marked_files.insert(path.clone());
                }
            }
            self.stash_remember_paths(&paths);
            self.set_prepared_clipboard_targets(&paths, operation);
        }
    }

    pub(crate) fn request_delete_for_paths(&mut self, paths: Vec<PathBuf>) {
        let existing_paths: Vec<PathBuf> = paths.into_iter().filter(|path| path.exists()).collect();
        if existing_paths.is_empty() {
            self.pending_single_delete_target = None;
            self.pending_marked_delete_targets.clear();
            self.clear_stale_marked_files();
            self.clear_stale_prepared_clipboard_paths();
            return;
        }

        self.file_action_menu = None;
        self.pending_single_delete_target = None;
        self.pending_marked_delete_targets.clear();
        // Nothing is released here: the video player for the target is only torn down once
        // `perform_delete_targets` actually moves the files. Cancelling the confirmation must
        // leave the viewer exactly as it was.

        if self.config.confirm_delete_to_recycle_bin {
            // A fresh window re-centers and takes the keyboard on its first frames.
            self.delete_dialog_place_frames = DIALOG_WINDOW_PLACE_FRAMES;
            if existing_paths.len() == 1 {
                self.pending_single_delete_target = existing_paths.into_iter().next();
            } else {
                self.pending_marked_delete_targets = existing_paths;
            }
        } else {
            self.perform_delete_targets(existing_paths);
        }
    }

    pub(crate) fn apply_clipboard_operation_to_single_file(
        &mut self,
        index: usize,
        operation: FileClipboardOperation,
    ) {
        let Some(path) = self.image_list.get(index).cloned() else {
            return;
        };

        self.apply_clipboard_operation_to_paths(vec![path], operation);
    }

    pub(crate) fn apply_clipboard_operation_to_marked_files(
        &mut self,
        operation: FileClipboardOperation,
    ) {
        let marked_paths = self.collect_marked_paths_in_current_order();
        if marked_paths.is_empty() {
            return;
        }

        self.apply_clipboard_operation_to_paths(marked_paths, operation);
    }

    pub(crate) fn request_single_file_delete(&mut self, index: usize) {
        let Some(path) = self.image_list.get(index).cloned() else {
            return;
        };

        self.request_delete_for_paths(vec![path]);
    }

    pub(crate) fn request_marked_files_delete(&mut self) {
        let marked_paths = self.collect_marked_paths_in_current_order();
        if marked_paths.is_empty() {
            return;
        }

        self.request_delete_for_paths(marked_paths);
    }

    pub(crate) fn request_paste_marked_files_into_current_folder(&mut self) {
        let file_list = match read_shell_file_list_from_clipboard() {
            Ok(list) => list,
            Err(err) => {
                self.error_message = Some(err);
                return;
            }
        };
        if file_list.is_empty() {
            return;
        }

        let operation = match read_drop_effect_from_clipboard() {
            Ok(op) => op,
            Err(_) => FileClipboardOperation::Copy,
        };

        let target_directory = if self.mode().is_strip() && !self.image_list.is_empty() {
            if !self.refresh_media_list_before_masonry_entry() {
                if let Some(path) = self.current_media_path() {
                    let dir = path.parent().unwrap_or(path.as_path()).to_path_buf();
                    if dir.exists() && dir.is_dir() {
                        dir
                    } else {
                        if let Some(first) = self.image_list.first().cloned() {
                            first.parent().unwrap_or(first.as_path()).to_path_buf()
                        } else {
                            return;
                        }
                    }
                } else if let Some(first) = self.image_list.first().cloned() {
                    first.parent().unwrap_or(first.as_path()).to_path_buf()
                } else {
                    return;
                }
            } else {
                if let Some(path) = self.current_media_path() {
                    path.parent().unwrap_or(path.as_path()).to_path_buf()
                } else if let Some(first) = self.image_list.first().cloned() {
                    first.parent().unwrap_or(first.as_path()).to_path_buf()
                } else {
                    return;
                }
            }
        } else if let Some(current_path) = self.current_media_path() {
            current_path
                .parent()
                .unwrap_or(current_path.as_path())
                .to_path_buf()
        } else if !self.image_list.is_empty() {
            if let Some(first) = self.image_list.first().cloned() {
                first.parent().unwrap_or(first.as_path()).to_path_buf()
            } else {
                return;
            }
        } else {
            return;
        };

        let mut new_paths: Vec<PathBuf> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for source_path in &file_list {
            if !source_path.exists() {
                continue;
            }

            let file_name = match source_path.file_name() {
                Some(name) => name,
                None => continue,
            };

            let mut dest_path = target_directory.join(file_name);
            let mut suffix = 1;
            while dest_path.exists() {
                let stem = source_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file");
                let ext = source_path.extension().and_then(|e| e.to_str());
                let new_name = if let Some(ext) = ext {
                    format!("{} ({}).{}", stem, suffix, ext)
                } else {
                    format!("{} ({})", stem, suffix)
                };
                dest_path = target_directory.join(&new_name);
                suffix += 1;
                if suffix > 1000 {
                    break;
                }
            }

            match operation {
                FileClipboardOperation::Copy => match std::fs::copy(source_path, &dest_path) {
                    Ok(_) => new_paths.push(dest_path),
                    Err(err) => errors.push(format!(
                        "Failed to copy '{}': {}",
                        file_name.to_string_lossy(),
                        err
                    )),
                },
                FileClipboardOperation::Cut => match std::fs::rename(source_path, &dest_path) {
                    Ok(_) => new_paths.push(dest_path.clone()),
                    Err(_) => match std::fs::copy(source_path, &dest_path) {
                        Ok(_) => {
                            if let Err(e) = std::fs::remove_file(source_path) {
                                errors.push(format!(
                                    "Copied '{}' but failed to remove original: {}",
                                    file_name.to_string_lossy(),
                                    e
                                ));
                            } else {
                                new_paths.push(dest_path.clone());
                            }
                        }
                        Err(copy_err) => errors.push(format!(
                            "Failed to move '{}': {}",
                            file_name.to_string_lossy(),
                            copy_err
                        )),
                    },
                },
            }
        }

        if !errors.is_empty() {
            self.error_message = Some(errors.join("\n"));
        }

        if !new_paths.is_empty() {
            if self.config.auto_unmark_after_paste {
                self.unmark_all_files();
            } else {
                self.clear_stale_marked_files();
                self.clear_stale_prepared_clipboard_paths();
            }
            let _ = clear_system_clipboard();

            // FIX: Use the robust synchronous refresh we just fixed for deletions
            let preferred_anchor = self.current_media_path();
            self.refresh_media_list_after_path_mutation(preferred_anchor);
        }
    }

    pub(crate) fn perform_delete_targets(&mut self, paths: Vec<PathBuf>) {
        let existing_paths: Vec<PathBuf> = paths.into_iter().filter(|path| path.exists()).collect();
        if existing_paths.is_empty() {
            self.pending_single_delete_target = None;
            self.pending_marked_delete_targets.clear();
            self.clear_stale_marked_files();
            self.clear_stale_prepared_clipboard_paths();
            return;
        }

        let removed_paths: HashSet<PathBuf> = existing_paths.iter().cloned().collect();
        let current_path_before = self.current_media_path();
        let fallback_path = self.choose_fallback_path_after_removal(&removed_paths);

        self.release_video_resources_for_paths(&existing_paths);

        // Captured before the deletion: the refresh that follows prunes the stash entries of
        // the files that are gone, so undo has to know which of them it owes the stash.
        let stashed_paths: Vec<PathBuf> = existing_paths
            .iter()
            .filter(|path| self.stash.entries.contains_key(*path))
            .cloned()
            .collect();

        match move_paths_to_recycle_bin(&existing_paths) {
            Ok(()) => {
                // Recorded before the view moves on: the entry is what Ctrl+Z restores.
                self.history_record(HistoryAction::DeleteToRecycle {
                    paths: existing_paths.clone(),
                    stashed: stashed_paths,
                });

                let mut prepared_clipboard_changed = false;
                for path in &existing_paths {
                    self.marked_files.remove(path);
                    if self.clear_prepared_clipboard_for_path(path) {
                        prepared_clipboard_changed = true;
                    }
                    self.modal_thumbnail_cache.remove(path);
                }

                if prepared_clipboard_changed {
                    self.sync_prepared_clipboard_with_system();
                }

                self.pending_single_delete_target = None;
                self.pending_marked_delete_targets.clear();
                self.rename_overlay = None;

                let removed_current = current_path_before
                    .as_ref()
                    .is_some_and(|current| removed_paths.contains(current));
                let refresh_anchor = fallback_path.clone().or(current_path_before.clone());

                if removed_current && self.mode().is_solo() {
                    if let Some(path) = refresh_anchor {
                        self.refresh_media_list_after_path_mutation(Some(path.clone()));
                        if self.image_list.iter().any(|candidate| candidate == &path) {
                            self.load_media(&path);
                        } else {
                            self.clear_current_media_after_all_files_removed();
                        }
                    } else {
                        self.clear_current_media_after_all_files_removed();
                    }
                } else {
                    self.refresh_media_list_after_path_mutation(refresh_anchor);
                }
            }
            Err(err) => {
                self.error_message = Some(err);
            }
        }
    }
}

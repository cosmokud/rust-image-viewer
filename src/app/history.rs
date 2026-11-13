//! Undo/redo of the file mutations the app performs.
//!
//! The history is RAM only: it is gone when the process is, and it never touches the .exe or the
//! settings. Every entry is applied only after its confirmation dialog has listed the files
//! involved, and an entry that can no longer be applied — a deleted file that has left the
//! Recycle Bin, a name that has since been taken — is reported and dropped, so the next Ctrl+Z
//! reaches the action before it.
//!
//! Both stacks hold the same kind of action, so an undo and the redo that follows it are the same
//! value moved between the stacks.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// How many mutations the history keeps. Anything older cannot be reached from the keyboard
/// anyway, and each entry holds paths.
pub(crate) const HISTORY_LIMIT: usize = 64;

impl ImageViewer {
    /// Records one applied mutation and drops the redo branch: a new action makes the future that
    /// was branched off unreachable, exactly like an editor.
    pub(crate) fn history_record(&mut self, action: HistoryAction) {
        // A confirmed entry is being applied: the mutations it performs are that entry, not new
        // user actions, so they must not push anything onto the stacks.
        if self.history_applying {
            return;
        }
        self.redo_history.clear();
        self.undo_history.push(action);
        if self.undo_history.len() > HISTORY_LIMIT {
            self.undo_history.remove(0);
        }
    }

    /// Ctrl+Z / Ctrl+Y, handled before the ordinary bindings so no mode can claim them.
    /// Returns true when the key press was consumed.
    pub(crate) fn try_handle_history_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.history_confirm.is_some() {
            return false;
        }

        let (undo, redo) = ctx.input(|input| {
            let primary = input.modifiers.command || input.modifiers.ctrl;
            (
                primary && !input.modifiers.shift && input.key_pressed(egui::Key::Z),
                primary && input.key_pressed(egui::Key::Y),
            )
        });

        if undo {
            self.request_history(HistoryDirection::Undo);
            return true;
        }
        if redo {
            self.request_history(HistoryDirection::Redo);
            return true;
        }
        false
    }

    /// Opens the confirmation for the top of one stack. Nothing is applied here.
    fn request_history(&mut self, direction: HistoryDirection) {
        let action = match direction {
            HistoryDirection::Undo => self.undo_history.last(),
            HistoryDirection::Redo => self.redo_history.last(),
        }
        .cloned();

        let Some(action) = action else {
            self.error_message = Some(
                match direction {
                    HistoryDirection::Undo => "Nothing to undo",
                    HistoryDirection::Redo => "Nothing to redo",
                }
                .to_string(),
            );
            return;
        };

        // The dialog can say in advance which deleted files will not come back.
        let unrestorable = match (&action, direction) {
            (HistoryAction::DeleteToRecycle { paths, .. }, HistoryDirection::Undo) => {
                recycle_bin_missing_paths(paths)
            }
            _ => Vec::new(),
        };

        self.history_confirm = Some(HistoryConfirmState {
            direction,
            action,
            place_frames: DIALOG_WINDOW_PLACE_FRAMES,
            unrestorable,
        });
        self.file_action_menu = None;
    }

    /// Applies the confirmed entry and moves it to the opposite stack.
    ///
    /// The entry leaves the stack it came from before anything runs: an action that cannot be
    /// applied is reported and forgotten, never retried forever.
    pub(crate) fn commit_history_confirmation(&mut self, direction: HistoryDirection) {
        let action = match direction {
            HistoryDirection::Undo => self.undo_history.pop(),
            HistoryDirection::Redo => self.redo_history.pop(),
        };
        let Some(action) = action else {
            return;
        };

        self.history_applying = true;
        let errors = self.apply_history_action(direction, &action);
        self.history_applying = false;

        if errors.is_empty() {
            match direction {
                HistoryDirection::Undo => self.redo_history.push(action),
                HistoryDirection::Redo => self.undo_history.push(action),
            }
        } else {
            self.error_message = Some(errors.join("\n"));
        }
    }

    /// Brings every cached view of the restored files back in line, then refreshes the windows
    /// that show them: the solo view, the strip/grid list, and the Stash gallery with its
    /// details column.
    fn restored_paths_changed_every_window(&mut self, restored: &[PathBuf], stashed: &[PathBuf]) {
        for path in restored {
            self.unloadable_files.remove(path);
            self.modal_thumbnail_cache.remove(path);
        }
        if self
            .stash
            .details_preview
            .as_ref()
            .is_some_and(|preview| restored.contains(&preview.path))
        {
            self.stash.details_preview = None;
        }
        if self
            .stash
            .details_preview_pending
            .as_ref()
            .is_some_and(|(path, _, _)| restored.contains(path))
        {
            self.stash.details_preview_pending = None;
        }

        // The stash entry was pruned when its file disappeared; the file is back, so is the
        // entry, with the same captured metadata the rest of the gallery shows.
        let stashed_back: Vec<PathBuf> = stashed
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect();
        if !stashed_back.is_empty() {
            self.stash_remember_paths(&stashed_back);
            self.stash.selected = stashed_back.first().cloned();
            self.stash.selection = stashed_back.iter().cloned().collect();
            self.stash.selection_anchor = stashed_back.first().cloned();
            self.stash.scroll_to_selection = true;
        }

        // The viewer had moved to another file (or to nothing) when these were deleted: it goes
        // back to the restored one instead of showing a list that no longer matches the disk.
        let current_missing = self.current_media_path().is_none_or(|path| !path.exists());
        self.refresh_media_list_after_path_mutation(restored.first().cloned());

        if current_missing {
            let target = restored
                .iter()
                .find(|path| self.image_list.iter().any(|candidate| candidate == *path))
                .cloned();
            if let Some(path) = target {
                if self.mode().is_strip() {
                    if let Some(index) = self
                        .image_list
                        .iter()
                        .position(|candidate| candidate == &path)
                    {
                        self.set_current_index_clamped(index);
                        self.scroll_strip_to_current_index();
                        self.manga_update_preload_queue();
                    }
                } else {
                    self.load_media(&path);
                }
            }
        }

        self.needs_repaint = true;
    }

    /// One mutation applied in one direction. Returns one message per file that could not be
    /// touched; the rest of the entry is applied regardless.
    fn apply_history_action(
        &mut self,
        direction: HistoryDirection,
        action: &HistoryAction,
    ) -> Vec<String> {
        match (action, direction) {
            (HistoryAction::DeleteToRecycle { paths, stashed }, HistoryDirection::Undo) => {
                let (restored, errors) = restore_paths_from_recycle_bin(paths);
                if !restored.is_empty() {
                    self.restored_paths_changed_every_window(&restored, stashed);
                }
                errors
            }
            (HistoryAction::DeleteToRecycle { paths, .. }, HistoryDirection::Redo) => {
                // `perform_delete_targets` is the one place a deletion refreshes the view; the
                // applying guard keeps it from recording itself as a new action.
                let error_before = self.error_message.take();
                self.perform_delete_targets(paths.clone());
                match self.error_message.take() {
                    Some(error) => vec![error],
                    None => {
                        self.error_message = error_before;
                        Vec::new()
                    }
                }
            }
            (HistoryAction::DeleteFromStash { paths }, HistoryDirection::Undo) => {
                self.stash_remember_paths(paths);
                let missing: Vec<String> = paths
                    .iter()
                    .filter(|path| !path.exists())
                    .map(|path| format!("{} no longer exists", path.display()))
                    .collect();
                if missing.len() < paths.len() {
                    self.stash.selected = paths.first().cloned();
                    self.stash.selection = paths.iter().cloned().collect();
                    self.stash.selection_anchor = paths.first().cloned();
                    self.stash.scroll_to_selection = true;
                }
                missing
            }
            (HistoryAction::DeleteFromStash { paths }, HistoryDirection::Redo) => {
                self.remove_paths_from_stash(paths);
                Vec::new()
            }
            (HistoryAction::Rename { pairs }, direction) => {
                let backwards = direction == HistoryDirection::Undo;
                let mut errors = Vec::new();
                let mut last_completed = None;

                for (before, after) in pairs {
                    let (from, to) = if backwards {
                        (after, before)
                    } else {
                        (before, after)
                    };
                    if !from.exists() {
                        errors.push(format!("{} no longer exists", from.display()));
                        continue;
                    }
                    if to.exists() {
                        errors.push(format!("{} already exists", to.display()));
                        continue;
                    }
                    if let Err(err) = fs::rename(from, to) {
                        errors.push(format!("{}: {err}", from.display()));
                        continue;
                    }

                    if self.marked_files.remove(from) {
                        self.marked_files.insert(to.clone());
                    }
                    if self.stash_rename_path(from, to) {
                        self.save_stash();
                    }
                    self.modal_thumbnail_cache.remove(from);
                    last_completed = Some(to.clone());
                }

                if let Some(anchor) = last_completed {
                    self.refresh_media_list_after_path_mutation(Some(anchor));
                }
                errors
            }
        }
    }
}

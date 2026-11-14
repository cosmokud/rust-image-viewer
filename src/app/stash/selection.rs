//! Stash multi-selection: click, Shift/Ctrl click, rubber band, and the bulk actions that both
//! the chips and the right-click menu run.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Selected paths in gallery order, so bulk actions follow what the user sees.
    pub(crate) fn stash_selected_paths(&self) -> Vec<PathBuf> {
        let mut ordered: Vec<PathBuf> = self
            .stash
            .layout
            .paths
            .iter()
            .filter(|path| self.stash.selection.contains(*path))
            .cloned()
            .collect();
        let mut extras: Vec<PathBuf> = self
            .stash
            .selection
            .iter()
            .filter(|path| !self.stash.layout.paths.contains(*path))
            .cloned()
            .collect();
        extras.sort();
        ordered.extend(extras);
        ordered
    }

    pub(crate) fn stash_selection_active(&self) -> bool {
        !self.stash.selection.is_empty()
    }

    /// Copied/cut badge for a stash tile. Every stashed file is marked by definition, so the
    /// plain marked visual is left out: only the clipboard state is worth a chip here.
    pub(crate) fn stash_clipboard_mark_visual(&self, path: &PathBuf) -> Option<FileMarkVisual> {
        match self.prepared_clipboard_operation_for_path(path.as_path()) {
            Some(FileClipboardOperation::Copy) => Some(FileMarkVisual::Copied),
            Some(FileClipboardOperation::Cut) => Some(FileMarkVisual::Cut),
            _ => None,
        }
    }

    /// Explorer semantics: plain click replaces, Shift extends from the anchor, Ctrl toggles.
    pub(crate) fn stash_click_path(&mut self, path: &PathBuf, modifiers: egui::Modifiers) {
        let additive = modifiers.command || modifiers.ctrl;

        if modifiers.shift {
            if let Some(anchor) = self.stash.selection_anchor.clone() {
                if let (Some(anchor_index), Some(target_index)) = (
                    self.stash.layout.paths.iter().position(|p| p == &anchor),
                    self.stash.layout.paths.iter().position(|p| p == path),
                ) {
                    let (start, end) = if anchor_index <= target_index {
                        (anchor_index, target_index)
                    } else {
                        (target_index, anchor_index)
                    };
                    self.stash.selection = self.stash.layout.paths[start..=end]
                        .iter()
                        .cloned()
                        .collect();
                    self.stash.selected = Some(path.clone());
                    return;
                }
            }
        }

        if additive {
            if !self.stash.selection.remove(path) {
                self.stash.selection.insert(path.clone());
                self.stash.selected = Some(path.clone());
            } else if self.stash.selected.as_ref() == Some(path) {
                self.stash.selected = self.stash.selection.iter().next().cloned();
            }
            self.stash.selection_anchor = Some(path.clone());
            return;
        }

        self.stash.selection.clear();
        self.stash.selection.insert(path.clone());
        self.stash.selected = Some(path.clone());
        self.stash.selection_anchor = Some(path.clone());
    }

    pub(crate) fn stash_select_all(&mut self) {
        self.stash.selection = self.stash.layout.paths.iter().cloned().collect();
        if self.stash.selected.is_none() {
            self.stash.selected = self.stash.layout.paths.first().cloned();
        }
        self.stash.selection_anchor = self.stash.selected.clone();
    }

    pub(crate) fn stash_clear_selection(&mut self) {
        self.stash.selection.clear();
    }

    /// Turns the rubber band into a selection. Called on release; a band that never moved is a
    /// click and leaves the selection to the tile.
    pub(crate) fn stash_apply_selection_box(&mut self, additive: bool) {
        let Some(box_state) = self.stash.selection_box.take() else {
            return;
        };
        if !box_state.is_drag() {
            return;
        }

        let band = box_state.rect();
        if !additive {
            self.stash.selection.clear();
        }
        for (path, tile_rect) in self.stash.layout.tile_rects.clone() {
            if tile_rect.intersects(band) {
                self.stash.selection.insert(path);
            }
        }
        self.stash.selected = self.stash_selected_paths().into_iter().next();
        self.stash.selection_anchor = self.stash.selected.clone();
    }

    /// Chips: nothing at all until a file is selected, then the whole set.
    pub(crate) fn stash_chip_actions(&self) -> Vec<StashAction> {
        if !self.stash_selection_active() {
            return Vec::new();
        }
        let mut actions = vec![StashAction::SelectAll, StashAction::DeselectAll];
        actions.extend(StashAction::SELECTION_ACTIONS);
        actions
    }

    /// Menu: Select All stays reachable with an empty selection; the rest needs one.
    pub(crate) fn stash_menu_actions(&self) -> Vec<StashAction> {
        let mut actions = vec![StashAction::SelectAll];
        if self.stash_selection_active() {
            actions.push(StashAction::DeselectAll);
            actions.extend(StashAction::SELECTION_ACTIONS);
        }
        actions
    }

    /// One dispatch for the chips and the context menu, so both stay identical. Every arm calls
    /// the same routine the file action menu uses: rename opens the rename dialog, Delete opens
    /// the delete confirmation, Copy/Cut go through the shell clipboard path.
    pub(crate) fn stash_run_action(&mut self, _ctx: &egui::Context, action: StashAction) {
        let paths = self.stash_selected_paths();

        match action {
            StashAction::SelectAll => self.stash_select_all(),
            StashAction::DeselectAll => self.stash_clear_selection(),
            StashAction::DeleteFromStash => {
                if paths.is_empty() {
                    return;
                }
                self.remove_paths_from_stash(&paths);
            }
            StashAction::Copy => {
                if paths.is_empty() {
                    return;
                }
                self.apply_clipboard_operation_to_paths(paths, FileClipboardOperation::Copy);
            }
            StashAction::Cut => {
                if paths.is_empty() {
                    return;
                }
                self.apply_clipboard_operation_to_paths(paths, FileClipboardOperation::Cut);
            }
            StashAction::Delete => {
                if paths.is_empty() {
                    return;
                }
                self.request_delete_for_paths(paths);
            }
            StashAction::Rename => {
                if paths.is_empty() {
                    return;
                }
                self.start_rename_dialog_for_paths(paths);
            }
        }
    }
}

pub(crate) fn stash_action_tooltip(action: StashAction) -> &'static str {
    match action {
        StashAction::SelectAll => "Select every file in the stash",
        StashAction::DeselectAll => "Clear the selection",
        StashAction::DeleteFromStash => "Remove from the stash (the file itself is kept)",
        StashAction::Copy => "Copy the selected files to the clipboard",
        StashAction::Cut => "Cut the selected files to the clipboard",
        StashAction::Delete => "Delete the selected files to the Recycle Bin",
        StashAction::Rename => "Rename the selected files",
    }
}

pub(crate) fn stash_action_icon(action: StashAction) -> MenuActionIcon {
    match action {
        StashAction::SelectAll => MenuActionIcon::MarkAll,
        StashAction::DeselectAll => MenuActionIcon::Deselect,
        StashAction::DeleteFromStash => MenuActionIcon::StashRemove,
        StashAction::Copy => MenuActionIcon::Copy,
        StashAction::Cut => MenuActionIcon::Cut,
        StashAction::Delete => MenuActionIcon::Delete,
        StashAction::Rename => MenuActionIcon::Rename,
    }
}

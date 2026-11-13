//! Keyboard navigation across the stash grid.
//!
//! Left/Right walk the flat order, Up/Down jump one row at a time, Page Up/Page Down jump one
//! visible screen, Home/End go to the first and last file, Enter opens the selection in the
//! viewer and Delete drops it from the stash. Ctrl+A selects every visible file; Ctrl+Shift+A
//! clears the selection (Esc belongs to the menu and the window, and Ctrl+D is a text-field
//! shortcut elsewhere in the app).

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn handle_stash_keyboard(&mut self, ctx: &egui::Context) {
        // The filter box owns the arrows while it has focus.
        if ctx.memory(|memory| memory.focused().is_some()) {
            return;
        }

        let layout = self.stash.layout.clone();
        if layout.paths.is_empty() {
            return;
        }

        let columns = layout.columns.max(1) as isize;
        let page = (columns as usize * layout.rows_visible.max(1)).max(1) as isize;
        let last = (layout.paths.len() - 1) as isize;
        let current = self
            .stash
            .selected
            .as_ref()
            .and_then(|selected| layout.paths.iter().position(|path| path == selected))
            .map(|index| index as isize)
            .unwrap_or(0);

        let mut target = current;
        let mut moved = false;
        let mut open_selected = false;
        let mut remove_selected = false;
        let mut select_all = false;
        let mut deselect_all = false;
        ctx.input(|input| {
            // Shift wins: Ctrl+Shift+A clears, Ctrl+A selects.
            if input.key_pressed(egui::Key::A) && (input.modifiers.command || input.modifiers.ctrl)
            {
                if input.modifiers.shift {
                    deselect_all = true;
                } else {
                    select_all = true;
                }
            }
            if input.key_pressed(egui::Key::ArrowRight) {
                target += 1;
                moved = true;
            }
            if input.key_pressed(egui::Key::ArrowLeft) {
                target -= 1;
                moved = true;
            }
            if input.key_pressed(egui::Key::ArrowDown) {
                target += columns;
                moved = true;
            }
            if input.key_pressed(egui::Key::ArrowUp) {
                target -= columns;
                moved = true;
            }
            if input.key_pressed(egui::Key::PageDown) {
                target += page;
                moved = true;
            }
            if input.key_pressed(egui::Key::PageUp) {
                target -= page;
                moved = true;
            }
            if input.key_pressed(egui::Key::Home) {
                target = 0;
                moved = true;
            }
            if input.key_pressed(egui::Key::End) {
                target = last;
                moved = true;
            }
            if input.key_pressed(egui::Key::Enter) {
                open_selected = true;
            }
            if input.key_pressed(egui::Key::Delete) {
                remove_selected = true;
            }
        });

        if select_all {
            self.stash_select_all();
            return;
        }

        if deselect_all {
            self.stash_clear_selection();
            return;
        }

        if open_selected {
            if let Some(path) = self.stash.selected.clone() {
                self.open_stash_path(ctx, &path);
            }
            return;
        }

        if remove_selected {
            let paths = self.stash_selected_paths();
            if !paths.is_empty() {
                self.remove_paths_from_stash(&paths);
            }
            return;
        }

        if !moved {
            return;
        }

        let index = target.clamp(0, last) as usize;
        let path = layout.paths[index].clone();
        if self.stash.selected.as_ref() != Some(&path) {
            self.stash.selected = Some(path.clone());
            self.stash.selection.clear();
            self.stash.selection.insert(path.clone());
            self.stash.selection_anchor = Some(path);
            self.stash.scroll_to_selection = true;
        }
    }
}

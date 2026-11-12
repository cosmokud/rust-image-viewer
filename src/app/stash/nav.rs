//! Keyboard navigation across the stash grid.
//!
//! Left/Right walk the flat order, Up/Down jump one row at a time, Page Up/Page Down jump one
//! visible screen, Home/End go to the first and last file.

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
        ctx.input(|input| {
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
        });

        if !moved {
            return;
        }

        let index = target.clamp(0, last) as usize;
        let path = layout.paths[index].clone();
        if self.stash.selected.as_ref() != Some(&path) {
            self.stash.selected = Some(path);
            self.stash.scroll_to_selection = true;
        }
    }
}

//! Menu rows and the in-view file action buttons.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn render_single_file_action_buttons(
        &mut self,
        ui: &mut egui::Ui,
        target_index: usize,
        current_labels: bool,
    ) -> bool {
        let mut activated = false;

        let is_marked = self.is_index_marked(target_index);
        let mark_label = if current_labels {
            if is_marked {
                "Unmark Current File"
            } else {
                "Mark Current File"
            }
        } else if is_marked {
            "Unmark"
        } else {
            "Mark"
        };
        let mark_icon = if is_marked {
            MenuActionIcon::Unmark
        } else {
            MenuActionIcon::Mark
        };
        if self.menu_action_row(ui, mark_label, mark_icon).clicked() {
            self.toggle_mark_for_index(target_index);
            activated = true;
        }

        let cut_label = if current_labels {
            "Cut Current File"
        } else {
            "Cut"
        };
        if self
            .menu_action_row(ui, cut_label, MenuActionIcon::Cut)
            .clicked()
        {
            self.apply_clipboard_operation_to_single_file(
                target_index,
                FileClipboardOperation::Cut,
            );
            activated = true;
        }

        let copy_label = if current_labels {
            "Copy Current File"
        } else {
            "Copy"
        };
        if self
            .menu_action_row(ui, copy_label, MenuActionIcon::Copy)
            .clicked()
        {
            self.apply_clipboard_operation_to_single_file(
                target_index,
                FileClipboardOperation::Copy,
            );
            activated = true;
        }

        let delete_label = if current_labels {
            "Delete Current File"
        } else {
            "Delete"
        };
        if self
            .menu_action_row(ui, delete_label, MenuActionIcon::Delete)
            .clicked()
        {
            self.request_single_file_delete(target_index);
            activated = true;
        }

        let rename_label = if current_labels {
            "Rename Current File"
        } else {
            "Rename"
        };
        if self
            .menu_action_row(ui, rename_label, MenuActionIcon::Rename)
            .clicked()
        {
            self.start_inline_rename_for_index(target_index);
            activated = true;
        }

        let open_location_label = if current_labels {
            "Open Current File Location"
        } else {
            "Open file location"
        };
        if self
            .menu_action_row(ui, open_location_label, MenuActionIcon::OpenLocation)
            .clicked()
        {
            self.open_file_location_for_index(target_index);
            activated = true;
        }

        activated
    }

    pub(crate) fn render_marked_file_action_buttons(&mut self, ui: &mut egui::Ui) -> bool {
        if self.image_list.is_empty() {
            return false;
        }

        let marked_paths = self.collect_marked_paths_in_current_order();
        let mut activated = false;

        if !marked_paths.is_empty() {
            if self
                .menu_action_row(ui, "Cut Marked Files", MenuActionIcon::Cut)
                .clicked()
            {
                self.apply_clipboard_operation_to_marked_files(FileClipboardOperation::Cut);
                activated = true;
            }
            if self
                .menu_action_row(ui, "Copy Marked Files", MenuActionIcon::Copy)
                .clicked()
            {
                self.apply_clipboard_operation_to_marked_files(FileClipboardOperation::Copy);
                activated = true;
            }
            if self
                .menu_action_row(ui, "Delete Marked Files", MenuActionIcon::Delete)
                .clicked()
            {
                self.request_marked_files_delete();
                activated = true;
            }
            if self
                .menu_action_row(ui, "Rename Marked Files", MenuActionIcon::Rename)
                .clicked()
            {
                self.start_inline_rename_for_marked_files();
                activated = true;
            }
        }
        if self
            .menu_action_row(ui, "Mark All", MenuActionIcon::MarkAll)
            .clicked()
        {
            self.mark_all_files();
            activated = true;
        }
        if !marked_paths.is_empty()
            && self
                .menu_action_row(ui, "Unmark All", MenuActionIcon::Unmark)
                .clicked()
        {
            self.unmark_all_files();
            activated = true;
        }

        activated
    }

    pub(crate) fn window_allows_keyboard_shortcuts(&self, ctx: &egui::Context) -> bool {
        ctx.input(|input| {
            let viewport = input.raw.viewport();
            viewport.focused.unwrap_or(true) && !viewport.minimized.unwrap_or(false)
        })
    }
}

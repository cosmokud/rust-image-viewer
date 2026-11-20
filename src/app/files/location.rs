//! Open-in-explorer, open-with and default-app actions.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn send_outer_position(&mut self, ctx: &egui::Context, pos: egui::Pos2) {
        #[cfg(target_os = "windows")]
        crate::diag::trace(
            "rect",
            &format!(
                "outer position asked={pos:?} hwnd={:?}",
                crate::windows_env::window_rect(self.resolved_main_hwnd())
                    .map(|rect| (rect.left, rect.top)),
            ),
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
    }

    /// Sends a window command to the viewer window, whichever viewport this frame belongs to.
    ///
    /// The Stash and the dialogs run inside their own viewports, where a plain
    /// `ctx.send_viewport_cmd` would move *them*: reopening the viewer from the Stash that way
    /// moved the Stash instead and left the viewer at its default place.
    pub(crate) fn send_viewer_viewport_cmd(
        &self,
        ctx: &egui::Context,
        command: egui::ViewportCommand,
    ) {
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, command);
    }

    /// Reveals `path` in the OS file manager, reporting a failure in the viewer's error banner.
    pub(crate) fn open_file_location_for_path(&mut self, path: &Path) {
        if let Err(e) = reveal_path_in_file_explorer(path) {
            self.error_message = Some(format!(
                "Failed to open file location ({}): {}",
                path.display(),
                e
            ));
        }
    }

    /// Hands `path` to whatever application the OS associates with its extension.
    pub(crate) fn open_in_default_app_for_path(&mut self, path: &Path) {
        if let Err(e) = open_path_in_default_app(path) {
            self.error_message = Some(format!(
                "Failed to open in the default app ({}): {}",
                path.display(),
                e
            ));
        }
    }

    /// Shows the OS "Open with" picker, to open this one file with another application.
    pub(crate) fn open_with_dialog_for_path(&mut self, path: &Path) {
        if let Err(e) = open_path_with_shell_dialog(path) {
            self.error_message = Some(format!(
                "Failed to open the Open With dialog ({}): {}",
                path.display(),
                e
            ));
        }
    }

    /// Viewer menus target a list index; the Stash passes the path it has selected.
    pub(crate) fn open_file_location_for_index(&mut self, target_index: usize) {
        let Some(path) = self.image_list.get(target_index).cloned() else {
            return;
        };
        self.open_file_location_for_path(path.as_path());
    }

    pub(crate) fn open_in_default_app_for_index(&mut self, target_index: usize) {
        let Some(path) = self.image_list.get(target_index).cloned() else {
            return;
        };
        self.open_in_default_app_for_path(path.as_path());
    }

    pub(crate) fn open_with_dialog_for_index(&mut self, target_index: usize) {
        let Some(path) = self.image_list.get(target_index).cloned() else {
            return;
        };
        self.open_with_dialog_for_path(path.as_path());
    }

    pub(crate) fn reset_floating_window_drag_anchor(&mut self) {
        self.floating_drag_start_outer_pos = None;
        self.floating_drag_start_cursor_screen = None;
    }
}

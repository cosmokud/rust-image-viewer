//! Open-in-explorer, open-with and default-app actions.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Frames the "Open with" picker waits for a fullscreen exit before it is shown anyway.
pub(crate) const OPEN_WITH_FULLSCREEN_EXIT_FRAMES: u8 = 3;

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
    ///
    /// The picker is a shell dialog owned by the viewer window; over borderless fullscreen it
    /// belongs to a window the size of the display, so fullscreen is left first and the picker
    /// opens once the window is back to its windowed size.
    pub(crate) fn open_with_dialog_for_path(&mut self, path: &Path) {
        if self.mode().is_fullscreen() {
            self.pending_open_with_path =
                Some((path.to_path_buf(), OPEN_WITH_FULLSCREEN_EXIT_FRAMES));
            self.request_shortcut_fullscreen_toggle();
            return;
        }
        self.show_open_with_dialog(path);
    }

    pub(crate) fn show_open_with_dialog(&mut self, path: &Path) {
        #[cfg(target_os = "windows")]
        let result = open_path_with_shell_dialog(self.resolved_main_hwnd(), path);
        #[cfg(not(target_os = "windows"))]
        let result = open_path_in_default_app(path);

        if let Err(e) = result {
            self.error_message = Some(format!(
                "Failed to open the Open With dialog ({}): {}",
                path.display(),
                e
            ));
        }
    }

    /// Runs a picker that was waiting for a fullscreen exit. Ticked by the frame driver: the
    /// exit needs a few frames to put the window back, and the dialog is owned by it.
    pub(crate) fn drive_pending_open_with(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = self.pending_open_with_path.take() else {
            return;
        };
        if self.mode().is_fullscreen() && frames > 0 {
            self.pending_open_with_path = Some((path, frames - 1));
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        self.show_open_with_dialog(&path);
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

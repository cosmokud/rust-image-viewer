//! Open-in-explorer actions.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn send_outer_position(&mut self, ctx: &egui::Context, pos: egui::Pos2) {
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

    pub(crate) fn reset_floating_window_drag_anchor(&mut self) {
        self.floating_drag_start_outer_pos = None;
        self.floating_drag_start_cursor_screen = None;
    }
}

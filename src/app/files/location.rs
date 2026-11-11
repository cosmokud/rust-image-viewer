//! Open-in-explorer actions.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn send_outer_position(&mut self, ctx: &egui::Context, pos: egui::Pos2) {
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
    }

    pub(crate) fn reset_floating_window_drag_anchor(&mut self) {
        self.floating_drag_start_outer_pos = None;
        self.floating_drag_start_cursor_screen = None;
    }
}

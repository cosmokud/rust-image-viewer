//! The settings window itself.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn apply_pending_window_title(&mut self, ctx: &egui::Context) {
        if let Some(title) = self.pending_window_title.take() {
            let title = self.truncate_window_title_for_viewport(ctx, title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}

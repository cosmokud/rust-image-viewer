//! Right-click menu inside the stash window. Rows are the chips' own actions.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const STASH_MENU_WIDTH: f32 = 200.0;

impl ImageViewer {
    pub(crate) fn draw_stash_context_menu(&mut self, ctx: &egui::Context) {
        let Some(menu_pos) = self.stash.menu_pos else {
            return;
        };

        let actions = self.stash_menu_actions();
        let mut chosen: Option<StashAction> = None;
        let close = ctx.input(|input| input.key_pressed(egui::Key::Escape));

        let area = egui::Area::new(egui::Id::new("stash_context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(menu_pos)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(18, 22, 28, 246))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36),
                    ))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::same(8.0))
                    .show(ui, |ui| {
                        // The rows paint from `available_width`; without this the Area hands
                        // them the whole screen.
                        ui.set_width(STASH_MENU_WIDTH);
                        for action in actions {
                            if self
                                .menu_action_row(ui, action.label(), stash_action_icon(action))
                                .clicked()
                            {
                                chosen = Some(action);
                            }
                        }
                    });
            });

        let menu_rect = area.response.rect;
        let clicked_outside = ctx.input(|input| {
            input.pointer.any_click()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !menu_rect.contains(pos))
        });

        if let Some(action) = chosen {
            self.stash.menu_pos = None;
            self.stash_run_action(ctx, action);
            return;
        }
        if close || clicked_outside {
            self.stash.menu_pos = None;
        }
    }
}

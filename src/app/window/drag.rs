//! Floating window dragging and window moves.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn floating_zoom_inside_window_active(&self, ctx: &egui::Context) -> bool {
        if self.mode().is_fullscreen() {
            return false;
        }

        let Some(display_size) = self.image_display_size_at_zoom() else {
            return false;
        };

        ctx.input(|i| i.raw.viewport().inner_rect)
            .map(|inner_rect| {
                display_size.x > inner_rect.width() + 1.0
                    || display_size.y > inner_rect.height() + 1.0
            })
            .unwrap_or(false)
    }

    /// Window outer position (egui points, as `ViewportCommand::OuterPosition` wants it) for a
    /// floating-window drag, computed entirely in physical screen pixels so a mid-drag
    /// `pixels_per_point` change (monitor with a different DPI scale) cannot leak into the delta.
    pub(crate) fn floating_drag_target_outer_pos(
        start_outer_physical: egui::Pos2,
        start_cursor_physical: egui::Pos2,
        current_cursor_physical: egui::Pos2,
        pixels_per_point: f32,
    ) -> egui::Pos2 {
        let ppp = pixels_per_point.max(0.01);
        (start_outer_physical + (current_cursor_physical - start_cursor_physical)) / ppp
    }

    pub(crate) fn drag_floating_window_without_native_snap(&mut self, ctx: &egui::Context) {
        if self.floating_zoom_inside_window_active(ctx) {
            self.floating_zoom_inside_window_locked = true;
        }

        let Some(current_cursor_physical) = get_global_cursor_pos() else {
            // Platforms without a global cursor query: hand the drag to the windowing system
            // once (it owns the move from there), instead of re-issuing StartDrag every frame.
            if self.floating_drag_start_outer_pos.is_none() {
                let ppp = ctx.pixels_per_point().max(0.01);
                self.floating_drag_start_outer_pos = ctx
                    .input(|i| i.raw.viewport().outer_rect)
                    .map(|r| r.min * ppp);
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                ctx.request_repaint();
            }
            return;
        };

        let (start_outer_physical, start_cursor_physical) = match (
            self.floating_drag_start_outer_pos,
            self.floating_drag_start_cursor_screen,
        ) {
            (Some(outer), Some(cursor)) => (outer, cursor),
            _ => {
                let ppp = ctx.pixels_per_point().max(0.01);
                let outer_pos = ctx
                    .input(|i| i.raw.viewport().outer_rect)
                    .map(|r| r.min * ppp)
                    .unwrap_or(egui::Pos2::ZERO);
                self.floating_drag_start_outer_pos = Some(outer_pos);
                self.floating_drag_start_cursor_screen = Some(current_cursor_physical);
                // Keep repainting while the button is held so the loop keeps sampling the
                // cursor at its own cadence instead of egui's event cadence: that is what
                // keeps the window glued to the cursor when it overshoots the window.
                ctx.request_repaint();
                return;
            }
        };

        let new_pos = Self::floating_drag_target_outer_pos(
            start_outer_physical,
            start_cursor_physical,
            current_cursor_physical,
            ctx.pixels_per_point(),
        );
        self.send_outer_position(ctx, new_pos);
        // A drag that outruns the window can lose pointer events (capture changes, and
        // cursor moves that no longer arrive once the cursor sits outside the window),
        // so keep the loop hot while the drag is alive: each frame re-samples the cursor.
        ctx.request_repaint();
    }

    pub(crate) fn apply_manga_pan_step(&mut self, direction: f32, multiplier: f32) {
        let scroll_amount = self.config.manga_arrow_scroll_speed * 0.5 * multiplier;
        if self.manga_add_scroll_target_delta(direction * scroll_amount) {
            self.manga_update_preload_queue();
        }
    }
}

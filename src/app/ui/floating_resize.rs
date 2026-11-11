//! Floating-window resize handling.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Get cursor icon for resize direction
    pub(crate) fn get_resize_cursor(&self, direction: ResizeDirection) -> egui::CursorIcon {
        match direction {
            ResizeDirection::Left | ResizeDirection::Right => egui::CursorIcon::ResizeHorizontal,
            ResizeDirection::Top | ResizeDirection::Bottom => egui::CursorIcon::ResizeVertical,
            ResizeDirection::TopLeft | ResizeDirection::BottomRight => egui::CursorIcon::ResizeNwSe,
            ResizeDirection::TopRight | ResizeDirection::BottomLeft => egui::CursorIcon::ResizeNeSw,
            ResizeDirection::None => egui::CursorIcon::Default,
        }
    }

    pub(crate) fn resize_floating_keep_aspect(
        &mut self,
        ctx: &egui::Context,
        direction: ResizeDirection,
    ) {
        if self.mode().is_fullscreen() {
            return;
        }
        let Some((media_w_u, media_h_u)) = self.media_display_dimensions() else {
            return;
        };
        if media_w_u == 0 || media_h_u == 0 {
            return;
        }
        let media_w = media_w_u as f32;
        let media_h = media_h_u as f32;
        let aspect = media_w / media_h;

        let Some(current_cursor_screen) = get_global_cursor_pos() else {
            return;
        };

        let (start_outer_pos, start_inner_size, start_cursor_screen) = match (
            self.resize_start_outer_pos,
            self.resize_start_inner_size,
            self.resize_start_cursor_screen,
        ) {
            (Some(p), Some(s), Some(c)) => (p, s, c),
            _ => {
                let outer_pos = ctx
                    .input(|i| i.raw.viewport().outer_rect)
                    .map(|r| r.min)
                    .unwrap_or(egui::Pos2::ZERO);
                let inner_size = ctx
                    .input(|i| i.raw.viewport().inner_rect)
                    .map(|r| r.size())
                    .unwrap_or_else(|| ctx.screen_rect().size());

                self.resize_start_outer_pos = Some(outer_pos);
                self.resize_start_inner_size = Some(inner_size);
                self.resize_start_cursor_screen = Some(current_cursor_screen);
                self.resize_last_size = None;
                return;
            }
        };

        let delta = current_cursor_screen - start_cursor_screen;

        let clamp_min_w: f32 = 200.0;
        let clamp_min_h: f32 = 150.0;
        let max_size = egui::Vec2::new(16000.0, 16000.0);

        let start_left = start_outer_pos.x.round();
        let start_top = start_outer_pos.y.round();
        let start_right = (start_outer_pos.x + start_inner_size.x).round();
        let start_bottom = (start_outer_pos.y + start_inner_size.y).round();
        let start_w = start_right - start_left;
        let start_h = start_bottom - start_top;
        let start_center_x = start_left + start_w * 0.5;
        let start_center_y = start_top + start_h * 0.5;

        let size_from_width = |w: f32| -> (f32, f32) {
            let w = w.clamp(clamp_min_w, max_size.x);
            let h = (w / aspect).clamp(clamp_min_h, max_size.y);
            let w = h * aspect;
            (w.round(), h.round())
        };

        let size_from_height = |h: f32| -> (f32, f32) {
            let h = h.clamp(clamp_min_h, max_size.y);
            let w = (h * aspect).clamp(clamp_min_w, max_size.x);
            let h = w / aspect;
            (w.round(), h.round())
        };

        let (new_w, new_h) = match direction {
            ResizeDirection::Right => size_from_width(start_w + delta.x),
            ResizeDirection::Bottom => size_from_height(start_h + delta.y),
            ResizeDirection::BottomRight => {
                let dx = start_w + delta.x;
                let dy = start_h + delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 {
                    (w1, h1)
                } else {
                    (w2, h2)
                }
            }
            ResizeDirection::Left => size_from_width(start_w - delta.x),
            ResizeDirection::Top => size_from_height(start_h - delta.y),
            ResizeDirection::TopLeft => {
                let dx = start_w - delta.x;
                let dy = start_h - delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 {
                    (w1, h1)
                } else {
                    (w2, h2)
                }
            }
            ResizeDirection::TopRight => {
                let dx = start_w + delta.x;
                let dy = start_h - delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 {
                    (w1, h1)
                } else {
                    (w2, h2)
                }
            }
            ResizeDirection::BottomLeft => {
                let dx = start_w - delta.x;
                let dy = start_h + delta.y;
                let (w1, h1) = size_from_width(dx);
                let (w2, h2) = size_from_height(dy);
                let e1 = (w1 - dx).powi(2) + (h1 - dy).powi(2);
                let e2 = (w2 - dx).powi(2) + (h2 - dy).powi(2);
                if e1 <= e2 {
                    (w1, h1)
                } else {
                    (w2, h2)
                }
            }
            ResizeDirection::None => return,
        };

        let (new_x, new_y) = match direction {
            ResizeDirection::Right => (start_left, start_center_y - new_h * 0.5),
            ResizeDirection::Bottom => (start_center_x - new_w * 0.5, start_top),
            ResizeDirection::BottomRight => (start_left, start_top),
            ResizeDirection::Left => (start_right - new_w, start_center_y - new_h * 0.5),
            ResizeDirection::Top => (start_center_x - new_w * 0.5, start_bottom - new_h),
            ResizeDirection::TopLeft => (start_right - new_w, start_bottom - new_h),
            ResizeDirection::TopRight => (start_left, start_bottom - new_h),
            ResizeDirection::BottomLeft => (start_right - new_w, start_top),
            ResizeDirection::None => return,
        };

        let new_size = egui::Vec2::new(new_w, new_h);
        let new_pos = egui::pos2(new_x, new_y);

        let keep_zoom_inside_window_mode = self
            .image_display_size_at_zoom()
            .and_then(|display_size| {
                ctx.input(|i| i.raw.viewport().inner_rect)
                    .map(|inner_rect| {
                        display_size.x > inner_rect.width() + 1.0
                            || display_size.y > inner_rect.height() + 1.0
                    })
            })
            .unwrap_or(false);

        if keep_zoom_inside_window_mode {
            self.floating_zoom_inside_window_locked = true;
            self.zoom_target = self.zoom;
        } else {
            self.floating_zoom_inside_window_locked = false;
            let new_zoom = self.clamp_zoom((new_w / media_w).min(new_h / media_h));
            self.zoom = new_zoom;
            self.zoom_target = new_zoom;
            self.offset = egui::Vec2::ZERO;
        }
        self.zoom_velocity = 0.0;
        self.resize_last_size = Some(new_size);

        match direction {
            ResizeDirection::BottomRight => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
            }
            _ => {
                self.send_outer_position(ctx, new_pos);
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
            }
        }
    }
}

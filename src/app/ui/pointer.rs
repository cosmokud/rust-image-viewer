//! Cursor autohide tracking and pointer/UI hit tests.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn point_over_current_media(
        &self,
        pos: egui::Pos2,
        screen_rect: egui::Rect,
    ) -> bool {
        self.current_media_rect(screen_rect)
            .is_some_and(|image_rect| image_rect.contains(pos))
    }

    pub(crate) fn strip_item_screen_rect_for_cursor_autohide(
        &mut self,
        index: usize,
    ) -> Option<egui::Rect> {
        if self.mode().is_solo()
            || !self.mode().is_fullscreen()
            || self.mode().is_masonry()
            || index >= self.image_list.len()
        {
            return None;
        }

        let display_width = self.manga_get_image_display_width(index);
        let display_height = self.manga_page_height_cached(index).max(1.0);
        let x = (self.screen_size.x - display_width) * 0.5 + self.offset.x;
        let y = self.manga_page_start_y(index) - self.manga_scroll_offset;

        Some(egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(display_width, display_height),
        ))
    }

    pub(crate) fn mark_chip_label(visual: FileMarkVisual) -> &'static str {
        match visual {
            FileMarkVisual::Preview => "READY",
            FileMarkVisual::Marked => "MARKED",
            FileMarkVisual::Copied => "COPIED",
            FileMarkVisual::Cut => "CUT",
        }
    }

    pub(crate) fn mark_chip_rect_for_cursor_autohide(
        &self,
        ctx: &egui::Context,
        item_rect: egui::Rect,
        visual: FileMarkVisual,
    ) -> egui::Rect {
        let overlay_rect = item_rect.shrink(1.5);
        let label = Self::mark_chip_label(visual);
        let text_size = ctx.fonts(|fonts| {
            fonts
                .layout_no_wrap(
                    label.to_string(),
                    egui::FontId::proportional(11.5),
                    egui::Color32::WHITE,
                )
                .size()
        });

        egui::Rect::from_min_size(
            egui::pos2(overlay_rect.min.x + 10.0, overlay_rect.min.y + 10.0),
            egui::vec2(text_size.x + 18.0, text_size.y + 10.0),
        )
    }

    pub(crate) fn pointer_over_mark_chip_for_cursor_autohide(
        &mut self,
        ctx: &egui::Context,
        pos: egui::Pos2,
        screen_rect: egui::Rect,
    ) -> bool {
        let (item_rect, visual) = if self.mode().is_strip() {
            let Some(index) = self.manga_index_at_screen_pos(pos) else {
                return false;
            };
            let Some(visual) = self.mark_visual_for_index(index) else {
                return false;
            };
            let item_rect = if self.mode().is_masonry() {
                self.masonry_item_screen_rect(index)
            } else {
                self.strip_item_screen_rect_for_cursor_autohide(index)
            };
            let Some(item_rect) = item_rect else {
                return false;
            };
            (item_rect, visual)
        } else {
            let Some(path) = self.image_list.get(self.current_index) else {
                return false;
            };
            let Some(visual) = self.mark_visual_for_path(path) else {
                return false;
            };
            let Some(item_rect) = self.current_media_rect(screen_rect) else {
                return false;
            };
            (item_rect, visual)
        };

        self.mark_chip_rect_for_cursor_autohide(ctx, item_rect, visual)
            .contains(pos)
    }

    pub(crate) fn pointer_over_ui_for_cursor_autohide(
        &mut self,
        ctx: &egui::Context,
        pos: egui::Pos2,
    ) -> bool {
        let screen_rect = ctx.screen_rect();

        if self.pointer_over_shortcut_blocking_ui(Some(pos), screen_rect) {
            return true;
        }

        if self.show_controls && pos.y < self.top_controls_hotzone_height() {
            return true;
        }

        if !self.mode().is_fullscreen()
            && self.get_resize_direction(pos, screen_rect) != ResizeDirection::None
        {
            return true;
        }

        if self.pointer_over_mark_chip_for_cursor_autohide(ctx, pos, screen_rect) {
            return true;
        }

        if self.mode().is_solo() || !self.mode().is_fullscreen() {
            return false;
        }

        let screen_width = screen_rect.width();
        let screen_height = screen_rect.height();
        let total_height = self.manga_total_height();
        let scrollbar_width = 12.0;
        let scrollbar_margin = 8.0;
        let scrollbar_track_height = screen_height - 20.0;
        let scrollbar_track_rect = egui::Rect::from_min_size(
            egui::pos2(screen_width - scrollbar_width - scrollbar_margin, 10.0),
            egui::Vec2::new(scrollbar_width, scrollbar_track_height),
        );

        if scrollbar_track_rect.contains(pos) {
            return true;
        }

        let scrollbar_hover_zone = egui::Rect::from_min_max(
            egui::pos2(
                (screen_width - (scrollbar_width + scrollbar_margin + 40.0)).max(0.0),
                0.0,
            ),
            egui::pos2(screen_width, screen_height),
        );
        let show_scrollbar = total_height > screen_height
            && (self.manga_scrollbar_dragging
                || scrollbar_track_rect.contains(pos)
                || scrollbar_hover_zone.contains(pos));

        if show_scrollbar && !self.image_list.is_empty() {
            let indicator_text = format!("{} / {}", self.current_index + 1, self.image_list.len());
            let indicator_size = ctx.fonts(|fonts| {
                fonts
                    .layout_no_wrap(
                        indicator_text,
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    )
                    .size()
            });
            let indicator_pos = egui::pos2(
                screen_width - scrollbar_width - scrollbar_margin - 60.0,
                screen_height / 2.0,
            );
            let pill_rect = egui::Rect::from_center_size(
                indicator_pos,
                egui::Vec2::new(indicator_size.x + 16.0, 28.0),
            );
            if pill_rect.contains(pos) {
                return true;
            }
        }

        if !self.image_list.is_empty() {
            let page_label_hover_zone = egui::Rect::from_min_max(
                egui::pos2(0.0, (screen_height - 100.0).max(0.0)),
                egui::pos2(screen_width, screen_height),
            );
            if page_label_hover_zone.contains(pos) {
                let indicator_text =
                    format!("{} / {}", self.current_index + 1, self.image_list.len());
                let indicator_size = ctx.fonts(|fonts| {
                    fonts
                        .layout_no_wrap(
                            indicator_text,
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE,
                        )
                        .size()
                });
                let pill_rect = egui::Rect::from_center_size(
                    egui::pos2(screen_width / 2.0, screen_height - 40.0),
                    egui::Vec2::new(indicator_size.x + 24.0, 30.0),
                );
                if pill_rect.contains(pos) {
                    return true;
                }
            }
        }

        false
    }

    pub(crate) fn cursor_autohide_state(
        &mut self,
        ctx: &egui::Context,
    ) -> (bool, Option<Duration>) {
        if self.config.cursor_idle_hide_delay <= 0.0 {
            return (false, None);
        }

        let pointer_pos = ctx.input(|input| input.pointer.hover_pos());
        let Some(pointer_pos) = pointer_pos else {
            return (false, None);
        };

        if self.is_panning
            || self.is_resizing
            || self.is_seeking
            || self.is_volume_dragging
            || self.title_text_dragging
            || self.mark_selection_box.is_some()
            || self.manga_scrollbar_dragging
            || self.manga_autoscroll_active
            || self.manga_zoom_plus_held
            || self.manga_zoom_minus_held
            || self.manga_video_seeking
            || self.manga_video_volume_dragging
            || self.gif_seeking
        {
            return (false, None);
        }

        if self.pointer_over_ui_for_cursor_autohide(ctx, pointer_pos) {
            return (false, None);
        }

        let delay = Duration::from_secs_f32(self.config.cursor_idle_hide_delay);
        let elapsed = self.last_pointer_activity_at.elapsed();
        if elapsed >= delay {
            (true, None)
        } else {
            (false, Some(delay - elapsed))
        }
    }

    pub(crate) fn right_click_black_bar_action(
        &self,
        pos: egui::Pos2,
        screen_rect: egui::Rect,
    ) -> Option<Action> {
        let image_rect = self.current_media_rect(screen_rect)?;

        const MIN_BAR_WIDTH: f32 = 0.5;
        let left_gap = image_rect.min.x - screen_rect.min.x;
        let right_gap = screen_rect.max.x - image_rect.max.x;

        if left_gap > MIN_BAR_WIDTH && pos.x < image_rect.min.x {
            return Some(Action::PreviousImage);
        }
        if right_gap > MIN_BAR_WIDTH && pos.x > image_rect.max.x {
            return Some(Action::NextImage);
        }

        None
    }

    pub(crate) fn request_floating_autosize(&mut self, ctx: &egui::Context) {
        if self.mode().is_fullscreen()
            || self.current_window_is_maximized(ctx)
            || self.is_resizing
            || self.pending_window_resize.is_some()
            || self.floating_drag_start_outer_pos.is_some()
            || self.defer_media_view_reset
        {
            return;
        }

        let Some(mut desired) = self.image_display_size_at_zoom() else {
            return;
        };

        desired.x = desired.x.max(200.0);
        desired.y = desired.y.max(150.0);

        let (current_inner_rect, current_outer_rect) =
            ctx.input(|i| (i.raw.viewport().inner_rect, i.raw.viewport().outer_rect));

        // While the zoomed media already exceeds the current floating viewport, keep this
        // "zoom inside window" mode stable and do not autosize the native window.
        let zoom_inside_current_viewport = current_inner_rect
            .map(|rect| desired.x > rect.width() + 1.0 || desired.y > rect.height() + 1.0)
            .unwrap_or(false);
        if zoom_inside_current_viewport && self.floating_zoom_inside_window_locked {
            return;
        } else if !zoom_inside_current_viewport {
            self.floating_zoom_inside_window_locked = false;
        }

        // Once the floating window reaches display bounds, keep zooming inside the viewport
        // instead of continuing to grow the native window.
        let mut monitor_bounds = ctx.input(|i| i.raw.viewport().monitor_size);
        #[cfg(target_os = "windows")]
        {
            // The display the window is actually on. The primary monitor's size used to be
            // floored over this, which is what forced monitor 1's dimensions onto a window
            // living on monitor 2.
            if let Some(display) = self.current_display(ctx) {
                let ppp = ctx.pixels_per_point();
                if ppp.is_finite() && ppp > 0.01 {
                    monitor_bounds = Some(egui::vec2(
                        display.monitor.width() as f32 / ppp,
                        display.monitor.height() as f32 / ppp,
                    ));
                }
            }
        }

        if let Some(monitor_bounds) = monitor_bounds {
            if monitor_bounds.x > 0.0
                && monitor_bounds.y > 0.0
                && (desired.x > monitor_bounds.x || desired.y > monitor_bounds.y)
            {
                // If the user has manually oversized the floating window beyond monitor bounds,
                // keep that size and only continue zooming inside the current viewport.
                let current_window_exceeds_monitor = current_inner_rect
                    .map(|rect| {
                        let size = rect.size();
                        size.x > monitor_bounds.x + 0.5 || size.y > monitor_bounds.y + 0.5
                    })
                    .unwrap_or(false);

                if current_window_exceeds_monitor {
                    return;
                }

                // Physical pixels on both sides: the wanted inner size is converted once and
                // fitted against the display, so no scale can shrink it twice.
                let ppp = ctx.pixels_per_point();
                let fitted = if ppp.is_finite() && ppp > 0.01 {
                    let (w, h) = crate::window_geometry::fit_within(
                        (
                            (desired.x * ppp).round() as i32,
                            (desired.y * ppp).round() as i32,
                        ),
                        (
                            (monitor_bounds.x * ppp).round() as i32,
                            (monitor_bounds.y * ppp).round() as i32,
                        ),
                    );
                    egui::vec2(w as f32 / ppp, h as f32 / ppp)
                } else {
                    Self::fit_size_preserving_aspect(desired, monitor_bounds)
                };
                desired = fitted;
                desired.x = desired.x.max(1.0);
                desired.y = desired.y.max(1.0);
            }
        }

        let should_send = current_inner_rect
            .map(|rect| (rect.size() - desired).length() > 0.5)
            .unwrap_or(true);

        if should_send {
            if let (Some(inner_rect), Some(outer_rect)) = (current_inner_rect, current_outer_rect) {
                // Keep the window center stable while changing inner size so zoom-resize
                // expands/contracts from center instead of top-left.
                let inner_to_outer_offset = inner_rect.min - outer_rect.min;
                let target_inner_min = inner_rect.center() - desired * 0.5;
                let target_outer_min = target_inner_min - inner_to_outer_offset;
                self.send_outer_position(ctx, target_outer_min);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
        }
    }
}

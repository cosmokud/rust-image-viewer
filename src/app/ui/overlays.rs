//! Overlay visibility, pointer-over-UI tests and the wheel guard.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn clear_video_playback_unavailable_state(&mut self) {
        self.video_playback_unavailable_reason = None;
        self.video_playback_popup_until = None;
    }

    pub(crate) fn pointer_over_shortcut_blocking_ui(
        &self,
        pointer_pos: Option<egui::Pos2>,
        screen_rect: egui::Rect,
    ) -> bool {
        if self.title_bar_ui_blocking()
            || self.mouse_over_video_controls
            || self.file_action_menu.is_some()
            || self.any_modal_dialog_open()
        {
            return true;
        }

        let Some(pos) = pointer_pos else {
            return false;
        };

        if self.show_video_controls {
            let bar_height = 56.0;
            if pos.y > screen_rect.height() - bar_height {
                return true;
            }
        }

        if !self.mode().is_fullscreen() {
            return false;
        }

        let scrollbar_padding = Self::BOTTOM_RIGHT_OVERLAY_SCROLLBAR_PADDING;
        let margin = Self::BOTTOM_RIGHT_OVERLAY_MARGIN;
        let video_controls_offset = if self.show_video_controls {
            56.0 + 8.0
        } else {
            0.0
        };

        if self.show_manga_zoom_bar {
            let bar_size =
                egui::Vec2::new(Self::MANGA_HUD_PANEL_WIDTH, Self::MANGA_HUD_PANEL_HEIGHT);
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(
                    screen_rect.max.x - bar_size.x - margin - scrollbar_padding,
                    screen_rect.max.y - bar_size.y - margin - video_controls_offset,
                ),
                bar_size,
            );
            if bar_rect.contains(pos) {
                return true;
            }

            if self.mode().is_masonry() {
                let rows_bar_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        bar_rect.min.x,
                        bar_rect.min.y - Self::MANGA_HUD_PANEL_VERTICAL_STEP,
                    ),
                    bar_size,
                );
                if rows_bar_rect.contains(pos) {
                    return true;
                }
            }
        }

        if self.show_manga_toggle {
            let button_size = egui::Vec2::new(130.0, 32.0);
            let button_spacing = 8.0;
            let stack_height = button_size.y * 2.0 + button_spacing;
            let y_offset = if self.show_manga_zoom_bar {
                if self.mode().is_masonry() {
                    Self::MANGA_HUD_PANEL_VERTICAL_STEP * 2.0
                } else {
                    Self::MANGA_HUD_PANEL_VERTICAL_STEP
                }
            } else {
                0.0
            };
            let stack_pos = egui::pos2(
                screen_rect.max.x - button_size.x - margin - scrollbar_padding,
                screen_rect.max.y - stack_height - margin - y_offset - video_controls_offset,
            );
            let masonry_rect = egui::Rect::from_min_size(stack_pos, button_size);
            let long_strip_rect = egui::Rect::from_min_size(
                egui::pos2(stack_pos.x, stack_pos.y + button_size.y + button_spacing),
                button_size,
            );
            if masonry_rect.contains(pos) || long_strip_rect.contains(pos) {
                return true;
            }
        }

        false
    }

    pub(crate) fn media_slider_wheel_guard_active(&self) -> bool {
        self.media_slider_wheel_guard_until
            .is_some_and(|until| Instant::now() < until)
    }

    pub(crate) fn arm_media_slider_wheel_guard(&mut self) {
        self.media_slider_wheel_guard_until =
            Some(Instant::now() + Self::MEDIA_SLIDER_WHEEL_GUARD_DURATION);
    }

    /// Right-hand strip of the title bar that holds the window buttons.
    ///
    /// The drawn cluster is four 32pt buttons and three gaps; this zone is exactly that area (same
    /// button size and spacing the bar lays the cluster out with), full bar height and out to the
    /// very right edge, so every pixel of a button is inside it. A press in here must never start
    /// a window drag or resize - both move the window under the cursor and the button click is
    /// lost (the window-drag branch is the one that shows the four-arrow move cursor).
    ///
    /// It must not be wider than the cluster: the title bar left of it is the drag handle, and
    /// widening the zone turns that empty bar back into a stretch where the press moves nothing.
    pub(crate) fn title_bar_buttons_pointer_zone(
        &self,
        ctx: &egui::Context,
        screen_rect: egui::Rect,
    ) -> egui::Rect {
        const BUTTON_COUNT: f32 = 4.0;

        let button_size_x = Self::TITLE_BAR_BUTTON_SIZE.x;
        let gap = ctx.style().spacing.item_spacing.x;
        let cluster_w = (button_size_x * BUTTON_COUNT) + (gap * (BUTTON_COUNT - 1.0));
        let width =
            (cluster_w + Self::TITLE_BAR_BUTTON_TRAILING_GAP + 1.0).min(screen_rect.width());
        egui::Rect::from_min_max(
            egui::pos2(screen_rect.max.x - width, screen_rect.min.y),
            egui::pos2(
                screen_rect.max.x,
                screen_rect.min.y + Self::TITLE_BAR_HEIGHT,
            ),
        )
    }
}

//! Title-bar button zones and drag handles.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// The chrome bar as the window's drag handle: the full-width strip the bar occupies, in the
    /// mode where dragging the bar moves the window.
    ///
    /// Measured on the bar itself, not on a drawn widget: title text and window buttons only
    /// sense clicks and drags on their own glyph rects, so a press on the empty stretch between
    /// the file details and the buttons lands on no widget at all. That stretch used to fall
    /// through to the main view, which pans the image unless it is zoomed past the window, so a
    /// drag there moved nothing.
    pub(crate) fn title_bar_drag_handle_rect(&self, screen_rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_size(
            screen_rect.min,
            egui::Vec2::new(screen_rect.width(), Self::TITLE_BAR_HEIGHT),
        )
    }

    /// Whether this frame's pointer state starts a drag-pan of the strip.
    ///
    /// `pan_down_action` is a held (level) test and stays true for the whole gesture, so only the
    /// frame that starts the drag may latch the anchor: re-latching it every frame would measure
    /// the frame delta from the current pointer, which cancels the drag and freezes the strip.
    pub(crate) fn manga_pan_starts(
        is_panning: bool,
        pan_down_action: bool,
        double_clicked: bool,
    ) -> bool {
        pan_down_action && !double_clicked && !is_panning
    }

    /// Whether a drag latched at press time moves the window rather than panning the image.
    ///
    /// Floating mode: the chrome bar always moves the window, and so does a drag anywhere else
    /// while the media still fits the window (there is nothing to pan). Fullscreen never moves
    /// the window.
    pub(crate) fn drag_moves_window(
        is_fullscreen: bool,
        over_title_bar_drag_handle: bool,
        media_exceeds_window: bool,
    ) -> bool {
        !is_fullscreen && (over_title_bar_drag_handle || !media_exceeds_window)
    }

    pub(crate) fn title_bar_ui_blocking(&self) -> bool {
        self.mouse_over_window_buttons
            || self.mouse_over_title_text
            || self.title_text_dragging
            || self.title_bar_menu_active
    }

    pub(crate) fn max_zoom_factor(&self) -> f32 {
        // Config stored as percent: 100 = 1.0x, 1000 = 10.0x.
        // Clamp defensively to keep math stable even if config is extreme.
        let factor = (self.config.max_zoom_percent / 100.0).max(0.1);
        factor.clamp(0.1, 1000.0)
    }
}

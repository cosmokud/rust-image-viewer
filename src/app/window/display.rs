//! Display lookup for the main window.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn monitor_size_points(&self, ctx: &egui::Context) -> egui::Vec2 {
        // The display the window is on, never the primary: a window moved to monitor 2 that still
        // sized itself against monitor 1 is what resized it on every layout pass.
        #[cfg(target_os = "windows")]
        if let Some(display) = self.current_display(ctx) {
            let ppp = ctx.pixels_per_point();
            if ppp.is_finite() && ppp > 0.01 {
                return egui::vec2(
                    display.monitor.width() as f32 / ppp,
                    display.monitor.height() as f32 / ppp,
                );
            }
        }

        Self::floating_monitor_bounds_for_layout(
            ctx.input(|i| i.raw.viewport().monitor_size),
            self.screen_size,
            self.last_known_monitor_size,
        )
    }

    /// The display owning the window: the one nearest the window's centre.
    #[cfg(target_os = "windows")]
    pub(crate) fn current_display(
        &self,
        ctx: &egui::Context,
    ) -> Option<crate::window_geometry::Display> {
        use crate::window_geometry::display_for_window_centre;

        if let Some(rect) = self.main_window_rect_physical(ctx) {
            return display_for_window_centre(rect);
        }

        // No usable window rect: fall back to the monitor size the viewport reports, which is
        // physical pixels, and ask the display nearest its centre.
        let monitor = Self::floating_monitor_bounds_for_layout(
            ctx.input(|i| i.raw.viewport().monitor_size),
            self.screen_size,
            self.last_known_monitor_size,
        );
        let ppp = ctx.pixels_per_point();
        if !ppp.is_finite() || ppp <= 0.01 {
            return None;
        }
        crate::window_geometry::display_at(((monitor.x * ppp) as i32, (monitor.y * ppp) as i32))
    }

    /// The display owning the window, without an egui context: the window rect is the only
    /// question, and the OS answers it synchronously.
    #[cfg(target_os = "windows")]
    pub(crate) fn current_display_from_window_rect(
        &self,
    ) -> Option<crate::window_geometry::Display> {
        let rect = crate::windows_env::window_rect(self.resolved_main_hwnd())?;
        crate::window_geometry::display_for_window_centre(crate::window_geometry::Rect::new(
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn current_display(
        &self,
        _ctx: &egui::Context,
    ) -> Option<crate::window_geometry::Display> {
        None
    }

    /// The main window's outer rectangle in physical pixels.
    #[cfg(target_os = "windows")]
    pub(crate) fn main_window_rect_physical(
        &self,
        ctx: &egui::Context,
    ) -> Option<crate::window_geometry::Rect> {
        use crate::window_geometry::Rect;

        let hwnd = self.resolved_main_hwnd();
        if let Some(rect) = crate::windows_env::window_rect(hwnd) {
            return Some(Rect::new(rect.left, rect.top, rect.right, rect.bottom));
        }

        let outer = ctx.input(|i| i.raw.viewport().outer_rect)?;
        let ppp = ctx.pixels_per_point();
        if !ppp.is_finite() || ppp <= 0.01 {
            return None;
        }
        let size = outer.size() * ppp;
        Some(Rect::new(
            (outer.min.x * ppp).round() as i32,
            (outer.min.y * ppp).round() as i32,
            (outer.min.x * ppp).round() as i32 + size.x.round() as i32,
            (outer.min.y * ppp).round() as i32 + size.y.round() as i32,
        ))
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn park_window_on_its_display(
        &mut self,
        hwnd: winapi::shared::windef::HWND,
        ctx: &egui::Context,
    ) {
        let Some(display) = self.current_display(ctx) else {
            return;
        };
        let Some(outer) = self.main_window_rect_physical(ctx) else {
            return;
        };
        // No-op when already inside: only the two edges the buttons live on are corrected.
        let (left, top) = crate::window_geometry::clamp_into(display.work, outer);
        if left == outer.left && top == outer.top {
            return;
        }
        crate::windows_env::set_window_rect(hwnd, left, top, outer.width(), outer.height());
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn reassert_position_after_restore(&mut self, ctx: &egui::Context) {
        let Some(anchor) = self.maximize_restore_anchor.take() else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        if !ppp.is_finite() || ppp <= 0.01 {
            return;
        }

        // OS restore puts the window back where it thinks it belongs; keep it on the display
        // the maximize started from, and inside that display's work area.
        let display = crate::window_geometry::display_for_window_centre(anchor)
            .or_else(|| self.current_display(ctx));
        let Some(display) = display else {
            return;
        };
        let size = self
            .main_window_rect_physical(ctx)
            .map(|rect| (rect.width(), rect.height()))
            .unwrap_or((anchor.width(), anchor.height()));
        let (left, top) = crate::window_geometry::clamp_into(
            display.work,
            crate::window_geometry::Rect::new(
                anchor.left,
                anchor.top,
                anchor.left + size.0,
                anchor.top + size.1,
            ),
        );
        self.send_outer_position(ctx, egui::pos2(left as f32 / ppp, top as f32 / ppp));
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn resolved_main_hwnd(&self) -> winapi::shared::windef::HWND {
        if self.main_hwnd != 0 {
            return self.main_hwnd as winapi::shared::windef::HWND;
        }
        crate::windows_env::active_window_handle()
    }

    pub(crate) fn center_window_on_monitor(&mut self, ctx: &egui::Context, inner_size: egui::Vec2) {
        #[cfg(target_os = "windows")]
        if let Some(display) = self.current_display(ctx) {
            let ppp = ctx.pixels_per_point();
            if ppp.is_finite() && ppp > 0.01 {
                // Physical pixels throughout: the outer box (inner + chrome) is centred in the
                // display's work area, and only the final position is handed back in points.
                let outer = self.outer_size_for_inner(ctx, inner_size);
                let outer_w = (outer.x * ppp).round() as i32;
                let outer_h = (outer.y * ppp).round() as i32;
                let (left, top) = crate::window_geometry::centre_in(display.work, outer_w, outer_h);
                self.send_outer_position(ctx, egui::pos2(left as f32 / ppp, top as f32 / ppp));
                return;
            }
        }

        let monitor = self.monitor_size_points(ctx);
        let x = (monitor.x - inner_size.x) * 0.5;
        let y = (monitor.y - inner_size.y) * 0.5;
        self.send_outer_position(ctx, egui::pos2(x.max(0.0), y));
    }

    /// Outer size (inner plus decorations) for a wanted inner size, in points.
    pub(crate) fn outer_size_for_inner(
        &self,
        ctx: &egui::Context,
        inner_size: egui::Vec2,
    ) -> egui::Vec2 {
        let (inner_rect, outer_rect) =
            ctx.input(|i| (i.raw.viewport().inner_rect, i.raw.viewport().outer_rect));
        match (inner_rect, outer_rect) {
            (Some(inner), Some(outer)) => inner_size + (outer.size() - inner.size()),
            _ => inner_size,
        }
    }
}

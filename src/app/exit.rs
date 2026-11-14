//! Fast-exit path: discard runtime work, close viewports, tear down.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Frames a reopened viewer's rectangle is re-asserted for. Enough for the OS's DPI adjustment
/// (and the scale guard's answer to it) to have run, short enough not to fight a real resize.
pub(crate) const VIEWER_PLACE_FRAMES: u8 = 4;

impl ImageViewer {
    pub(crate) fn should_short_circuit_frame_for_exit(
        &self,
        viewport_close_requested: bool,
    ) -> bool {
        self.should_exit || viewport_close_requested
    }

    /// True while a window that is not the viewer is open.
    ///
    /// Settings and Stash are windows of their own, not dialogs owned by the viewer window, so
    /// closing the viewer leaves them on screen: the app keeps running until the last of them is
    /// closed too.
    pub(crate) fn viewer_has_independent_window(&self) -> bool {
        self.settings.open || self.stash.open
    }

    /// Gives the screen to those windows: the viewer stops being visible without being hidden,
    /// and keeps its state (current file, mode, stash, history) for the next time it is shown.
    ///
    /// It is parked just outside the desktop rather than hidden, because Windows stops sending
    /// redraws to a hidden window — and the Settings and Stash viewports are drawn from this
    /// window's frames, so hiding it froze them.
    pub(crate) fn hide_viewer_window(&mut self, ctx: &egui::Context) {
        if self.viewer_window_hidden {
            // Already parked: the close it asked for is still refused, or the window would be
            // destroyed under the Settings/Stash windows it is drawing.
            self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::CancelClose);
            ctx.request_repaint();
            return;
        }
        // Nothing should keep playing behind a window the user just closed.
        if let Some(player) = self.video_player.as_mut() {
            if player.is_playing() {
                player.pause_async();
            }
        }
        // The window is about to leave the screen with whatever view it was showing: keep that
        // exact camera (zoom, pan, rotation) next to the file it belongs to, so reopening that
        // file from the Stash comes back to the same place instead of a fresh fit.
        self.restore_parked_fullscreen_view_pending = false;
        if self.mode().is_fullscreen() && self.mode().is_solo() {
            self.current_fullscreen_view_has_memory = true;
            self.save_current_fullscreen_view_state();
            self.parked_fullscreen_view = self
                .image_list
                .get(self.current_index)
                .cloned()
                .map(|path| (path, self.current_fullscreen_view_state()));
        } else {
            self.parked_fullscreen_view = None;
        }

        self.parked_viewer_inner_size =
            ctx.input(|input| input.raw.viewport().inner_rect.map(|rect| rect.size()));
        // Captured before the park below: parking leaves the window just outside the desktop, so
        // the display and rectangle the OS reports afterwards belong to the neighbour monitor.
        #[cfg(target_os = "windows")]
        {
            self.parked_display = self.current_display(ctx);
            self.parked_viewer_rect = self.main_window_rect_physical(ctx);
        }
        self.restore_viewer_size_pending = false;
        self.viewer_window_hidden = true;
        // Refuse the close itself: the window keeps running, it just moves out of sight.
        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::CancelClose);
        #[cfg(target_os = "windows")]
        crate::windows_env::park_window_offscreen(self.resolved_main_hwnd());
        ctx.request_repaint();
    }

    /// Brings the viewer back after it was closed behind the Settings/Stash windows.
    ///
    /// It comes back exactly where and as large as it was left, and the file that reopened it is
    /// fitted into that window instead of resizing it. Every command is addressed to the root
    /// viewport: this runs inside the Stash's own frame, and a plain viewport command there would
    /// move the Stash instead.
    pub(crate) fn show_viewer_window_if_hidden(&mut self, ctx: &egui::Context) {
        if !self.viewer_window_hidden {
            return;
        }
        self.viewer_window_hidden = false;

        // Taken from the close, not from the parked window: the parked window stands beside the
        // desktop, so asking the OS which display owns it answers with the neighbour.
        #[cfg(target_os = "windows")]
        let parked_rect = self.parked_viewer_rect.take();
        #[cfg(target_os = "windows")]
        let parked_display = self.parked_display.take();
        #[cfg(target_os = "windows")]
        crate::windows_env::unpark_window_offscreen(self.resolved_main_hwnd());

        // The physical restore runs before any point-based command: those are converted with the
        // scale of whatever display the parked window stood on, which is what left the reopened
        // viewer on the second monitor with a fullscreen rectangle that did not cover it.
        #[cfg(target_os = "windows")]
        if self.restore_parked_viewer_rect(ctx, parked_rect, parked_display) {
            return;
        }

        // A viewer closed in fullscreen comes back in fullscreen: the mode never changed while it
        // was parked, so laying a floating rectangle over it produced a floating/fullscreen mix.
        if self.mode().is_fullscreen() {
            self.restore_viewer_size_pending = false;
            if let Some(position) = self.floating_window_position {
                self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::OuterPosition(position));
            }
            self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::Visible(true));
            self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::Minimized(false));
            self.request_borderless_fullscreen_rect(ctx);
            // The opened file has to be laid out again, and if it is the file that was closed,
            // that layout puts the parked camera back.
            self.restore_parked_fullscreen_view_pending =
                self.mode().is_solo() && self.parked_fullscreen_view.is_some();
            self.pending_media_layout = true;
            ctx.request_repaint();
            return;
        }

        let size = self.parked_viewer_inner_size;
        match self.floating_window_position {
            Some(position) => {
                self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::OuterPosition(position))
            }
            None => {
                let fallback = size.unwrap_or(egui::vec2(800.0, 600.0));
                let anchor = ctx
                    .input(|input| input.raw.viewport().monitor_size)
                    .unwrap_or(fallback);
                let centred = egui::pos2(
                    ((anchor.x - fallback.x) * 0.5).max(0.0),
                    ((anchor.y - fallback.y) * 0.5).max(0.0),
                );
                self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::OuterPosition(centred));
            }
        }
        if let Some(size) = size {
            self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::InnerSize(size));
            // The reopened window keeps its size; the media is fitted into it.
            self.restore_viewer_size_pending = true;
        }

        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::Visible(true));
        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::Minimized(false));
        ctx.request_repaint();
    }

    /// Puts the parked viewer back on the display and rectangle it was closed on, written in
    /// physical pixels before any point-based command.
    ///
    /// Returns false when nothing was remembered, so the caller can fall back to those commands.
    /// A fullscreen viewer is restored to the display's fullscreen rectangle rather than to the
    /// floating one: measured around the parked window, that rectangle belongs to the neighbour
    /// display.
    #[cfg(target_os = "windows")]
    fn restore_parked_viewer_rect(
        &mut self,
        ctx: &egui::Context,
        parked_rect: Option<crate::window_geometry::Rect>,
        parked_display: Option<crate::window_geometry::Display>,
    ) -> bool {
        let display = parked_display
            .or_else(|| parked_rect.and_then(crate::window_geometry::display_for_window_centre));
        // A rectangle no display owns — the placeholder a minimized window carries — is not a
        // place to come back to.
        let parked_rect = parked_rect.filter(|rect| {
            rect.width() > 0
                && rect.height() > 0
                && display.is_some_and(|display| {
                    rect.left < display.monitor.right
                        && rect.right > display.monitor.left
                        && rect.top < display.monitor.bottom
                        && rect.bottom > display.monitor.top
                })
        });

        let target = if self.mode().is_fullscreen() {
            let Some(display) = display else {
                return false;
            };
            crate::windows_env::set_fullscreen_guard(true);
            self.restore_viewer_size_pending = false;
            // The opened file has to be laid out again, and if it is the file that was closed,
            // that layout puts the parked camera back.
            self.restore_parked_fullscreen_view_pending =
                self.mode().is_solo() && self.parked_fullscreen_view.is_some();
            self.pending_media_layout = true;
            self.fullscreen_target_rect(display)
        } else {
            let Some(rect) = parked_rect else {
                return false;
            };
            // The reopened window keeps its size; the media is fitted into it.
            self.restore_viewer_size_pending = self.parked_viewer_inner_size.is_some();
            rect
        };

        crate::windows_env::set_window_rect(
            self.resolved_main_hwnd(),
            target.left,
            target.top,
            target.width(),
            target.height(),
        );
        // Asserted again for the next frames: moving off the parked position crosses displays,
        // and the OS's DPI adjustment rewrites the rectangle the window was just given.
        self.viewer_place_rect = Some(target);
        self.viewer_place_frames = VIEWER_PLACE_FRAMES;
        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::Visible(true));
        self.send_viewer_viewport_cmd(ctx, egui::ViewportCommand::Minimized(false));
        ctx.request_repaint();
        true
    }

    /// Keeps a reopened viewer on the rectangle it was just put back at.
    ///
    /// The move back from the parked position crosses displays, and the OS answers it with a DPI
    /// adjustment that rewrites the window rectangle. Without this the window came back at the
    /// size it had while parked, in the corner of the new display. Once the placement settles the
    /// media is laid out again against the window it actually has.
    pub(crate) fn assert_viewer_placement(&mut self, ctx: &egui::Context) {
        if self.viewer_place_frames == 0 {
            return;
        }
        // The window has been taken over: parked again, resized by the user, or put into a new
        // mode by a resize this frame already queued.
        if self.viewer_window_hidden
            || self.is_resizing
            || self.pending_window_resize.is_some()
            || self.toggle_fullscreen
        {
            self.viewer_place_frames = 0;
            self.viewer_place_rect = None;
            return;
        }

        self.viewer_place_frames -= 1;
        if let Some(rect) = self.viewer_place_rect {
            #[cfg(target_os = "windows")]
            crate::windows_env::set_window_rect(
                self.resolved_main_hwnd(),
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
            );
            #[cfg(not(target_os = "windows"))]
            let _ = rect;
        }
        if self.viewer_place_frames == 0 {
            self.viewer_place_rect = None;
            // Settled: lay the media out once against the window it now has, and keep the size
            // the viewer was reopened at instead of resizing the window to the file.
            self.restore_viewer_size_pending = !self.mode().is_fullscreen();
            self.pending_media_layout = true;
        }
        ctx.request_repaint();
    }

    pub(crate) fn begin_fast_app_exit(&mut self) {
        if self.should_exit {
            return;
        }

        // The stash window can still be open: write the rectangle it last settled on before
        // the debounced save is dropped with the process.
        if self.stash.geometry_save_due.take().is_some() {
            self.save_stash_window_geometry();
        }

        self.discard_runtime_work_for_exit();
        #[cfg(not(test))]
        crate::metadata_cache::request_metadata_cache_shutdown();
        self.should_exit = true;
    }

    pub(crate) fn discard_runtime_work_for_exit(&mut self) {
        self.media_load_coordinator.cancel_pending();
        self.solo_probe_coordinator.cancel_pending();
        self.manga_video_load_coordinator.cancel_pending();

        self.clear_pending_media_load();
        self.clear_manga_runtime_workloads();
        self.reset_masonry_metadata_preload();

        if let Some(loader) = self.manga_loader.as_mut() {
            loader.cancel_all();
        }
        self.manga_loader = None;
        // The loader that held the failure history is gone; nothing is pinned any more.
        self.manga_unloadable_media.clear();
        self.manga_texture_cache.clear();
        self.manga_decoded_mailbox.clear();

        self.image = None;
        self.texture = None;
        self.video_texture = None;
        self.pending_video_thumbnail_placeholder = None;
        self.retained_media_placeholder_visible = false;
    }

    pub(crate) fn close_viewport_commands() -> [egui::ViewportCommand; 1] {
        [egui::ViewportCommand::Close]
    }

    pub(crate) fn send_fast_close_viewport_commands(ctx: &egui::Context) {
        for command in Self::close_viewport_commands() {
            ctx.send_viewport_cmd(command);
        }
        ctx.request_repaint();
    }

    pub(crate) fn masonry_authoritative_dimension_lock_active(&self) -> bool {
        self.mode().is_masonry()
            && self.masonry_authoritative_dimension_signature != 0
            && self.masonry_authoritative_dimension_signature == self.image_list_signature
            && self.masonry_authoritative_dimension_folder.is_some()
    }

    pub(crate) fn manga_dimension_cache_covers_current_list(&self) -> bool {
        let Some(loader) = self.manga_loader.as_ref() else {
            return false;
        };

        let mut required_entries = 0usize;
        for (idx, path) in self.image_list.iter().enumerate() {
            if self.is_folder_navigation_entry_path(path.as_path()) {
                continue;
            }

            required_entries += 1;
            let Some((width, height, _)) = loader.dimension_cache.get(&idx).copied() else {
                return false;
            };
            if width == 0 || height == 0 {
                return false;
            }
        }

        required_entries > 0
    }
}

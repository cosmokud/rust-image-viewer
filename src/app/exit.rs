//! Fast-exit path: discard runtime work, close viewports, tear down.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn should_short_circuit_frame_for_exit(
        &self,
        viewport_close_requested: bool,
    ) -> bool {
        self.should_exit
            || (viewport_close_requested
                && !self.pending_exit_confirmation
                && !self.has_marked_files())
    }

    pub(crate) fn begin_fast_app_exit(&mut self) {
        if self.should_exit {
            return;
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

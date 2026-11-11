//! State prepared when entering manga/strip mode.

use crate::app::*;

impl ImageViewer {
    pub(crate) fn reset_masonry_metadata_preload(&mut self) {
        self.masonry_metadata_preload_active = false;
        self.masonry_metadata_preload_total = 0;
        self.masonry_metadata_preload_loaded = 0;
        self.masonry_metadata_preload_cursor = 0;
        self.masonry_metadata_preload_list_signature = 0;
        self.masonry_metadata_preload_restore_index = None;
        self.masonry_metadata_preload_overlay_hold_until = None;
        self.masonry_metadata_preload_defer_first_tick = false;
        self.masonry_metadata_preload_stall_since = None;
        self.pending_masonry_folder_travel_restore = None;
    }
}

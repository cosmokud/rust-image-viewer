//!

use crate::app::*;

impl ImageViewer {
    pub(crate) fn clear_pending_media_load(&mut self) {
        self.pending_media_load = None;
        self.retained_media_placeholder_visible = false;
        self.defer_media_view_reset = false;
    }
}

//! The diagnostics overlay drawn over the viewer.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn touch_bottom_overlays(&mut self) {
        let now = Instant::now();
        self.video_controls_show_time = now;
        self.manga_toggle_show_time = now;
        self.manga_zoom_bar_show_time = now;
    }
}

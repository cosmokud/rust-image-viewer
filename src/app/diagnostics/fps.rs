//! FPS sampling and the metrics series.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn manga_mark_placeholder_visible(&mut self, index: usize) {
        if self.mode().is_solo() {
            return;
        }
        // A tile nothing will ever load is not "pending work". Keeping it here drove the 16ms
        // background poll (and thus a full-frame repaint) for as long as the corrupt file was
        // on screen, which is where the sustained 100% CPU came from.
        if self.manga_media_is_unloadable(index)
            || self
                .manga_loader
                .as_ref()
                .is_some_and(|loader| loader.retry_blocked(index))
        {
            return;
        }
        self.manga_ttv_pending
            .entry(index)
            .or_insert_with(Instant::now);
    }
}

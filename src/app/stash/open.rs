//! Opening and closing the stash, and the filtered list it shows.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn open_stash(&mut self) {
        self.stash.open = true;
        self.stash.search.clear();
        self.stash.hovered = None;
        self.stash.preview.clear();
        self.file_action_menu = None;
        self.show_controls = true;
        self.controls_show_time = Instant::now();
    }

    pub(crate) fn close_stash(&mut self) {
        self.stash.open = false;
        self.stash.hovered = None;
        self.stash.preview.clear();
    }

    /// Marked files in viewer order, narrowed by the stash filter box.
    pub(crate) fn filtered_stash_paths(&self) -> Vec<PathBuf> {
        let query = self.stash.search.trim().to_lowercase();
        self.collect_marked_paths_in_current_order()
            .into_iter()
            .filter(|path| {
                query.is_empty()
                    || path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(&query))
            })
            .collect()
    }
}

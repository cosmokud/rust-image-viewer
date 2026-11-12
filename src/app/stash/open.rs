//! Opening and closing the stash, and the filtered list it shows.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn open_stash(&mut self) {
        self.stash.open = true;
        // Geometry is recomputed from the screen on every open, like the settings window: the
        // stash always comes back centered instead of at the size of the last manual resize.
        self.stash.screen = None;
        self.stash.place_frames = STASH_WINDOW_PLACE_FRAMES;
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

    /// Stashed files in viewer order, narrowed by the stash filter box. Files stashed from
    /// another folder follow, sorted, so the order is stable across sessions.
    pub(crate) fn filtered_stash_paths(&self) -> Vec<PathBuf> {
        let query = self.stash.search.trim().to_lowercase();
        let mut ordered: Vec<PathBuf> = self
            .image_list
            .iter()
            .filter(|path| self.stash.entries.contains_key(*path) && path.exists())
            .cloned()
            .collect();
        let mut extras: Vec<PathBuf> = self
            .stash
            .entries
            .keys()
            .filter(|path| !self.image_list.contains(*path) && path.exists())
            .cloned()
            .collect();
        extras.sort();
        ordered.extend(extras);

        ordered
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

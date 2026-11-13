//! Opening and closing the stash, and the filtered list it shows.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn open_stash(&mut self) {
        self.stash.open = true;
        // The screen is resolved again on every open, so a remembered rectangle that belongs to
        // another display is measured against that display: the stash comes back where it was
        // left, and only a first open is centered.
        self.stash.screen = None;
        self.stash.place_frames = STASH_WINDOW_PLACE_FRAMES;
        self.stash.search.clear();
        self.stash.hovered = None;
        self.stash.preview.clear();
        self.file_action_menu = None;
        self.show_controls = true;
        self.controls_show_time = Instant::now();
    }

    /// Opens a stash file the way the current window already works: same folder means moving
    /// the view (scroll the grid, jump the strip, swap the solo page) instead of reloading it,
    /// and the current mode — solo, solo fullscreen, Long Strip, Masonry — is kept.
    pub(crate) fn open_stash_path(&mut self, ctx: &egui::Context, path: &PathBuf) {
        if !path.exists() {
            self.remove_path_from_stash(path.as_path());
            return;
        }

        // With single-instance off the user expects a separate window for this file.
        if !self.config.single_instance {
            if let Ok(executable) = std::env::current_exe() {
                if let Err(err) = std::process::Command::new(executable).arg(path).spawn() {
                    self.error_message = Some(format!("Failed to open a new window: {err}"));
                }
            }
            return;
        }

        let same_folder = Self::same_media_folder(
            self.current_media_path()
                .as_ref()
                .or(self.image_list.first()),
            path,
        );

        if same_folder {
            if let Some(index) = self
                .image_list
                .iter()
                .position(|candidate| candidate == path)
            {
                self.reveal_media_index(index);
                ctx.request_repaint();
                return;
            }
        }

        // Another folder: rebuild the list for that file, keeping the mode we are in. The strip
        // and grid already own a "list changed" path; the solo view just opens the file.
        if self.mode().is_strip() {
            self.refresh_media_list_after_path_mutation(Some(path.clone()));
            self.scroll_strip_to_current_index();
            self.manga_update_preload_queue();
        } else {
            self.load_media(path);
        }
        ctx.request_repaint();
    }

    /// Moves the current view onto `index`, whatever the mode is showing right now.
    fn reveal_media_index(&mut self, index: usize) {
        if self.mode().is_strip() {
            self.set_current_index_clamped(index);
            self.scroll_strip_to_current_index();
            self.manga_update_preload_queue();
            return;
        }

        self.save_current_fullscreen_view_state();
        self.set_current_index_clamped(index);
        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            return;
        };
        self.load_image_retaining_visible_media(&path);
    }

    fn same_media_folder(anchor: Option<&PathBuf>, path: &Path) -> bool {
        let anchor = anchor.and_then(|anchor| anchor.parent());
        let candidate = path.parent();
        match (anchor, candidate) {
            (Some(anchor), Some(candidate)) => {
                anchor.to_string_lossy().to_lowercase()
                    == candidate.to_string_lossy().to_lowercase()
            }
            _ => false,
        }
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

//! Opening and closing the stash, and the filtered list it shows.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// Title of the stash viewport's native window, which is also how that window is found when it
/// has to be raised.
pub(crate) const STASH_WINDOW_TITLE: &str = "Stash";

/// How long the filtered gallery order is trusted. Every rebuild stats every stashed file, so it
/// cannot run every frame, but it must still notice a file deleted behind the window.
pub(crate) const STASH_FILTERED_REFRESH: Duration = Duration::from_millis(1000);

/// Frames the stash window is re-raised after a file is opened: the raise is repeated for a few
/// frames so it survives the viewer repainting itself behind it.
pub(crate) const STASH_RAISE_FRAMES: u8 = 3;

impl ImageViewer {
    pub(crate) fn open_stash(&mut self, ctx: &egui::Context) {
        // Already open: raise and focus the existing window instead of resetting its state,
        // which would re-center it on the remembered placement. `Focus` alone is a no-op on a
        // minimized window, so it is restored first.
        if self.stash.open {
            let viewport_id = stash_viewport_id();
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Focus);
            ctx.request_repaint();
            return;
        }
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

        // The viewer is updated behind this window, so the stash is raised over it on the next
        // frames without being activated: the keyboard focus stays on the stash.
        self.stash.raise_frames = STASH_RAISE_FRAMES;
        // Opening a file is a request to see it, even if the viewer window was closed behind
        // this one while the stash stayed open. With multiple instances the file opens in a
        // process of its own, so this window has nothing to show.
        if self.config.single_instance {
            self.show_viewer_window_if_hidden(ctx);
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

    /// Lifts the stash window to the top of the z-order without activating it, once per pending
    /// frame. Called from inside its own viewport, which is the only place its native window is
    /// known to exist.
    pub(crate) fn stash_raise_window(&mut self, ctx: &egui::Context) {
        if 0 == self.stash.raise_frames {
            return;
        }
        self.stash.raise_frames -= 1;
        #[cfg(target_os = "windows")]
        crate::windows_env::raise_process_window_without_focus(STASH_WINDOW_TITLE);
        ctx.request_repaint();
    }

    /// Files dropped on the stash window join the stash instead of being opened in the viewer.
    /// Only the formats the viewer itself opens are taken, and the last one becomes the
    /// selection so the drop is visible in the gallery.
    pub(crate) fn stash_accept_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }

        let supported: Vec<PathBuf> = dropped
            .into_iter()
            .filter(|path| get_media_type(path.as_path()).is_some())
            .collect();
        if supported.is_empty() {
            return;
        }

        self.stash_remember_paths(&supported);
        if let Some(last) = supported.last().cloned() {
            self.stash.selected = Some(last.clone());
            self.stash.selection.clear();
            self.stash.selection.insert(last.clone());
            self.stash.selection_anchor = Some(last);
            self.stash.scroll_to_selection = true;
        }
        self.stash.raise_frames = STASH_RAISE_FRAMES;
    }

    pub(crate) fn close_stash(&mut self) {
        self.stash.open = false;
        self.stash.hovered = None;
        self.stash.preview.clear();
    }

    /// Stashed files in one stable, sorted order, narrowed by the stash filter box.
    ///
    /// The gallery must not rearrange itself while it is open. Ordering the tiles by the
    /// viewer's folder list did exactly that: opening a stashed file from another folder
    /// replaces that list, so the group that had been shown in viewer order was rebuilt around
    /// the new folder and every tile moved. Paths are sorted instead, which is the same order in
    /// every session, every folder and every filter.
    /// The gallery order: every stashed file that still exists, sorted, then filtered.
    ///
    /// Rebuilt only when the stash or the filter changes, and refreshed on a slow tick so a file
    /// deleted outside the app still leaves the grid. It used to rebuild every frame, which cost
    /// one `exists()` per entry plus a sort: a few hundred files turned that into the dominant
    /// cost of scrolling the gallery.
    pub(crate) fn filtered_stash_paths(&mut self) -> std::sync::Arc<Vec<PathBuf>> {
        let query = self.stash.search.trim().to_lowercase();
        if let Some(cached) = self.stash.filtered.as_ref() {
            if cached.query == query && cached.built_at.elapsed() < STASH_FILTERED_REFRESH {
                return std::sync::Arc::clone(&cached.paths);
            }
        }

        let mut ordered: Vec<PathBuf> = self
            .stash
            .entries
            .keys()
            .filter(|path| path.exists())
            .cloned()
            .collect();
        ordered.sort_by_key(|path| path.to_string_lossy().to_lowercase());
        if !query.is_empty() {
            ordered.retain(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(&query))
            });
        }

        let paths = std::sync::Arc::new(ordered);
        self.stash.filtered = Some(StashFilteredPaths {
            query,
            built_at: Instant::now(),
            paths: std::sync::Arc::clone(&paths),
        });
        paths
    }

    /// Forces the next draw to rebuild the gallery order, after the stash changed.
    pub(crate) fn stash_invalidate_filtered_paths(&mut self) {
        self.stash.filtered = None;
    }
}

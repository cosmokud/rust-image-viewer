//! Authoritative masonry dimension bookkeeping and folder snapshots.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn mark_manga_dimension_cache_current_if_complete(&mut self) -> bool {
        if self.manga_dimension_cache_covers_current_list() {
            self.manga_dimension_cache_list_signature = self.image_list_signature;
            true
        } else {
            false
        }
    }

    pub(crate) fn current_masonry_metadata_folder_key(&self) -> Option<PathBuf> {
        self.current_media_path()
            .filter(|path| !self.is_folder_navigation_entry_path(path.as_path()))
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.image_list
                    .iter()
                    .find(|path| !self.is_folder_navigation_entry_path(path.as_path()))
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
    }

    pub(crate) fn collect_current_masonry_folder_metadata_snapshot(
        &self,
    ) -> Option<(PathBuf, HashMap<PathBuf, (u32, u32, MangaMediaType)>)> {
        let folder = self.current_masonry_metadata_folder_key()?;
        let loader = self.manga_loader.as_ref()?;

        let mut entries_by_path: HashMap<PathBuf, (u32, u32, MangaMediaType)> = HashMap::new();

        for (idx, path) in self.image_list.iter().enumerate() {
            if self.is_folder_navigation_entry_path(path.as_path()) {
                continue;
            }

            if path.parent() != Some(folder.as_path()) {
                continue;
            }

            let Some((width, height, media_type)) = loader.dimension_cache.get(&idx).copied()
            else {
                continue;
            };

            if width == 0 || height == 0 {
                continue;
            }

            entries_by_path.insert(path.clone(), (width, height, media_type));
        }

        if entries_by_path.is_empty() {
            None
        } else {
            Some((folder, entries_by_path))
        }
    }

    pub(crate) fn persist_current_masonry_folder_metadata_snapshot(&mut self) {
        self.masonry_folder_metadata_ram_cache
            .set_max_bytes(self.masonry_metadata_ram_cache_limit_bytes());

        let Some((folder, entries_by_path)) =
            self.collect_current_masonry_folder_metadata_snapshot()
        else {
            return;
        };

        self.masonry_folder_metadata_ram_cache
            .insert_snapshot(folder, entries_by_path);
    }

    pub(crate) fn restore_masonry_folder_metadata_snapshot(&mut self) -> usize {
        self.masonry_folder_metadata_ram_cache
            .set_max_bytes(self.masonry_metadata_ram_cache_limit_bytes());
        self.clear_masonry_authoritative_dimension_lock();

        let Some(folder) = self.current_masonry_metadata_folder_key() else {
            return 0;
        };

        let Some(entries_by_path) = self
            .masonry_folder_metadata_ram_cache
            .snapshot_for_folder(folder.as_path())
        else {
            return 0;
        };

        let mut restored_entries: Vec<(usize, (u32, u32, MangaMediaType))> = Vec::new();
        for (idx, path) in self.image_list.iter().enumerate() {
            if self.is_folder_navigation_entry_path(path.as_path()) {
                continue;
            }

            if let Some(dims) = entries_by_path.get(path).copied() {
                restored_entries.push((idx, dims));
            }
        }

        if restored_entries.is_empty() {
            return 0;
        }

        let Some(loader) = self.manga_loader.as_mut() else {
            return 0;
        };

        for (idx, dims) in &restored_entries {
            loader.dimension_cache.insert(*idx, *dims);
        }

        let total_non_folder_entries = self
            .image_list
            .iter()
            .filter(|path| !self.is_folder_navigation_entry_path(path.as_path()))
            .count();

        let restored_all_entries =
            total_non_folder_entries > 0 && restored_entries.len() >= total_non_folder_entries;
        if restored_all_entries {
            self.masonry_authoritative_dimension_signature = self.image_list_signature;
            self.masonry_authoritative_dimension_folder = Some(folder);
        }

        if restored_all_entries || self.manga_dimension_cache_covers_current_list() {
            self.manga_dimension_cache_list_signature = self.image_list_signature;
        }
        if self.mode().is_grid() {
            self.mark_masonry_runtime_cache_resident();
        }

        restored_entries.len()
    }

    pub(crate) fn capture_masonry_mode_session_snapshot(&mut self) {
        if !self.mode().is_masonry() || self.image_list.is_empty() {
            return;
        }

        self.masonry_ensure_layout_cache();

        self.masonry_mode_session_snapshot = Some(MasonryModeSessionSnapshot {
            image_list: self.image_list.clone(),
            image_list_signature: self.image_list_signature,
            current_index: self
                .current_index
                .min(self.image_list.len().saturating_sub(1)),
            current_path: self.current_media_path(),
            folder_key: self.current_masonry_metadata_folder_key(),
            zoom: self.zoom,
            zoom_target: self.zoom_target,
            offset: self.offset,
            scroll_offset: self.manga_scroll_offset,
            scroll_target: self.manga_scroll_target,
            layout_items: self.masonry_layout_items.clone(),
            layout_total_height: self.masonry_layout_total_height,
            layout_screen_x: self.masonry_layout_screen_x,
            layout_items_per_row: self.masonry_layout_items_per_row,
            layout_len: self.masonry_layout_len,
            layout_list_signature: self.masonry_layout_list_signature,
            layout_valid: self.masonry_layout_valid,
            layout_is_gallery: self.mode().is_gallery(),
        });
    }

    pub(crate) fn try_restore_masonry_mode_session_snapshot(
        &mut self,
        preferred_path: Option<PathBuf>,
    ) -> bool {
        let Some(snapshot) = self.masonry_mode_session_snapshot.clone() else {
            return false;
        };

        if snapshot.image_list.is_empty() {
            return false;
        }

        let snapshot_folder_key = snapshot.folder_key.clone().or_else(|| {
            snapshot
                .current_path
                .as_ref()
                .and_then(|path| path.parent().map(Path::to_path_buf))
        });

        if let (Some(path), Some(folder_key)) =
            (preferred_path.as_ref(), snapshot_folder_key.as_ref())
        {
            if path.parent() != Some(folder_key.as_path()) {
                return false;
            }
        }

        self.set_image_list_raw(snapshot.image_list.clone());

        let restore_path = preferred_path
            .and_then(|path| {
                if self.is_folder_navigation_entry_path(path.as_path()) {
                    None
                } else {
                    Some(path)
                }
            })
            .or(snapshot.current_path.clone());

        if let Some(path) = restore_path {
            if let Some(index) = self
                .image_list
                .iter()
                .position(|candidate| candidate == &path)
            {
                self.set_current_index_clamped(index);
            } else {
                self.set_current_index_clamped(
                    snapshot
                        .current_index
                        .min(self.image_list.len().saturating_sub(1)),
                );
            }
        } else {
            self.set_current_index_clamped(
                snapshot
                    .current_index
                    .min(self.image_list.len().saturating_sub(1)),
            );
        }

        let expected_gallery_layout = self.mode().layout() == MangaLayoutMode::Gallery;
        let compatible_layout = snapshot.layout_valid
            && snapshot.layout_is_gallery == expected_gallery_layout
            && snapshot.image_list_signature == self.image_list_signature
            && snapshot.layout_list_signature == self.image_list_signature
            && snapshot.layout_len == self.image_list.len()
            && snapshot.layout_items.len() == self.image_list.len()
            && snapshot.layout_items_per_row == self.masonry_items_per_row
            && (snapshot.layout_screen_x - self.screen_size.x.round()).abs() <= 1.0;

        if !compatible_layout {
            return false;
        }

        self.masonry_layout_items = snapshot.layout_items;
        self.masonry_layout_total_height = snapshot.layout_total_height;
        self.masonry_layout_screen_x = snapshot.layout_screen_x;
        self.masonry_layout_items_per_row = snapshot.layout_items_per_row;
        self.masonry_layout_len = snapshot.layout_len;
        self.masonry_layout_list_signature = self.image_list_signature;
        self.masonry_layout_valid = true;
        self.masonry_spatial_index = None;

        self.zoom = self.clamp_zoom(snapshot.zoom);
        self.zoom_target = self.clamp_zoom(snapshot.zoom_target);
        self.zoom_velocity = 0.0;
        self.offset = snapshot.offset;
        self.manga_total_height_cache_valid = false;

        let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
        self.manga_scroll_offset = snapshot.scroll_offset.clamp(0.0, max_scroll);
        self.manga_scroll_target = snapshot.scroll_target.clamp(0.0, max_scroll);
        self.manga_scroll_velocity = 0.0;
        self.stop_manga_wheel_scroll();

        self.manga_dimension_cache_list_signature = self.image_list_signature;
        self.mark_masonry_runtime_cache_resident();
        true
    }

    pub(crate) fn hydrate_masonry_dimensions_from_session_snapshot(&mut self) -> usize {
        let Some(snapshot) = self.masonry_mode_session_snapshot.as_ref() else {
            return 0;
        };

        if snapshot.image_list_signature != self.image_list_signature
            || snapshot.layout_items.len() != self.image_list.len()
        {
            return 0;
        }

        let snapshot_folder_key = snapshot.folder_key.clone();
        let mut restored_entries: Vec<(usize, (u32, u32, MangaMediaType))> = Vec::new();
        let mut total_non_folder_entries = 0usize;

        {
            let loader = self.manga_loader.as_ref();

            for (idx, path) in self.image_list.iter().enumerate() {
                let Some(media_type_hint) = get_media_type(path) else {
                    continue;
                };
                total_non_folder_entries = total_non_folder_entries.saturating_add(1);

                let Some(item) = snapshot.layout_items.get(idx) else {
                    continue;
                };

                if item.source_width == 0 || item.source_height == 0 {
                    continue;
                }

                let media_type = loader
                    .and_then(|loader| loader.dimension_cache.get(&idx).map(|(_, _, mt)| *mt))
                    .unwrap_or(match media_type_hint {
                        MediaType::Video => MangaMediaType::Video,
                        MediaType::Image => MangaMediaType::StaticImage,
                    });

                restored_entries.push((idx, (item.source_width, item.source_height, media_type)));
            }
        }

        if restored_entries.is_empty() {
            return 0;
        }

        let Some(loader) = self.manga_loader.as_mut() else {
            return 0;
        };

        for (idx, dims) in &restored_entries {
            loader.dimension_cache.insert(*idx, *dims);
        }

        let restored_all_entries =
            total_non_folder_entries > 0 && restored_entries.len() >= total_non_folder_entries;
        if restored_all_entries {
            self.masonry_authoritative_dimension_signature = self.image_list_signature;
            self.masonry_authoritative_dimension_folder = self
                .current_masonry_metadata_folder_key()
                .or(snapshot_folder_key);
        }

        if restored_all_entries || self.manga_dimension_cache_covers_current_list() {
            self.manga_dimension_cache_list_signature = self.image_list_signature;
        }
        restored_entries.len()
    }

    pub(crate) fn sync_masonry_authoritative_dimensions(&mut self) -> usize {
        if !self.masonry_authoritative_dimension_lock_active() {
            return 0;
        }

        let Some(folder) = self.masonry_authoritative_dimension_folder.clone() else {
            self.clear_masonry_authoritative_dimension_lock();
            return 0;
        };

        let Some(entries_by_path) = self
            .masonry_folder_metadata_ram_cache
            .peek_entries_for_folder(folder.as_path())
        else {
            self.clear_masonry_authoritative_dimension_lock();
            return 0;
        };

        let Some(loader) = self.manga_loader.as_ref() else {
            return 0;
        };

        let mut corrections: Vec<(usize, (u32, u32, MangaMediaType))> = Vec::new();
        for (idx, path) in self.image_list.iter().enumerate() {
            if self.is_folder_navigation_entry_path(path.as_path()) {
                continue;
            }

            let Some(snapshot_dims) = entries_by_path.get(path).copied() else {
                continue;
            };

            if loader.dimension_cache.get(&idx).copied() != Some(snapshot_dims) {
                corrections.push((idx, snapshot_dims));
            }
        }

        if corrections.is_empty() {
            return 0;
        }

        let Some(loader) = self.manga_loader.as_mut() else {
            return 0;
        };

        for (idx, dims) in &corrections {
            loader.dimension_cache.insert(*idx, *dims);
        }

        corrections.len()
    }

    pub(crate) fn update_pointer_activity_tracking(&mut self, ctx: &egui::Context) {
        let (pointer_pos, pointer_event_seen) = ctx.input(|input| {
            let pointer_pos = input.pointer.hover_pos();
            let pointer_event_seen = input.raw.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerMoved(_)
                        | egui::Event::PointerButton { .. }
                        | egui::Event::MouseWheel { .. }
                )
            });
            (pointer_pos, pointer_event_seen)
        });

        let pointer_moved = match (self.last_pointer_hover_pos, pointer_pos) {
            (Some(previous), Some(current)) => (current - previous).length_sq() > 0.0,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };

        if pointer_event_seen || pointer_moved {
            self.last_pointer_activity_at = Instant::now();
        }

        self.last_pointer_hover_pos = pointer_pos;
    }

    pub(crate) fn handle_masonry_preload_focus_loss(&mut self, ctx: &egui::Context) {
        if !(self.mode().is_manga()
            && self.mode().is_masonry()
            && self.masonry_metadata_preload_active)
        {
            return;
        }

        let viewport_has_focus = ctx.input(|i| i.raw.viewport().focused.unwrap_or(true));
        if viewport_has_focus {
            return;
        }

        // Losing focus can drop key/pointer release events. Clear transient interaction
        // latches so masonry warmup keeps advancing while the app is in the background.
        self.manga_zoom_plus_held = false;
        self.manga_zoom_minus_held = false;
        self.manga_video_seeking = false;
        self.manga_video_volume_dragging = false;
        self.gif_seeking = false;
        self.manga_scrollbar_dragging = false;
        self.masonry_scrollbar_last_motion_at = None;
        self.is_panning = false;
        self.last_mouse_pos = None;
        self.stop_manga_wheel_scroll();
        self.stop_manga_autoscroll();
    }

    pub(crate) fn set_image_list_raw(&mut self, files: Vec<PathBuf>) {
        let new_signature = Self::compute_image_list_signature(&files);
        if self.image_list_signature != new_signature {
            self.masonry_runtime_cache_signature = 0;
            self.clear_masonry_authoritative_dimension_lock();
            self.solo_image_texture_cache.clear();
            self.solo_image_texture_cache_order.clear();
            // Index-keyed X marks describe a different list now.
            self.manga_unloadable_media.clear();
        }

        self.image_list = files;
        self.image_list_signature = new_signature;
    }
}

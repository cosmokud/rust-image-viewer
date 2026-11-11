//! `ImageViewer::new` and startup initialisation.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn poll_pending_media_directory_scan(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.pending_media_directory_scan.as_ref() else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.pending_media_directory_scan = None;
                self.pending_media_directory_target = None;
                self.pending_media_directory_scan_kind = None;
                self.pending_media_directory_started_at = None;
                return;
            }
        };

        self.pending_media_directory_scan = None;
        let scan_kind = self
            .pending_media_directory_scan_kind
            .take()
            .unwrap_or(PendingMediaDirectoryScanKind::InitialLoad);
        let Some(target_path) = self.pending_media_directory_target.take() else {
            self.pending_media_directory_started_at = None;
            return;
        };

        if let Some(started_at) = self.pending_media_directory_started_at.take() {
            self.perf_metrics
                .record(Metric::MediaIndexAsyncScanMs, started_at.elapsed());
        }

        let scanned_directory = result.directory.clone();
        let mut files = self
            .media_directory_index
            .apply_directory_scan_result(result);

        match scan_kind {
            PendingMediaDirectoryScanKind::InitialLoad => {
                if files.is_empty() {
                    files.push(target_path.clone());
                }

                let current_path = self.image_list.get(self.current_index).cloned();
                if current_path.as_ref() != Some(&target_path) {
                    return;
                }

                self.set_image_list(files);
                let resolved_index = self
                    .image_list
                    .iter()
                    .position(|candidate| candidate == &target_path)
                    .unwrap_or(0);
                self.set_current_index_clamped(resolved_index);
                if !self.defer_directory_work_for_fast_startup() {
                    self.schedule_solo_probe_window(&target_path, self.current_media_type);
                }
                ctx.request_repaint();
            }
            PendingMediaDirectoryScanKind::ExternalRefresh => {
                let current_path_before = self.current_media_path();
                let current_index_before = self.current_index;
                let current_directory = current_path_before
                    .as_ref()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
                    .or_else(|| {
                        self.image_list
                            .first()
                            .and_then(|path| path.parent().map(Path::to_path_buf))
                    });

                if current_directory.as_deref() != Some(scanned_directory.as_path()) {
                    return;
                }

                if self.try_append_new_entries_in_strip_mode(&files) {
                    self.clear_stale_marked_files();
                    self.clear_stale_prepared_clipboard_paths();
                    self.modal_thumbnail_cache.retain(|path, _| path.exists());
                    ctx.request_repaint();
                    return;
                }

                if self.mode().is_manga() && self.mode().is_true_masonry() {
                    self.persist_current_masonry_folder_metadata_snapshot();
                }

                self.set_image_list(files);
                self.clear_stale_marked_files();
                self.clear_stale_prepared_clipboard_paths();
                self.modal_thumbnail_cache.retain(|path, _| path.exists());

                if self.image_list.is_empty() {
                    self.clear_current_media_after_all_files_removed();
                    ctx.request_repaint();
                    return;
                }

                let previous_was_folder_entry = current_path_before
                    .as_ref()
                    .is_some_and(|path| self.is_folder_navigation_entry_path(path.as_path()));
                let same_path_index = current_path_before.as_ref().and_then(|path| {
                    self.image_list
                        .iter()
                        .position(|candidate| candidate == path)
                });
                let first_media_index = self
                    .image_list
                    .iter()
                    .position(|path| !self.is_folder_navigation_entry_path(path.as_path()));

                let resolved_index = if previous_was_folder_entry {
                    first_media_index.or(same_path_index).unwrap_or_else(|| {
                        current_index_before.min(self.image_list.len().saturating_sub(1))
                    })
                } else {
                    same_path_index.or(first_media_index).unwrap_or_else(|| {
                        current_index_before.min(self.image_list.len().saturating_sub(1))
                    })
                };
                self.set_current_index_clamped(resolved_index);

                if let Some(path) = self.current_media_path() {
                    self.pending_window_title = Some(self.compute_window_title_for_path(&path));
                }

                if self.mode().is_manga() {
                    self.manga_clear_cache();
                    self.ensure_manga_loader();
                    if self.mode().is_grid() {
                        self.restore_masonry_folder_metadata_snapshot();
                        self.mark_manga_dimension_cache_current_if_complete();
                    }
                    self.manga_update_preload_queue();
                }

                ctx.request_repaint();
            }
        }
    }
}

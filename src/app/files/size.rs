//! Asynchronous file-size probing for dialogs.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn file_size_label_for_path(path: &Path) -> Option<String> {
        std::fs::metadata(path)
            .ok()
            .map(|metadata| Self::format_file_size(metadata.len()))
    }

    pub(crate) fn delete_modal_item_info(&self, path: &PathBuf) -> DeleteModalItemInfo {
        let display_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let file_size_label =
            Self::file_size_label_for_path(path).unwrap_or_else(|| "Unknown size".to_string());

        let current_path = self.current_media_path();
        let known_dimensions = get_media_type(path).and_then(|media_type| {
            if current_path.as_ref().is_some_and(|current| current == path) {
                self.media_display_dimensions()
                    .or_else(|| self.solo_known_media_dimensions(path, media_type, true))
            } else {
                self.solo_known_media_dimensions(path, media_type, true)
            }
        });
        let dimensions_label = known_dimensions
            .map(|(width, height)| format!("{} x {} px", width, height))
            .unwrap_or_else(|| "Unknown dimensions".to_string());

        DeleteModalItemInfo {
            path: path.clone(),
            display_name,
            file_size_label,
            dimensions_label,
        }
    }

    pub(crate) fn start_async_file_size_probe(&mut self, path: PathBuf) {
        let (tx, rx) = crossbeam_channel::bounded::<(PathBuf, Option<String>)>(1);
        self.pending_file_size_probe = Some(rx);
        self.pending_file_size_probe_path = Some(path.clone());

        crate::async_runtime::spawn_blocking_or_thread("file-size-probe", move || {
            let label = Self::file_size_label_for_path(path.as_path());
            let _ = tx.send((path, label));
        });
    }

    pub(crate) fn poll_pending_file_size_probe(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.pending_file_size_probe.as_ref() else {
            return;
        };

        match rx.try_recv() {
            Ok((path, label)) => {
                let matches_pending = self
                    .pending_file_size_probe_path
                    .as_ref()
                    .is_some_and(|pending_path| pending_path == &path);
                self.pending_file_size_probe = None;
                self.pending_file_size_probe_path = None;

                if !matches_pending {
                    return;
                }

                if self
                    .image_list
                    .get(self.current_index)
                    .is_some_and(|current| current == &path)
                {
                    self.current_file_size_label_path = Some(path.clone());
                    self.current_file_size_label = label;
                    ctx.request_repaint();
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.pending_file_size_probe = None;
                self.pending_file_size_probe_path = None;
            }
        }
    }

    pub(crate) fn ensure_current_file_size_label(&mut self) {
        let Some(path) = self.image_list.get(self.current_index).cloned() else {
            self.current_file_size_label = None;
            self.current_file_size_label_path = None;
            return;
        };

        if self.defer_directory_work_for_fast_startup() {
            return;
        }

        if self
            .current_file_size_label_path
            .as_ref()
            .is_some_and(|current| current == &path)
        {
            return;
        }

        if self.pending_file_size_probe.is_some()
            || self
                .pending_file_size_probe_path
                .as_ref()
                .is_some_and(|pending| pending == &path)
        {
            return;
        }

        self.current_file_size_label = None;
        self.current_file_size_label_path = None;
        self.start_async_file_size_probe(path);
    }

    pub(crate) fn animated_image_label_for_path(path: Option<&PathBuf>) -> &'static str {
        if let Some(path) = path {
            let is_webp = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("webp"))
                .unwrap_or(false);
            if is_webp {
                "WEBP"
            } else {
                "GIF"
            }
        } else {
            "GIF"
        }
    }
}

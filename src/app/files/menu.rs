//! File action menu labels, widths and the menu content.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn open_file_action_menu(&mut self, screen_pos: egui::Pos2, target_index: usize) {
        if target_index >= self.image_list.len() {
            return;
        }

        self.file_action_menu = Some(FileContextMenuState {
            screen_pos,
            target_index,
        });
        self.show_controls = true;
        self.controls_show_time = Instant::now();
    }

    pub(crate) fn file_action_menu_labels(&self, target_index: usize) -> Vec<&'static str> {
        let mut labels = Vec::with_capacity(12);
        labels.push(if self.is_index_marked(target_index) {
            "Unmark"
        } else {
            "Mark"
        });
        labels.extend(["Cut", "Copy", "Delete", "Rename", "Open file location"]);

        let has_marked_paths = !self.collect_marked_paths_in_current_order().is_empty();
        if has_marked_paths {
            labels.extend([
                "Cut Marked Files",
                "Copy Marked Files",
                "Delete Marked Files",
                "Rename Marked Files",
            ]);
        }

        labels.push("Mark All");
        if has_marked_paths {
            labels.push("Unmark All");
        }

        labels
    }

    pub(crate) fn file_action_menu_content_width(
        &self,
        ctx: &egui::Context,
        target_index: usize,
    ) -> f32 {
        let labels = self.file_action_menu_labels(target_index);
        let font_id = egui::TextStyle::Body.resolve(ctx.style().as_ref());
        let widest_label = ctx.fonts(|fonts| {
            labels
                .iter()
                .map(|label| {
                    fonts
                        .layout_no_wrap((*label).to_string(), font_id.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
                .fold(0.0, f32::max)
        });

        (widest_label + 46.0).clamp(128.0, 240.0)
    }

    pub(crate) fn release_video_resources_for_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let path_is_targeted = |candidate: &Path| paths.iter().any(|path| path == candidate);

        if self
            .current_media_path()
            .as_deref()
            .is_some_and(path_is_targeted)
        {
            self.clear_pending_media_load();
            self.stop_fullscreen_video_playback();
            self.reset_fullscreen_anim_stream_state();
        }

        if self
            .pending_manga_video_load
            .as_ref()
            .is_some_and(|pending| path_is_targeted(&pending.path))
        {
            self.clear_pending_manga_video_load();
        }

        let image_list = &self.image_list;
        let player_paths = &self.manga_video_player_paths;
        let focused_manga_video = self.manga_focused_video_index;
        let mut removed_focused_manga_video = false;
        self.manga_video_players.retain(|index, player| {
            let should_remove = image_list
                .get(*index)
                .is_none_or(|path| player_paths.get(index) != Some(path) || path_is_targeted(path));

            if should_remove {
                // Save the timestamp to RAM before destroying the list player
                if let Some(path) = player_paths.get(index) {
                    if let Some(current_pos) = player.position() {
                        self.manga_video_preview_resume_by_path
                            .insert(path.clone(), current_pos.as_secs_f64());
                    }
                }

                if Some(*index) == focused_manga_video {
                    removed_focused_manga_video = true;
                }
            }
            !should_remove
        });
        self.manga_video_player_paths
            .retain(|index, path| image_list.get(*index) == Some(path) && !path_is_targeted(path));
        self.manga_video_preview_resume_secs.retain(|index, _| {
            !image_list
                .get(*index)
                .is_some_and(|path| path_is_targeted(path))
        });
        self.manga_video_preview_resume_by_path
            .retain(|path, _| !path_is_targeted(path));
        self.manga_video_textures.retain(|index, _| {
            !image_list
                .get(*index)
                .is_some_and(|path| path_is_targeted(path))
        });
        self.manga_video_texture_paths
            .retain(|index, path| image_list.get(*index) == Some(path) && !path_is_targeted(path));
        if removed_focused_manga_video {
            self.manga_focused_video_index = None;
        }
    }

    pub(crate) fn start_inline_rename_for_index(&mut self, index: usize) {
        let Some(path) = self.image_list.get(index).cloned() else {
            return;
        };

        self.start_rename_dialog_for_paths(vec![path]);
    }
}

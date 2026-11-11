//! Masonry metadata preload pipeline.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn pause_masonry_metadata_preload(&mut self) {
        let total = self.masonry_metadata_preload_total;
        let can_resume = total > 0
            && self.masonry_metadata_preload_loaded < total
            && self.masonry_metadata_preload_list_signature == self.image_list_signature;

        self.masonry_metadata_preload_active = false;
        self.masonry_metadata_preload_overlay_hold_until = None;
        self.masonry_metadata_preload_defer_first_tick = false;
        self.masonry_metadata_preload_stall_since = None;

        if !can_resume {
            self.masonry_metadata_preload_total = 0;
            self.masonry_metadata_preload_loaded = 0;
            self.masonry_metadata_preload_cursor = 0;
            self.masonry_metadata_preload_list_signature = 0;
            self.masonry_metadata_preload_restore_index = None;
            self.pending_masonry_folder_travel_restore = None;
        }
    }

    pub(crate) fn begin_masonry_metadata_preload(&mut self) {
        let total = self.image_list.len();
        let resume_preload = self.masonry_metadata_preload_total == total
            && self.masonry_metadata_preload_loaded < total
            && self.masonry_metadata_preload_list_signature == self.image_list_signature;

        self.masonry_metadata_preload_total = total;
        self.masonry_metadata_preload_list_signature = self.image_list_signature;

        if resume_preload {
            self.masonry_metadata_preload_loaded = self.masonry_metadata_preload_loaded.min(total);
            self.masonry_metadata_preload_cursor = self
                .masonry_metadata_preload_cursor
                .min(total.saturating_sub(1));
            self.masonry_metadata_preload_restore_index = self
                .masonry_metadata_preload_restore_index
                .map(|index| index.min(total.saturating_sub(1)))
                .or_else(|| Some(self.current_index.min(total.saturating_sub(1))));
        } else {
            self.masonry_metadata_preload_loaded = 0;
            self.masonry_metadata_preload_restore_index = if self.image_list.is_empty() {
                None
            } else {
                Some(
                    self.current_index
                        .min(self.image_list.len().saturating_sub(1)),
                )
            };
            let preload_window = 96usize.max(self.masonry_items_per_row.clamp(2, 10) * 48);
            self.masonry_metadata_preload_cursor = self
                .current_index
                .min(self.masonry_metadata_preload_total.saturating_sub(1))
                .saturating_sub(preload_window / 2);
            self.pending_masonry_folder_travel_restore = None;
        }

        self.masonry_metadata_preload_active = self.mode().is_manga()
            && self.mode().is_masonry()
            && self.masonry_metadata_preload_total > 0
            && self.manga_loader.is_some();

        if !self.masonry_metadata_preload_active {
            self.masonry_metadata_preload_restore_index = None;
            self.masonry_metadata_preload_overlay_hold_until = None;
            self.masonry_metadata_preload_defer_first_tick = false;
            return;
        }

        self.masonry_metadata_preload_overlay_hold_until =
            Some(Instant::now() + Duration::from_millis(220));
        self.masonry_metadata_preload_defer_first_tick = true;
        self.masonry_metadata_preload_stall_since = None;

        self.manga_scrollbar_dragging = false;
        self.is_panning = false;
        self.last_mouse_pos = None;
        self.manga_hovered_media_index = None;
        self.manga_zoom_plus_held = false;
        self.manga_zoom_minus_held = false;
        self.manga_video_seeking = false;
        self.manga_video_volume_dragging = false;
        self.gif_seeking = false;
        self.manga_scroll_target = self.manga_scroll_offset;
        self.manga_scroll_velocity = 0.0;
        self.stop_manga_wheel_scroll();
        self.stop_manga_autoscroll();
    }

    pub(crate) fn masonry_metadata_overlay_visible(&self) -> bool {
        if self.masonry_metadata_preload_active {
            return true;
        }

        self.masonry_metadata_preload_overlay_hold_until
            .is_some_and(|hold_until| Instant::now() < hold_until)
    }

    pub(crate) fn maybe_begin_masonry_metadata_preload(&mut self, allow_startup_preload: bool) {
        if self.image_list.is_empty() {
            self.reset_masonry_metadata_preload();
            return;
        }
        if self.mode().layout() != MangaLayoutMode::Masonry {
            self.pause_masonry_metadata_preload();
            return;
        }

        let total = self.image_list.len();
        let folder_ready = self
            .image_list
            .iter()
            .filter(|path| self.is_folder_navigation_entry_path(path.as_path()))
            .count();
        let fully_warm = self.manga_loader.as_ref().is_some_and(|loader| {
            let backlog = loader.backlog();
            loader
                .cached_dimensions_count(total)
                .saturating_add(folder_ready)
                >= total
                && backlog.dimension_probes == 0
                && backlog.dimension_results == 0
        });

        if fully_warm || !allow_startup_preload {
            self.reset_masonry_metadata_preload();
        } else {
            self.begin_masonry_metadata_preload();
        }
    }

    pub(crate) fn restore_masonry_scroll_after_metadata_preload(&mut self) {
        if self.mode().is_solo() || !self.mode().is_masonry() || self.image_list.is_empty() {
            self.masonry_metadata_preload_restore_index = None;
            self.pending_masonry_folder_travel_restore = None;
            return;
        }

        if let Some(target_index) = self.masonry_metadata_preload_restore_index.take() {
            let target_index = target_index.min(self.image_list.len().saturating_sub(1));
            self.set_current_index_clamped(target_index);

            let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
            let scroll_to = self
                .masonry_scroll_offset_for_index_centered(target_index)
                .unwrap_or_else(|| {
                    self.manga_get_scroll_offset_for_index(target_index)
                        .clamp(0.0, max_scroll)
                });

            self.manga_scroll_offset = scroll_to;
            self.manga_scroll_target = scroll_to;
            self.manga_scroll_velocity = 0.0;
            self.manga_scrollbar_dragging = false;
            self.masonry_scrollbar_last_motion_at = None;
            self.masonry_autoscroll_last_motion_at = None;
            self.is_panning = false;
            self.last_mouse_pos = None;
            self.manga_hovered_media_index = None;
            self.stop_manga_wheel_scroll();
        }

        if let Some((target_index, restored_offset)) =
            self.pending_masonry_folder_travel_restore.take()
        {
            let target_index = target_index.min(self.image_list.len().saturating_sub(1));
            self.set_current_index_clamped(target_index);

            let max_scroll = (self.manga_total_height() - self.screen_size.y).max(0.0);
            let restored_offset = restored_offset.clamp(0.0, max_scroll);
            self.manga_scroll_offset = restored_offset;
            self.manga_scroll_target = restored_offset;
            self.manga_scroll_velocity = 0.0;
            self.manga_scrollbar_dragging = false;
            self.masonry_scrollbar_last_motion_at = None;
            self.masonry_autoscroll_last_motion_at = None;
            self.is_panning = false;
            self.last_mouse_pos = None;
            self.manga_hovered_media_index = None;
            self.stop_manga_wheel_scroll();
        }
    }

    pub(crate) fn tick_masonry_metadata_preload(&mut self) {
        if !self.masonry_metadata_preload_active {
            return;
        }

        if self.mode().is_solo() || !self.mode().is_masonry() {
            self.reset_masonry_metadata_preload();
            return;
        }

        let total = self
            .masonry_metadata_preload_total
            .min(self.image_list.len());
        if total == 0 {
            self.reset_masonry_metadata_preload();
            return;
        }

        if self.masonry_metadata_preload_defer_first_tick {
            self.masonry_metadata_preload_defer_first_tick = false;
            return;
        }

        let navigation_active = self.masonry_navigation_active_for_heavy_work();
        let mut allow_preload_step = !navigation_active;
        let now = Instant::now();
        let preload_cursor = self
            .masonry_metadata_preload_cursor
            .min(total.saturating_sub(1));
        let preload_window = 96usize.max(self.masonry_items_per_row.clamp(2, 10) * 48);
        let preload_end = (preload_cursor + preload_window).min(total);
        let folder_ready = self
            .image_list
            .iter()
            .take(total)
            .filter(|path| self.is_folder_navigation_entry_path(path.as_path()))
            .count();

        let (mut loaded_count, mut pending_probe_count, mut pending_result_count) = {
            let Some(loader) = self.manga_loader.as_mut() else {
                self.reset_masonry_metadata_preload();
                return;
            };

            if allow_preload_step {
                loader.warm_dimensions_background(&self.image_list, preload_cursor, preload_end, 0);
            }

            let backlog = loader.backlog();
            (
                loader
                    .cached_dimensions_count(total)
                    .saturating_add(folder_ready)
                    .min(total),
                backlog.dimension_probes,
                backlog.dimension_results,
            )
        };

        let previous_loaded = self.masonry_metadata_preload_loaded.min(total);
        let mut progress_advanced = loaded_count > previous_loaded;

        if progress_advanced || loaded_count >= total {
            self.masonry_metadata_preload_stall_since = None;
        } else {
            let stall_since = self.masonry_metadata_preload_stall_since.get_or_insert(now);
            let stall_elapsed = now.saturating_duration_since(*stall_since);
            if stall_elapsed >= Duration::from_millis(900) {
                allow_preload_step = true;

                let (next_loaded, next_pending_probe, next_pending_result, fallback_seeded) = {
                    let Some(loader) = self.manga_loader.as_mut() else {
                        self.reset_masonry_metadata_preload();
                        return;
                    };

                    let fallback_seeded = loader.warm_dimensions_background(
                        &self.image_list,
                        preload_cursor,
                        preload_end,
                        24,
                    );
                    let backlog = loader.backlog();

                    (
                        loader
                            .cached_dimensions_count(total)
                            .saturating_add(folder_ready)
                            .min(total),
                        backlog.dimension_probes,
                        backlog.dimension_results,
                        fallback_seeded,
                    )
                };

                loaded_count = next_loaded;
                pending_probe_count = next_pending_probe;
                pending_result_count = next_pending_result;
                progress_advanced = loaded_count > previous_loaded || fallback_seeded > 0;

                if progress_advanced {
                    self.masonry_metadata_preload_stall_since = None;
                } else {
                    // Keep retry cadence bounded instead of retrying every frame.
                    self.masonry_metadata_preload_stall_since = Some(now);
                }
            }
        }

        if allow_preload_step {
            self.masonry_metadata_preload_cursor =
                if preload_end >= total { 0 } else { preload_end };
        }

        self.masonry_metadata_preload_loaded = loaded_count;

        let scan_complete =
            loaded_count >= total && pending_probe_count == 0 && pending_result_count == 0;

        if scan_complete {
            self.masonry_metadata_preload_loaded = total;
            self.masonry_metadata_preload_active = false;
            self.masonry_metadata_preload_stall_since = None;
            self.manga_update_preload_queue();
            if self.masonry_pending_dimension_updates.is_empty() {
                self.restore_masonry_scroll_after_metadata_preload();
            }
        }
    }

    pub(crate) fn draw_masonry_metadata_loading_overlay(&self, ctx: &egui::Context) {
        if !self.masonry_metadata_overlay_visible() {
            return;
        }

        let total = self.masonry_metadata_preload_total.max(1);
        let loaded = self.masonry_metadata_preload_loaded.min(total);
        let progress_ratio = (loaded as f32 / total as f32).clamp(0.0, 1.0);
        let progress_text = format!("Warming layout  {} / {}", loaded, total);
        let screen_rect = ctx.screen_rect();
        let panel_width = (screen_rect.width() - 48.0).clamp(280.0, 420.0);
        let panel_size = egui::vec2(panel_width, 144.0);

        egui::Area::new(egui::Id::new("masonry_metadata_loading_overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                let overlay_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, screen_rect.size());
                let _ = ui.allocate_rect(overlay_rect, egui::Sense::click_and_drag());
                ui.painter().rect_filled(
                    overlay_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(5, 8, 12, 150),
                );

                let panel_rect = egui::Rect::from_center_size(overlay_rect.center(), panel_size);
                ui.painter().rect_filled(
                    panel_rect,
                    18.0,
                    egui::Color32::from_rgba_unmultiplied(18, 22, 28, 240),
                );
                ui.painter().rect_stroke(
                    panel_rect,
                    18.0,
                    egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgba_unmultiplied(130, 188, 255, 72),
                    ),
                );

                ui.painter().text(
                    egui::pos2(panel_rect.center().x, panel_rect.min.y + 34.0),
                    egui::Align2::CENTER_CENTER,
                    "Preparing masonry layout",
                    egui::FontId::proportional(20.0),
                    egui::Color32::WHITE,
                );
                ui.painter().text(
                    egui::pos2(panel_rect.center().x, panel_rect.min.y + 64.0),
                    egui::Align2::CENTER_CENTER,
                    progress_text,
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_gray(214),
                );
                ui.painter().text(
                    egui::pos2(panel_rect.center().x, panel_rect.min.y + 88.0),
                    egui::Align2::CENTER_CENTER,
                    "Navigation is paused until the layout stabilizes.",
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_gray(170),
                );

                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + 24.0, panel_rect.max.y - 30.0),
                    egui::vec2(panel_rect.width() - 48.0, 10.0),
                );
                ui.painter().rect_filled(
                    bar_rect,
                    5.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                );
                if progress_ratio > 0.0 {
                    let fill_rect = egui::Rect::from_min_max(
                        bar_rect.min,
                        egui::pos2(
                            bar_rect.min.x + bar_rect.width() * progress_ratio,
                            bar_rect.max.y,
                        ),
                    );
                    ui.painter().rect_filled(
                        fill_rect,
                        5.0,
                        egui::Color32::from_rgb(104, 184, 255),
                    );
                }
            });
    }

    pub(crate) fn clear_manga_runtime_workloads(&mut self) {
        self.clear_pending_manga_video_load();
        self.manga_decoded_mailbox.clear();
        self.clear_manga_video_players();
        self.manga_video_failed.clear();
        self.manga_focused_video_index = None;
        self.manga_hovered_media_index = None;
        self.manga_hover_autoplay_resume_at = Instant::now();
        self.manga_anim_streams.clear();
        self.manga_anim_stream_done.clear();
        self.manga_focused_anim_index = None;
    }
}

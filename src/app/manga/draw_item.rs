//! Painting a single strip/masonry tile.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_manga_item(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        image_rect: egui::Rect,
        navigation_active_for_visible_retry: bool,
    ) -> bool {
        let image_rect = if self.mode().is_masonry() {
            self.masonry_item_display_rect(idx, image_rect)
        } else {
            image_rect
        };
        let gallery_mode = self.mode().is_gallery();
        let gallery_fit_rect = |rect: egui::Rect, width: u32, height: u32| -> egui::Rect {
            if !gallery_mode || width == 0 || height == 0 {
                return rect;
            }
            let fitted = Self::fit_size_preserving_aspect(
                egui::vec2(width as f32, height as f32),
                rect.size(),
            );
            egui::Rect::from_center_size(rect.center(), fitted)
        };
        let mut requested_retry = false;

        let folder_entry = self.image_list.get(idx).cloned().and_then(|path| {
            if self.is_folder_navigation_entry_path(path.as_path()) {
                Some(path)
            } else {
                None
            }
        });

        if let Some(path) = folder_entry.as_ref() {
            self.paint_folder_entry_card(ui, image_rect, path.as_path());

            let preview_only = self.mark_selection_preview_contains(idx)
                && self.mark_visual_for_index(idx).is_none();
            let mark_visual = if preview_only {
                Some(FileMarkVisual::Preview)
            } else {
                self.mark_visual_for_index(idx)
            };
            if let Some(mark_visual) = mark_visual {
                self.paint_marked_item_overlay(ui.painter(), image_rect, mark_visual);
            }

            return false;
        }

        let media_path = self.image_list.get(idx).cloned();
        let probably_animated_by_path = media_path
            .as_ref()
            .is_some_and(|path| self.is_probably_animated_image_path(path.as_path()))
            && !self.manga_anim_failed.contains(&idx);

        let cached_media_type = self
            .manga_loader
            .as_ref()
            .and_then(|loader| loader.get_media_type(idx));

        // Check if this item is a video
        let is_video = cached_media_type.map_or(false, |mt| mt == MangaMediaType::Video);

        let is_animated_image = cached_media_type
            .map_or(false, |mt| mt == MangaMediaType::AnimatedImage)
            || probably_animated_by_path;

        // Also check by file extension as a fallback
        let is_video = is_video
            || self
                .image_list
                .get(idx)
                .map_or(false, |p| is_supported_video(p));

        let retry_media_type = if is_video {
            MangaMediaType::Video
        } else if is_animated_image {
            MangaMediaType::AnimatedImage
        } else {
            MangaMediaType::StaticImage
        };
        let video_playback_runtime_available = !is_video || gstreamer_runtime_available();
        let video_thumbnail_retry_allowed =
            !is_video || video_playback_runtime_available || cfg!(target_os = "windows");
        let (fill_retry_target_side, final_retry_target_side) =
            self.manga_retry_target_sides_for_rect(idx, image_rect, retry_media_type);
        let final_retry_dims =
            self.manga_bucket_dimensions_for_side(idx, final_retry_target_side, image_rect.size());

        let video_load_pending = video_playback_runtime_available
            && is_video
            && self.manga_video_load_pending_for_index(idx);

        if is_video {
            // Video item: prioritize live video texture, fall back to first-frame thumbnail
            if !self.manga_video_texture_matches(idx) {
                self.remove_manga_video_texture(idx);
            }

            if let Some((texture, tex_w, tex_h)) = self.manga_video_textures.get(&idx) {
                // Live video frame available - use it
                let draw_rect = gallery_fit_rect(image_rect, *tex_w, *tex_h);
                ui.painter().image(
                    texture.id(),
                    draw_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Draw play indicator only when not actively playing.
                let is_playing = self
                    .manga_video_players
                    .get(&idx)
                    .map_or(false, |p| p.is_playing());

                if !is_playing {
                    let icon_bg_rect =
                        egui::Rect::from_center_size(image_rect.center(), egui::Vec2::splat(50.0));
                    ui.painter().rect_filled(
                        icon_bg_rect,
                        25.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 128),
                    );
                    ui.painter().text(
                        image_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "▶",
                        egui::FontId::proportional(28.0),
                        egui::Color32::WHITE,
                    );
                }
            } else if self.strip_entry_placeholder_matches(idx) {
                // Immediate fallback when entering strip mode from solo-video fullscreen.
                // Keeps only the strip-entry frame visible until manga cache catches up.
                if self.strip_entry_video_texture_matches_placeholder_path() {
                    if let Some(texture) = self.video_texture.as_ref() {
                        let draw_rect = self
                            .video_texture_dims
                            .map(|(w, h)| gallery_fit_rect(image_rect, w, h))
                            .unwrap_or(image_rect);
                        ui.painter().image(
                            texture.id(),
                            draw_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        ui.painter()
                            .rect_filled(image_rect, 0.0, egui::Color32::from_gray(25));
                        ui.painter().text(
                            image_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "🎬",
                            egui::FontId::proportional(32.0),
                            egui::Color32::from_gray(100),
                        );
                    }
                } else {
                    ui.painter()
                        .rect_filled(image_rect, 0.0, egui::Color32::from_gray(25));
                    ui.painter().text(
                        image_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "🎬",
                        egui::FontId::proportional(32.0),
                        egui::Color32::from_gray(100),
                    );
                }

                if video_thumbnail_retry_allowed {
                    requested_retry |= self.manga_request_retry_for_visible_item(
                        idx,
                        fill_retry_target_side,
                        false,
                        retry_media_type,
                        navigation_active_for_visible_retry,
                    );
                }
            } else if let Some((texture_id, tex_w, tex_h)) =
                self.image_list.get(idx).and_then(|path| {
                    self.manga_texture_cache
                        .get_texture_info_for_path(idx, path)
                })
            {
                // First-frame thumbnail from texture cache - use it as a preview
                let draw_rect = gallery_fit_rect(image_rect, tex_w, tex_h);
                ui.painter().image(
                    texture_id,
                    draw_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                let is_playing = self
                    .manga_video_players
                    .get(&idx)
                    .map_or(false, |p| p.is_playing());

                if !is_playing {
                    // Draw a play icon overlay to indicate it's a video
                    let icon_bg_rect =
                        egui::Rect::from_center_size(image_rect.center(), egui::Vec2::splat(60.0));
                    ui.painter().rect_filled(
                        icon_bg_rect,
                        30.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                    );
                    ui.painter().text(
                        image_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "▶",
                        egui::FontId::proportional(32.0),
                        egui::Color32::WHITE,
                    );
                }

                if video_thumbnail_retry_allowed
                    && self.manga_texture_upgrade_needed(
                        tex_w,
                        tex_h,
                        final_retry_dims.0,
                        final_retry_dims.1,
                        retry_media_type,
                    )
                {
                    requested_retry |= self.manga_request_retry_for_visible_item(
                        idx,
                        final_retry_target_side,
                        true,
                        retry_media_type,
                        navigation_active_for_visible_retry,
                    );
                }
            } else {
                // Video not yet loaded - draw placeholder with video icon
                ui.painter()
                    .rect_filled(image_rect, 0.0, egui::Color32::from_gray(25));
                ui.painter().text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🎬",
                    egui::FontId::proportional(32.0),
                    egui::Color32::from_gray(100),
                );
                self.manga_paint_placeholder_spinner(ui, idx, image_rect);

                if video_thumbnail_retry_allowed {
                    self.manga_mark_placeholder_visible(idx);

                    requested_retry |= self.manga_request_retry_for_visible_item(
                        idx,
                        fill_retry_target_side,
                        false,
                        retry_media_type,
                        navigation_active_for_visible_retry,
                    );
                }
            }

            if video_load_pending
                && self.manga_focused_video_index == Some(idx)
                && !self.manga_video_texture_matches(idx)
            {
                let time = ui.input(|i| i.time);
                paint_loading_spinner(ui.painter(), image_rect, time);
            }
        } else if self.manga_media_is_unloadable(idx) {
            // The file already failed to produce a drawable texture: X card, no request, no
            // spinner. Nothing here may touch the loader, or every frame would re-decode it.
            self.paint_unloadable_media_placeholder(ui, image_rect, "Can't load");
        } else {
            // Image item: use regular texture cache
            if let Some((texture_id, tex_w, tex_h)) = self.image_list.get(idx).and_then(|path| {
                self.manga_texture_cache
                    .get_texture_info_for_path(idx, path)
            }) {
                let draw_rect = gallery_fit_rect(image_rect, tex_w, tex_h);
                ui.painter().image(
                    texture_id,
                    draw_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Show loading spinner only for the focused animated image.
                let is_focused_anim = self.manga_focused_anim_index == Some(idx);
                let still_streaming = self
                    .manga_anim_stream_done
                    .get(&idx)
                    .map_or(false, |&done| !done);
                let has_active_stream = self.manga_anim_streams.contains_key(&idx);
                if is_focused_anim && (still_streaming || has_active_stream) {
                    let time = ui.input(|i| i.time);
                    paint_loading_spinner(ui.painter(), image_rect, time);
                }

                if is_animated_image {
                    let is_actively_animating = self.manga_focused_anim_index == Some(idx)
                        && !self.gif_paused
                        && self
                            .manga_animated_images
                            .get(&idx)
                            .is_some_and(|img| img.is_animated());

                    if !is_actively_animating {
                        let icon_bg_rect = egui::Rect::from_center_size(
                            image_rect.center(),
                            egui::Vec2::splat(60.0),
                        );
                        ui.painter().rect_filled(
                            icon_bg_rect,
                            30.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                        );
                        ui.painter().text(
                            image_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "▶",
                            egui::FontId::proportional(32.0),
                            egui::Color32::WHITE,
                        );
                    }
                }

                if self.manga_texture_upgrade_needed(
                    tex_w,
                    tex_h,
                    final_retry_dims.0,
                    final_retry_dims.1,
                    retry_media_type,
                ) {
                    requested_retry |= self.manga_request_retry_for_visible_item(
                        idx,
                        final_retry_target_side,
                        true,
                        retry_media_type,
                        navigation_active_for_visible_retry,
                    );
                }
            } else if self.strip_entry_placeholder_matches(idx) {
                // Immediate fallback when entering strip mode from solo-image fullscreen.
                // Keeps only the strip-entry image visible while manga textures are still loading.
                if self.strip_entry_image_texture_matches_placeholder_path() {
                    if let Some(texture) = self.texture.as_ref() {
                        let draw_rect = self
                            .image_texture_dims
                            .map(|(w, h)| gallery_fit_rect(image_rect, w, h))
                            .unwrap_or(image_rect);
                        ui.painter().image(
                            texture.id(),
                            draw_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        ui.painter()
                            .rect_filled(image_rect, 0.0, egui::Color32::from_gray(30));
                        ui.painter().text(
                            image_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "⏳",
                            egui::FontId::proportional(24.0),
                            egui::Color32::from_gray(80),
                        );
                    }
                } else {
                    ui.painter()
                        .rect_filled(image_rect, 0.0, egui::Color32::from_gray(30));
                    ui.painter().text(
                        image_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "⏳",
                        egui::FontId::proportional(24.0),
                        egui::Color32::from_gray(80),
                    );
                }

                requested_retry |= self.manga_request_retry_for_visible_item(
                    idx,
                    fill_retry_target_side,
                    false,
                    retry_media_type,
                    navigation_active_for_visible_retry,
                );
            } else {
                // Image not loaded yet - draw a placeholder
                ui.painter()
                    .rect_filled(image_rect, 0.0, egui::Color32::from_gray(30));

                // Draw a subtle loading spinner or indicator
                ui.painter().text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "⏳",
                    egui::FontId::proportional(24.0),
                    egui::Color32::from_gray(80),
                );
                self.manga_paint_placeholder_spinner(ui, idx, image_rect);

                if is_animated_image {
                    let icon_bg_rect =
                        egui::Rect::from_center_size(image_rect.center(), egui::Vec2::splat(60.0));
                    ui.painter().rect_filled(
                        icon_bg_rect,
                        30.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                    );
                    ui.painter().text(
                        image_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "▶",
                        egui::FontId::proportional(32.0),
                        egui::Color32::WHITE,
                    );
                }

                self.manga_mark_placeholder_visible(idx);

                requested_retry |= self.manga_request_retry_for_visible_item(
                    idx,
                    fill_retry_target_side,
                    false,
                    retry_media_type,
                    navigation_active_for_visible_retry,
                );
            }
        }

        let preview_only =
            self.mark_selection_preview_contains(idx) && self.mark_visual_for_index(idx).is_none();
        let mark_visual = if preview_only {
            Some(FileMarkVisual::Preview)
        } else {
            self.mark_visual_for_index(idx)
        };
        if let Some(mark_visual) = mark_visual {
            self.paint_marked_item_overlay(ui.painter(), image_rect, mark_visual);
        }

        requested_retry
    }
}

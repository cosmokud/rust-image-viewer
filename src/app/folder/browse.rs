//! Folder placeholder cards, entry traversal and drive roots.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn is_up_navigation_entry_path(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(Self::is_up_navigation_entry_name)
    }

    pub(crate) fn folder_entry_display_name(path: &Path) -> String {
        path.file_name()
            .map(|name| {
                let label = name.to_string_lossy().to_string();
                if Self::is_up_navigation_entry_name(label.as_str()) {
                    "[..]".to_string()
                } else {
                    label
                }
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| path.display().to_string())
    }

    pub(crate) fn folder_entry_target_directory(path: &Path) -> Option<PathBuf> {
        if Self::is_up_navigation_entry_path(path) {
            let current_directory = path.parent()?;
            current_directory.parent().map(Path::to_path_buf)
        } else if path.is_dir() {
            Some(path.to_path_buf())
        } else if let Some(target_directory) = resolve_folder_shortcut_target(path) {
            Some(target_directory)
        } else {
            None
        }
    }

    pub(crate) fn is_folder_navigation_entry_path(&self, path: &Path) -> bool {
        if get_media_type(path).is_some() {
            return false;
        }

        Self::folder_entry_target_directory(path).is_some()
    }

    pub(crate) fn traverse_folder_entry_path(&mut self, path: &Path) -> bool {
        let Some(target_directory) = Self::folder_entry_target_directory(path) else {
            return false;
        };

        self.navigate_to_breadcrumb_directory(target_directory.as_path());
        true
    }

    pub(crate) fn build_folder_placeholder_image(path: PathBuf, is_up_entry: bool) -> LoadedImage {
        const SIZE: usize = 512;

        let mut pixels = vec![0_u8; SIZE * SIZE * 4];
        let mut fill_rect = |x0: usize, y0: usize, x1: usize, y1: usize, rgba: [u8; 4]| {
            let x_start = x0.min(SIZE);
            let y_start = y0.min(SIZE);
            let x_end = x1.min(SIZE);
            let y_end = y1.min(SIZE);
            for y in y_start..y_end {
                for x in x_start..x_end {
                    let base = (y * SIZE + x) * 4;
                    pixels[base] = rgba[0];
                    pixels[base + 1] = rgba[1];
                    pixels[base + 2] = rgba[2];
                    pixels[base + 3] = rgba[3];
                }
            }
        };

        fill_rect(0, 0, SIZE, SIZE, [24, 28, 34, 255]);
        fill_rect(64, 132, 448, 424, [221, 178, 73, 255]);
        fill_rect(100, 92, 284, 170, [234, 196, 108, 255]);
        fill_rect(84, 176, 428, 392, [247, 219, 149, 255]);
        fill_rect(84, 392, 428, 424, [194, 146, 48, 255]);

        if is_up_entry {
            fill_rect(248, 228, 264, 332, [255, 255, 255, 255]);
            fill_rect(216, 228, 296, 244, [255, 255, 255, 255]);
            fill_rect(224, 208, 288, 224, [255, 255, 255, 255]);
            fill_rect(232, 188, 280, 204, [255, 255, 255, 255]);
        } else {
            fill_rect(220, 248, 292, 264, [255, 255, 255, 255]);
            fill_rect(220, 248, 236, 322, [255, 255, 255, 255]);
            fill_rect(276, 248, 292, 322, [255, 255, 255, 255]);
            fill_rect(220, 306, 292, 322, [255, 255, 255, 255]);
        }

        LoadedImage::from_single_frame(
            path,
            ImageFrame {
                pixels,
                width: SIZE as u32,
                height: SIZE as u32,
                delay_ms: 0,
            },
            SIZE as u32,
            SIZE as u32,
        )
    }

    pub(crate) fn collect_folder_placeholder_preview_media_paths(
        target_directory: &Path,
        max_count: usize,
    ) -> Vec<PathBuf> {
        let max_count = max_count.min(4);
        if max_count == 0 || !target_directory.is_dir() {
            return Vec::new();
        }

        let Ok(entries) = fs::read_dir(target_directory) else {
            return Vec::new();
        };

        // Folder cards are non-critical UI. Stop as soon as enough preview
        // candidates are found so a large child folder cannot monopolize disk I/O.
        const MAX_SCANNED_ENTRIES: usize = 2048;
        let mut media_paths: Vec<PathBuf> = Vec::with_capacity(max_count);
        for entry in entries.flatten().take(MAX_SCANNED_ENTRIES) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }

            let candidate = entry.path();
            if get_media_type(candidate.as_path()).is_none() {
                continue;
            }

            media_paths.push(candidate);
            if media_paths.len() >= max_count {
                break;
            }
        }

        media_paths
    }

    pub(crate) fn request_folder_placeholder_preview_scan(
        &mut self,
        target_directory: &PathBuf,
        max_count: usize,
    ) -> bool {
        if self
            .folder_placeholder_preview_scan_pending
            .contains(target_directory)
        {
            return true;
        }

        if self.folder_placeholder_preview_scan_pending.len()
            >= self.folder_placeholder_preview_scan_pending_soft_limit()
        {
            return false;
        }

        let directory = target_directory.clone();
        self.folder_placeholder_preview_request_priority_seed = self
            .folder_placeholder_preview_request_priority_seed
            .saturating_add(1);
        let priority = -self.folder_placeholder_preview_request_priority_seed;
        self.folder_placeholder_preview_scan_pending
            .insert(directory.clone());

        let request = FolderPlaceholderPreviewScanRequest {
            directory: directory.clone(),
            max_count,
            priority,
        };

        if self
            .folder_placeholder_preview_scan_request_tx
            .try_send(request)
            .is_err()
        {
            self.folder_placeholder_preview_scan_pending
                .remove(&directory);
            return false;
        }

        true
    }

    pub(crate) fn folder_entry_preview_media_paths(
        &mut self,
        entry_path: &Path,
        max_count: usize,
    ) -> (Vec<PathBuf>, bool) {
        let max_count = max_count.min(4);
        if max_count == 0 {
            return (Vec::new(), false);
        }

        let Some(target_directory) = Self::folder_entry_target_directory(entry_path) else {
            return (Vec::new(), false);
        };

        let cached_selection = self
            .folder_placeholder_thumbnail_cache
            .get(target_directory.as_path())
            .cloned();
        if let Some(cached) = cached_selection {
            if cached.loading {
                return (cached.media_paths, true);
            }

            let stamp = self.cached_file_stamp(
                target_directory.as_path(),
                Self::FOLDER_PLACEHOLDER_STAMP_CACHE_TTL,
            );
            if cached.stamp == stamp {
                return (cached.media_paths, cached.loading);
            }

            if self.folder_placeholder_heavy_work_deferred() {
                return (cached.media_paths, true);
            }

            let loading =
                self.request_folder_placeholder_preview_scan(&target_directory, max_count);
            if let Some(cached) = self
                .folder_placeholder_thumbnail_cache
                .get_mut(target_directory.as_path())
            {
                cached.loading = loading;
                return (cached.media_paths.clone(), cached.loading);
            }
        }

        let loading = self.request_folder_placeholder_preview_scan(&target_directory, max_count);

        if let Some(cached) = self
            .folder_placeholder_thumbnail_cache
            .get_mut(target_directory.as_path())
        {
            cached.loading = loading;
            return (cached.media_paths.clone(), cached.loading);
        }

        self.folder_placeholder_thumbnail_cache.insert(
            target_directory,
            FolderPlaceholderThumbnailSelection {
                stamp: None,
                media_paths: Vec::new(),
                loading,
            },
        );

        (Vec::new(), loading)
    }

    pub(crate) fn paint_folder_entry_icon(
        painter: &egui::Painter,
        body: egui::Rect,
        is_up_entry: bool,
    ) {
        let stroke = egui::Stroke::new(2.0_f32, egui::Color32::WHITE);
        let center = body.center();
        if is_up_entry {
            let shaft_top = egui::pos2(center.x, center.y - body.height() * 0.18);
            let shaft_bottom = egui::pos2(center.x, center.y + body.height() * 0.2);
            painter.line_segment([shaft_bottom, shaft_top], stroke);
            painter.line_segment(
                [
                    shaft_top,
                    egui::pos2(
                        center.x - body.width() * 0.12,
                        center.y - body.height() * 0.02,
                    ),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    shaft_top,
                    egui::pos2(
                        center.x + body.width() * 0.12,
                        center.y - body.height() * 0.02,
                    ),
                ],
                stroke,
            );
        } else {
            let icon = egui::Rect::from_center_size(
                center,
                egui::vec2(body.width() * 0.26, body.height() * 0.28),
            );
            painter.rect_stroke(icon, 3.0, stroke);
        }
    }

    pub(crate) fn paint_folder_entry_card(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        entry_path: &Path,
    ) {
        let painter = ui.painter();
        let is_up_entry = Self::is_up_navigation_entry_path(entry_path);
        let label = Self::folder_entry_display_name(entry_path);

        painter.rect_filled(rect, 6.0, egui::Color32::from_rgb(30, 34, 40));

        let body = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + rect.width() * 0.12,
                rect.top() + rect.height() * 0.36,
            ),
            egui::pos2(
                rect.right() - rect.width() * 0.12,
                rect.bottom() - rect.height() * 0.14,
            ),
        );
        let tab = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + rect.width() * 0.2,
                rect.top() + rect.height() * 0.2,
            ),
            egui::pos2(
                rect.left() + rect.width() * 0.5,
                rect.top() + rect.height() * 0.36,
            ),
        );

        painter.rect_filled(body, 5.0, egui::Color32::from_rgb(221, 178, 73));
        painter.rect_filled(tab, 4.0, egui::Color32::from_rgb(234, 196, 108));

        let (preview_paths, preview_list_loading) =
            self.folder_entry_preview_media_paths(entry_path, 4);
        if preview_paths.is_empty() {
            if preview_list_loading {
                paint_static_hourglass_placeholder(painter, body, 5.0);
            } else {
                // Preserve previous behavior when no media thumbnail candidates exist.
                Self::paint_folder_entry_icon(painter, body, is_up_entry);
            }
        } else {
            let preview_margin = body.width().min(body.height()) * 0.06;
            let preview_rect = body.shrink(preview_margin);
            let grid_gap = (preview_rect.width().min(preview_rect.height()) * 0.04).clamp(2.0, 8.0);
            let tile_width = ((preview_rect.width() - grid_gap) * 0.5).max(1.0);
            let tile_height = ((preview_rect.height() - grid_gap) * 0.5).max(1.0);
            let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));

            for (slot, media_path) in preview_paths.iter().take(4).enumerate() {
                let row = (slot / 2) as f32;
                let col = (slot % 2) as f32;
                let tile_min = egui::pos2(
                    preview_rect.left() + col * (tile_width + grid_gap),
                    preview_rect.top() + row * (tile_height + grid_gap),
                );
                let tile_rect =
                    egui::Rect::from_min_size(tile_min, egui::vec2(tile_width, tile_height));

                painter.rect_filled(
                    tile_rect,
                    3.0,
                    egui::Color32::from_rgba_unmultiplied(22, 26, 31, 235),
                );

                if let Some((texture_id, image_size)) =
                    self.try_get_cached_modal_thumbnail_texture(media_path)
                {
                    let fitted = tile_rect.shrink(2.0);
                    let scale = if image_size.x <= 0.0 || image_size.y <= 0.0 {
                        1.0
                    } else {
                        (fitted.width() / image_size.x)
                            .min(fitted.height() / image_size.y)
                            .max(0.01)
                    };
                    let fitted_size = egui::vec2(image_size.x * scale, image_size.y * scale);
                    let fitted_rect = egui::Rect::from_center_size(tile_rect.center(), fitted_size);
                    painter.image(texture_id, fitted_rect, uv, egui::Color32::WHITE);
                } else if self.request_folder_placeholder_thumbnail_load(media_path) {
                    paint_static_hourglass_placeholder(painter, tile_rect, 3.0);
                } else if self.unloadable_files.contains_key(media_path) {
                    // Undrawable thumbnail: the X replaces the hourglass and no decode is queued.
                    paint_unloadable_cross(
                        painter,
                        tile_rect,
                        egui::Color32::from_rgb(214, 90, 82),
                    );
                } else {
                    paint_static_hourglass_placeholder(painter, tile_rect, 3.0);
                }
            }

            if is_up_entry {
                let badge_size = egui::vec2(body.width() * 0.22, body.height() * 0.2);
                let badge = egui::Rect::from_center_size(body.center(), badge_size);
                painter.rect_filled(
                    badge,
                    6.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160),
                );
                let stroke = egui::Stroke::new(1.8_f32, egui::Color32::WHITE);
                let shaft_top =
                    egui::pos2(badge.center().x, badge.center().y - badge.height() * 0.28);
                let shaft_bottom =
                    egui::pos2(badge.center().x, badge.center().y + badge.height() * 0.22);
                painter.line_segment([shaft_bottom, shaft_top], stroke);
                painter.line_segment(
                    [
                        shaft_top,
                        egui::pos2(
                            badge.center().x - badge.width() * 0.16,
                            badge.center().y - badge.height() * 0.02,
                        ),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        shaft_top,
                        egui::pos2(
                            badge.center().x + badge.width() * 0.16,
                            badge.center().y - badge.height() * 0.02,
                        ),
                    ],
                    stroke,
                );
            }

            painter.rect_stroke(
                preview_rect,
                4.0,
                egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26),
                ),
            );
        }

        let label_size = (rect.width() * 0.045).clamp(13.0, 22.0);
        let label_font = egui::FontId::proportional(label_size);
        let label_color = egui::Color32::from_rgb(245, 245, 245);
        let max_label_width = (rect.width() * 0.82).max(90.0);
        let galley = painter.layout(label, label_font, label_color, max_label_width);
        let label_bottom_padding = (rect.height() * 0.06).clamp(10.0, 28.0);
        let label_top = rect.bottom() - label_bottom_padding - galley.rect.height();
        let label_pos = egui::pos2(rect.center().x - galley.rect.width() * 0.5, label_top);
        painter.galley(label_pos, galley, label_color);
    }

    pub(crate) fn breadcrumb_segments_for_path(path: &Path) -> Vec<(String, PathBuf)> {
        let directory = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };

        let mut chain: Vec<PathBuf> = directory.ancestors().map(Path::to_path_buf).collect();
        chain.reverse();

        let mut segments: Vec<(String, PathBuf)> = Vec::with_capacity(chain.len());
        for segment_path in chain {
            let label = segment_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| segment_path.display().to_string());

            if label.is_empty() {
                continue;
            }

            if segments
                .last()
                .is_some_and(|(_, existing_path)| *existing_path == segment_path)
            {
                continue;
            }

            segments.push((label, segment_path));
        }

        segments
    }
}

//! Rubber-band mark selection overlay.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn collect_mark_selection_preview_indices(
        &mut self,
        selection_rect: egui::Rect,
    ) -> Vec<usize> {
        if self.mode().is_solo() || !self.mode().is_fullscreen() || self.image_list.is_empty() {
            return Vec::new();
        }

        if self.mode().is_masonry() {
            self.masonry_ensure_layout_cache();
            return self
                .masonry_layout_items
                .iter()
                .enumerate()
                .filter_map(|(index, _)| {
                    self.masonry_item_screen_rect(index)
                        .filter(|rect| rect.intersects(selection_rect))
                        .map(|_| index)
                })
                .collect();
        }

        let screen_width = self.screen_size.x.max(1.0);
        let image_count = self.image_list.len();
        let mut preview_indices = Vec::new();

        for index in 0..image_count {
            let display_height = self.manga_page_height_cached(index).max(1.0);
            let display_width = self.manga_get_image_display_width(index);
            let x = (screen_width - display_width) * 0.5 + self.offset.x;
            let y = self.manga_page_start_y(index) - self.manga_scroll_offset;
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(display_width, display_height),
            );

            if rect.intersects(selection_rect) {
                preview_indices.push(index);
            }
        }

        preview_indices
    }

    pub(crate) fn paint_marked_item_overlay(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        visual: FileMarkVisual,
    ) {
        let (label, border_color, chip_fill) = match visual {
            FileMarkVisual::Preview => (
                "READY",
                egui::Color32::from_rgba_unmultiplied(255, 199, 92, 255),
                egui::Color32::from_rgba_unmultiplied(72, 44, 0, 220),
            ),
            FileMarkVisual::Marked => (
                "MARKED",
                egui::Color32::from_rgb(
                    self.config.marked_file_border_rgb[0],
                    self.config.marked_file_border_rgb[1],
                    self.config.marked_file_border_rgb[2],
                ),
                egui::Color32::from_rgba_unmultiplied(16, 56, 74, 220),
            ),
            FileMarkVisual::Copied => (
                "COPIED",
                egui::Color32::from_rgb(164, 231, 170),
                egui::Color32::from_rgba_unmultiplied(28, 86, 38, 220),
            ),
            FileMarkVisual::Cut => (
                "CUT",
                egui::Color32::from_rgb(255, 170, 170),
                egui::Color32::from_rgba_unmultiplied(96, 36, 36, 220),
            ),
        };
        let overlay_rect = rect.shrink(1.5);

        painter.rect_stroke(overlay_rect, 0.0, egui::Stroke::new(2.0_f32, border_color));
        paint_mark_chip(
            painter,
            overlay_rect,
            label,
            chip_fill,
            egui::Stroke::new(1.0_f32, border_color),
            egui::Color32::WHITE,
        );
    }

    pub(crate) fn mark_masonry_runtime_cache_resident(&mut self) {
        if !self.image_list.is_empty() {
            self.masonry_runtime_cache_signature = self.image_list_signature;
        }
    }
}

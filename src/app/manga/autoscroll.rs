//! Manga freehand autoscroll motion and indicator.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn paint_manga_autoscroll_indicator(
        &self,
        painter: &egui::Painter,
        anchor: egui::Pos2,
        pointer_pos: Option<egui::Pos2>,
    ) {
        let fill_alpha = self.config.manga_autoscroll_circle_fill_alpha;
        let [arrow_r, arrow_g, arrow_b] = self.config.manga_autoscroll_arrow_rgb;
        let arrow_alpha = self.config.manga_autoscroll_arrow_alpha;

        painter.circle_filled(
            anchor,
            18.0,
            egui::Color32::from_rgba_unmultiplied(35, 35, 35, fill_alpha),
        );
        painter.circle_stroke(
            anchor,
            18.0,
            egui::Stroke::new(
                1.6_f32,
                egui::Color32::from_rgba_unmultiplied(210, 210, 210, 190),
            ),
        );
        painter.circle_filled(
            anchor,
            4.5,
            egui::Color32::from_rgba_unmultiplied(245, 245, 245, 205),
        );
        painter.line_segment(
            [
                egui::pos2(anchor.x - 7.0, anchor.y),
                egui::pos2(anchor.x + 7.0, anchor.y),
            ],
            egui::Stroke::new(
                1.2_f32,
                egui::Color32::from_rgba_unmultiplied(210, 210, 210, 180),
            ),
        );
        painter.line_segment(
            [
                egui::pos2(anchor.x, anchor.y - 7.0),
                egui::pos2(anchor.x, anchor.y + 7.0),
            ],
            egui::Stroke::new(
                1.2_f32,
                egui::Color32::from_rgba_unmultiplied(210, 210, 210, 180),
            ),
        );

        if let Some(cursor) = pointer_pos {
            let delta = cursor - anchor;
            let len = delta.length();
            if len > 2.0 {
                let direction = delta / len;
                let tip = anchor + direction * len.min(44.0);
                let perp = egui::vec2(-direction.y, direction.x);
                let stroke = egui::Stroke::new(
                    2.0_f32,
                    egui::Color32::from_rgba_unmultiplied(arrow_r, arrow_g, arrow_b, arrow_alpha),
                );

                painter.line_segment([anchor, tip], stroke);

                let head_a = tip - direction * 8.0 + perp * 5.0;
                let head_b = tip - direction * 8.0 - perp * 5.0;
                painter.line_segment([tip, head_a], stroke);
                painter.line_segment([tip, head_b], stroke);
            }
        }
    }

    pub(crate) fn strip_item_open_uses_right_click(&self) -> bool {
        self.config.action_uses_binding(
            self.manga_layout_goto_file_action(),
            &InputBinding::MouseRight,
        )
    }
}

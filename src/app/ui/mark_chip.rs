//! The mark-count chip painted over marked tiles.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) fn paint_mark_chip(
    painter: &egui::Painter,
    rect: egui::Rect,
    text: &str,
    fill: egui::Color32,
    stroke: egui::Stroke,
    text_color: egui::Color32,
) {
    let chip_pos = egui::pos2(rect.min.x + 10.0, rect.min.y + 10.0);
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(11.5),
        text_color,
    );
    let chip_rect = egui::Rect::from_min_size(
        chip_pos,
        egui::vec2(galley.rect.width() + 18.0, galley.rect.height() + 10.0),
    );

    painter.rect_filled(chip_rect, 10.0, fill);
    painter.rect_stroke(chip_rect, 10.0, stroke);
    painter.galley(
        chip_rect.center() - galley.rect.size() * 0.5,
        galley,
        text_color,
    );
}

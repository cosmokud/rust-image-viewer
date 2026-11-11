//! Floating action button target selection.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    /// Filled convex polygon with softened corners. The interior is inset along
    /// each edge and no corner discs are added, so a transport triangle keeps its
    /// points instead of bulging into a rounded blob.
    pub(crate) fn fill_round_polygon(
        painter: &egui::Painter,
        points: &[egui::Pos2],
        radius: f32,
        color: egui::Color32,
    ) {
        if points.len() < 3 {
            return;
        }
        let mut inner = Vec::with_capacity(points.len() * 2);
        for (index, point) in points.iter().enumerate() {
            let previous = points[(index + points.len() - 1) % points.len()];
            let next = points[(index + 1) % points.len()];
            for neighbour in [previous, next] {
                let offset = neighbour - *point;
                let distance = offset.length();
                if distance > f32::EPSILON {
                    inner.push(*point + offset / distance * radius.min(distance * 0.5));
                }
            }
        }
        painter.add(egui::Shape::convex_polygon(
            inner,
            color,
            egui::Stroke::NONE,
        ));
    }
}

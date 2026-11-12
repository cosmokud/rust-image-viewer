//! Vector icon painters (menu, volume, breadcrumb).

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Paint a Lucide SVG at `rect`, snapped to whole device pixels, so the
    /// rasterised texture is sampled 1:1 instead of being resampled (which blurs).
    pub(crate) fn paint_lucide_icon(
        ui: &egui::Ui,
        rect: egui::Rect,
        source: egui::ImageSource<'static>,
        color: egui::Color32,
    ) {
        let pixels_per_point = ui.ctx().pixels_per_point().max(0.5);
        let snap = |value: f32| (value * pixels_per_point).round();
        let snapped = egui::Rect::from_min_size(
            egui::pos2(snap(rect.left()), snap(rect.top())) / pixels_per_point,
            egui::vec2(
                snap(rect.width()).max(1.0) / pixels_per_point,
                snap(rect.height()).max(1.0) / pixels_per_point,
            ),
        );

        egui::Image::new(source)
            .texture_options(egui::TextureOptions::NEAREST)
            .tint(color)
            .paint_at(ui, snapped);
    }

    /// Lucide artwork for a transport-volume state, carrying the same tint and
    /// stroked outline as the drawn transport glyphs next to it.
    pub(crate) fn paint_volume_lucide_icon(
        ui: &egui::Ui,
        rect: egui::Rect,
        muted: bool,
        color: egui::Color32,
    ) {
        let source = if muted {
            egui::include_image!("../../../assets/lucide/volume-x.svg")
        } else {
            egui::include_image!("../../../assets/lucide/volume-2.svg")
        };
        Self::paint_lucide_icon(ui, rect, source, color);
    }

    pub(crate) fn paint_menu_action_icon(
        ui: &egui::Ui,
        rect: egui::Rect,
        icon: MenuActionIcon,
        color: egui::Color32,
    ) {
        // Lucide artwork, tinted to the row colour.
        let source = match icon {
            MenuActionIcon::Mark => egui::include_image!("../../../assets/lucide/square-check.svg"),
            MenuActionIcon::MarkAll => {
                egui::include_image!("../../../assets/lucide/list-checks.svg")
            }
            MenuActionIcon::Unmark => {
                egui::include_image!("../../../assets/lucide/square-minus.svg")
            }
            MenuActionIcon::Cut => egui::include_image!("../../../assets/lucide/scissors.svg"),
            MenuActionIcon::Copy => egui::include_image!("../../../assets/lucide/copy.svg"),
            MenuActionIcon::Paste => {
                egui::include_image!("../../../assets/lucide/clipboard-paste.svg")
            }
            MenuActionIcon::Delete => egui::include_image!("../../../assets/lucide/trash-2.svg"),
            MenuActionIcon::Rename => {
                egui::include_image!("../../../assets/lucide/pencil-line.svg")
            }
            MenuActionIcon::OpenLocation => {
                egui::include_image!("../../../assets/lucide/folder-open.svg")
            }
            MenuActionIcon::Stash => {
                egui::include_image!("../../../assets/lucide/briefcase-plus.svg")
            }
            MenuActionIcon::StashRemove => {
                egui::include_image!("../../../assets/lucide/archive-x.svg")
            }
            MenuActionIcon::Deselect => {
                egui::include_image!("../../../assets/lucide/square-x.svg")
            }
            MenuActionIcon::Settings => {
                egui::include_image!("../../../assets/lucide/sliders-horizontal.svg")
            }
        };

        Self::paint_lucide_icon(ui, rect, source, color);
    }

    pub(crate) fn paint_breadcrumb_toggle_folder_icon(
        ui: &egui::Ui,
        rect: egui::Rect,
        tint: egui::Color32,
    ) {
        egui::Image::new(egui::include_image!(
            "../../../assets/breadcrumb_toggle_folder.svg"
        ))
        .fit_to_exact_size(rect.size())
        .tint(tint)
        .paint_at(ui, rect);
    }

    pub(crate) fn menu_action_row(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        icon: MenuActionIcon,
    ) -> egui::Response {
        let desired_size = egui::vec2(ui.available_width(), 32.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

        if ui.is_rect_visible(rect) {
            let destructive = icon == MenuActionIcon::Delete;
            let fill = if response.is_pointer_button_down_on() {
                if destructive {
                    egui::Color32::from_rgba_unmultiplied(172, 44, 44, 210)
                } else {
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 28)
                }
            } else if response.hovered() {
                if destructive {
                    egui::Color32::from_rgba_unmultiplied(160, 42, 42, 170)
                } else {
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16)
                }
            } else {
                egui::Color32::TRANSPARENT
            };
            let stroke_color = if destructive {
                egui::Color32::from_rgba_unmultiplied(255, 132, 132, 110)
            } else {
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36)
            };
            let text_color = if destructive {
                egui::Color32::from_rgb(255, 225, 225)
            } else {
                egui::Color32::WHITE
            };

            ui.painter().rect_filled(rect, 8.0, fill);
            ui.painter()
                .rect_stroke(rect, 8.0, egui::Stroke::new(1.0_f32, stroke_color));

            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 17.0, rect.center().y),
                egui::vec2(15.0, 15.0),
            );
            Self::paint_menu_action_icon(ui, icon_rect, icon, text_color);

            ui.painter().text(
                egui::pos2(rect.left() + 34.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::TextStyle::Body.resolve(ui.style()),
                text_color,
            );
        }

        response
    }
}

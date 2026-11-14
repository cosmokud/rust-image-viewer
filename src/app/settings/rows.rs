//! A settings row and its groups.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_settings_group_header(ui: &mut egui::Ui, group: &str) {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(group.to_uppercase())
                .size(11.0)
                .color(UI_TEXT_FAINT),
        );
        ui.add_space(6.0);
    }

    /// Draws `ids` (registry order) as uppercase group headers with one card per group.
    pub(crate) fn draw_settings_groups(
        &mut self,
        ui: &mut egui::Ui,
        ids: &[usize],
        read_only: bool,
    ) {
        let label_column = Self::settings_label_column(ui, ids);
        let mut index = 0;
        while index < ids.len() {
            let group = crate::settings_registry::SETTINGS[ids[index]].group;
            let start = index;
            while index < ids.len() && crate::settings_registry::SETTINGS[ids[index]].group == group
            {
                index += 1;
            }
            let card_ids = &ids[start..index];
            Self::draw_settings_group_header(ui, group);
            let mut changed = false;
            let mut write_now = false;
            // Two stacked fills give the card a 1px border without a stroke (see
            // `ui_paint_surface`): strokes are what made the old card edges look aliased.
            egui::Frame::none()
                .fill(UI_BORDER)
                .rounding(12.0)
                .inner_margin(egui::Margin::same(1.0))
                .show(ui, |ui| {
                    egui::Frame::none()
                        .fill(UI_SURFACE)
                        .rounding(11.0)
                        .inner_margin(egui::Margin::symmetric(16.0, 6.0))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (offset, id) in card_ids.iter().enumerate() {
                                if offset > 0 {
                                    ui.add_space(2.0);
                                }
                                let setting = &crate::settings_registry::SETTINGS[*id];
                                if let Some(row_write_now) =
                                    self.draw_setting_row(ui, setting, label_column, read_only)
                                {
                                    changed = true;
                                    write_now |= row_write_now;
                                    // The fullscreen rect is written when fullscreen is entered,
                                    // so a mode change while already fullscreen re-applies it.
                                    if setting.key == "fullscreen_mode"
                                        && self.mode().is_fullscreen()
                                    {
                                        #[cfg(target_os = "windows")]
                                        self.request_borderless_fullscreen_rect(ui.ctx());
                                    }
                                }
                            }
                        });
                });
            if changed {
                self.finish_settings_edit(write_now);
            }
            ui.add_space(6.0);
        }
    }

    /// Width of the label column that fits the longest label in `ids`, so no row in the tab
    /// truncates its label while the dialog is wide enough to hold it.
    pub(crate) fn settings_label_column(ui: &egui::Ui, ids: &[usize]) -> f32 {
        let font = egui::FontId::proportional(13.0);
        let mut widest = 0.0_f32;
        for id in ids {
            let setting = &crate::settings_registry::SETTINGS[*id];
            let needed = ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(setting.label.to_owned(), font.clone(), UI_TEXT)
                    .size()
                    .x
            });
            widest = widest.max(needed);
        }
        widest + 16.0
    }

    /// Search results: the same cards, grouped by tab then group.
    pub(crate) fn draw_settings_search_results(
        &mut self,
        ui: &mut egui::Ui,
        ids: &[usize],
        read_only: bool,
    ) {
        let mut index = 0;
        while index < ids.len() {
            let tab = crate::settings_registry::SETTINGS[ids[index]].tab;
            let start = index;
            while index < ids.len() && crate::settings_registry::SETTINGS[ids[index]].tab == tab {
                index += 1;
            }
            if start > 0 {
                ui.add_space(10.0);
            }
            ui.label(
                egui::RichText::new(SettingsTabSlot::Tab(tab).label())
                    .size(10.5)
                    .color(UI_TEXT_DIM),
            );
            ui.add_space(4.0);
            self.draw_settings_groups(ui, &ids[start..index], read_only);
        }
    }

    /// Right-aligned mini pill for preset rows; returns the width it used.
    pub(crate) fn paint_settings_pill(
        ui: &egui::Ui,
        right_center: egui::Pos2,
        text: &str,
        fill: egui::Color32,
        color: egui::Color32,
        size: f32,
    ) -> f32 {
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_string(), egui::FontId::proportional(size), color);
        // 8x4 of padding: the pill text used to sit right against the rounded edge.
        let pad = egui::vec2(8.0, 4.0);
        let width = galley.size().x + pad.x * 2.0;
        let height = galley.size().y + pad.y * 2.0;
        let rect = egui::Rect::from_min_max(
            egui::pos2(right_center.x - width, right_center.y - height * 0.5),
            egui::pos2(right_center.x, right_center.y + height * 0.5),
        );
        let painter = ui.painter();
        painter.rect_filled(rect, height * 0.5, fill);
        painter.galley(
            egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
        width
    }
}

//! Preset list panel.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn draw_settings_presets_panel(
        &mut self,
        ui: &mut egui::Ui,
        body_height: f32,
        modified: bool,
        action: &mut Option<SettingsPresetAction>,
    ) {
        egui::ScrollArea::vertical()
            .id_salt("settings_presets_panel")
            .max_height(body_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("PRESETS")
                        .size(10.0)
                        .color(UI_TEXT_FAINT),
                );
                ui.add_space(8.0);

                let presets = self.settings.presets.clone();
                let selected = self.settings.selected.clone();
                egui::ScrollArea::vertical()
                    .id_salt("settings_preset_list")
                    .max_height(8.0 * 36.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if presets.is_empty() {
                            ui.label(
                                egui::RichText::new("No presets found.")
                                    .size(12.0)
                                    .color(UI_TEXT_DIM),
                            );
                        }
                        for entry in presets.iter() {
                            let is_selected = selected.as_deref() == Some(entry.name.as_str());
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 32.0),
                                egui::Sense::click(),
                            );
                            let painter = ui.painter();
                            if is_selected {
                                painter.rect_filled(rect, 8.0, UI_SURFACE);
                            } else if response.hovered() {
                                painter.rect_filled(rect, 8.0, ui_alpha(UI_TEXT_STRONG, 10));
                            }
                            let mut right = rect.right() - 12.0;
                            if entry.builtin {
                                right -= Self::paint_settings_pill(
                                    ui,
                                    egui::pos2(right, rect.center().y),
                                    "BUILT-IN",
                                    ui_alpha(UI_TEXT_STRONG, 16),
                                    UI_TEXT_DIM,
                                    9.0,
                                ) + 6.0;
                            }
                            if modified && is_selected {
                                right -= Self::paint_settings_pill(
                                    ui,
                                    egui::pos2(right, rect.center().y),
                                    "modified",
                                    UI_ACCENT,
                                    UI_ACCENT_TEXT,
                                    9.0,
                                ) + 6.0;
                            }
                            ui.painter()
                                .with_clip_rect(egui::Rect::from_min_max(
                                    egui::pos2(rect.left() + 12.0, rect.top()),
                                    egui::pos2(right - 2.0, rect.bottom()),
                                ))
                                .text(
                                    egui::pos2(rect.left() + 12.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &entry.name,
                                    egui::FontId::proportional(12.5),
                                    if is_selected {
                                        UI_TEXT_STRONG
                                    } else {
                                        UI_TEXT_DIM
                                    },
                                );
                            if response.clicked() {
                                // Clicking a preset loads it: the selected file is the live
                                // config, so there is no separate Load step.
                                self.settings.rename = None;
                                *action = Some(SettingsPresetAction::Load(entry.name.clone()));
                            }
                            response.on_hover_cursor(egui::CursorIcon::PointingHand);
                            ui.add_space(4.0);
                        }
                    });

                ui.add_space(10.0);
                let full = ui.available_width();
                let selected_is_builtin = selected
                    .as_deref()
                    .is_some_and(crate::settings_registry::is_builtin);
                let can_edit_preset = selected.is_some() && !selected_is_builtin;
                let button_size = egui::vec2(full, 30.0);

                if settings_button(
                    ui,
                    "Rename",
                    button_size,
                    can_edit_preset,
                    SettingsButtonKind::Secondary,
                )
                .clicked()
                {
                    if let Some(name) = selected.clone() {
                        *action = Some(SettingsPresetAction::BeginRename(name));
                    }
                }
                ui.add_space(4.0);
                if settings_button(
                    ui,
                    "Delete",
                    button_size,
                    can_edit_preset,
                    SettingsButtonKind::Secondary,
                )
                .clicked()
                {
                    self.settings.rename = None;
                    self.settings.pending_confirm = selected.clone().map(SettingsConfirm::Delete);
                }

                if let Some((old_name, mut draft)) = self.settings.rename.clone() {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(format!("Rename '{old_name}'"))
                            .size(11.5)
                            .color(UI_TEXT_DIM),
                    );
                    ui.add_space(4.0);
                    let mut layouter = text_field_layouter(
                        egui::TextStyle::Body.resolve(ui.style()),
                        SETTINGS_INPUT_HEIGHT,
                    );
                    let mut output = egui::TextEdit::singleline(&mut draft)
                        .margin(egui::Margin::symmetric(TEXT_FIELD_PADDING_X, 0.0))
                        .min_size(egui::vec2(0.0, SETTINGS_INPUT_HEIGHT))
                        .desired_width(full)
                        .hint_text("preset name")
                        .layouter(&mut layouter)
                        .show(ui);
                    tame_triple_click(ui, &mut output);
                    self.settings.rename = Some((old_name.clone(), draft.clone()));
                    ui.add_space(6.0);
                    let gap = ui.spacing().item_spacing.x;
                    let half = egui::vec2((full - gap) * 0.5, 28.0);
                    ui.horizontal(|ui| {
                        if settings_button(ui, "Confirm", half, true, SettingsButtonKind::Primary)
                            .clicked()
                        {
                            *action = Some(SettingsPresetAction::Rename(
                                old_name.clone(),
                                draft.clone(),
                            ));
                        }
                        if settings_button(ui, "Cancel", half, true, SettingsButtonKind::Secondary)
                            .clicked()
                        {
                            *action = Some(SettingsPresetAction::CancelRename);
                        }
                    });
                }

                ui.add_space(12.0);
                ui_divider(ui);
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("Save current as")
                        .size(11.5)
                        .color(UI_TEXT_DIM),
                );
                ui.add_space(4.0);
                let mut layouter = text_field_layouter(
                    egui::TextStyle::Body.resolve(ui.style()),
                    SETTINGS_INPUT_HEIGHT,
                );
                let mut output = egui::TextEdit::singleline(&mut self.settings.new_name)
                    .margin(egui::Margin::symmetric(TEXT_FIELD_PADDING_X, 0.0))
                    .min_size(egui::vec2(0.0, SETTINGS_INPUT_HEIGHT))
                    .hint_text("new preset name")
                    .desired_width(full)
                    .layouter(&mut layouter)
                    .show(ui);
                tame_triple_click(ui, &mut output);
                ui.add_space(6.0);
                if settings_button(
                    ui,
                    "Create",
                    egui::vec2(full, 30.0),
                    !self.settings.new_name.trim().is_empty(),
                    SettingsButtonKind::Primary,
                )
                .clicked()
                {
                    self.settings.rename = None;
                    *action = Some(SettingsPresetAction::Create);
                }
            });
    }

    /// Browser-style tab bar: monochrome pills. The window is sized to fit them on one row, the
    /// wrapping stays as a fallback for a dialog narrowed by the user.
    pub(crate) fn draw_settings_tab_strip(&mut self, ui: &mut egui::Ui) {
        let font = egui::FontId::proportional(SETTINGS_TAB_FONT_SIZE);
        let widths: Vec<f32> = SettingsTabSlot::ALL
            .iter()
            .map(|slot| {
                ui.fonts(|fonts| {
                    fonts
                        .layout_no_wrap(slot.label().to_owned(), font.clone(), UI_TEXT)
                        .size()
                        .x
                }) + SETTINGS_TAB_PADDING * 2.0
            })
            .collect();

        // Row breaks first, so the strip height is known before anything is allocated.
        let available = ui.available_width();
        let mut row_of: Vec<usize> = Vec::with_capacity(widths.len());
        let mut row = 0usize;
        let mut cursor = 0.0_f32;
        for width in widths.iter() {
            let advance = if cursor == 0.0 {
                *width
            } else {
                SETTINGS_TAB_GAP + *width
            };
            if 0.0 < cursor && available < cursor + advance {
                row += 1;
                cursor = *width;
            } else {
                cursor += advance;
            }
            row_of.push(row);
        }
        let row_count = row_of.last().map_or(1, |row| row + 1);
        let strip_height = SETTINGS_TAB_HEIGHT * row_count as f32
            + SETTINGS_TAB_GAP * (row_count.saturating_sub(1)) as f32;
        let (strip_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), strip_height),
            egui::Sense::hover(),
        );

        let mut x = strip_rect.left();
        let mut y = strip_rect.top();
        let mut current_row = 0usize;
        for (index, slot) in SettingsTabSlot::ALL.iter().enumerate() {
            if row_of[index] != current_row {
                current_row = row_of[index];
                x = strip_rect.left();
                y += SETTINGS_TAB_HEIGHT + SETTINGS_TAB_GAP;
            }
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(widths[index], SETTINGS_TAB_HEIGHT),
            );
            x += widths[index] + SETTINGS_TAB_GAP;

            let response = ui.interact(
                rect,
                egui::Id::new(("settings_tab", index)),
                egui::Sense::click(),
            );
            let is_active = self.settings.tab == *slot;
            let painter = ui.painter();
            if is_active {
                ui_paint_surface(painter, rect, 8.0, UI_SURFACE, Some(UI_BORDER));
            } else if response.hovered() {
                painter.rect_filled(rect, 8.0, ui_alpha(UI_TEXT_STRONG, 10));
            }
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                slot.label(),
                font.clone(),
                if is_active {
                    UI_TEXT_STRONG
                } else if response.hovered() {
                    UI_TEXT
                } else {
                    UI_TEXT_DIM
                },
            );
            if response.clicked() && !is_active {
                self.settings.tab = *slot;
                self.settings.search.clear();
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }
}

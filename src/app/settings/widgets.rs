//! Slider/toggle/number widgets for settings rows.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    /// Custom track + knob slider; drag maps pointer x to the value, Shift makes it 4x finer.
    pub(crate) fn draw_settings_slider(
        ui: &mut egui::Ui,
        value: &mut f64,
        min: f64,
        max: f64,
        width: f32,
    ) -> (egui::Response, bool) {
        let enabled = ui.is_enabled();
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(width, 26.0),
            if enabled {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            },
        );
        let track_left = rect.left() + 9.0;
        let track_right = (rect.right() - 9.0).max(track_left + 1.0);
        let span = max - min;
        let normalized = |value: f64| {
            if span > 0.0 {
                ((value - min) / span).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let mut changed = false;
        if let Some(pointer) = response.interact_pointer_pos() {
            if enabled && (response.dragged() || response.clicked()) {
                let target = (((pointer.x - track_left) / (track_right - track_left)) as f64)
                    .clamp(0.0, 1.0);
                let current = normalized(*value);
                // Shift: the value trails the pointer instead of snapping to it (4x finer).
                let next = if ui.input(|i| i.modifiers.shift) {
                    current + (target - current) * 0.25
                } else {
                    target
                };
                let next_value = min + next * span;
                if (next_value - *value).abs() > span * 1e-6 {
                    *value = next_value;
                    changed = true;
                }
            }
        }

        let t = normalized(*value) as f32;
        let y = rect.center().y;
        let knob = egui::pos2(track_left + (track_right - track_left) * t, y);
        let painter = ui.painter();
        let track = egui::Rect::from_min_max(
            egui::pos2(track_left, y - 2.0),
            egui::pos2(track_right, y + 2.0),
        );
        painter.rect_filled(track, 2.0, UI_SURFACE_ACTIVE);
        if knob.x > track_left {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(track_left, y - 2.0),
                    egui::pos2(knob.x, y + 2.0),
                ),
                2.0,
                if enabled {
                    UI_ACCENT
                } else {
                    ui_alpha(UI_ACCENT, 70)
                },
            );
        }
        let hovered = enabled && (response.hovered() || response.dragged());
        let knob_fill = if !enabled {
            ui_alpha(UI_TEXT_FAINT, 110)
        } else if hovered {
            UI_TEXT_STRONG
        } else {
            UI_ACCENT
        };
        painter.circle_filled(knob, if hovered { 8.0 } else { 7.0 }, knob_fill);
        painter.circle_filled(knob, 2.5, if hovered { UI_ACCENT } else { UI_WINDOW });
        (response, changed)
    }

    pub(crate) fn draw_settings_toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
        let enabled = ui.is_enabled();
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(40.0, 22.0),
            if enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        if enabled && response.clicked() {
            *on = !*on;
        }
        let t = ui
            .ctx()
            .animate_value_with_time(response.id, if *on { 1.0 } else { 0.0 }, 0.12);
        let hovered = enabled && response.hovered();
        let track = if !enabled {
            UI_SURFACE_ACTIVE
        } else if *on {
            if hovered {
                UI_TEXT_STRONG
            } else {
                UI_ACCENT
            }
        } else if hovered {
            UI_SURFACE_ACTIVE
        } else {
            UI_SURFACE
        };
        let painter = ui.painter();
        painter.rect_filled(rect, rect.height() * 0.5, track);
        painter.circle_filled(
            egui::pos2(
                egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t),
                rect.center().y,
            ),
            8.0,
            if *on { UI_ACCENT_TEXT } else { UI_TEXT_DIM },
        );
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// Float/Int row: slider + DragValue, clamped and mirrored through the registry.
    pub(crate) fn draw_settings_number_control(
        &mut self,
        ui: &mut egui::Ui,
        setting: &'static crate::settings_registry::Setting,
        min: f64,
        max: f64,
        decimals: usize,
        controls_width: f32,
    ) -> (bool, bool) {
        let id = setting.id;
        let mut value = self.settings.values[id];
        let step = 10f64.powi(-(decimals as i32)) * 5.0;
        let number_box_width = 84.0_f32.min((controls_width * 0.45).max(56.0));
        let drag_response = ui
            .scope(|ui| {
                ui.style_mut()
                    .text_styles
                    .insert(egui::TextStyle::Button, egui::FontId::monospace(12.0));
                ui.add_sized(
                    [number_box_width, SETTINGS_INPUT_HEIGHT],
                    egui::DragValue::new(&mut value)
                        .fixed_decimals(decimals)
                        .range(min..=max)
                        .speed(step),
                )
            })
            .inner;
        ui.add_space(10.0);
        let slider_width = (controls_width - number_box_width - 10.0).max(56.0);
        let (slider_response, slider_changed) =
            Self::draw_settings_slider(ui, &mut value, min, max, slider_width);
        if !slider_changed && !drag_response.changed() {
            return (false, false);
        }
        // Release and keyboard edits write at once: the idle viewer runs no frames, so a
        // timer-only debounce could leave the last drag unsaved.
        let write_now = slider_response.drag_stopped()
            || drag_response.drag_stopped()
            || !ui.input(|i| i.pointer.any_down());
        match crate::settings_registry::set_num(&mut self.config, id, value) {
            Ok(()) => {
                // Registry clamps and keeps paired keys ordered, so remirror every number.
                self.settings.values = Self::settings_values_from_config(&self.config);
                self.apply_app_consequences(std::iter::once(id));
                (true, write_now)
            }
            Err(err) => {
                self.set_settings_status(err, true);
                (false, false)
            }
        }
    }

    /// One settings row; `Some(write_now)` when `config` changed.
    pub(crate) fn draw_setting_row(
        &mut self,
        ui: &mut egui::Ui,
        setting: &'static crate::settings_registry::Setting,
        label_column: f32,
        read_only: bool,
    ) -> Option<bool> {
        let id = setting.id;
        let tooltip = Self::settings_row_tooltip(setting, read_only);
        let tooltip_width = ui.spacing().tooltip_width.min(420.0);
        let row_width = ui.available_width();
        // Fits the widest label of this tab, so labels are not cut while there is room. On a
        // narrow dialog the label elides first: the control keeps `SETTINGS_CONTROLS_MIN_WIDTH`.
        let label_slot = label_column.min((row_width - SETTINGS_CONTROLS_MIN_WIDTH).max(90.0));
        let mut changed = false;
        let mut write_now = false;

        let row = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            // The label takes the row height so the row itself is tall enough to right-click.
            let label_response = ui.add_sized(
                [(label_slot - 6.0).max(40.0), SETTINGS_ROW_HEIGHT],
                egui::Label::new(
                    egui::RichText::new(setting.label)
                        .size(13.0)
                        .color(if read_only { UI_TEXT_FAINT } else { UI_TEXT }),
                )
                .truncate()
                // Selectable so the label can be highlighted and copied; the response rect is the
                // galley, so it doubles as the tooltip hit region (text only, not the row).
                .selectable(true),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!read_only, |ui| {
                    let controls = ui.available_width().max(80.0);
                    match setting.control() {
                        crate::settings_registry::RowControl::Slider { min, max, decimals } => {
                            let (row_changed, row_write_now) = self.draw_settings_number_control(
                                ui, setting, min, max, decimals, controls,
                            );
                            changed |= row_changed;
                            write_now |= row_write_now;
                        }
                        crate::settings_registry::RowControl::Toggle => {
                            let before = self.settings.values[id] != 0.0;
                            let mut on = before;
                            if Self::draw_settings_toggle(ui, &mut on).clicked() && on != before {
                                match crate::settings_registry::set(
                                    &mut self.config,
                                    id,
                                    &crate::settings_registry::SettingValue::Bool(on),
                                ) {
                                    Ok(()) => {
                                        self.settings.values =
                                            Self::settings_values_from_config(&self.config);
                                        changed = true;
                                        write_now = true;
                                    }
                                    Err(err) => self.set_settings_status(err, true),
                                }
                            }
                        }
                        crate::settings_registry::RowControl::Choice(values) => {
                            let current = Self::settings_text_value(&self.config, id);
                            let mut draft = current.clone();
                            egui::ComboBox::from_id_salt(("settings_enum", id))
                                .width(180.0_f32.min(controls.max(90.0)))
                                .selected_text(
                                    egui::RichText::new(&current).size(12.0).color(UI_TEXT),
                                )
                                .show_ui(ui, |ui| {
                                    for entry in values {
                                        ui.selectable_value(
                                            &mut draft,
                                            (*entry).to_string(),
                                            *entry,
                                        );
                                    }
                                });
                            if draft != current {
                                match crate::settings_registry::set(
                                    &mut self.config,
                                    id,
                                    &crate::settings_registry::SettingValue::Text(draft),
                                ) {
                                    Ok(()) => {
                                        self.settings.texts[id] =
                                            Self::settings_text_value(&self.config, id);
                                        changed = true;
                                        write_now = true;
                                    }
                                    Err(err) => self.set_settings_status(err, true),
                                }
                            }
                        }
                        crate::settings_registry::RowControl::RgbText => {
                            let mut rgb = [0u8; 3];
                            for (slot, part) in
                                rgb.iter_mut().zip(self.settings.texts[id].split(','))
                            {
                                *slot = part.trim().parse::<u8>().unwrap_or(0);
                            }
                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                let text = format!("{}, {}, {}", rgb[0], rgb[1], rgb[2]);
                                match crate::settings_registry::set(
                                    &mut self.config,
                                    id,
                                    &crate::settings_registry::SettingValue::Text(text),
                                ) {
                                    Ok(()) => {
                                        self.settings.texts[id] =
                                            Self::settings_text_value(&self.config, id);
                                        changed = true;
                                        write_now = true;
                                    }
                                    Err(err) => self.set_settings_status(err, true),
                                }
                            }
                            ui.label(
                                egui::RichText::new(format!(
                                    "#{:02X}{:02X}{:02X}",
                                    rgb[0], rgb[1], rgb[2]
                                ))
                                .monospace()
                                .size(11.5)
                                .color(UI_TEXT_DIM),
                            );
                        }
                        crate::settings_registry::RowControl::KeyText
                        | crate::settings_registry::RowControl::ModifierText
                        | crate::settings_registry::RowControl::BindingText
                        | crate::settings_registry::RowControl::BindingListText => {
                            let hint = if matches!(
                                setting.control(),
                                crate::settings_registry::RowControl::BindingListText
                            ) {
                                "empty = unbound"
                            } else {
                                "none"
                            };
                            // Full control width: in the shortcuts tab the binding is the payload,
                            // so the field reaches the card edge instead of leaving a gap.
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.settings.texts[id])
                                    .hint_text(hint)
                                    .font(egui::FontId::monospace(11.5))
                                    .margin(egui::Margin::symmetric(8.0, 6.0))
                                    .min_size(egui::vec2(0.0, SETTINGS_INPUT_HEIGHT))
                                    .desired_width(controls)
                                    .horizontal_align(egui::Align::LEFT),
                            );
                            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            // Commit only on Enter/focus loss: a half-typed binding must never be
                            // written, so rejected text stays in the box for the user to fix.
                            if response.lost_focus() || (response.has_focus() && enter) {
                                let text = self.settings.texts[id].clone();
                                match crate::settings_registry::set(
                                    &mut self.config,
                                    id,
                                    &crate::settings_registry::SettingValue::Text(text),
                                ) {
                                    Ok(()) => {
                                        self.settings.texts[id] =
                                            Self::settings_text_value(&self.config, id);
                                        changed = true;
                                        write_now = true;
                                    }
                                    Err(err) => self.set_settings_status(err, true),
                                }
                            }
                        }
                    }
                });
            });
            label_response
        });
        // The row is registered by rect for the right-click menu (`open_settings_row_menu`):
        // its controls allocate their own hit areas and sink the pointer, so widget hover only
        // ever covers the label and the empty body, never the slider, box or toggle.
        self.settings.row_rects.push((id, row.response.rect));
        // Paint the tooltip on the hover frame instead of `on_hover_text`: that delayed path needs a
        // frame after `interaction.tooltip_delay`, and an idle viewer only wakes the dialog's own
        // viewport on input, so the deadline frame usually never arrives and nothing is shown.
        // Only the label galley hovers: an empty part of the row must not spawn a tooltip.
        if row.inner.hovered() && !ui.ctx().input(|i| i.pointer.any_down()) {
            row.inner.show_tooltip_ui(|ui| {
                ui.set_max_width(tooltip_width);
                ui.label(tooltip.as_str());
            });
        }
        if changed {
            Some(write_now)
        } else {
            None
        }
    }
}

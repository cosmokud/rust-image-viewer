//! Right-click menu for a settings row: one "Reset to Default" entry.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const ROW_MENU_WIDTH: f32 = 216.0;
pub(crate) const ROW_MENU_ITEM_HEIGHT: f32 = 30.0;
pub(crate) const ROW_MENU_MARGIN: f32 = 6.0;

impl ImageViewer {
    /// Opens the row menu when this frame's right-click landed on a row. Runs after the body, so
    /// `row_hover` already holds the row under the pointer.
    pub(crate) fn open_settings_row_menu(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.pointer.secondary_clicked()) {
            return;
        }
        let Some(pos) = ctx.input(|input| input.pointer.interact_pos()) else {
            return;
        };
        // The built-in preset is a read-only reference, and so is a pending confirmation.
        let editable = self.settings.pending_confirm.is_none()
            && self
                .settings
                .selected
                .as_deref()
                .is_some_and(|name| !crate::settings_registry::is_builtin(name));
        if !editable {
            return;
        }
        // A right-click inside the open menu keeps it anchored where it is.
        if self
            .settings
            .row_menu_rect
            .is_some_and(|rect| rect.contains(pos))
        {
            return;
        }
        if let Some((id, rect)) = self.settings.row_hover {
            if rect.contains(pos) {
                self.settings.row_menu = Some(SettingsRowMenu { id, pos });
            }
        }
    }

    /// Draws the open row menu and applies its one action. Runs after the body so the menu paints
    /// above every card, in the top-most layer.
    pub(crate) fn draw_settings_row_menu(&mut self, ctx: &egui::Context) {
        let Some(menu) = self.settings.row_menu else {
            return;
        };
        let setting = &crate::settings_registry::SETTINGS[menu.id];
        let default_text = crate::settings_registry::default_value(menu.id)
            .map(|value| crate::settings_registry::value_text(setting, value))
            .unwrap_or_default();
        let screen = ctx.screen_rect();
        let menu_height = ROW_MENU_ITEM_HEIGHT + ROW_MENU_MARGIN * 2.0 + 2.0;
        let pos = egui::pos2(
            menu.pos.x.clamp(
                screen.min.x + 8.0,
                (screen.max.x - ROW_MENU_WIDTH - 8.0).max(screen.min.x + 8.0),
            ),
            menu.pos.y.clamp(
                screen.min.y + 8.0,
                (screen.max.y - menu_height - 8.0).max(screen.min.y + 8.0),
            ),
        );

        let mut reset = false;
        let response = egui::Area::new(egui::Id::new("settings_row_menu"))
            .fixed_pos(pos)
            // Tooltip order: the menu floats over cards and over any popup the dialog opened.
            .order(egui::Order::Tooltip)
            .constrain_to(screen)
            .show(ctx, |ui| {
                ui.set_width(ROW_MENU_WIDTH);
                // Same border trick as the settings cards: two fills, never a stroke.
                egui::Frame::none()
                    .fill(UI_BORDER)
                    .rounding(11.0)
                    .inner_margin(egui::Margin::same(1.0))
                    .show(ui, |ui| {
                        egui::Frame::none()
                            .fill(UI_SURFACE)
                            .rounding(10.0)
                            .inner_margin(egui::Margin::same(ROW_MENU_MARGIN))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                ui.label(
                                    egui::RichText::new(setting.label)
                                        .size(10.5)
                                        .color(UI_TEXT_FAINT),
                                );
                                ui.add_space(4.0);
                                if settings_row_menu_item(
                                    ui,
                                    "Reset to Default",
                                    &default_text,
                                    true,
                                )
                                .clicked()
                                {
                                    reset = true;
                                }
                            });
                    });
            });

        let menu_rect = response.response.rect;
        self.settings.row_menu_rect = Some(menu_rect);
        if reset {
            self.close_settings_row_menu();
            self.reset_setting_to_default(menu.id);
            return;
        }
        // Any click outside the menu closes it. A click inside the menu lands on the entry, and a
        // right-click on another row is handled after this call by `open_settings_row_menu`.
        let clicked = ctx.input(|input| {
            input
                .pointer
                .any_click()
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        let dismissed = clicked.is_some_and(|pos| !menu_rect.contains(pos));
        if dismissed || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.close_settings_row_menu();
        }
    }

    pub(crate) fn close_settings_row_menu(&mut self) {
        self.settings.row_menu = None;
        self.settings.row_menu_rect = None;
    }

    /// Writes one key back to the value `default.ini` ships and persists the active preset.
    pub(crate) fn reset_setting_to_default(&mut self, id: usize) {
        let Some(default) = crate::settings_registry::default_value(id).cloned() else {
            return;
        };
        let setting = &crate::settings_registry::SETTINGS[id];
        match crate::settings_registry::set(&mut self.config, id, &default) {
            Ok(()) => {
                self.refresh_settings_mirrors();
                self.apply_app_consequences(std::iter::once(id));
                // A row reset is one discrete edit: there is no drag to debounce.
                self.finish_settings_edit(true);
                self.load_selected_preset_values();
                let text = crate::settings_registry::value_text(setting, &default);
                self.set_settings_status(format!("Reset '{}' to {text}", setting.label), false);
            }
            Err(err) => self.set_settings_status(err, true),
        }
    }
}

/// One menu entry: a label plus the value it would restore, highlighted on hover.
///
/// A plain `egui::Button` brings its own outline stroke, the aliasing source the dialog avoids
/// everywhere, so the entry is a filled rect with an explicitly painted text.
pub(crate) fn settings_row_menu_item(
    ui: &mut egui::Ui,
    label: &str,
    detail: &str,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_MENU_ITEM_HEIGHT),
        sense,
    );
    let fill = if !enabled {
        egui::Color32::TRANSPARENT
    } else if response.is_pointer_button_down_on() {
        UI_SURFACE_ACTIVE
    } else if response.hovered() {
        UI_SURFACE_HOVER
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = if enabled {
        UI_TEXT_STRONG
    } else {
        UI_TEXT_FAINT
    };
    let detail_color = if enabled { UI_TEXT_DIM } else { UI_TEXT_FAINT };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if fill != egui::Color32::TRANSPARENT {
            painter.rect_filled(rect, 7.0, fill);
        }
        painter.text(
            egui::pos2(rect.left() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label.to_owned(),
            egui::FontId::proportional(12.5),
            text_color,
        );
        if !detail.is_empty() {
            painter.text(
                egui::pos2(rect.right() - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                detail.to_owned(),
                egui::FontId::monospace(11.0),
                detail_color,
            );
        }
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

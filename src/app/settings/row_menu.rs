//! Right-click menu for a settings row: one "Reset to Default" entry.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) const ROW_MENU_WIDTH: f32 = 232.0;
pub(crate) const ROW_MENU_ITEM_HEIGHT: f32 = 30.0;
pub(crate) const ROW_MENU_MARGIN: f32 = 6.0;
pub(crate) const ROW_MENU_ENTRY_GAP: f32 = 12.0;
pub(crate) const ROW_MENU_ENTRY_PAD: f32 = 10.0;

impl ImageViewer {
    /// Starts a fresh row-rect collection for this frame's settings body.
    pub(crate) fn begin_settings_row_rects(&mut self) {
        self.settings.row_rects.clear();
    }

    /// Opens the row menu when this frame's right-click landed on a row. Runs after the body, so
    /// `row_rects` already holds every row drawn above it.
    ///
    /// The hit test walks the rects by distance and takes the closest, which is how egui's own
    /// hit test compensates for the small gap between two rows: a click a few pixels outside a
    /// row still lands on it instead of doing nothing.
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
        if let Some(id) = Self::settings_row_at(&self.settings.row_rects, pos) {
            self.settings.row_menu = Some(SettingsRowMenu { id, pos });
        }
    }

    /// The row under `pos`: the closest one whose rect (grown by the interaction radius) holds it.
    pub(crate) fn settings_row_at(rows: &[(usize, egui::Rect)], pos: egui::Pos2) -> Option<usize> {
        let radius = egui::Vec2::splat(4.0);
        rows.iter()
            .filter(|(_, rect)| rect.expand2(radius).contains(pos))
            .min_by(|(_, a), (_, b)| {
                let distance = |rect: egui::Rect| rect.distance_sq_to_pos(pos);
                distance(*a)
                    .partial_cmp(&distance(*b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| *id)
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
        let editable = self.settings.pending_confirm.is_none()
            && self
                .settings
                .selected
                .as_deref()
                .is_some_and(|name| !crate::settings_registry::is_builtin(name));

        let screen = ctx.screen_rect();
        let width = ROW_MENU_WIDTH.min(screen.width());
        let pos = egui::pos2(
            menu.pos.x.clamp(
                screen.min.x + 8.0,
                (screen.max.x - width - 8.0).max(screen.min.x + 8.0),
            ),
            menu.pos.y.clamp(
                screen.min.y + 8.0,
                (screen.max.y - ROW_MENU_ITEM_HEIGHT - 16.0).max(screen.min.y + 8.0),
            ),
        );

        let mut reset = false;
        let response = egui::Area::new(egui::Id::new("settings_row_menu"))
            .order(egui::Order::Tooltip)
            .pivot(egui::Align2::LEFT_TOP)
            .fixed_pos(pos)
            .constrain_to(screen)
            .show(ctx, |ui| {
                ui.set_width(width);
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
                                if settings_row_menu_item(
                                    ui,
                                    "Reset to Default",
                                    &default_text,
                                    width - 14.0,
                                    editable,
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

/// One menu entry: "Reset to Default" on the left with the value it restores beside it.
///
/// A short value (`30`, `true`, `Alpha`) sits right-aligned on the same line; a long one (a
/// binding list) wraps under the label and the entry grows taller, both lines centred against
/// each other. A plain `egui::Button` would bring its own outline stroke, the aliasing source the
/// dialog avoids everywhere, so the entry is a filled rect with explicitly painted text.
pub(crate) fn settings_row_menu_item(
    ui: &mut egui::Ui,
    label: &str,
    detail: &str,
    inner_width: f32,
    enabled: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(12.5);
    let detail_font = egui::FontId::monospace(11.0);
    let label_galley =
        ui.fonts(|fonts| fonts.layout_no_wrap(label.to_owned(), font.clone(), UI_TEXT_STRONG));
    let detail_galley = (!detail.is_empty()).then(|| {
        ui.fonts(|fonts| fonts.layout_no_wrap(detail.to_owned(), detail_font.clone(), UI_TEXT_DIM))
    });
    // One line when label + gap + value fit, otherwise the value takes a full second line and
    // wraps inside it: `photo/2024/**` style values stay readable instead of being clipped.
    let detail_width = detail_galley.as_ref().map_or(0.0, |galley| galley.size().x);
    let same_line = detail_galley.is_none()
        || label_galley.size().x + ROW_MENU_ENTRY_GAP + detail_width
            <= inner_width - ROW_MENU_ENTRY_PAD * 2.0;
    let detail_galley = detail_galley.map(|galley| {
        if same_line {
            galley
        } else {
            // Re-layout wrapping at the entry's inner width, so nothing is cut horizontally.
            ui.fonts(|fonts| {
                fonts.layout(
                    detail.to_owned(),
                    detail_font.clone(),
                    UI_TEXT_DIM,
                    inner_width - ROW_MENU_ENTRY_PAD * 2.0,
                )
            })
        }
    });

    let line_height = label_galley.size().y.max(15.0);
    let height = match &detail_galley {
        Some(galley) if !same_line => {
            label_galley.size().y + 4.0 + galley.size().y + 2.0 * (ROW_MENU_ENTRY_PAD * 0.8)
        }
        _ => ROW_MENU_ITEM_HEIGHT,
    };
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    let fill = if !enabled {
        egui::Color32::TRANSPARENT
    } else if response.is_pointer_button_down_on() {
        UI_SURFACE_ACTIVE
    } else if response.hovered() {
        UI_SURFACE_HOVER
    } else {
        egui::Color32::TRANSPARENT
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if fill != egui::Color32::TRANSPARENT {
            painter.rect_filled(rect, 7.0, fill);
        }
        let text_color = if enabled {
            UI_TEXT_STRONG
        } else {
            UI_TEXT_FAINT
        };
        let detail_color = if enabled { UI_TEXT_DIM } else { UI_TEXT_FAINT };
        match detail_galley.as_ref().filter(|_| !same_line) {
            Some(detail_galley) => {
                // Stacked: label over value, the block centred vertically in the entry and both
                // lines starting on the same left edge.
                let block = label_galley.size().y + 4.0 + detail_galley.size().y;
                let top = rect.center().y - block * 0.5;
                let left = rect.left() + ROW_MENU_ENTRY_PAD;
                painter.galley(egui::pos2(left, top), label_galley.clone(), text_color);
                painter.galley(
                    egui::pos2(left, top + label_galley.size().y + 4.0),
                    detail_galley.clone(),
                    detail_color,
                );
            }
            None => {
                // One line: label left, value right, both on the entry's centre line.
                let y = rect.center().y;
                painter.galley(
                    egui::pos2(rect.left() + ROW_MENU_ENTRY_PAD, y - line_height * 0.5),
                    label_galley.clone(),
                    text_color,
                );
                if let Some(detail_galley) = detail_galley.as_ref() {
                    let detail_height = detail_galley.size().y;
                    painter.galley(
                        egui::pos2(
                            rect.right() - ROW_MENU_ENTRY_PAD - detail_galley.size().x,
                            y - detail_height * 0.5,
                        ),
                        detail_galley.clone(),
                        detail_color,
                    );
                }
            }
        }
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

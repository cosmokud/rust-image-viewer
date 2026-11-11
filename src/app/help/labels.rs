//! Key/action help labels.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn binding_to_help_label(binding: &InputBinding) -> String {
        match binding {
            InputBinding::Key(key) => Self::key_to_help_label(*key),
            InputBinding::KeyWithCtrl(key) => {
                format!("Ctrl + {}", Self::key_to_help_label(*key))
            }
            InputBinding::KeyWithShift(key) => {
                format!("Shift + {}", Self::key_to_help_label(*key))
            }
            InputBinding::KeyWithAlt(key) => {
                format!("Alt + {}", Self::key_to_help_label(*key))
            }
            InputBinding::MouseLeft => "Left Click".to_string(),
            InputBinding::MouseRight => "Right Click".to_string(),
            InputBinding::MouseMiddle => "Middle Click".to_string(),
            InputBinding::Mouse4 => "Mouse 4".to_string(),
            InputBinding::Mouse5 => "Mouse 5".to_string(),
            InputBinding::ScrollUp => "Wheel Up".to_string(),
            InputBinding::ScrollDown => "Wheel Down".to_string(),
            InputBinding::CtrlScrollUp => "Ctrl + Wheel Up".to_string(),
            InputBinding::CtrlScrollDown => "Ctrl + Wheel Down".to_string(),
            InputBinding::ShiftScrollUp => "Shift + Wheel Up".to_string(),
            InputBinding::ShiftScrollDown => "Shift + Wheel Down".to_string(),
        }
    }

    pub(crate) fn action_bindings_help_label(&self, action: Action) -> String {
        let bindings = self.config.get_bindings(action);
        if bindings.is_empty() {
            "Unbound".to_string()
        } else {
            bindings
                .iter()
                .map(Self::binding_to_help_label)
                .collect::<Vec<_>>()
                .join("  |  ")
        }
    }

    pub(crate) fn draw_shortcuts_help_section_header(
        ui: &mut egui::Ui,
        title: &str,
        subtitle: &str,
    ) {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(title).color(UI_TEXT_STRONG).size(15.0));
        ui.add_space(2.0);
        ui.label(egui::RichText::new(subtitle).color(UI_TEXT_DIM).size(12.0));
        ui.add_space(10.0);
    }

    pub(crate) fn draw_shortcuts_help_row(
        ui: &mut egui::Ui,
        trigger: &str,
        title: &str,
        detail: &str,
    ) {
        // Inside `ui.horizontal` labels never wrap on their own, and a long binding CSV used to
        // stretch the whole dialog. Bound both columns explicitly and wrap the text.
        let total_width = ui.available_width();
        let trigger_width = 248.0_f32.min((total_width * 0.42).max(120.0));
        let text_width = (total_width - trigger_width - 16.0).max(160.0);
        ui.horizontal_top(|ui| {
            // Filled chip, no stroke, and real padding: the old 1px stroke frame both aliased and
            // left the binding text almost touching its border.
            egui::Frame::none()
                .fill(UI_SURFACE)
                .rounding(8.0)
                .inner_margin(egui::Margin::symmetric(11.0, 7.0))
                .show(ui, |ui| {
                    ui.set_min_width(trigger_width - 22.0);
                    ui.set_max_width(trigger_width - 22.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(trigger)
                                .monospace()
                                .color(UI_TEXT)
                                .size(12.0),
                        )
                        .wrap(),
                    );
                });

            ui.add_space(16.0);

            ui.allocate_ui_with_layout(
                egui::vec2(text_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_width(text_width);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(title).color(UI_TEXT_STRONG).size(13.0),
                        )
                        .wrap(),
                    );
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(detail).color(UI_TEXT_DIM).size(12.0))
                            .wrap(),
                    );
                },
            );
        });
        ui.add_space(9.0);
    }
}

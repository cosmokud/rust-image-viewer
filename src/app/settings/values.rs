//! Mirror between `Config` and the settings UI values.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn settings_text_value(config: &Config, id: usize) -> String {
        let decimals = crate::settings_registry::SETTINGS[id].decimals();
        match crate::settings_registry::get(config, id) {
            crate::settings_registry::SettingValue::Text(text) => text,
            crate::settings_registry::SettingValue::Bool(value) => value.to_string(),
            crate::settings_registry::SettingValue::Num(value) => {
                crate::settings_registry::format_value(value, decimals)
            }
        }
    }

    /// Raw INI text mirror for the Rgb/Key/Modifier/Binding/BindingList editors.
    pub(crate) fn settings_texts_from_config(config: &Config) -> Vec<String> {
        crate::settings_registry::SETTINGS
            .iter()
            .map(|setting| Self::settings_text_value(config, setting.id))
            .collect()
    }

    pub(crate) fn refresh_settings_mirrors(&mut self) {
        self.settings.values = Self::settings_values_from_config(&self.config);
        self.settings.texts = Self::settings_texts_from_config(&self.config);
    }

    pub(crate) fn set_settings_status(&mut self, message: String, is_error: bool) {
        self.settings.status = Some((message, Instant::now()));
        self.settings.status_is_error = is_error;
    }

    pub(crate) fn open_settings_modal(&mut self, ctx: &egui::Context, tab: SettingsTabSlot) {
        // Already open: bring the existing dialog to the front instead of resetting its state.
        if self.settings.open {
            ctx.send_viewport_cmd_to(settings_viewport_id(), egui::ViewportCommand::Focus);
            ctx.request_repaint();
            return;
        }
        self.settings.open = true;
        // Geometry is recomputed from the screen on every open: the dialog always comes back
        // centered at 50% of the screen, never at the size of the last manual resize.
        self.settings.screen = None;
        self.settings.place_frames = SETTINGS_WINDOW_PLACE_FRAMES;
        self.settings.tab = tab;
        self.settings.search.clear();
        self.settings.focus_search = false;
        self.settings.rename = None;
        self.settings.pending_confirm = None;
        self.settings.status = None;
        self.settings.status_is_error = false;
        self.settings.save_due_at = None;
        self.refresh_settings_mirrors();
        self.settings.pending_confirm = None;
        // Opening Settings is the first-run entry point: with only `Default` in the folder, seed
        // an editable `User` preset as an exact copy of `default.ini`.
        let _ = crate::settings_registry::ensure_user_preset();
        self.refresh_settings_presets();
        // Always reflect the preset that actually backs the live config.
        self.settings.selected = self
            .config
            .active_preset
            .clone()
            .filter(|name| {
                self.settings
                    .presets
                    .iter()
                    .any(|entry| &entry.name == name)
            })
            .or_else(|| {
                self.settings
                    .presets
                    .iter()
                    .find(|entry| !entry.builtin)
                    .map(|entry| entry.name.clone())
            });
        self.load_selected_preset_values();
    }
}

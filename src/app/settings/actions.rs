//! Opening the settings window, presets and applying changes.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn refresh_settings_presets(&mut self) {
        let _ = crate::settings_registry::ensure_builtin_presets();
        self.settings.presets = crate::settings_registry::list_presets();
        self.load_selected_preset_values();
    }

    /// Caches the selected preset's file values so the `modified` badge costs no disk read.
    pub(crate) fn load_selected_preset_values(&mut self) {
        self.settings.selected_preset_values = self.settings.selected.as_ref().and_then(|name| {
            let entry = self
                .settings
                .presets
                .iter()
                .find(|entry| &entry.name == name)?;
            let content = std::fs::read_to_string(&entry.path).ok()?;
            Some(crate::settings_registry::parse_preset(&content).0)
        });
    }

    /// True when the live config differs from the selected preset file.
    pub(crate) fn selected_preset_modified(&self) -> bool {
        if self
            .settings
            .selected
            .as_deref()
            .is_some_and(crate::settings_registry::is_builtin)
        {
            return false;
        }
        let Some(values) = self.settings.selected_preset_values.as_ref() else {
            return false;
        };
        values
            .iter()
            .any(|(id, value)| crate::settings_registry::get(&self.config, *id) != *value)
    }

    /// Runs after the debounce window so slider drags produce one preset-file write.
    pub(crate) fn flush_pending_settings_save(&mut self, ctx: &egui::Context) {
        let Some(due_at) = self.settings.save_due_at else {
            return;
        };
        let now = Instant::now();
        if now >= due_at {
            self.settings.save_due_at = None;
            self.config.save();
        } else {
            // Nothing else schedules frames while the viewer is idle.
            ctx.request_repaint_after(due_at - now);
        }
    }

    /// The one place that turns a registry consequence into an app-side setter call.
    pub(crate) fn apply_app_consequences(&mut self, ids: impl IntoIterator<Item = usize>) {
        for consequence in ids
            .into_iter()
            .filter_map(crate::settings_registry::app_consequence)
        {
            match consequence {
                crate::settings_registry::AppConsequence::MasonryItemsPerRow => {
                    self.set_masonry_items_per_row(self.config.masonry_items_per_row)
                }
                crate::settings_registry::AppConsequence::AlphaOpaqueFallback => {
                    crate::video_player::set_alpha_opaque_fallback_enabled(
                        self.config.video_alpha_opaque_fallback,
                    )
                }
            }
        }
    }

    /// Writes the active preset now (pointer released / keyboard edit) or debounces a drag.
    pub(crate) fn finish_settings_edit(&mut self, write_now: bool) {
        if write_now {
            self.settings.save_due_at = None;
            self.config.save();
        } else {
            self.settings.save_due_at = Some(Instant::now() + Duration::from_millis(300));
        }
    }

    pub(crate) fn apply_settings_preset_action(&mut self, action: SettingsPresetAction) {
        match action {
            SettingsPresetAction::Load(name) => {
                let Some(path) = self
                    .settings
                    .presets
                    .iter()
                    .find(|entry| entry.name == name)
                    .map(|entry| entry.path.clone())
                else {
                    return;
                };
                let content = match std::fs::read_to_string(&path) {
                    Ok(content) => content,
                    Err(err) => {
                        self.set_settings_status(
                            format!("Could not load '{}': {err}", path.display()),
                            true,
                        );
                        return;
                    }
                };
                let (values, mut warnings) = crate::settings_registry::parse_preset(&content);
                warnings.extend(crate::settings_registry::apply_values(
                    &mut self.config,
                    &values,
                ));
                // A registry key may own an app-side setter (layout cache invalidation).
                self.apply_app_consequences(values.iter().map(|(id, _)| *id));
                self.refresh_settings_mirrors();
                self.settings.selected = Some(name.clone());
                self.load_selected_preset_values();
                // The last loaded preset *is* the live config: point it at this file and let
                // `save` persist values plus the remembered active name.
                self.config.active_preset = Some(name.clone());
                self.config.save();
                self.settings.save_due_at = None;
                if warnings.is_empty() {
                    self.set_settings_status(format!("Loaded preset '{name}'"), false);
                } else {
                    self.set_settings_status(
                        format!(
                            "Loaded '{name}' with {} warning(s): {}",
                            warnings.len(),
                            warnings.join("; ")
                        ),
                        true,
                    );
                }
            }
            SettingsPresetAction::BeginRename(name) => {
                self.settings.pending_confirm = None;
                self.settings.rename = Some((name.clone(), name));
            }
            SettingsPresetAction::Rename(old, new) => {
                match crate::settings_registry::sanitize_preset_name(&new) {
                    Ok(clean) => {
                        self.settings.rename = None;
                        if clean == old {
                            return;
                        }
                        match crate::settings_registry::rename_preset(&old, &clean) {
                            Ok(()) => {
                                self.settings.selected = Some(clean.clone());
                                // An active preset whose file is renamed must follow the new name,
                                // or the next save would recreate the old file.
                                if self.config.active_preset.as_deref() == Some(old.as_str()) {
                                    self.config.active_preset = Some(clean.clone());
                                    self.config.save_state();
                                }
                                self.refresh_settings_presets();
                                self.set_settings_status(
                                    format!("Renamed '{old}' to '{clean}'"),
                                    false,
                                );
                            }
                            Err(err) => {
                                self.set_settings_status(format!("Could not rename: {err}"), true)
                            }
                        }
                    }
                    Err(err) => self.set_settings_status(err, true),
                }
            }
            SettingsPresetAction::CancelRename => self.settings.rename = None,
            SettingsPresetAction::ConfirmDelete(name) => {
                self.settings.pending_confirm = None;
                match crate::settings_registry::delete_preset(&name) {
                    Ok(()) => {
                        let selected_was_deleted =
                            self.settings.selected.as_deref() == Some(name.as_str());
                        self.refresh_settings_presets();
                        // Deleting the preset that backs the live config must not resurrect it on
                        // the next save: fall back to any remaining user preset, else Default.
                        if self.config.active_preset.as_deref() == Some(name.as_str()) {
                            let fallback = self
                                .settings
                                .presets
                                .iter()
                                .find(|entry| !entry.builtin)
                                .map(|entry| entry.name.clone())
                                .unwrap_or_else(|| "default".to_string());
                            self.config.active_preset = Some(fallback);
                            self.config.save_state();
                        }
                        if selected_was_deleted {
                            self.settings.selected = self.config.active_preset.clone();
                            self.load_selected_preset_values();
                        }
                        self.set_settings_status(format!("Deleted preset '{name}'"), false);
                    }
                    Err(err) => self.set_settings_status(format!("Could not delete: {err}"), true),
                }
            }
            SettingsPresetAction::Reset(name) => {
                match crate::settings_registry::reset_preset(&name) {
                    Ok(_) => {
                        if self.config.active_preset.as_deref() == Some(name.as_str()) {
                            let defaults = crate::settings_registry::capture(&Config::default());
                            let _ =
                                crate::settings_registry::apply_values(&mut self.config, &defaults);
                            self.apply_app_consequences(defaults.iter().map(|(id, _)| *id));
                            self.refresh_settings_mirrors();
                        }
                        self.refresh_settings_presets();
                        self.set_settings_status(
                            format!("Reset preset '{name}' to default settings"),
                            false,
                        );
                    }
                    Err(err) => self.set_settings_status(format!("Could not reset: {err}"), true),
                }
            }
            SettingsPresetAction::Create => {
                let raw = self.settings.new_name.clone();
                match crate::settings_registry::sanitize_preset_name(&raw) {
                    Ok(name) => {
                        if crate::settings_registry::presets_dir()
                            .join(format!("{name}.ini"))
                            .exists()
                        {
                            self.set_settings_status(
                                format!("Preset '{name}' already exists; pick another name."),
                                true,
                            );
                            return;
                        }
                        let values = crate::settings_registry::capture(&self.config);
                        match crate::settings_registry::save_preset(&name, &values) {
                            Ok(_) => {
                                self.settings.new_name.clear();
                                self.settings.selected = Some(name.clone());
                                self.refresh_settings_presets();
                                self.set_settings_status(format!("Created preset '{name}'"), false);
                            }
                            Err(err) => self.set_settings_status(
                                format!("Could not create preset: {err}"),
                                true,
                            ),
                        }
                    }
                    Err(err) => self.set_settings_status(err, true),
                }
            }
        }
    }

    /// Tooltip body for one settings row: the help text, the ini key, and why a change may not
    /// show up (restart pending, or no reader at all).
    pub(crate) fn settings_row_tooltip(
        setting: &'static crate::settings_registry::Setting,
        read_only: bool,
    ) -> String {
        let mut tooltip = format!("{}\n\nkey: {}", setting.help, setting.key);
        let hint = setting.status_hint();
        if !hint.is_empty() {
            tooltip.push('\n');
            tooltip.push_str(hint);
        }
        if read_only {
            tooltip.push_str("\n\nThe Default preset is read-only: pick another preset to edit.");
        }
        tooltip
    }
}

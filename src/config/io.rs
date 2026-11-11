//! Locating, loading, saving and the state file round-trip.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::app_dirs;
#[allow(unused_imports)]
use crate::settings_registry::{self, SETTINGS};
#[allow(unused_imports)]
use std::borrow::Cow;
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::path::PathBuf;

impl Config {
    /// Get the configuration directory in AppData/Roaming.
    /// Creates the directory if it doesn't exist.
    pub(crate) fn config_dir() -> PathBuf {
        let config_dir = if cfg!(target_os = "windows") {
            app_dirs::app_config_dir().unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.join(app_dirs::APP_DIR_NAME)))
                    .unwrap_or_else(|| PathBuf::from(".").join(app_dirs::APP_DIR_NAME))
            })
        } else {
            // On Unix-like systems, use ~/.config
            let base_dir = std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| PathBuf::from(h).join(".config"))
                })
                .unwrap_or_else(|| PathBuf::from("."));
            base_dir.join(app_dirs::APP_DIR_NAME)
        };

        // Create directory if it doesn't exist
        let _ = fs::create_dir_all(&config_dir);

        config_dir
    }

    /// Get settings file path.
    ///
    /// Uses `config.ini` in AppData/Roaming/rust-image-viewer/ on Windows.
    ///
    /// Migrates from legacy locations (`rust-image-viewer-config.ini` / `setting.ini`) if needed.
    pub fn config_path() -> PathBuf {
        let config_dir = Self::config_dir();
        let config = config_dir.join(CONFIG_FILE_NAME);

        // Migration from legacy AppData filename
        if !config.exists() {
            let legacy_appdata_config = config_dir.join(LEGACY_CONFIG_FILE_NAME);
            if legacy_appdata_config.exists() {
                if fs::rename(&legacy_appdata_config, &config).is_err() {
                    if fs::copy(&legacy_appdata_config, &config).is_ok() {
                        let _ = fs::remove_file(&legacy_appdata_config);
                    }
                }
            }
        }

        // Migration from executable directory (portable / old locations)
        if !config.exists() {
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    for legacy_name in [
                        CONFIG_FILE_NAME,
                        LEGACY_CONFIG_FILE_NAME,
                        LEGACY_SETTINGS_FILE_NAME,
                    ] {
                        let legacy_config = exe_dir.join(legacy_name);
                        if legacy_config.exists() {
                            let _ = fs::copy(&legacy_config, &config);
                            if config.exists() {
                                break;
                            }
                        }
                    }
                }
            }
        }

        config
    }

    /// Path of `state.ini`: the only file that keeps runtime state (`[State]` plus the active
    /// preset name) outside of a preset.
    pub fn state_path() -> PathBuf {
        Self::config_dir().join(STATE_FILE_NAME)
    }

    /// Path of the active preset file, when one is loaded.
    pub fn active_preset_path(&self) -> Option<PathBuf> {
        self.active_preset
            .as_deref()
            .map(crate::settings_registry::preset_path)
    }

    /// Loads the active preset (`presets/<name>.ini`); the app no longer has a `config.ini`.
    ///
    /// Startup order: migrate a leftover legacy `config.ini` into `presets/User.ini`, then use the
    /// preset remembered in `state.ini`, else the first user preset, else create `User.ini` as an
    /// exact copy of `default.ini`.
    pub fn load() -> Self {
        let _ = crate::settings_registry::ensure_builtin_presets();
        Self::migrate_legacy_config_file();

        let state = Self::read_state_file();
        let mut config = Self::default();
        let active = Self::resolve_active_preset(state.active_preset.as_deref());
        if let Some(content) =
            fs::read_to_string(crate::settings_registry::preset_path(&active)).ok()
        {
            let (values, _warnings) = crate::settings_registry::parse_preset(&content);
            let _ = crate::settings_registry::apply_values(&mut config, &values);
            // Hand-edited files are repaired in place: only offending lines change.
            if !crate::settings_registry::is_builtin(&active) {
                let (repaired, _) = crate::settings_registry::rewrite_preset(
                    &content,
                    &crate::settings_registry::capture(&config),
                );
                if repaired != content {
                    let _ = fs::write(crate::settings_registry::preset_path(&active), repaired);
                }
            }
        }
        config.active_preset = Some(active);
        if let Some(muted) = state.muted {
            config.state_muted = muted;
        }
        if let Some(volume) = state.volume {
            config.state_volume = volume.clamp(0.0, 1.0);
        }
        if let Some(show_breadcrumb_bar) = state.show_breadcrumb_bar {
            config.state_show_breadcrumb_bar = show_breadcrumb_bar;
        }
        config.save_state();
        config
    }

    /// Chooses the preset that backs the live config, creating `User.ini` when none exists.
    pub(crate) fn resolve_active_preset(remembered: Option<&str>) -> String {
        if let Some(name) = remembered {
            let valid = crate::settings_registry::is_builtin(name)
                || crate::settings_registry::sanitize_preset_name(name).is_ok();
            if valid && crate::settings_registry::preset_path(name).exists() {
                return name.to_string();
            }
        }
        crate::settings_registry::ensure_user_preset()
            .unwrap_or_else(|_| USER_PRESET_NAME.to_string())
    }

    /// One-shot migration from the deprecated `config.ini` in the roaming root.
    ///
    /// A value that differs from the hardcoded defaults is carried over into `presets/User.ini`;
    /// the remembered `[State]` is carried into `state.ini`. The old file is then deleted so the
    /// check never runs again.
    pub(crate) fn migrate_legacy_config_file() {
        let legacy_path = Self::config_path();
        if !legacy_path.exists() {
            return;
        }
        let content = fs::read_to_string(&legacy_path).unwrap_or_default();
        let (content, _) = strip_legacy_config_version_tag(&content);
        let legacy = Self::parse_ini(content.as_ref());
        let differs = crate::settings_registry::capture(&legacy)
            != crate::settings_registry::capture(&Self::default());

        if differs {
            let user_path = crate::settings_registry::preset_path(USER_PRESET_NAME);
            if !user_path.exists() {
                let values = crate::settings_registry::capture(&legacy);
                let rendered =
                    crate::settings_registry::render_preset(USER_PRESET_NAME, false, &values);
                let _ = crate::settings_registry::presets_dir();
                let _ = fs::write(&user_path, rendered);
            }
        }

        let active = if differs {
            Some(USER_PRESET_NAME.to_string())
        } else {
            Self::read_state_file().active_preset
        };
        let _ = Self::write_state_file(
            active.as_deref(),
            legacy.state_muted,
            legacy.state_volume,
            legacy.state_show_breadcrumb_bar,
        );
        let _ = fs::remove_file(&legacy_path);
    }

    pub(crate) fn read_state_file() -> StateFile {
        let mut state = StateFile::default();
        let Ok(content) = fs::read_to_string(Self::state_path()) else {
            return state;
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with('[')
            {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "active_preset" => {
                    if !value.is_empty() {
                        state.active_preset = Some(value.to_string());
                    }
                }
                "muted_state" => state.muted = parse_bool(value),
                "volume_state" => state.volume = value.parse::<f64>().ok(),
                "show_breadcrumb_bar" => state.show_breadcrumb_bar = parse_bool(value),
                _ => {}
            }
        }
        state
    }

    pub(crate) fn write_state_file(
        active_preset: Option<&str>,
        muted: bool,
        volume: f64,
        show_breadcrumb_bar: bool,
    ) -> std::io::Result<()> {
        let mut content = String::with_capacity(256);
        content.push_str("; rust-image-viewer runtime state (not a preset)\n");
        content.push_str("[State]\n");
        if let Some(name) = active_preset {
            content.push_str(&format!("active_preset = {name}\n"));
        }
        content.push_str(&format!("muted_state = {}\n", bool_to_ini(muted)));
        content.push_str(&format!(
            "volume_state = {}\n",
            format_with_optional_trailing_zero_f64(volume)
        ));
        content.push_str(&format!(
            "show_breadcrumb_bar = {}\n",
            bool_to_ini(show_breadcrumb_bar)
        ));
        fs::write(Self::state_path(), content)
    }

    /// Persists `[State]` and the active preset name into `state.ini`.
    pub fn save_state(&self) {
        let _ = Self::write_state_file(
            self.active_preset.as_deref(),
            self.state_muted,
            self.state_volume,
            self.state_show_breadcrumb_bar,
        );
    }

    /// Saves the live settings into the active preset file and refreshes `state.ini`.
    ///
    /// The write is surgical: existing lines keep their comments and ordering, only value text
    /// changes, unknown keys are commented out with a reason and missing keys are appended.
    /// A built-in preset (read-only `Default`) is never written.
    pub fn save(&self) {
        if let Some(name) = self.active_preset.as_deref() {
            if !crate::settings_registry::is_builtin(name) {
                let values = crate::settings_registry::capture(self);
                let _ = crate::settings_registry::save_preset(name, &values);
            }
        }
        self.save_state();
    }

    /// Repairs the active preset on disk and refreshes `state.ini` (idle-time maintenance).
    pub fn sync_disk_file_with_template(&self) {
        self.save();
    }

    pub(crate) fn action_bindings_csv(&self, action: Action) -> String {
        self.action_bindings
            .get(&action)
            .map(|bindings| {
                bindings
                    .iter()
                    .map(binding_to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    }

    /// Check if an input matches a specific action
    pub fn is_action(&self, input: &InputBinding, action: Action) -> bool {
        self.action_bindings
            .get(&action)
            .is_some_and(|bindings| bindings.contains(input))
    }

    /// Get all bindings for an action
    pub fn get_bindings(&self, action: Action) -> Vec<InputBinding> {
        self.action_bindings
            .get(&action)
            .cloned()
            .unwrap_or_default()
    }

    pub fn action_uses_binding(&self, action: Action, binding: &InputBinding) -> bool {
        self.is_action(binding, action)
    }

    pub fn any_action_uses_binding(&self, binding: &InputBinding) -> bool {
        self.action_bindings
            .values()
            .any(|bindings| bindings.contains(binding))
    }

    pub fn update_video_state(&mut self, muted: bool, volume: f64) {
        self.state_muted = muted;
        self.state_volume = volume.clamp(0.0, 1.0);
    }
}

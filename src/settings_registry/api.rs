//! The public registry API used by the settings UI and by `Config`.

#![allow(unused_imports)]

use super::*;

use crate::config::{
    optional_binding_to_string, optional_mark_key_to_string, optional_shortcut_modifier_to_string,
    parse_binding_list, parse_bool, parse_optional_binding, parse_optional_mark_key,
    parse_optional_shortcut_modifier, parse_rgb_triplet, Action, Config, ImageFilter,
    MangaVirtualizationBackend, StartupWindowMode, TextureFilter, VideoSeekPolicy,
    WindowTitlePathMode,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn setting_index_for_key(key: &str) -> Option<usize> {
    SETTINGS
        .iter()
        .find(|setting| setting.key.eq_ignore_ascii_case(key))
        .map(|setting| setting.id)
}

pub fn get(config: &Config, id: usize) -> SettingValue {
    match SETTINGS.get(id) {
        Some(_) => value_of(config, id),
        None => SettingValue::Num(0.0),
    }
}

/// Numeric view for sliders: bools become 1/0, text (enum/rgb/binding) renders as 0.
pub fn get_num(config: &Config, id: usize) -> f64 {
    match get(config, id) {
        SettingValue::Bool(value) => {
            if value {
                1.0
            } else {
                0.0
            }
        }
        SettingValue::Num(value) => value,
        SettingValue::Text(_) => 0.0,
    }
}

pub fn set(config: &mut Config, id: usize, value: &SettingValue) -> Result<(), String> {
    let Some(setting) = SETTINGS.get(id) else {
        return Err(format!("unknown setting id {id}"));
    };
    // Slider drags produce full f64 precision (`29.915012`); snap to the precision the UI
    // displays so the preset file and the numeric box never show noise digits.
    let quantized;
    let value = match (&setting.kind, value) {
        (SettingKind::Int { .. }, SettingValue::Num(raw)) => {
            quantized = SettingValue::Num(raw.round());
            &quantized
        }
        (SettingKind::Float { decimals, .. }, SettingValue::Num(raw)) => {
            let factor = 10f64.powi(*decimals as i32);
            quantized = SettingValue::Num((raw * factor).round() / factor);
            &quantized
        }
        _ => value,
    };
    set_value(config, setting, value).map_err(|err| format!("{}: {err}", setting.key))?;
    enforce_cross_guards(config, setting.key);
    Ok(())
}

pub fn set_num(config: &mut Config, id: usize, value: f64) -> Result<(), String> {
    set(config, id, &SettingValue::Num(value))
}

/// The value `default.ini` ships for one key: the file is rendered from `Config::default()`,
/// so a per-key reset lands on exactly the text the built-in preset carries.
pub fn default_value(id: usize) -> Option<&'static SettingValue> {
    static DEFAULTS: OnceLock<Vec<SettingValue>> = OnceLock::new();
    DEFAULTS
        .get_or_init(|| {
            capture(&Config::default())
                .into_iter()
                .map(|(_, value)| value)
                .collect()
        })
        .get(id)
}

pub fn capture(config: &Config) -> Vec<(usize, SettingValue)> {
    SETTINGS
        .iter()
        .map(|setting| (setting.id, get(config, setting.id)))
        .collect()
}

/// Applies what parsed, and returns one warning per rejected key instead of failing the load.
pub fn apply_values(config: &mut Config, values: &[(usize, SettingValue)]) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, value) in values {
        if let Err(err) = set(config, *id, value) {
            warnings.push(err);
        }
    }
    warnings
}

/// Registry-declared side effect of writing `id`: `main` matches this once instead of
/// knowing which keys have app-side setters.
pub fn app_consequence(id: usize) -> Option<AppConsequence> {
    match id {
        47 => Some(AppConsequence::MasonryItemsPerRow),
        138 => Some(AppConsequence::AlphaOpaqueFallback),
        _ => None,
    }
}

/// Search over registry data: case-folded substring over key/label/group/help; an empty
/// query matches nothing so the dialog keeps its normal tabs.
pub fn settings_matches(query: &str, setting: &Setting) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    [setting.key, setting.label, setting.group, setting.help]
        .iter()
        .any(|haystack| haystack.to_lowercase().contains(&needle))
}

/// Matching ids in `SETTINGS` order.
pub fn settings_search_results(query: &str) -> Vec<usize> {
    SETTINGS
        .iter()
        .filter(|setting| settings_matches(query, setting))
        .map(|setting| setting.id)
        .collect()
}

/// `decimals == 0` renders integer text; otherwise trailing zeros/dot are trimmed so a
/// value never reads like `2.0000001`.
pub fn format_value(value: f64, decimals: usize) -> String {
    if decimals == 0 {
        return format!("{}", value.round() as i64);
    }
    let mut text = format!("{:.1$}", value, decimals);
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

//! Value formatting, section names and cross-setting guards.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::config::{
    optional_binding_to_string, optional_mark_key_to_string, optional_shortcut_modifier_to_string,
    parse_binding_list, parse_bool, parse_optional_binding, parse_optional_mark_key,
    parse_optional_shortcut_modifier, parse_rgb_triplet, Action, Config, ImageFilter,
    MangaVirtualizationBackend, StartupWindowMode, TextureFilter, VideoSeekPolicy,
    WindowTitlePathMode,
};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::OnceLock;

pub(crate) fn value_text(setting: &Setting, value: &SettingValue) -> String {
    match value {
        SettingValue::Bool(value) => {
            if *value {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        SettingValue::Num(value) => format_value(*value, setting.decimals()),
        SettingValue::Text(text) => text.clone(),
    }
}

pub(crate) fn section_of(tab: SettingsTab) -> &'static str {
    match tab {
        SettingsTab::Video => "Video",
        SettingsTab::Performance => "Performance",
        SettingsTab::Shortcuts => "Shortcuts",
        _ => "Settings",
    }
}

pub(crate) fn as_num(value: &SettingValue) -> Result<f64, String> {
    match value {
        SettingValue::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
        SettingValue::Num(value) => Ok(*value),
        SettingValue::Text(text) => text
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("`{text}` is not a number")),
    }
}

pub(crate) fn as_bool(value: &SettingValue) -> Result<bool, String> {
    match value {
        SettingValue::Bool(value) => Ok(*value),
        SettingValue::Num(value) => Ok(*value != 0.0),
        SettingValue::Text(text) => {
            parse_bool(text).ok_or_else(|| format!("`{text}` is not true/false"))
        }
    }
}

pub(crate) fn as_text(value: &SettingValue, decimals: usize) -> Result<String, String> {
    match value {
        SettingValue::Bool(value) => Ok(if *value { "true" } else { "false" }.to_string()),
        SettingValue::Num(value) => Ok(format_value(*value, decimals)),
        SettingValue::Text(text) => Ok(text.clone()),
    }
}

pub(crate) fn set_value(
    config: &mut Config,
    setting: &Setting,
    value: &SettingValue,
) -> Result<(), String> {
    match setting.kind {
        SettingKind::Bool => set_bool(config, setting.id, as_bool(value)?),
        SettingKind::Int { min, max } => {
            let value = as_num(value)?;
            if !value.is_finite() {
                return Err("value must be finite".to_string());
            }
            set_num_field(
                config,
                setting.id,
                value.round().clamp(min as f64, max as f64),
            );
        }
        SettingKind::Float { min, max, .. } => {
            let value = as_num(value)?;
            if !value.is_finite() {
                return Err("value must be finite".to_string());
            }
            set_num_field(config, setting.id, value.clamp(min, max));
        }
        _ => set_text(config, setting, &as_text(value, setting.decimals())?)?,
    }
    Ok(())
}

/// Keeps the autoscroll min/max pairs ordered: the key the caller just wrote wins.
pub(crate) fn enforce_cross_guards(config: &mut Config, key: &str) {
    match key {
        "manga_autoscroll_min_speed_multiplier" | "manga_autoscroll_max_speed_multiplier" => {
            let min = config.manga_autoscroll_min_speed_multiplier;
            let max = config.manga_autoscroll_max_speed_multiplier;
            if min > max {
                if key == "manga_autoscroll_min_speed_multiplier" {
                    config.manga_autoscroll_max_speed_multiplier = min;
                } else {
                    config.manga_autoscroll_min_speed_multiplier = max;
                }
            }
        }
        "manga_autoscroll_min_speed_px_per_sec" | "manga_autoscroll_max_speed_px_per_sec" => {
            let min = config.manga_autoscroll_min_speed_px_per_sec;
            let max = config.manga_autoscroll_max_speed_px_per_sec;
            if min > max {
                if key == "manga_autoscroll_min_speed_px_per_sec" {
                    config.manga_autoscroll_max_speed_px_per_sec = min;
                } else {
                    config.manga_autoscroll_min_speed_px_per_sec = max;
                }
            }
        }
        _ => {}
    }
}

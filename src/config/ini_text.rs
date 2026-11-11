//! Value parsers and formatters used when reading/writing INI text.

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

pub(crate) fn strip_legacy_config_version_tag(content: &str) -> (Cow<'_, str>, bool) {
    let content = content.trim_start_matches('\u{feff}');
    let Some(first_line) = content.lines().next() else {
        return (Cow::Borrowed(content), false);
    };

    if !is_config_version_tag_line(first_line) {
        return (Cow::Borrowed(content), false);
    }

    let remaining = &content[first_line.len()..];
    let remaining = remaining.trim_start_matches(&['\r', '\n'][..]);
    (Cow::Owned(remaining.to_string()), true)
}

pub(crate) fn is_config_version_tag_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return false;
    }

    let version = &trimmed[1..trimmed.len() - 1];
    is_semver_triplet(version)
}

pub(crate) fn is_semver_triplet(version: &str) -> bool {
    let mut parts = version.split('.');

    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(major), Some(minor), Some(patch), None) => [major, minor, patch]
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit())),
        _ => false,
    }
}

pub(crate) fn format_with_optional_trailing_zero_f64(value: f64) -> String {
    let mut value_str = format!("{}", value);
    if !value_str.contains('.') && !value_str.contains('e') && !value_str.contains('E') {
        value_str.push_str(".0");
    }
    value_str
}

pub(crate) fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

pub(crate) fn parse_optional_binding(value: &str) -> Option<Option<InputBinding>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("disabled")
    {
        return Some(None);
    }

    parse_input_binding(value).map(Some)
}

pub(crate) fn parse_optional_mark_key(value: &str) -> Option<Option<egui::Key>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("disabled")
    {
        return Some(None);
    }

    parse_key(value).map(Some)
}

pub(crate) fn parse_optional_shortcut_modifier(value: &str) -> Option<Option<ShortcutModifier>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("disabled")
    {
        return Some(None);
    }

    ShortcutModifier::from_str(value).map(Some)
}

pub(crate) fn parse_rgb_triplet(value: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = value
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].parse::<u8>().ok()?;
    let g = parts[1].parse::<u8>().ok()?;
    let b = parts[2].parse::<u8>().ok()?;
    Some([r, g, b])
}

pub(crate) fn parse_u8_clamped(value: &str) -> Option<u8> {
    if let Ok(v) = value.trim().parse::<i32>() {
        return Some(v.clamp(0, 255) as u8);
    }

    if let Ok(v) = value.trim().parse::<f32>() {
        if v.is_finite() {
            return Some(v.round().clamp(0.0, 255.0) as u8);
        }
    }

    None
}

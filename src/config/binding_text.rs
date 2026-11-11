//! Binding list text round-trip helpers.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub(crate) fn parse_binding_list(value: &str) -> Vec<InputBinding> {
    let mut bindings = Vec::new();

    for binding_str in value.split(',') {
        let trimmed = binding_str.trim();
        if let Some(binding) = parse_input_binding(trimmed) {
            if !bindings.contains(&binding) {
                bindings.push(binding);
            }
        }
    }

    bindings
}

/// Convert InputBinding back to string representation
pub(crate) fn binding_to_string(binding: &InputBinding) -> String {
    match binding {
        InputBinding::Key(key) => key_to_string(key),
        InputBinding::MouseLeft => "mouse_left".to_string(),
        InputBinding::MouseRight => "mouse_right".to_string(),
        InputBinding::MouseMiddle => "mouse_middle".to_string(),
        InputBinding::Mouse4 => "mouse4".to_string(),
        InputBinding::Mouse5 => "mouse5".to_string(),
        InputBinding::ScrollUp => "scroll_up".to_string(),
        InputBinding::ScrollDown => "scroll_down".to_string(),
        InputBinding::CtrlScrollUp => "ctrl+scroll_up".to_string(),
        InputBinding::CtrlScrollDown => "ctrl+scroll_down".to_string(),
        InputBinding::ShiftScrollUp => "shift+scroll_up".to_string(),
        InputBinding::ShiftScrollDown => "shift+scroll_down".to_string(),
        InputBinding::KeyWithCtrl(key) => format!("ctrl+{}", key_to_string(key)),
        InputBinding::KeyWithShift(key) => format!("shift+{}", key_to_string(key)),
        InputBinding::KeyWithAlt(key) => format!("alt+{}", key_to_string(key)),
    }
}

pub(crate) fn optional_binding_to_string(binding: Option<&InputBinding>) -> String {
    binding
        .map(binding_to_string)
        .unwrap_or_else(|| "none".to_string())
}

pub(crate) fn optional_mark_key_to_string(key: Option<&egui::Key>) -> String {
    key.map(key_to_string).unwrap_or_else(|| "none".to_string())
}

pub(crate) fn optional_shortcut_modifier_to_string(modifier: Option<&ShortcutModifier>) -> String {
    modifier
        .map(ShortcutModifier::as_str)
        .unwrap_or("none")
        .to_string()
}

pub(crate) fn key_to_string(key: &egui::Key) -> String {
    format!("{:?}", key).to_lowercase()
}

pub(crate) fn bool_to_ini(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

//! Input binding types and their text form.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

impl ImageFilter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Triangle => "triangle",
            Self::CatmullRom => "catmullrom",
            Self::Gaussian => "gaussian",
            Self::Lanczos3 => "lanczos3",
        }
    }
}

impl TextureFilter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Linear => "linear",
        }
    }
}

/// Represents all possible input types for shortcuts
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InputBinding {
    // Keyboard keys
    Key(egui::Key),
    // Mouse buttons
    MouseLeft,
    MouseRight,
    MouseMiddle,
    Mouse4,
    Mouse5,
    // Scroll wheel
    ScrollUp,
    ScrollDown,
    // Scroll wheel with modifiers
    CtrlScrollUp,
    CtrlScrollDown,
    ShiftScrollUp,
    ShiftScrollDown,
    // Key modifiers
    KeyWithCtrl(egui::Key),
    KeyWithShift(egui::Key),
    KeyWithAlt(egui::Key),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutModifier {
    Ctrl,
    Shift,
    Alt,
}

impl ShortcutModifier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
        }
    }
}

/// Parse an input binding from string
pub fn parse_input_binding(s: &str) -> Option<InputBinding> {
    let s = s.trim().to_lowercase();

    // Check for modifiers with scroll wheel first (special case)
    if let Some(scroll_str) = s.strip_prefix("ctrl+") {
        match scroll_str {
            "scroll_up" | "wheel_up" => return Some(InputBinding::CtrlScrollUp),
            "scroll_down" | "wheel_down" => return Some(InputBinding::CtrlScrollDown),
            _ => return parse_key(scroll_str).map(InputBinding::KeyWithCtrl),
        }
    }
    if let Some(key_str) = s.strip_prefix("shift+") {
        match key_str {
            "scroll_up" | "wheel_up" => return Some(InputBinding::ShiftScrollUp),
            "scroll_down" | "wheel_down" => return Some(InputBinding::ShiftScrollDown),
            _ => return parse_key(key_str).map(InputBinding::KeyWithShift),
        }
    }
    if let Some(key_str) = s.strip_prefix("alt+") {
        return parse_key(key_str).map(InputBinding::KeyWithAlt);
    }

    // Mouse buttons
    match s.as_str() {
        "mouse_left" | "left_click" | "lmb" => return Some(InputBinding::MouseLeft),
        "mouse_right" | "right_click" | "rmb" => return Some(InputBinding::MouseRight),
        "mouse_middle" | "middle_click" | "mmb" => return Some(InputBinding::MouseMiddle),
        "mouse4" | "mouse_4" | "xbutton1" => return Some(InputBinding::Mouse4),
        "mouse5" | "mouse_5" | "xbutton2" => return Some(InputBinding::Mouse5),
        "scroll_up" | "wheel_up" => return Some(InputBinding::ScrollUp),
        "scroll_down" | "wheel_down" => return Some(InputBinding::ScrollDown),
        _ => {}
    }

    // Regular key
    parse_key(&s).map(InputBinding::Key)
}

pub(crate) fn is_strip_item_open_binding(binding: &InputBinding) -> bool {
    matches!(
        binding,
        InputBinding::MouseRight
            | InputBinding::MouseMiddle
            | InputBinding::Key(_)
            | InputBinding::KeyWithCtrl(_)
            | InputBinding::KeyWithShift(_)
            | InputBinding::KeyWithAlt(_)
    )
}

impl VideoSeekPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Accurate => "accurate",
            Self::Keyframe => "keyframe",
        }
    }
}

impl MangaVirtualizationBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Linear => "linear",
            Self::RTree => "rtree",
        }
    }
}

impl StartupWindowMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Floating => "floating",
            Self::Fullscreen => "fullscreen",
        }
    }
}

impl WindowTitlePathMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::FullPath => "true",
            Self::Filename => "false",
        }
    }
}

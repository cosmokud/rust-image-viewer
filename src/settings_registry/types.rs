//! The setting vocabulary: tabs, kinds, controls, values, and one setting's metadata.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    ZoomScroll,
    Motion,
    Autoscroll,
    Window,
    Files,
    Video,
    Performance,
    Shortcuts,
}

/// Whether a changed value needs the app restarted, and whether anything reads the key at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    Live,
    Restart,
}

#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    Bool,
    Int { min: u64, max: u64 },
    Float { min: f64, max: f64, decimals: usize },
    Enum { values: &'static [&'static str] },
    Rgb,
    Key,
    Modifier,
    Binding,
    BindingList,
}

/// What the settings dialog draws for one row. The dialog matches this, never `SettingKind`,
/// so a new kind is a registry change only.
#[derive(Clone, Copy, Debug)]
pub enum RowControl {
    Slider { min: f64, max: f64, decimals: usize },
    Toggle,
    Choice(&'static [&'static str]),
    RgbText,
    KeyText,
    ModifierText,
    BindingText,
    BindingListText,
}

/// App-side effects a registry write must trigger; the only place `main` inspects ids for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppConsequence {
    MasonryItemsPerRow,
    AlphaOpaqueFallback,
}

/// `Text` carries the raw INI string for `Rgb`/`Key`/`Modifier`/`Binding`/`BindingList`
/// (and for the `remember` words of the tri-state video keys).
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Num(f64),
    Text(String),
}

#[derive(Clone, Copy, Debug)]
pub struct Setting {
    pub id: usize,
    pub key: &'static str,
    pub tab: SettingsTab,
    pub label: &'static str,
    pub group: &'static str,
    pub help: &'static str,
    pub kind: SettingKind,
    pub applies: Applies,
    pub unused: bool,
    pub action: Option<Action>,
}

impl Setting {
    pub fn control(&self) -> RowControl {
        match self.kind {
            SettingKind::Bool => RowControl::Toggle,
            SettingKind::Int { min, max } => RowControl::Slider {
                min: min as f64,
                max: max as f64,
                decimals: 0,
            },
            SettingKind::Float { min, max, decimals } => RowControl::Slider { min, max, decimals },
            SettingKind::Enum { values } => RowControl::Choice(values),
            SettingKind::Rgb => RowControl::RgbText,
            SettingKind::Key => RowControl::KeyText,
            SettingKind::Modifier => RowControl::ModifierText,
            SettingKind::Binding => RowControl::BindingText,
            SettingKind::BindingList => RowControl::BindingListText,
        }
    }

    /// Digits shown for numeric rows; `0` renders integer text.
    pub fn decimals(&self) -> usize {
        match self.kind {
            SettingKind::Float { decimals, .. } => decimals,
            _ => 0,
        }
    }

    /// Empty for live keys; the modal appends this to the row tooltip.
    pub fn status_hint(&self) -> &'static str {
        if self.unused {
            return "no reader: changing this has no effect";
        }
        match self.applies {
            Applies::Restart => "restart required: takes effect on the next launch",
            Applies::Live => "",
        }
    }
}

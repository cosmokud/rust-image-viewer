//! INI section identities and the legacy alias table that migrates old keys.

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

/// Legacy `config.ini` section: gates which alias spellings are accepted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IniSection {
    Settings,
    Video,
    Performance,
    Shortcuts,
    State,
    Other,
}

impl IniSection {
    pub(crate) fn of(name: &str) -> Self {
        let name = name.trim();
        if name.eq_ignore_ascii_case("shortcuts") {
            Self::Shortcuts
        } else if name.eq_ignore_ascii_case("settings") {
            Self::Settings
        } else if name.eq_ignore_ascii_case("video") {
            Self::Video
        } else if name.eq_ignore_ascii_case("quality")
            || name.eq_ignore_ascii_case("performance")
            || name.eq_ignore_ascii_case("image_quality")
            || name.eq_ignore_ascii_case("filters")
        {
            Self::Performance
        } else if name.eq_ignore_ascii_case("state") || name.eq_ignore_ascii_case("video_state") {
            Self::State
        } else {
            Self::Other
        }
    }
}

pub(crate) fn alias_target(section: IniSection, key: &str) -> Option<&'static str> {
    INI_ALIASES
        .iter()
        .find(|(sec, keys, _)| *sec == section && keys.iter().any(|alias| *alias == key))
        .map(|(_, _, target)| *target)
}

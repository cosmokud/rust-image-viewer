//! Config/state file names and where they live.

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

#[cfg(test)]
pub(crate) const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../../assets/presets/default.ini");

pub(crate) const CONFIG_FILE_NAME: &str = "config.ini";

pub(crate) const STATE_FILE_NAME: &str = "state.ini";

/// Name of the auto-created working preset (a byte copy of `default.ini`).
pub(crate) const USER_PRESET_NAME: &str = "user";

pub(crate) const LEGACY_CONFIG_FILE_NAME: &str = "rust-image-viewer-config.ini";

pub(crate) const LEGACY_SETTINGS_FILE_NAME: &str = "setting.ini";

#[cfg(test)]
pub(crate) fn default_config_ini() -> &'static str {
    DEFAULT_CONFIG_TEMPLATE
}

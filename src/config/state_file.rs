//! `state.ini`, which holds the non-user-facing resume state.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use crate::settings_registry::{self, SETTINGS};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Parsed `state.ini`: runtime state plus the remembered active preset name.
#[derive(Default)]
pub(crate) struct StateFile {
    pub(crate) active_preset: Option<String>,
    pub(crate) muted: Option<bool>,
    pub(crate) volume: Option<f64>,
    pub(crate) show_breadcrumb_bar: Option<bool>,
}

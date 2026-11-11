//! `state.ini`, which holds the non-user-facing resume state.

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

/// Parsed `state.ini`: runtime state plus the remembered active preset name.
#[derive(Default)]
pub(crate) struct StateFile {
    pub(crate) active_preset: Option<String>,
    pub(crate) muted: Option<bool>,
    pub(crate) volume: Option<f64>,
    pub(crate) show_breadcrumb_bar: Option<bool>,
}

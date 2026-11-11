//! Module map for the split of `settings_registry.rs`.
//!
//! Every editable setting is declared once in [`table`] (`SETTINGS`) and read or
//! written generically through [`accessors`], which is what keeps `config.ini`
//! and the Settings window from drifting apart.
//!
//! | Concern | Where |
//! | --- | --- |
//! | Tabs, kinds, controls, values, one setting's metadata | [`types`] |
//! | The `SETTINGS` table (keys, defaults, help text, ranges) | [`table`] |
//! | Generic `value_of` / `set_*` against a `Config` | [`accessors`] |
//! | Public registry API (`get`, `set`, `capture`, `apply_values`) | [`api`] |
//! | Value formatting, section names, cross-setting guards | [`text`] |
//! | Preset files (render/parse/rewrite, built-in + user lifecycle) | [`presets`] |

#![allow(unused_imports)]

pub(crate) mod accessors;
pub(crate) mod api;
pub(crate) mod guards;
pub(crate) mod presets;
pub(crate) mod table;
pub(crate) mod tests;
pub(crate) mod text;
pub(crate) mod types;

pub(crate) use accessors::*;
pub(crate) use api::*;
pub(crate) use guards::*;
pub(crate) use presets::*;
pub(crate) use table::*;
pub(crate) use tests::*;
pub(crate) use text::*;
pub(crate) use types::*;

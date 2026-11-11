//! Module map for the split of `config.rs`.
//!
//! One `Config` struct ([`config_struct`]) is the app's whole persisted state.
//! The INI reader ([`parse_ini`]) maps every accepted key through the alias
//! table ([`ini_aliases`]), so the on-disk format and the Rust fields can be
//! renamed independently.
//!
//! | Concern | Where |
//! | --- | --- |
//! | File names / locations | [`paths`] |
//! | INI sections and legacy alias table | [`ini_sections`], [`ini_aliases`] |
//! | Resume state file | [`state_file`] |
//! | Public vocabulary (filters, seek policy, modes) | [`types`] |
//! | Action + input binding model | [`actions`], [`bindings`], [`keys`] |
//! | The `Config` struct | [`config_struct`] |
//! | Defaults and legacy migrations | [`defaults`] |
//! | Load/save and the state round-trip | [`io`], [`parse_ini`] |
//! | Binding text round-trip, value parsers | [`binding_text`], [`ini_text`] |

#![allow(unused_imports)]

pub(crate) mod actions;
pub(crate) mod binding_text;
pub(crate) mod bindings;
pub(crate) mod config_struct;
pub(crate) mod defaults;
pub(crate) mod ini_aliases;
pub(crate) mod ini_sections;
pub(crate) mod ini_text;
pub(crate) mod io;
pub(crate) mod keys;
pub(crate) mod parse_ini;
pub(crate) mod paths;
pub(crate) mod state_file;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod types;

pub(crate) use actions::*;
pub(crate) use binding_text::*;
pub(crate) use bindings::*;
pub(crate) use config_struct::*;
pub(crate) use defaults::*;
pub(crate) use ini_aliases::*;
pub(crate) use ini_sections::*;
pub(crate) use ini_text::*;
pub(crate) use io::*;
pub(crate) use keys::*;
pub(crate) use parse_ini::*;
pub(crate) use paths::*;
pub(crate) use state_file::*;
pub(crate) use types::*;

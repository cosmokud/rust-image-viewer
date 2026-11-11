//! File-level operations (menus, marks, rename, clipboard, delete).

#![allow(unused_imports)]

pub(crate) mod clipboard;
pub(crate) mod context_menu;
pub(crate) mod delete;
pub(crate) mod location;
pub(crate) mod mark_selection;
pub(crate) mod marks;
pub(crate) mod menu;
pub(crate) mod rename;
pub(crate) mod shortcuts;
pub(crate) mod size;

pub(crate) use clipboard::*;
pub(crate) use context_menu::*;
pub(crate) use delete::*;
pub(crate) use location::*;
pub(crate) use mark_selection::*;
pub(crate) use marks::*;
pub(crate) use menu::*;
pub(crate) use rename::*;
pub(crate) use shortcuts::*;
pub(crate) use size::*;

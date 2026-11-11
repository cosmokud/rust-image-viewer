//! File-level operations (menus, marks, rename, clipboard, delete).

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

#[allow(unused_imports)]
pub(crate) use clipboard::*;
#[allow(unused_imports)]
pub(crate) use context_menu::*;
#[allow(unused_imports)]
pub(crate) use delete::*;
#[allow(unused_imports)]
pub(crate) use location::*;
#[allow(unused_imports)]
pub(crate) use mark_selection::*;
#[allow(unused_imports)]
pub(crate) use marks::*;
#[allow(unused_imports)]
pub(crate) use menu::*;
#[allow(unused_imports)]
pub(crate) use rename::*;
#[allow(unused_imports)]
pub(crate) use shortcuts::*;
#[allow(unused_imports)]
pub(crate) use size::*;

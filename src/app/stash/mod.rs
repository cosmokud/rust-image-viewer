//! Stash dialog: every marked file in one gallery, opened from the title-bar menu.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) mod draw;
pub(crate) mod menu;
pub(crate) mod nav;
pub(crate) mod open;
pub(crate) mod preview;
pub(crate) mod selection;
pub(crate) mod store;

pub(crate) use draw::*;
pub(crate) use menu::*;
pub(crate) use nav::*;
pub(crate) use open::*;
pub(crate) use preview::*;
pub(crate) use selection::*;
pub(crate) use store::*;

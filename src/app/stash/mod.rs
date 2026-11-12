//! Stash dialog: every marked file in one gallery, opened from the title-bar menu.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) mod draw;
pub(crate) mod open;
pub(crate) mod preview;

pub(crate) use draw::*;
pub(crate) use open::*;
pub(crate) use preview::*;

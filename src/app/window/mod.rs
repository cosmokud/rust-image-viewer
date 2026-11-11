//! Window geometry, displays, fullscreen and dragging.

#![allow(unused_imports)]

pub(crate) mod autosize;
pub(crate) mod display;
pub(crate) mod drag;
pub(crate) mod fullscreen;
pub(crate) mod layout;
pub(crate) mod layout_apply;
pub(crate) mod monitor;
pub(crate) mod rects;
pub(crate) mod startup;
pub(crate) mod title;

pub(crate) use autosize::*;
pub(crate) use display::*;
pub(crate) use drag::*;
pub(crate) use fullscreen::*;
pub(crate) use layout::*;
pub(crate) use layout_apply::*;
pub(crate) use monitor::*;
pub(crate) use rects::*;
pub(crate) use startup::*;
pub(crate) use title::*;

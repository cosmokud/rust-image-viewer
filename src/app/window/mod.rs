//! Window geometry, displays, fullscreen and dragging.

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

#[allow(unused_imports)]
pub(crate) use autosize::*;
#[allow(unused_imports)]
pub(crate) use display::*;
#[allow(unused_imports)]
pub(crate) use drag::*;
#[allow(unused_imports)]
pub(crate) use fullscreen::*;
#[allow(unused_imports)]
pub(crate) use layout::*;
#[allow(unused_imports)]
pub(crate) use layout_apply::*;
#[allow(unused_imports)]
pub(crate) use monitor::*;
#[allow(unused_imports)]
pub(crate) use rects::*;
#[allow(unused_imports)]
pub(crate) use startup::*;
#[allow(unused_imports)]
pub(crate) use title::*;

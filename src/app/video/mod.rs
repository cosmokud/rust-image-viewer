//! Video playback, tracks, icons, resume and seek UI.

pub(crate) mod controls;
pub(crate) mod first_frame;
pub(crate) mod icons;
pub(crate) mod nav;
pub(crate) mod resume;
pub(crate) mod seek_mode;
pub(crate) mod seekbar;
pub(crate) mod state;
pub(crate) mod tracks;

#[allow(unused_imports)]
pub(crate) use controls::*;
#[allow(unused_imports)]
pub(crate) use first_frame::*;
#[allow(unused_imports)]
pub(crate) use icons::*;
#[allow(unused_imports)]
pub(crate) use nav::*;
#[allow(unused_imports)]
pub(crate) use resume::*;
#[allow(unused_imports)]
pub(crate) use seek_mode::*;
#[allow(unused_imports)]
pub(crate) use seekbar::*;
#[allow(unused_imports)]
pub(crate) use state::*;
#[allow(unused_imports)]
pub(crate) use tracks::*;

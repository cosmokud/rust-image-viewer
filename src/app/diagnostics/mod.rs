//! FPS/diag overlays and per-frame stats.

pub(crate) mod fps;
pub(crate) mod frame_stats;
pub(crate) mod overlay;

#[allow(unused_imports)]
pub(crate) use fps::*;
#[allow(unused_imports)]
pub(crate) use frame_stats::*;
#[allow(unused_imports)]
pub(crate) use overlay::*;

//! Single-media loading and displayed-size maths.

pub(crate) mod animated;
pub(crate) mod dims;
pub(crate) mod gif_seekbar;
pub(crate) mod load;
pub(crate) mod solo_load;
pub(crate) mod swap;
pub(crate) mod webp_fps;

#[allow(unused_imports)]
pub(crate) use animated::*;
#[allow(unused_imports)]
pub(crate) use dims::*;
#[allow(unused_imports)]
pub(crate) use gif_seekbar::*;
#[allow(unused_imports)]
pub(crate) use load::*;
#[allow(unused_imports)]
pub(crate) use solo_load::*;
#[allow(unused_imports)]
pub(crate) use swap::*;
#[allow(unused_imports)]
pub(crate) use webp_fps::*;

//! Module map for the split of `image_loader.rs`.
//!
//! `LoadedImage` ([`loaded`] / [`types`]) is the decoded payload every consumer
//! shares; the rest of the module is the decode path that produces it.
//!
//! | Concern | Where |
//! | --- | --- |
//! | Decode allocation budget | [`budget`] |
//! | Extension sets and decode constants | [`constants`] |
//! | Reader abstraction over files | [`reader`] |
//! | Static decode, `zune-image` vs `image` crate | [`zune`], [`image_decode`], [`decode_static`] |
//! | Header-only dimension probing | [`probe`] |
//! | Extension helpers and `.lnk` folder shortcuts | [`extensions`], [`shortcuts`] |
//! | Media type and extension tables | [`types`] |
//! | Directory enumeration | [`directory`] |
//! | Loading entry points | [`constructors`], [`load`] |
//! | On-screen frame, display size, rotation | [`screen_frame`], [`display`], [`rotation`], [`rotate`] |
//! | GIF decode and disposal | [`gif_decode`] |
//! | Animated WebP (streamed) | [`webp_stream`], [`webp_frames`] |
//! | The animation interface both share | [`animation`] |

pub(crate) mod animation;
pub(crate) mod budget;
pub(crate) mod constants;
pub(crate) mod constructors;
pub(crate) mod decode_static;
pub(crate) mod directory;
pub(crate) mod display;
pub(crate) mod extensions;
pub(crate) mod gif_decode;
pub(crate) mod image_decode;
pub(crate) mod load;
pub(crate) mod loaded;
pub(crate) mod probe;
pub(crate) mod reader;
pub(crate) mod rotate;
pub(crate) mod rotation;
pub(crate) mod screen_frame;
pub(crate) mod shortcuts;
pub(crate) mod sorting;
pub(crate) mod tests;
pub(crate) mod types;
pub(crate) mod webp_frames;
pub(crate) mod webp_stream;
pub(crate) mod zune;

#[allow(unused_imports)]
pub(crate) use animation::*;
#[allow(unused_imports)]
pub(crate) use budget::*;
#[allow(unused_imports)]
pub(crate) use constants::*;
#[allow(unused_imports)]
pub(crate) use constructors::*;
#[allow(unused_imports)]
pub(crate) use decode_static::*;
#[allow(unused_imports)]
pub(crate) use directory::*;
#[allow(unused_imports)]
pub(crate) use display::*;
#[allow(unused_imports)]
pub(crate) use extensions::*;
#[allow(unused_imports)]
pub(crate) use gif_decode::*;
#[allow(unused_imports)]
pub(crate) use image_decode::*;
#[allow(unused_imports)]
pub(crate) use load::*;
#[allow(unused_imports)]
pub(crate) use loaded::*;
#[allow(unused_imports)]
pub(crate) use probe::*;
#[allow(unused_imports)]
pub(crate) use reader::*;
#[allow(unused_imports)]
pub(crate) use rotate::*;
#[allow(unused_imports)]
pub(crate) use rotation::*;
#[allow(unused_imports)]
pub(crate) use screen_frame::*;
#[allow(unused_imports)]
pub(crate) use shortcuts::*;
#[allow(unused_imports)]
pub(crate) use sorting::*;
#[allow(unused_imports)]
pub(crate) use tests::*;
#[allow(unused_imports)]
pub(crate) use types::*;
#[allow(unused_imports)]
pub(crate) use webp_frames::*;
#[allow(unused_imports)]
pub(crate) use webp_stream::*;
#[allow(unused_imports)]
pub(crate) use zune::*;

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

#![allow(unused_imports)]

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

pub(crate) use animation::*;
pub(crate) use budget::*;
pub(crate) use constants::*;
pub(crate) use constructors::*;
pub(crate) use decode_static::*;
pub(crate) use directory::*;
pub(crate) use display::*;
pub(crate) use extensions::*;
pub(crate) use gif_decode::*;
pub(crate) use image_decode::*;
pub(crate) use load::*;
pub(crate) use loaded::*;
pub(crate) use probe::*;
pub(crate) use reader::*;
pub(crate) use rotate::*;
pub(crate) use rotation::*;
pub(crate) use screen_frame::*;
pub(crate) use shortcuts::*;
pub(crate) use sorting::*;
pub(crate) use tests::*;
pub(crate) use types::*;
pub(crate) use webp_frames::*;
pub(crate) use webp_stream::*;
pub(crate) use zune::*;

//! Module map for the split of `video_player.rs`.
//!
//! `VideoPlayer` ([`player`]) is one GStreamer pipeline driven from the UI
//! thread through a small control queue ([`control`]); everything that decides
//! *which* elements go into it lives on its own.
//!
//! | Concern | Where |
//! | --- | --- |
//! | Windows GStreamer environment discovery | [`env`] |
//! | Decoder preference/rank policy, demotions, candidates | [`decoders`], [`demotions`], [`candidates`] |
//! | VP8/VP9 alpha handling and the opaque fallback memo | [`alpha`] |
//! | Building the pipeline with fallback | [`create`] |
//! | Hardware-acceleration probes | [`caps`] |
//! | Public types (seek mode, tracks, subtitle selection, frames) | [`types`] |
//! | Zero-copy D3D11/D3D12 frame extraction | [`d3d11`], [`d3d12`] |
//! | The shared stream state written from bus/appsink callbacks | [`state`] |
//! | Bounded GStreamer calls and the control thread | [`gst_threads`] |
//! | playbin property helpers, limited-range expansion | [`properties`], [`limited_range`] |
//! | Track labels and subtitle font fallbacks | [`labels`] |
//! | Player construction | [`player_init`] |
//! | Transport (play/pause, position, duration) | [`player_transport`] |
//! | Seeking | [`player_seek`] |
//! | Volume/mute | [`player_volume`] |
//! | Track selection | [`player_tracks`] |
//! | Frames, EOS, stall detection, shutdown | [`player_frames`] |
//! | Bus error draining | [`bus_errors`] |

#![allow(unused_imports)]

pub(crate) mod alpha;
pub(crate) mod bus_errors;
pub(crate) mod candidates;
pub(crate) mod caps;
pub(crate) mod control;
pub(crate) mod create;
pub(crate) mod d3d11;
pub(crate) mod d3d12;
pub(crate) mod decoders;
pub(crate) mod demotions;
pub(crate) mod env;
pub(crate) mod format;
pub(crate) mod gst_threads;
pub(crate) mod labels;
pub(crate) mod limited_range;
pub(crate) mod player;
pub(crate) mod player_frames;
pub(crate) mod player_init;
pub(crate) mod player_seek;
pub(crate) mod player_tracks;
pub(crate) mod player_transport;
pub(crate) mod player_volume;
pub(crate) mod properties;
pub(crate) mod state;
pub(crate) mod tests;
pub(crate) mod types;

pub(crate) use alpha::*;
pub(crate) use bus_errors::*;
pub(crate) use candidates::*;
pub(crate) use caps::*;
pub(crate) use control::*;
pub(crate) use create::*;
pub(crate) use d3d11::*;
pub(crate) use d3d12::*;
pub(crate) use decoders::*;
pub(crate) use demotions::*;
pub(crate) use env::*;
pub(crate) use format::*;
pub(crate) use gst_threads::*;
pub(crate) use labels::*;
pub(crate) use limited_range::*;
pub(crate) use player::*;
pub(crate) use player_frames::*;
pub(crate) use player_init::*;
pub(crate) use player_seek::*;
pub(crate) use player_tracks::*;
pub(crate) use player_transport::*;
pub(crate) use player_volume::*;
pub(crate) use properties::*;
pub(crate) use state::*;
pub(crate) use tests::*;
pub(crate) use types::*;

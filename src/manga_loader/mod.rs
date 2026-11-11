//! Module map for the split of `manga_loader.rs`.
//!
//! `MangaLoader` owns the background pipeline for Long Strip and Masonry:
//! dimension probing, prioritized decode scheduling, LOD bookkeeping, retry and
//! revive policy, and the uploaded-texture cache. The pipeline is split by
//! concern; `MangaLoader` itself is declared in [`loader`] and its inherent
//! methods live in the child modules named below.
//!
//! | Concern | Where |
//! | --- | --- |
//! | Video-probe permits and per-index cooldown | [`probe_gate`] |
//! | Capacity/limits constants | [`limits`], [`budget`], [`constants`] |
//! | Retry, probe-blocked and revive state | [`states`] |
//! | Dimension probing helpers | [`probe_dims`] |
//! | Diag tracing | [`trace`] |
//! | Data types handed to/from the UI | [`types`], [`demand_types`] |
//! | The loader struct and its channels | [`loader`] |
//! | Construction / teardown | [`ctor`] |
//! | Dimension queueing and warm-up | [`dim_base`] |
//! | Read-only queries and demand accounting | [`queries`] |
//! | The coordinator thread | [`coordinator`] |
//! | Decode-failure retry bookkeeping | [`retry`] |
//! | Image/video decode of one tile | [`decode`] |
//! | Prefetch and demand window maths | [`prefetch`], [`demand`] |
//! | Draining results back to the UI | [`take`] |
//! | Clear/forget/cancel family | [`cache_control`] |
//! | Per-index backoff and bookkeeping | [`backoff`] |
//! | Video-thumbnail revive scheduler | [`revive`] |
//! | Visible-tile retries | [`retry_visible`] |
//! | Uploaded texture LRU | [`texture_cache`] |

#![allow(unused_imports)]

pub(crate) mod backoff;
pub(crate) mod budget;
pub(crate) mod cache_control;
pub(crate) mod constants;
pub(crate) mod coordinator;
pub(crate) mod ctor;
pub(crate) mod decode;
pub(crate) mod demand;
pub(crate) mod demand_types;
pub(crate) mod dim_base;
pub(crate) mod limits;
pub(crate) mod loader;
pub(crate) mod prefetch;
pub(crate) mod probe_dims;
pub(crate) mod probe_gate;
pub(crate) mod queries;
pub(crate) mod retry;
pub(crate) mod retry_visible;
pub(crate) mod revive;
pub(crate) mod states;
pub(crate) mod take;
pub(crate) mod tests;
pub(crate) mod texture_cache;
pub(crate) mod trace;
pub(crate) mod types;

pub(crate) use backoff::*;
pub(crate) use budget::*;
pub(crate) use cache_control::*;
pub(crate) use constants::*;
pub(crate) use coordinator::*;
pub(crate) use ctor::*;
pub(crate) use decode::*;
pub(crate) use demand::*;
pub(crate) use demand_types::*;
pub(crate) use dim_base::*;
pub(crate) use limits::*;
pub(crate) use loader::*;
pub(crate) use prefetch::*;
pub(crate) use probe_dims::*;
pub(crate) use probe_gate::*;
pub(crate) use queries::*;
pub(crate) use retry::*;
pub(crate) use retry_visible::*;
pub(crate) use revive::*;
pub(crate) use states::*;
pub(crate) use take::*;
pub(crate) use tests::*;
pub(crate) use texture_cache::*;
pub(crate) use trace::*;
pub(crate) use types::*;

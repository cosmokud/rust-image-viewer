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

#[allow(unused_imports)]
pub(crate) use backoff::*;
#[allow(unused_imports)]
pub(crate) use budget::*;
#[allow(unused_imports)]
pub(crate) use cache_control::*;
#[allow(unused_imports)]
pub(crate) use constants::*;
#[allow(unused_imports)]
pub(crate) use coordinator::*;
#[allow(unused_imports)]
pub(crate) use ctor::*;
#[allow(unused_imports)]
pub(crate) use decode::*;
#[allow(unused_imports)]
pub(crate) use demand::*;
#[allow(unused_imports)]
pub(crate) use demand_types::*;
#[allow(unused_imports)]
pub(crate) use dim_base::*;
#[allow(unused_imports)]
pub(crate) use limits::*;
#[allow(unused_imports)]
pub(crate) use loader::*;
#[allow(unused_imports)]
pub(crate) use prefetch::*;
#[allow(unused_imports)]
pub(crate) use probe_dims::*;
#[allow(unused_imports)]
pub(crate) use probe_gate::*;
#[allow(unused_imports)]
pub(crate) use queries::*;
#[allow(unused_imports)]
pub(crate) use retry::*;
#[allow(unused_imports)]
pub(crate) use retry_visible::*;
#[allow(unused_imports)]
pub(crate) use revive::*;
#[allow(unused_imports)]
pub(crate) use states::*;
#[allow(unused_imports)]
pub(crate) use take::*;
#[allow(unused_imports)]
pub(crate) use tests::*;
#[allow(unused_imports)]
pub(crate) use texture_cache::*;
#[allow(unused_imports)]
pub(crate) use trace::*;
#[allow(unused_imports)]
pub(crate) use types::*;

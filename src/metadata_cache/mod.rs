//! Module map for the split of `metadata_cache.rs`.
//!
//! The cache is a `redb` file plus an in-RAM mirror. Lookups hit the mirror and
//! only fall back to disk; writes are queued and committed by one background
//! writer, which is what keeps the UI frame path free of file I/O.
//!
//! | Concern | Where |
//! | --- | --- |
//! | Limits and capacity constants | [`constants`] |
//! | Process switches/counters | [`flags`] |
//! | Public types and stats | [`types`] |
//! | The on-disk file-type code table | [`file_type`] |
//! | Record / write-op types | [`records`] |
//! | In-memory mirror, batching, pruning | [`cache`] |
//! | Global handle and writer thread | [`global`], [`writer`] |
//! | Shutdown handshake | [`shutdown`] |
//! | Module API used by the app | [`api`] |
//! | Stats for diagnostics | [`stats`] |
//! | Size-limited backend, schema checks | [`backend`], [`open`] |
//! | Header-based type/animation detection | [`detect`] |
//! | Record encoding | [`encode`] |

#![allow(unused_imports)]

pub(crate) mod api;
pub(crate) mod backend;
pub(crate) mod cache;
pub(crate) mod constants;
pub(crate) mod detect;
pub(crate) mod encode;
pub(crate) mod file_type;
pub(crate) mod flags;
pub(crate) mod global;
pub(crate) mod open;
pub(crate) mod records;
pub(crate) mod shutdown;
pub(crate) mod stats;
pub(crate) mod test_hooks;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod time;
pub(crate) mod types;
pub(crate) mod writer;

pub(crate) use api::*;
pub(crate) use backend::*;
pub(crate) use cache::*;
pub(crate) use constants::*;
pub(crate) use detect::*;
pub(crate) use encode::*;
pub(crate) use file_type::*;
pub(crate) use flags::*;
pub(crate) use global::*;
pub(crate) use open::*;
pub(crate) use records::*;
pub(crate) use shutdown::*;
pub(crate) use stats::*;
pub(crate) use test_hooks::*;
pub(crate) use time::*;
pub(crate) use types::*;
pub(crate) use writer::*;

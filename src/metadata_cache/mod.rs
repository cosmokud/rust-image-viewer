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

#[allow(unused_imports)]
pub(crate) use api::*;
#[allow(unused_imports)]
pub(crate) use backend::*;
#[allow(unused_imports)]
pub(crate) use cache::*;
#[allow(unused_imports)]
pub(crate) use constants::*;
#[allow(unused_imports)]
pub(crate) use detect::*;
#[allow(unused_imports)]
pub(crate) use encode::*;
#[allow(unused_imports)]
pub(crate) use file_type::*;
#[allow(unused_imports)]
pub(crate) use flags::*;
#[allow(unused_imports)]
pub(crate) use global::*;
#[allow(unused_imports)]
pub(crate) use open::*;
#[allow(unused_imports)]
pub(crate) use records::*;
#[allow(unused_imports)]
pub(crate) use shutdown::*;
#[allow(unused_imports)]
pub(crate) use stats::*;
#[allow(unused_imports)]
pub(crate) use test_hooks::*;
#[allow(unused_imports)]
pub(crate) use time::*;
#[allow(unused_imports)]
pub(crate) use types::*;
#[allow(unused_imports)]
pub(crate) use writer::*;

//! The stored record, the write-op enum and a pending dimension.

#[allow(unused_imports)]
use super::*;

#[allow(unused_imports)]
use crate::app_dirs;
#[allow(unused_imports)]
use parking_lot::RwLock;
#[allow(unused_imports)]
use redb::backends::FileBackend;
#[allow(unused_imports)]
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition, TableHandle};
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::fs::{File, OpenOptions};
#[allow(unused_imports)]
use std::io::{self, Read};
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::sync::{Arc, OnceLock};
#[allow(unused_imports)]
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CachedRecord {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) file_type: CachedFileType,
    pub(crate) animated: bool,
}

pub(crate) enum CacheWriteOp {
    // Resolution to a durable record (file type, animated) is deferred to the writer: those
    // reads open the media file, and a store can be called from the frame loop.
    Dimensions(PendingDimension),
    Shutdown,
}

/// A dimension store awaiting resolution. Carries no disk-derived facts so that
/// [`store_cached_dimensions`] never opens a file on the caller's thread.
pub(crate) struct PendingDimension {
    pub(crate) key: String,
    pub(crate) path: PathBuf,
    pub(crate) media_kind: CachedMediaKind,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

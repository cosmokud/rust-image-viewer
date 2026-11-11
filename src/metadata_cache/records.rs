//! The stored record, the write-op enum and a pending dimension.

#![allow(unused_imports)]

use super::*;

use crate::app_dirs;
use parking_lot::RwLock;
use redb::backends::FileBackend;
use redb::{Database, DatabaseError, ReadableTable, StorageBackend, TableDefinition, TableHandle};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
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

//! Public types: `CachedMediaKind` and the stats struct.

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

#[derive(Debug, Clone, Copy, Default)]
pub struct MetadataCacheStats {
    pub dimension_hits: u64,
    pub dimension_misses: u64,
    pub dimension_expired: u64,
    pub dimension_evicted: u64,
    /// Writes dropped because the bounded write queue was full (recomputed on next miss).
    pub dimension_write_drops: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CachedMediaKind {
    Image,
    Video,
}

impl CachedMediaKind {
    pub(crate) fn matches_file_type(self, file_type: CachedFileType) -> bool {
        file_type.media_kind() == self
    }
}

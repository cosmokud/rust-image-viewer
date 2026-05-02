//! Persistent folder-travel position cache for manga long-strip and masonry modes.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use redb::backends::FileBackend;
use redb::{Database, DatabaseError, StorageBackend, TableDefinition};

use crate::app_dirs;

const FOLDER_TRAVEL_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("folder_travel_positions");
const CACHE_FILE_NAME: &str = "folder_travel_cache.redb";
const CACHE_SCHEMA_VERSION: u8 = 1;
const FOLDER_TRAVEL_CACHE_DEFAULT_MAX_SIZE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderTravelLayoutMode {
    LongStrip,
    Masonry,
}

impl FolderTravelLayoutMode {
    fn key_suffix(self) -> &'static str {
        match self {
            FolderTravelLayoutMode::LongStrip => "long_strip",
            FolderTravelLayoutMode::Masonry => "masonry",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FolderTravelPosition {
    pub current_path: PathBuf,
    pub current_index: usize,
    pub scroll_offset: f32,
}

struct FolderTravelCache {
    db: Database,
}

impl FolderTravelCache {
    fn open_default() -> Option<Self> {
        let path = default_cache_path()?;
        let db = open_database_with_size_limit(
            path.as_path(),
            FOLDER_TRAVEL_CACHE_DEFAULT_MAX_SIZE_BYTES,
        )?;

        Some(Self { db })
    }

    fn lookup(
        &self,
        directory: &Path,
        layout_mode: FolderTravelLayoutMode,
    ) -> Option<FolderTravelPosition> {
        let key = folder_travel_key(directory, layout_mode)?;

        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(FOLDER_TRAVEL_TABLE).ok()?;
        let raw = table.get(key.as_str()).ok()??;
        decode_position_record(raw.value())
    }

    fn store(
        &mut self,
        directory: &Path,
        layout_mode: FolderTravelLayoutMode,
        position: &FolderTravelPosition,
    ) {
        let Some(key) = folder_travel_key(directory, layout_mode) else {
            return;
        };
        let Some(encoded) = encode_position_record(position) else {
            return;
        };

        let Ok(write_txn) = self.db.begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(FOLDER_TRAVEL_TABLE) else {
                return;
            };

            if table.insert(key.as_str(), encoded.as_slice()).is_err() {
                return;
            }
        }

        let _ = write_txn.commit();
    }
}

static GLOBAL_FOLDER_TRAVEL_CACHE: OnceLock<Option<Arc<Mutex<FolderTravelCache>>>> =
    OnceLock::new();

fn global_folder_travel_cache_handle() -> Option<&'static Arc<Mutex<FolderTravelCache>>> {
    GLOBAL_FOLDER_TRAVEL_CACHE
        .get_or_init(|| FolderTravelCache::open_default().map(|cache| Arc::new(Mutex::new(cache))))
        .as_ref()
}

pub fn lookup_folder_travel_position(
    directory: &Path,
    layout_mode: FolderTravelLayoutMode,
) -> Option<FolderTravelPosition> {
    let Some(cache) = global_folder_travel_cache_handle() else {
        return None;
    };

    cache.lock().lookup(directory, layout_mode)
}

pub fn store_folder_travel_position(
    directory: &Path,
    layout_mode: FolderTravelLayoutMode,
    position: &FolderTravelPosition,
) {
    let Some(cache) = global_folder_travel_cache_handle() else {
        return;
    };

    cache.lock().store(directory, layout_mode, position);
}

fn folder_travel_key(directory: &Path, layout_mode: FolderTravelLayoutMode) -> Option<String> {
    let normalized = normalize_path_key(directory)?;
    Some(format!("{}#{}", normalized, layout_mode.key_suffix()))
}

fn encode_position_record(position: &FolderTravelPosition) -> Option<Vec<u8>> {
    let normalized_path = normalize_path_key(position.current_path.as_path())?;
    let path_bytes = normalized_path.as_bytes();
    let path_len = u32::try_from(path_bytes.len()).ok()?;
    let index = u64::try_from(position.current_index).ok()?;

    let mut encoded = Vec::with_capacity(1 + 8 + 4 + 4 + path_bytes.len());
    encoded.push(CACHE_SCHEMA_VERSION);
    encoded.extend_from_slice(&index.to_le_bytes());
    encoded.extend_from_slice(&position.scroll_offset.max(0.0).to_le_bytes());
    encoded.extend_from_slice(&path_len.to_le_bytes());
    encoded.extend_from_slice(path_bytes);
    Some(encoded)
}

fn decode_position_record(raw: &[u8]) -> Option<FolderTravelPosition> {
    if raw.len() < 17 {
        return None;
    }

    if raw[0] != CACHE_SCHEMA_VERSION {
        return None;
    }

    let index = u64::from_le_bytes(raw.get(1..9)?.try_into().ok()?);
    let scroll_offset = f32::from_le_bytes(raw.get(9..13)?.try_into().ok()?);
    let path_len = u32::from_le_bytes(raw.get(13..17)?.try_into().ok()?) as usize;

    if raw.len() != 17 + path_len {
        return None;
    }

    let path_bytes = raw.get(17..17 + path_len)?;
    let path = std::str::from_utf8(path_bytes).ok()?;

    Some(FolderTravelPosition {
        current_path: PathBuf::from(path),
        current_index: usize::try_from(index).ok()?,
        scroll_offset: if scroll_offset.is_finite() {
            scroll_offset.max(0.0)
        } else {
            0.0
        },
    })
}

fn default_cache_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(base_dir) = app_dirs::app_local_data_dir() {
            if std::fs::create_dir_all(&base_dir).is_ok() {
                return Some(base_dir.join(CACHE_FILE_NAME));
            }
        }
    }

    let base_dir = std::env::temp_dir().join(app_dirs::APP_DIR_NAME);
    if std::fs::create_dir_all(&base_dir).is_ok() {
        return Some(base_dir.join(CACHE_FILE_NAME));
    }

    None
}

fn normalize_path_key(path: &Path) -> Option<String> {
    let normalized_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        path.canonicalize()
            .ok()
            .unwrap_or_else(|| path.to_path_buf())
    };

    let key = normalized_path.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        if key.is_empty() {
            return None;
        }
        Some(key.to_lowercase())
    }

    #[cfg(not(target_os = "windows"))]
    {
        if key.is_empty() {
            return None;
        }
        Some(key)
    }
}

fn io_other_error(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message)
}

#[derive(Debug)]
struct SizeLimitedFileBackend {
    inner: FileBackend,
    max_size_bytes: u64,
    current_len: AtomicU64,
}

impl SizeLimitedFileBackend {
    fn new(inner: FileBackend, max_size_bytes: u64, current_len: u64) -> Self {
        Self {
            inner,
            max_size_bytes,
            current_len: AtomicU64::new(current_len),
        }
    }

    fn exceeds_limit(&self, required_len: u64) -> bool {
        self.max_size_bytes > 0 && required_len > self.max_size_bytes
    }
}

impl StorageBackend for SizeLimitedFileBackend {
    fn len(&self) -> std::result::Result<u64, io::Error> {
        let actual_len = self.inner.len()?;
        self.current_len.store(actual_len, Ordering::Relaxed);
        Ok(actual_len)
    }

    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, io::Error> {
        self.inner.read(offset, len)
    }

    fn set_len(&self, len: u64) -> std::result::Result<(), io::Error> {
        if self.exceeds_limit(len) {
            return Err(io_other_error("folder travel cache size limit reached"));
        }

        self.inner.set_len(len)?;
        self.current_len.store(len, Ordering::Relaxed);
        Ok(())
    }

    fn sync_data(&self, eventual: bool) -> std::result::Result<(), io::Error> {
        self.inner.sync_data(eventual)
    }

    fn write(&self, offset: u64, data: &[u8]) -> std::result::Result<(), io::Error> {
        let write_end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| io_other_error("folder travel cache size overflow"))?;
        let tracked_len = self.current_len.load(Ordering::Relaxed);
        let required_len = tracked_len.max(write_end);

        if self.exceeds_limit(required_len) {
            return Err(io_other_error("folder travel cache size limit reached"));
        }

        self.inner.write(offset, data)?;
        if required_len > tracked_len {
            self.current_len.store(required_len, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn open_database_with_size_limit(path: &Path, max_size_bytes: u64) -> Option<Database> {
    if max_size_bytes > 0 {
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > max_size_bytes {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()?;
    let current_len = file.metadata().ok().map(|m| m.len()).unwrap_or(0);

    let base_backend = FileBackend::new(file).ok()?;
    let limited_backend = SizeLimitedFileBackend::new(base_backend, max_size_bytes, current_len);

    match Database::builder().create_with_backend(limited_backend) {
        Ok(db) => Some(db),
        Err(DatabaseError::Storage(redb::StorageError::Corrupted(_))) if path.exists() => {
            let _ = std::fs::remove_file(path);
            let recreated_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .ok()?;
            let recreated_len = recreated_file.metadata().ok().map(|m| m.len()).unwrap_or(0);
            let recreated_backend = FileBackend::new(recreated_file).ok()?;
            let limited_backend =
                SizeLimitedFileBackend::new(recreated_backend, max_size_bytes, recreated_len);
            Database::builder()
                .create_with_backend(limited_backend)
                .ok()
        }
        Err(_) => None,
    }
}

//! `MetadataCache`: the in-memory mirror, batch apply and pruning.

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

pub struct MetadataCache {
    // Readers share this handle and never take the writer's path: redb read transactions run
    // concurrently with writes, so reads used to block only on the lock the writer held. The
    // inner `Arc` is cloned out under the lock and released first, so no I/O runs under it.
    pub(crate) db: RwLock<Arc<Database>>,
    pub(crate) cache_path: PathBuf,
    // Backend-tracked file length: the write-side size gate without a per-write stat.
    pub(crate) file_len: Arc<AtomicU64>,
    pub(crate) mirror: RwLock<HashMap<String, CachedRecord>>,
}

impl MetadataCache {
    pub fn open_default() -> Option<Self> {
        let path = default_cache_path()?;
        let max_size_bytes = metadata_cache_max_size_bytes();

        let (db, file_len) = open_database_with_size_limit(path.as_path(), max_size_bytes)?;

        Some(Self {
            db: RwLock::new(Arc::new(db)),
            cache_path: path,
            file_len,
            mirror: RwLock::new(HashMap::new()),
        })
    }

    pub fn lookup_dimensions(
        &self,
        path: &Path,
        expected_kind: CachedMediaKind,
    ) -> Option<(u32, u32)> {
        let key = crate::cache_key::key(path, crate::cache_key::KeyScope::Bare).unwrap_or_default();
        let record = self.mirror_record(key.as_str())?;

        if !expected_kind.matches_file_type(record.file_type) {
            return None;
        }

        if record.width == 0 || record.height == 0 {
            return None;
        }

        Some((record.width, record.height))
    }

    pub fn lookup_dimensions_batch(
        &self,
        items: &[(PathBuf, CachedMediaKind)],
    ) -> Vec<Option<(u32, u32)>> {
        let mut records: Vec<Option<CachedRecord>> = Vec::with_capacity(items.len());
        let mut misses: Vec<(usize, String)> = Vec::new();

        for (index, (path, _)) in items.iter().enumerate() {
            let key = crate::cache_key::key(path.as_path(), crate::cache_key::KeyScope::Bare)
                .unwrap_or_default();
            match self.mirror_peek(key.as_str()) {
                Some(record) => records.push(Some(record)),
                None => {
                    misses.push((index, key));
                    records.push(None);
                }
            }
        }

        if !misses.is_empty() {
            // One read transaction for the whole batch; mirror-only hits skip it entirely.
            if let Ok(read_txn) = self.db_clone().begin_read() {
                if let Ok(table) = read_txn.open_table(METADATA_TABLE) {
                    for (index, key) in misses {
                        let Some(record) = table
                            .get(key.as_str())
                            .ok()
                            .flatten()
                            .and_then(|raw| decode_record(raw.value()))
                        else {
                            continue;
                        };
                        self.insert_mirror(key, record);
                        records[index] = Some(record);
                    }
                }
            }
        }

        items
            .iter()
            .zip(records)
            .map(|((_, expected_kind), record)| {
                let record = record?;
                if !expected_kind.matches_file_type(record.file_type)
                    || record.width == 0
                    || record.height == 0
                {
                    return None;
                }
                Some((record.width, record.height))
            })
            .collect()
    }

    pub(crate) fn mirror_peek(&self, key: &str) -> Option<CachedRecord> {
        self.mirror.read().get(key).copied()
    }

    pub(crate) fn mirror_record(&self, key: &str) -> Option<CachedRecord> {
        if let Some(record) = self.mirror_peek(key) {
            return Some(record);
        }

        let record = self.read_record_from_db(key)?;
        self.insert_mirror(key.to_string(), record);
        Some(record)
    }

    pub(crate) fn db_clone(&self) -> Arc<Database> {
        self.db.read().clone()
    }

    pub(crate) fn read_record_from_db(&self, key: &str) -> Option<CachedRecord> {
        let read_txn = self.db_clone().begin_read().ok()?;
        let table = read_txn.open_table(METADATA_TABLE).ok()?;
        let raw = table.get(key).ok()??;
        decode_record(raw.value())
    }

    pub(crate) fn insert_mirror(&self, key: String, record: CachedRecord) {
        let mut mirror = self.mirror.write();
        if mirror.len() >= DIMENSION_MIRROR_MAX_ENTRIES && !mirror.contains_key(&key) {
            mirror.clear();
        }
        mirror.insert(key, record);
    }

    /// Writer thread only. One write transaction (and one fsync) per drained batch: the batch
    /// runs behind a queue, so per-record transactions are pure latency for everyone.
    /// Returns how many records were stored.
    pub(crate) fn apply_batch(&self, batch: &[(String, CachedRecord)]) -> usize {
        let max_size_bytes = metadata_cache_max_size_bytes();
        let mut estimated_write_bytes = 0_u64;
        let mut accepted: Vec<&(String, CachedRecord)> = Vec::with_capacity(batch.len());

        for entry in batch {
            if max_size_bytes > 0 {
                // Slack for btree page churn; the store is a hint, so dropping beats blocking.
                let bytes = (entry.0.len() as u64).saturating_add(RECORD_DISK_BYTES_ESTIMATE);
                let projected = self
                    .file_len
                    .load(Ordering::Relaxed)
                    .saturating_add(estimated_write_bytes)
                    .saturating_add(bytes);
                if projected > max_size_bytes {
                    continue;
                }
                estimated_write_bytes = estimated_write_bytes.saturating_add(bytes);
            }
            accepted.push(entry);
        }

        if accepted.is_empty() {
            return 0;
        }

        let Ok(write_txn) = self.db_clone().begin_write() else {
            return 0;
        };

        {
            let Ok(mut table) = write_txn.open_table(METADATA_TABLE) else {
                return 0;
            };

            for (key, record) in &accepted {
                let encoded = encode_record(*record);
                // One bad record must not abort the rest of the batch.
                let _ = table.insert(key.as_str(), encoded.as_str());
            }
        }

        if write_txn.commit().is_err() {
            return 0;
        }

        #[cfg(test)]
        CACHE_BATCH_COMMITS.fetch_add(1, Ordering::Relaxed);

        for (key, record) in &accepted {
            self.insert_mirror(key.clone(), *record);
        }

        accepted.len()
    }

    /// Idle-only (writer loop). Never inside a batch: `prune_dimension_table` rewrites the whole
    /// table and `compact` rewrites the file, which is what used to stall every reader.
    pub(crate) fn prune_if_due(&self) {
        let last_prune_secs = LAST_PRUNE_SECS.load(Ordering::Relaxed);
        let now_secs = unix_now_secs();
        let prune_due_to_interval = now_secs.saturating_sub(last_prune_secs) >= PRUNE_INTERVAL_SECS;
        let prune_due_to_size = self.cache_needs_size_prune();

        if !prune_due_to_interval && !prune_due_to_size {
            return;
        }

        LAST_PRUNE_SECS.store(now_secs, Ordering::Relaxed);

        self.prune_dimension_table();

        if prune_due_to_size {
            self.prune_to_size_limit();
        }

        self.refresh_file_len();
    }

    /// Idle-only: re-sync the atomic size estimate with the real file length.
    pub(crate) fn refresh_file_len(&self) {
        if let Ok(metadata) = std::fs::metadata(&self.cache_path) {
            self.file_len.store(metadata.len(), Ordering::Relaxed);
        }
    }

    pub(crate) fn prune_dimension_table(&self) {
        let Ok(write_txn) = self.db_clone().begin_write() else {
            return;
        };

        {
            let Ok(mut table) = write_txn.open_table(METADATA_TABLE) else {
                return;
            };

            let (expired_keys, mut retained_entries) = {
                let mut expired = Vec::new();
                let mut retained = Vec::new();

                let Ok(iter) = table.iter() else {
                    return;
                };

                for item in iter {
                    let Ok((key, value)) = item else {
                        continue;
                    };

                    let key_owned = key.value().to_string();
                    let Some(record) = decode_record(value.value()) else {
                        expired.push(key_owned);
                        continue;
                    };

                    if record.width == 0 || record.height == 0 {
                        expired.push(key_owned);
                    } else {
                        retained.push(key_owned);
                    }
                }

                (expired, retained)
            };

            if !expired_keys.is_empty() {
                DIMENSION_EXPIRED.fetch_add(expired_keys.len() as u64, Ordering::Relaxed);
                for key in &expired_keys {
                    let _ = table.remove(key.as_str());
                }
            }

            if retained_entries.len() > DIMENSION_CACHE_MAX_ENTRIES {
                retained_entries.sort_unstable();
                let remove_count = retained_entries.len() - DIMENSION_CACHE_MAX_ENTRIES;
                for key in retained_entries.into_iter().take(remove_count) {
                    let _ = table.remove(key.as_str());
                }
                DIMENSION_EVICTED.fetch_add(remove_count as u64, Ordering::Relaxed);
            }
        }

        let _ = write_txn.commit();
    }

    pub(crate) fn cache_needs_size_prune(&self) -> bool {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return false;
        }

        self.file_len.load(Ordering::Relaxed) > max_size_bytes
    }

    pub(crate) fn prune_to_size_limit(&self) {
        let max_size_bytes = metadata_cache_max_size_bytes();
        if max_size_bytes == 0 {
            return;
        }

        if !self.cache_needs_size_prune() {
            return;
        }

        // `compact` needs `&mut Database`; skip the tick while a reader still holds a clone
        // (retried on the next idle tick) instead of making readers wait for the rewrite.
        let mut db = self.db.write();
        let Some(db) = Arc::get_mut(&mut db) else {
            return;
        };
        let _ = db.compact();
    }
}

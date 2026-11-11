//! Cache tests (limits, pruning, shutdown, header detection).

#[allow(unused_imports)]
use super::*;

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

use super::*;
use redb::ReadableTableMetadata;
use std::sync::Mutex as StdMutex;

pub(crate) static CACHE_SHUTDOWN_TEST_LOCK: StdMutex<()> = StdMutex::new(());
pub(crate) static BATCH_TEST_LOCK: StdMutex<()> = StdMutex::new(());

pub(crate) fn test_cache(test_name: &str) -> MetadataCache {
    let path = temp_cache_path(test_name);
    let (db, file_len) = open_database_with_size_limit(path.as_path(), 0).unwrap();
    MetadataCache {
        db: RwLock::new(Arc::new(db)),
        cache_path: path,
        file_len,
        mirror: RwLock::new(HashMap::new()),
    }
}

pub(crate) fn test_record(width: u32, height: u32) -> CachedRecord {
    CachedRecord {
        width,
        height,
        file_type: CachedFileType::Png,
        animated: false,
    }
}

#[test]
pub(crate) fn mirror_miss_falls_back_to_redb_and_repopulates() {
    let _guard = BATCH_TEST_LOCK.lock().unwrap();
    let cache = test_cache("mirror-fallback");
    let media = std::env::temp_dir().join("riv-metadata-mirror-fallback.png");
    let key = crate::cache_key::key(media.as_path(), crate::cache_key::KeyScope::Bare).unwrap();

    assert_eq!(
        cache.apply_batch(&[(key.clone(), test_record(640, 480))]),
        1
    );
    assert!(cache.mirror_peek(&key).is_some());

    // Evicted from RAM: the record must still come back from redb and refill the mirror.
    cache.mirror.write().clear();
    assert_eq!(
        cache.lookup_dimensions(media.as_path(), CachedMediaKind::Image),
        Some((640, 480))
    );
    assert!(cache.mirror_peek(&key).is_some());

    // Kind mismatch and zero dimensions stay rejected from the mirror path too.
    assert_eq!(
        cache.lookup_dimensions(media.as_path(), CachedMediaKind::Video),
        None
    );
    assert_eq!(
        cache.lookup_dimensions_batch(&[(media.clone(), CachedMediaKind::Image)]),
        vec![Some((640, 480))]
    );

    let path = cache.cache_path.clone();
    drop(cache);
    let _ = std::fs::remove_file(path);
}

#[test]
pub(crate) fn dimension_batch_commits_once_for_all_records() {
    let _guard = BATCH_TEST_LOCK.lock().unwrap();
    let cache = test_cache("one-txn-per-batch");
    let batch: Vec<(String, CachedRecord)> = (0..32)
        .map(|index| (format!("batch-key-{index}"), test_record(100 + index, 200)))
        .collect();

    let commits_before = CACHE_BATCH_COMMITS.load(Ordering::Relaxed);
    assert_eq!(cache.apply_batch(&batch), batch.len());
    assert_eq!(
        CACHE_BATCH_COMMITS.load(Ordering::Relaxed) - commits_before,
        1
    );

    let read_txn = cache.db_clone().begin_read().unwrap();
    let table = read_txn.open_table(METADATA_TABLE).unwrap();
    for (key, _) in &batch {
        assert!(table.get(key.as_str()).unwrap().is_some());
    }
    drop(table);
    drop(read_txn);

    let path = cache.cache_path.clone();
    drop(cache);
    let _ = std::fs::remove_file(path);
}

#[test]
pub(crate) fn full_write_queue_drops_op_instead_of_writing_inline() {
    let (tx, rx) = crossbeam_channel::bounded::<CacheWriteOp>(1);
    let drops_before = DIMENSION_WRITE_DROPS.load(Ordering::Relaxed);

    assert!(enqueue_dimension_write(
        &tx,
        PendingDimension {
            key: "key-1".to_string(),
            path: std::path::PathBuf::from("key-1"),
            media_kind: CachedMediaKind::Image,
            width: 1,
            height: 1,
        }
    ));
    assert!(!enqueue_dimension_write(
        &tx,
        PendingDimension {
            key: "key-2".to_string(),
            path: std::path::PathBuf::from("key-2"),
            media_kind: CachedMediaKind::Image,
            width: 2,
            height: 2,
        }
    ));

    // The rejected op was dropped, not written: the queue still holds exactly the first.
    assert_eq!(rx.len(), 1);
    assert!(matches!(
        rx.try_recv(),
        Ok(CacheWriteOp::Dimensions(op)) if op.key == "key-1"
    ));
    assert_eq!(
        DIMENSION_WRITE_DROPS.load(Ordering::Relaxed) - drops_before,
        1
    );
}

#[test]
pub(crate) fn dimension_store_defers_file_reads_to_the_writer() {
    let (tx, rx) = crossbeam_channel::bounded::<CacheWriteOp>(4);
    let absent = std::env::temp_dir()
        .join("riv-metadata-absent")
        .join("missing.png");

    // Caller side never opens the media file: an absent path still enqueues.
    assert!(enqueue_dimension_write(
        &tx,
        PendingDimension {
            key: "absent".to_string(),
            path: absent.clone(),
            media_kind: CachedMediaKind::Image,
            width: 10,
            height: 20,
        }
    ));
    assert_eq!(rx.len(), 1);

    // Writer side is where the file is read, and an absent file resolves to nothing.
    assert!(resolve_record(&absent, CachedMediaKind::Image, 10, 20).is_none());
    assert!(resolve_record(&absent, CachedMediaKind::Image, 0, 20).is_none());
}

#[test]
pub(crate) fn metadata_record_encodes_only_placeholder_columns() {
    let record = CachedRecord {
        width: 320,
        height: 240,
        file_type: CachedFileType::Png,
        animated: false,
    };

    let encoded = encode_record(record);

    assert_eq!(encoded, "320,240,2,0");
    assert_eq!(decode_record(&encoded).unwrap(), record);
}

#[test]
pub(crate) fn file_type_codes_round_trip_to_real_extensions() {
    assert_eq!(CachedFileType::from_code(1).unwrap().extension(), "jpg");
    assert_eq!(CachedFileType::from_code(2).unwrap().extension(), "png");
    assert_eq!(CachedFileType::from_code(5).unwrap().extension(), "webp");
    assert_eq!(CachedFileType::from_code(100).unwrap().extension(), "mp4");
    assert!(CachedFileType::from_code(255).is_none());
}

#[test]
pub(crate) fn detects_png_by_signature_even_when_extension_is_jpg() {
    let path = std::env::temp_dir().join(format!(
        "riv-metadata-cache-{}-wrong.jpg",
        std::process::id()
    ));
    std::fs::write(
        &path,
        [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0,
        ],
    )
    .unwrap();

    let detected = detect_file_type(path.as_path());

    let _ = std::fs::remove_file(&path);
    assert_eq!(detected, Some(CachedFileType::Png));
}

#[test]
pub(crate) fn detects_animated_webp_from_vp8x_header_without_decoding() {
    let header = [
        b"RIFF".as_slice(),
        &30_u32.to_le_bytes(),
        b"WEBP",
        b"VP8X",
        &10_u32.to_le_bytes(),
        &[0b0000_0010, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]
    .concat();

    assert_eq!(webp_header_is_animated(&header), Some(true));
}

#[test]
pub(crate) fn detects_static_webp_from_vp8x_header_without_decoding() {
    let header = [
        b"RIFF".as_slice(),
        &30_u32.to_le_bytes(),
        b"WEBP",
        b"VP8X",
        &10_u32.to_le_bytes(),
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ]
    .concat();

    assert_eq!(webp_header_is_animated(&header), Some(false));
}

#[test]
pub(crate) fn metadata_cache_shutdown_waits_for_active_write_only() {
    let _guard = CACHE_SHUTDOWN_TEST_LOCK.lock().unwrap();
    reset_cache_write_shutdown_for_test();

    begin_cache_write_for_test();
    let shutdown = std::thread::spawn(request_metadata_cache_shutdown);

    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(metadata_cache_shutdown_requested_for_test());
    assert_eq!(active_cache_writes_for_test(), 1);

    finish_cache_write_for_test();
    shutdown.join().unwrap();
    assert!(metadata_cache_shutdown_requested_for_test());
    assert_eq!(active_cache_writes_for_test(), 0);
}

#[test]
pub(crate) fn metadata_cache_shutdown_drops_future_dimension_writes() {
    let _guard = CACHE_SHUTDOWN_TEST_LOCK.lock().unwrap();
    reset_cache_write_shutdown_for_test();
    request_metadata_cache_shutdown();

    assert!(!metadata_cache_accepts_dimension_writes_for_test());
}

#[test]
pub(crate) fn old_dimension_schema_deletes_database_on_open() {
    let path = temp_cache_path("old-dimension-schema");
    {
        let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(METADATA_TABLE).unwrap();
            table
                .insert("D:/Images/Cats/Angora/2121.jpg", "1,2,3,320,240,1,123")
                .unwrap();
        }
        write_txn.commit().unwrap();
    }

    let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
    assert!(!database_uses_old_schema(&db));
    let read_txn = db.begin_read().unwrap();
    let table = read_txn.open_table(METADATA_TABLE);
    assert!(table.is_err() || table.unwrap().is_empty().unwrap());

    let _ = std::fs::remove_file(path);
}

#[test]
pub(crate) fn old_thumbnail_table_deletes_database_on_open() {
    const OLD_THUMBNAIL_TABLE: TableDefinition<&str, &[u8]> =
        TableDefinition::new("video_first_frame_rgba");

    let path = temp_cache_path("old-thumbnail-table");
    {
        let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
        let write_txn = db.begin_write().unwrap();
        {
            let mut table = write_txn.open_table(OLD_THUMBNAIL_TABLE).unwrap();
            table
                .insert("thumb", &[1_u8, 2, 3, 4][..].as_ref())
                .unwrap();
        }
        write_txn.commit().unwrap();
    }

    let (db, _) = open_database_with_size_limit(path.as_path(), 0).unwrap();
    let read_txn = db.begin_read().unwrap();
    let tables: Vec<String> = read_txn
        .list_tables()
        .unwrap()
        .map(|table| table.name().to_string())
        .collect();
    assert!(!tables.iter().any(|name| name == "video_first_frame_rgba"));

    let _ = std::fs::remove_file(path);
}

#[test]
pub(crate) fn bare_key_spelling_is_stable_on_disk() {
    // Absolute on purpose: canonicalize would prepend `\\?\` and change the persisted bytes.
    let key = crate::cache_key::key(
        Path::new("D:/Images/Cats/Angora/2121.jpg"),
        crate::cache_key::KeyScope::Bare,
    )
    .unwrap();

    #[cfg(target_os = "windows")]
    assert_eq!(key, "d:/images/cats/angora/2121.jpg");

    #[cfg(not(target_os = "windows"))]
    assert_eq!(key, "D:/Images/Cats/Angora/2121.jpg");
}

pub(crate) fn temp_cache_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "riv-metadata-cache-{}-{}.redb",
        std::process::id(),
        test_name
    ))
}

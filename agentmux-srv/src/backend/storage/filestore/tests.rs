// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for FileStore.

use super::core::{PART_DATA_SIZE, CACHE_TTL_SECS, MAX_CACHE_BYTES};
use super::{FileOpts, FileMeta, FileStore, WaveFile};
use crate::backend::storage::error::StoreError;

fn make_store() -> FileStore {
    FileStore::open_in_memory().unwrap()
}

#[test]
fn test_create_and_stat() {
    let store = make_store();
    store
        .make_file("zone1", "test.txt", FileMeta::new(), FileOpts::default())
        .unwrap();

    let file = store.stat("zone1", "test.txt").unwrap().unwrap();
    assert_eq!(file.zoneid, "zone1");
    assert_eq!(file.name, "test.txt");
    assert_eq!(file.size, 0);
    assert!(file.createdts > 0);
}

#[test]
fn test_create_duplicate_fails() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    let result = store.make_file("z1", "f1", FileMeta::new(), FileOpts::default());
    assert!(matches!(result, Err(StoreError::AlreadyExists)));
}

#[test]
fn test_write_and_read() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();

    let data = b"hello world";
    store.write_file("z1", "f1", data).unwrap();

    let read_data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(read_data, data);
}

#[test]
fn test_write_large_file() {
    let store = make_store();
    store
        .make_file("z1", "big", FileMeta::new(), FileOpts::default())
        .unwrap();

    // Write data larger than one part (64KB)
    let data: Vec<u8> = (0..PART_DATA_SIZE * 3 + 1000)
        .map(|i| (i % 256) as u8)
        .collect();
    store.write_file("z1", "big", &data).unwrap();

    let read_data = store.read_file("z1", "big").unwrap().unwrap();
    assert_eq!(read_data.len(), data.len());
    assert_eq!(read_data, data);
}

#[test]
fn test_append_data() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();

    store.write_file("z1", "f1", b"hello").unwrap();
    store.append_data("z1", "f1", b" world").unwrap();

    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"hello world");
}

#[test]
fn test_append_across_parts() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();

    // Write nearly a full part
    let initial: Vec<u8> = vec![0xAA; PART_DATA_SIZE - 10];
    store.write_file("z1", "f1", &initial).unwrap();

    // Append across boundary
    let append_data: Vec<u8> = vec![0xBB; 100];
    store.append_data("z1", "f1", &append_data).unwrap();

    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data.len(), PART_DATA_SIZE - 10 + 100);
    assert_eq!(&data[..PART_DATA_SIZE - 10], &initial[..]);
    assert_eq!(&data[PART_DATA_SIZE - 10..], &append_data[..]);
}

#[test]
fn test_delete_file() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"data").unwrap();

    store.delete_file("z1", "f1").unwrap();
    assert!(store.stat("z1", "f1").unwrap().is_none());
    assert!(store.read_file("z1", "f1").unwrap().is_none());
}

#[test]
fn test_delete_zone() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store
        .make_file("z1", "f2", FileMeta::new(), FileOpts::default())
        .unwrap();
    store
        .make_file("z2", "f3", FileMeta::new(), FileOpts::default())
        .unwrap();

    store.delete_zone("z1").unwrap();

    assert!(store.stat("z1", "f1").unwrap().is_none());
    assert!(store.stat("z1", "f2").unwrap().is_none());
    assert!(store.stat("z2", "f3").unwrap().is_some()); // Different zone
}

#[test]
fn test_list_files() {
    let store = make_store();
    store
        .make_file("z1", "a.txt", FileMeta::new(), FileOpts::default())
        .unwrap();
    store
        .make_file("z1", "b.txt", FileMeta::new(), FileOpts::default())
        .unwrap();
    store
        .make_file("z2", "c.txt", FileMeta::new(), FileOpts::default())
        .unwrap();

    let files = store.list_files("z1").unwrap();
    assert_eq!(files.len(), 2);
}

#[test]
fn test_get_all_zone_ids() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store
        .make_file("z2", "f2", FileMeta::new(), FileOpts::default())
        .unwrap();

    let mut zones = store.get_all_zone_ids().unwrap();
    zones.sort();
    assert_eq!(zones, vec!["z1", "z2"]);
}

#[test]
fn test_stat_nonexistent() {
    let store = make_store();
    assert!(store.stat("z1", "nope").unwrap().is_none());
}

#[test]
fn test_read_empty_file() {
    let store = make_store();
    store
        .make_file("z1", "empty", FileMeta::new(), FileOpts::default())
        .unwrap();
    let data = store.read_file("z1", "empty").unwrap().unwrap();
    assert!(data.is_empty());
}

#[test]
fn test_write_to_nonexistent_fails() {
    let store = make_store();
    let result = store.write_file("z1", "nope", b"data");
    assert!(matches!(result, Err(StoreError::NotFound)));
}

#[test]
fn test_circular_file_data_length() {
    let file = WaveFile {
        zoneid: "z1".to_string(),
        name: "circ".to_string(),
        size: 200,
        createdts: 0,
        modts: 0,
        opts: FileOpts {
            maxsize: 100,
            circular: true,
            ..Default::default()
        },
        meta: FileMeta::new(),
    };
    assert_eq!(file.data_length(), 100);
    assert_eq!(file.data_start_idx(), 100);
}

#[test]
fn test_circular_file_under_max() {
    let file = WaveFile {
        zoneid: "z1".to_string(),
        name: "circ".to_string(),
        size: 50,
        createdts: 0,
        modts: 0,
        opts: FileOpts {
            maxsize: 100,
            circular: true,
            ..Default::default()
        },
        meta: FileMeta::new(),
    };
    assert_eq!(file.data_length(), 50);
    assert_eq!(file.data_start_idx(), 0);
}

#[test]
fn test_file_meta_with_opts() {
    let store = make_store();
    let mut meta = FileMeta::new();
    meta.insert("custom".into(), serde_json::json!("value"));

    let opts = FileOpts {
        maxsize: 1024,
        circular: true,
        ..Default::default()
    };

    store.make_file("z1", "f1", meta, opts).unwrap();
    let file = store.stat("z1", "f1").unwrap().unwrap();
    assert_eq!(file.opts.maxsize, 1024);
    assert!(file.opts.circular);
    assert_eq!(file.meta.get("custom").unwrap(), "value");
}

#[test]
fn test_flush_cache_no_dirty() {
    let store = make_store();
    let (files, parts) = store.flush_cache().unwrap();
    assert_eq!(files, 0);
    assert_eq!(parts, 0);
}

#[test]
fn test_cache_evicts_stale_clean_entries() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"data").unwrap();

    // Entry should be in cache — stat hits cache (no DB round trip needed, but file is present)
    assert!(store.stat("z1", "f1").unwrap().is_some());

    // Backdate the cache entry's last_access_ms beyond the TTL
    {
        let ttl_ms = (CACHE_TTL_SECS * 1000 + 1000) as i64; // TTL + 1s in the past
        let now = FileStore::now_ms();
        let mut cache = store.cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(&("z1".to_string(), "f1".to_string())) {
            entry.last_access_ms = now - ttl_ms;
        }
    }

    // flush_cache should evict the stale clean entry
    let (files, parts) = store.flush_cache().unwrap();
    assert_eq!(files, 0); // no dirty files flushed
    assert_eq!(parts, 0);

    // Cache should be empty now
    {
        let cache = store.cache.lock().unwrap();
        assert!(!cache.contains_key(&("z1".to_string(), "f1".to_string())),
            "stale clean entry should have been evicted");
    }

    // File must still be readable from DB after eviction
    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"data");
}

#[test]
fn test_cache_does_not_evict_recently_accessed() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"data").unwrap();

    // Access it (updates last_access_ms to now)
    store.stat("z1", "f1").unwrap();

    // flush_cache should NOT evict a recently accessed entry
    store.flush_cache().unwrap();

    {
        let cache = store.cache.lock().unwrap();
        assert!(cache.contains_key(&("z1".to_string(), "f1".to_string())),
            "recently accessed entry should not be evicted");
    }
}

#[test]
fn test_write_file_does_not_cache_data_parts() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();

    let data: Vec<u8> = vec![0xAB; PART_DATA_SIZE * 2];
    store.write_file("z1", "f1", &data).unwrap();

    // data_entries in the cache entry should be empty — no duplicate data copies in memory
    {
        let cache = store.cache.lock().unwrap();
        if let Some(entry) = cache.get(&("z1".to_string(), "f1".to_string())) {
            assert!(entry.data_entries.is_empty(),
                "write_file must not cache data parts (they are already in DB)");
        }
    }

    // Data must still be readable from DB
    let read = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(read, data);
}

// ---- write_at tests ----

#[test]
fn test_write_at_beginning() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello world").unwrap();

    // Overwrite beginning
    store.write_at("z1", "f1", 0, b"HELLO").unwrap();
    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"HELLO world");
}

#[test]
fn test_write_at_middle() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello world").unwrap();

    store.write_at("z1", "f1", 6, b"WORLD").unwrap();
    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"hello WORLD");
}

#[test]
fn test_write_at_extends() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello").unwrap();

    // Write past current end (at exactly size boundary)
    store.write_at("z1", "f1", 5, b" world").unwrap();
    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"hello world");
    let file = store.stat("z1", "f1").unwrap().unwrap();
    assert_eq!(file.size, 11);
}

#[test]
fn test_write_at_offset_past_size_fails() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello").unwrap();

    let result = store.write_at("z1", "f1", 10, b"data");
    assert!(result.is_err());
}

#[test]
fn test_write_at_empty_data_noop() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello").unwrap();
    store.write_at("z1", "f1", 0, b"").unwrap();
    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"hello");
}

#[test]
fn test_write_at_cross_part_boundary() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();

    // Write a full part plus some extra
    let initial: Vec<u8> = vec![0xAA; PART_DATA_SIZE + 100];
    store.write_file("z1", "f1", &initial).unwrap();

    // Overwrite across part boundary
    let overwrite_offset = (PART_DATA_SIZE - 10) as i64;
    let overwrite_data = vec![0xBB; 20]; // 10 bytes in part 0, 10 bytes in part 1
    store
        .write_at("z1", "f1", overwrite_offset, &overwrite_data)
        .unwrap();

    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data.len(), PART_DATA_SIZE + 100);
    // Check the overwritten region
    for i in 0..20 {
        assert_eq!(
            data[PART_DATA_SIZE - 10 + i],
            0xBB,
            "byte at offset {} should be 0xBB",
            PART_DATA_SIZE - 10 + i
        );
    }
}

// ---- write_meta tests ----

#[test]
fn test_write_meta_replace() {
    let store = make_store();
    let mut initial_meta = FileMeta::new();
    initial_meta.insert("key1".into(), serde_json::json!("val1"));
    initial_meta.insert("key2".into(), serde_json::json!("val2"));
    store
        .make_file("z1", "f1", initial_meta, FileOpts::default())
        .unwrap();

    let mut new_meta = FileMeta::new();
    new_meta.insert("key3".into(), serde_json::json!("val3"));
    store.write_meta("z1", "f1", new_meta, false).unwrap();

    let file = store.stat("z1", "f1").unwrap().unwrap();
    assert!(file.meta.get("key1").is_none()); // replaced
    assert!(file.meta.get("key2").is_none()); // replaced
    assert_eq!(file.meta.get("key3").unwrap(), "val3");
}

#[test]
fn test_write_meta_merge() {
    let store = make_store();
    let mut initial_meta = FileMeta::new();
    initial_meta.insert("key1".into(), serde_json::json!("val1"));
    initial_meta.insert("key2".into(), serde_json::json!("val2"));
    store
        .make_file("z1", "f1", initial_meta, FileOpts::default())
        .unwrap();

    let mut merge_meta = FileMeta::new();
    merge_meta.insert("key2".into(), serde_json::json!("updated"));
    merge_meta.insert("key3".into(), serde_json::json!("new"));
    store.write_meta("z1", "f1", merge_meta, true).unwrap();

    let file = store.stat("z1", "f1").unwrap().unwrap();
    assert_eq!(file.meta.get("key1").unwrap(), "val1"); // unchanged
    assert_eq!(file.meta.get("key2").unwrap(), "updated"); // merged
    assert_eq!(file.meta.get("key3").unwrap(), "new"); // added
}

#[test]
fn test_write_meta_merge_delete() {
    let store = make_store();
    let mut initial_meta = FileMeta::new();
    initial_meta.insert("key1".into(), serde_json::json!("val1"));
    initial_meta.insert("key2".into(), serde_json::json!("val2"));
    store
        .make_file("z1", "f1", initial_meta, FileOpts::default())
        .unwrap();

    let mut merge_meta = FileMeta::new();
    merge_meta.insert("key1".into(), serde_json::Value::Null); // delete
    store.write_meta("z1", "f1", merge_meta, true).unwrap();

    let file = store.stat("z1", "f1").unwrap().unwrap();
    assert!(file.meta.get("key1").is_none()); // deleted
    assert_eq!(file.meta.get("key2").unwrap(), "val2"); // unchanged
}

// ---- read_at tests ----

#[test]
fn test_read_at_full() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello world").unwrap();

    let (offset, data) = store.read_at("z1", "f1", 0, 0).unwrap();
    assert_eq!(offset, 0);
    assert_eq!(data, b"hello world");
}

#[test]
fn test_read_at_partial() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello world").unwrap();

    let (offset, data) = store.read_at("z1", "f1", 6, 5).unwrap();
    assert_eq!(offset, 6);
    assert_eq!(data, b"world");
}

#[test]
fn test_read_at_past_end() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello").unwrap();

    let (_, data) = store.read_at("z1", "f1", 100, 10).unwrap();
    assert!(data.is_empty());
}

#[test]
fn test_read_at_clamps_size() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    store.write_file("z1", "f1", b"hello").unwrap();

    let (offset, data) = store.read_at("z1", "f1", 3, 100).unwrap();
    assert_eq!(offset, 3);
    assert_eq!(data, b"lo"); // only 2 bytes available from offset 3
}

// ---- IJson tests ----

#[test]
fn test_append_ijson_basic() {
    let store = make_store();
    let opts = FileOpts {
        ijson: true,
        ..Default::default()
    };
    store.make_file("z1", "f1", FileMeta::new(), opts).unwrap();

    // Write base JSON
    store
        .write_file("z1", "f1", b"{\"type\":\"set\",\"path\":[],\"data\":{}}\n")
        .unwrap();

    // Append a command
    let cmd = serde_json::json!({"type": "set", "path": ["name"], "data": "Alice"});
    store.append_ijson("z1", "f1", &cmd).unwrap();

    let file = store.stat("z1", "f1").unwrap().unwrap();
    assert!(file.size > 0);

    // Check metadata counters were updated
    let num_cmds = file
        .meta
        .get("ijson:numcmds")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    assert_eq!(num_cmds, 1);
}

#[test]
fn test_append_ijson_not_ijson_file_fails() {
    let store = make_store();
    store
        .make_file("z1", "f1", FileMeta::new(), FileOpts::default())
        .unwrap();
    let cmd = serde_json::json!({"type": "set", "path": ["x"], "data": 1});
    let result = store.append_ijson("z1", "f1", &cmd);
    assert!(result.is_err());
}

#[test]
fn test_compact_ijson() {
    let store = make_store();
    let opts = FileOpts {
        ijson: true,
        ..Default::default()
    };
    store.make_file("z1", "f1", FileMeta::new(), opts).unwrap();

    // Write base + commands
    let content = concat!(
        "{\"type\":\"set\",\"path\":[],\"data\":{}}\n",
        "{\"type\":\"set\",\"path\":[\"x\"],\"data\":1}\n",
        "{\"type\":\"set\",\"path\":[\"y\"],\"data\":2}\n",
    );
    store.write_file("z1", "f1", content.as_bytes()).unwrap();

    // Set up metadata to indicate commands exist
    let mut meta = FileMeta::new();
    meta.insert("ijson:numcmds".into(), serde_json::json!(2));
    meta.insert("ijson:incbytes".into(), serde_json::json!(100));
    store.write_meta("z1", "f1", meta, true).unwrap();

    store.compact_ijson("z1", "f1").unwrap();

    // After compaction, counters should be reset
    let file = store.stat("z1", "f1").unwrap().unwrap();
    let num_cmds = file
        .meta
        .get("ijson:numcmds")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    assert_eq!(num_cmds, 0);
}

// ---- LRU eviction tests ----

/// Helper: sum up file sizes tracked in the cache.
fn cache_total(store: &FileStore) -> usize {
    *store.cache_total_bytes.lock().unwrap()
}

/// Helper: count entries currently in the cache.
fn cache_len(store: &FileStore) -> usize {
    store.cache.lock().unwrap().len()
}

#[test]
fn test_lru_entries_under_cap_all_retained() {
    // Cap = 1 MB; insert two small files — both should stay in cache.
    let store = FileStore::open_in_memory_with_cap(1024 * 1024).unwrap();

    store.make_file("z1", "f1", FileMeta::new(), FileOpts::default()).unwrap();
    store.make_file("z1", "f2", FileMeta::new(), FileOpts::default()).unwrap();

    // Write 100 bytes each
    let data = vec![0xAB_u8; 100];
    store.write_file("z1", "f1", &data).unwrap();
    store.write_file("z1", "f2", &data).unwrap();

    assert_eq!(cache_len(&store), 2, "both entries should be in cache");
    assert!(
        cache_total(&store) <= 1024 * 1024,
        "total bytes should be under cap"
    );
}

#[test]
fn test_lru_oldest_evicted_when_cap_exceeded() {
    // Cap = 200 bytes.  Each file charges max(size, 64) bytes.
    // We'll write files of 100 bytes each (charged as 100 bytes).
    // Three files = 300 bytes > 200 cap → oldest should be evicted.
    let cap: usize = 200;
    let store = FileStore::open_in_memory_with_cap(cap).unwrap();

    // Create files and write 100-byte payloads, accessing them in order f1, f2, f3.
    for name in &["f1", "f2", "f3"] {
        store.make_file("z1", name, FileMeta::new(), FileOpts::default()).unwrap();
    }

    let data = vec![0xBB_u8; 100];

    // Write f1 first (oldest), then f2, then f3 (newest).
    store.write_file("z1", "f1", &data).unwrap();
    store.write_file("z1", "f2", &data).unwrap();
    store.write_file("z1", "f3", &data).unwrap();

    // After inserting f3 the cap (200) is exceeded.  evict_to_cap should have
    // removed at least f1 (oldest) so total <= cap.
    assert!(
        cache_total(&store) <= cap,
        "total {} exceeds cap {}",
        cache_total(&store),
        cap
    );

    // f1 is the oldest; it should have been evicted.
    let has_f1 = store
        .cache
        .lock()
        .unwrap()
        .contains_key(&("z1".to_string(), "f1".to_string()));
    assert!(!has_f1, "f1 (oldest) should have been evicted");

    // The data must still be readable from DB after eviction.
    let read = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(read, data, "f1 data must survive LRU eviction (lives in DB)");
}

#[test]
fn test_lru_access_promotes_entry() {
    // Cap sized to hold exactly 2 × 100-byte entries.
    // Insert f1, f2 (fills to cap).  Then re-access f1 (promotes it to MRU).
    // Insert f3 → cap exceeded → eviction should remove f2 (now the oldest),
    // not f1 (recently touched).
    let cap: usize = 200;
    let store = FileStore::open_in_memory_with_cap(cap).unwrap();

    for name in &["f1", "f2", "f3"] {
        store.make_file("z1", name, FileMeta::new(), FileOpts::default()).unwrap();
    }

    let data = vec![0xCC_u8; 100];
    store.write_file("z1", "f1", &data).unwrap();
    store.write_file("z1", "f2", &data).unwrap();

    // Re-access f1 to promote it (make it newer than f2).
    // We use a small sleep-free timestamp backdate trick: directly set f2's
    // last_access_ms to an older value so f1 is definitively newer.
    {
        let mut cache = store.cache.lock().unwrap();
        if let Some(e) = cache.get_mut(&("z1".to_string(), "f2".to_string())) {
            e.last_access_ms = 1; // artificially old
        }
        // Touch f1 to make it newer
        if let Some(e) = cache.get_mut(&("z1".to_string(), "f1".to_string())) {
            e.last_access_ms = FileStore::now_ms();
        }
    }

    // Insert f3 — should trigger eviction of f2 (oldest), not f1.
    store.write_file("z1", "f3", &data).unwrap();

    assert!(
        cache_total(&store) <= cap,
        "total {} should be <= cap {}",
        cache_total(&store),
        cap
    );

    let cache_guard = store.cache.lock().unwrap();
    let has_f1 = cache_guard.contains_key(&("z1".to_string(), "f1".to_string()));
    let has_f2 = cache_guard.contains_key(&("z1".to_string(), "f2".to_string()));
    drop(cache_guard);

    assert!(!has_f2, "f2 (oldest / LRU) should have been evicted");
    assert!(has_f1, "f1 (recently accessed / MRU) should be retained");
}

#[test]
fn test_lru_ttl_still_works_alongside_lru() {
    // Verify that TTL eviction via flush_cache still removes stale entries
    // independently of the LRU cap.
    let store = make_store(); // default 128 MB cap — LRU won't kick in for tiny data

    store.make_file("z1", "f1", FileMeta::new(), FileOpts::default()).unwrap();
    store.write_file("z1", "f1", b"tiny").unwrap();

    // Backdate the entry beyond TTL.
    {
        let ttl_ms = (CACHE_TTL_SECS * 1000 + 1000) as i64;
        let now = FileStore::now_ms();
        let mut cache = store.cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(&("z1".to_string(), "f1".to_string())) {
            entry.last_access_ms = now - ttl_ms;
        }
    }

    let before_total = cache_total(&store);
    store.flush_cache().unwrap();

    // Entry should be gone from cache.
    let has_f1 = store
        .cache
        .lock()
        .unwrap()
        .contains_key(&("z1".to_string(), "f1".to_string()));
    assert!(!has_f1, "stale entry should have been TTL-evicted by flush_cache");

    // cache_total_bytes must have decreased.
    let after_total = cache_total(&store);
    assert!(
        after_total < before_total,
        "cache_total_bytes should decrease after TTL eviction"
    );

    // Data still readable from DB.
    let data = store.read_file("z1", "f1").unwrap().unwrap();
    assert_eq!(data, b"tiny");
}

#[test]
fn test_lru_delete_updates_total_bytes() {
    let store = make_store();
    store.make_file("z1", "f1", FileMeta::new(), FileOpts::default()).unwrap();
    store.write_file("z1", "f1", b"some data").unwrap();

    let before = cache_total(&store);
    store.delete_file("z1", "f1").unwrap();
    let after = cache_total(&store);

    assert!(after < before, "cache_total_bytes should decrease after delete_file");
}

#[test]
fn test_lru_default_cap_constant() {
    // Sanity check that the public constant is what we expect.
    assert_eq!(MAX_CACHE_BYTES, 128 * 1024 * 1024);
}

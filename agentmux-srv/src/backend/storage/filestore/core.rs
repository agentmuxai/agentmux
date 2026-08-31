// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! FileStore struct and CRUD operations.


use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

use super::cache::CacheEntry;
use super::types::{FileMeta, FileOpts, WaveFile};
use crate::backend::storage::error::StoreError;
use crate::backend::storage::migrations::{
    check_schema_compat, run_filestore_migrations, stamp_version, FILESTORE_SCHEMA_VERSION,
};

/// Default part size: 64KB (matches Go's DefaultPartDataSize).
pub(super) const PART_DATA_SIZE: usize = 64 * 1024;

/// Default flush interval in seconds.
#[allow(dead_code)]
pub const DEFAULT_FLUSH_SECS: u64 = 5;

/// Clean cache entries idle longer than this are evicted during flush.
#[allow(dead_code)]
pub const CACHE_TTL_SECS: u64 = 60;

/// Hard cap on the total byte size held in the metadata cache (128 MB).
/// When this is exceeded, LRU eviction removes the oldest entries first.
pub const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;

/// SQLite-backed file storage with write-through cache.
pub struct FileStore {
    pub(super) conn: Mutex<Connection>,
    pub(super) cache: Mutex<HashMap<(String, String), CacheEntry>>,
    /// Total bytes currently accounted for across all cache entries.
    pub(super) cache_total_bytes: Mutex<usize>,
    /// Maximum bytes the cache may hold before LRU eviction kicks in.
    pub(super) cache_max_bytes: usize,
}

impl FileStore {
    /// Open a FileStore backed by a file on disk.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory FileStore for testing.
    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory FileStore with a custom LRU byte cap.  Used in tests.
    #[allow(dead_code)]
    pub fn open_in_memory_with_cap(max_bytes: usize) -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self::configure_and_migrate(conn)?;
        store.cache_max_bytes = max_bytes;
        Ok(store)
    }

    /// Raw connection access for tests that need to force a specific
    /// failure mode (e.g. dropping a table) — mirrors `Store::conn()`.
    pub(crate) fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    /// Run `PRAGMA wal_checkpoint(TRUNCATE)` on the filestore connection.
    /// Same semantics as `Store::checkpoint` — 5s busy_timeout, partial
    /// truncate on contention is safe.
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        self.conn
            .lock()
            .unwrap()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;",
        )?;
        // Safety lock BEFORE migrations — same discipline as wstore /
        // sagas: refuse to touch a newer-schema DB on disk before any
        // mutating step runs. See `check_schema_compat` doc.
        check_schema_compat(&conn, FILESTORE_SCHEMA_VERSION, "filestore.db")?;
        run_filestore_migrations(&conn)?;
        stamp_version(&conn, FILESTORE_SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
            cache: Mutex::new(HashMap::new()),
            cache_total_bytes: Mutex::new(0),
            cache_max_bytes: MAX_CACHE_BYTES,
        })
    }

    pub(super) fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    /// Evict the least-recently-used cache entries until `cache_total_bytes <= cache_max_bytes`.
    /// Must be called with *neither* `cache` nor `cache_total_bytes` lock held.
    pub(super) fn evict_to_cap(&self) {
        // Fast path: check total without evicting.
        let total = *self.cache_total_bytes.lock().unwrap();
        if total <= self.cache_max_bytes {
            return;
        }

        // Collect (last_access_ms, key, size) for all entries, sort oldest-first.
        let candidates: Vec<(i64, (String, String), usize)> = {
            let cache = self.cache.lock().unwrap();
            cache
                .iter()
                .map(|(k, e)| (e.last_access_ms, k.clone(), e.cached_size_bytes))
                .collect()
        };

        // Sort by last_access_ms ascending (oldest first).
        let mut candidates = candidates;
        candidates.sort_by_key(|(ts, _, _)| *ts);

        let mut evicted_count = 0usize;
        let mut evicted_bytes = 0usize;

        for (_, key, size) in candidates {
            {
                let total = *self.cache_total_bytes.lock().unwrap();
                if total <= self.cache_max_bytes {
                    break;
                }
            }
            {
                let mut cache = self.cache.lock().unwrap();
                if cache.remove(&key).is_some() {
                    let mut total = self.cache_total_bytes.lock().unwrap();
                    *total = total.saturating_sub(size);
                    evicted_count += 1;
                    evicted_bytes += size;
                }
            }
        }

        if evicted_count > 0 {
            tracing::debug!(
                "filestore lru: evicted {} entries, freed {} bytes (cap={})",
                evicted_count,
                evicted_bytes,
                self.cache_max_bytes,
            );
        }
    }

    /// Create a new file. Fails if file already exists.
    #[allow(dead_code)]
    pub fn make_file(
        &self,
        zone_id: &str,
        name: &str,
        meta: FileMeta,
        opts: FileOpts,
    ) -> Result<(), StoreError> {
        let now = Self::now_ms();
        let file = WaveFile {
            zoneid: zone_id.to_string(),
            name: name.to_string(),
            size: 0,
            createdts: now,
            modts: now,
            opts,
            meta,
        };

        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                params![zone_id, name],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if exists {
            return Err(StoreError::AlreadyExists);
        }

        let opts_json = serde_json::to_string(&file.opts)?;
        let meta_json = serde_json::to_string(&file.meta)?;
        conn.execute(
            "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts, opts, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![file.zoneid, file.name, file.size, file.createdts, file.modts, opts_json, meta_json],
        )?;

        // Add to cache
        let key = (zone_id.to_string(), name.to_string());
        let entry = CacheEntry {
            file: Some(file),
            data_entries: HashMap::new(),
            dirty: false,
            last_access_ms: now,
            cached_size_bytes: 64, // new file is size=0; charge minimum overhead
        };
        {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(key, entry);
            *self.cache_total_bytes.lock().unwrap() += 64;
        }
        self.evict_to_cap();

        Ok(())
    }

    /// Delete a file and all its data parts.
    #[allow(dead_code)]
    pub fn delete_file(&self, zone_id: &str, name: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
        )?;
        conn.execute(
            "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
        )?;
        drop(conn);

        // Remove from cache
        let key = (zone_id.to_string(), name.to_string());
        let mut cache = self.cache.lock().unwrap();
        if let Some(removed) = cache.remove(&key) {
            let mut total = self.cache_total_bytes.lock().unwrap();
            *total = total.saturating_sub(removed.cached_size_bytes);
        }

        Ok(())
    }

    /// Delete all files in a zone.
    #[allow(dead_code)]
    pub fn delete_zone(&self, zone_id: &str) -> Result<(), StoreError> {
        // Get file names first for cache cleanup
        let names: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT name FROM db_wave_file WHERE zoneid = ?1")?;
            let rows = stmt.query_map(params![zone_id], |row| row.get(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM db_wave_file WHERE zoneid = ?1",
            params![zone_id],
        )?;
        conn.execute(
            "DELETE FROM db_file_data WHERE zoneid = ?1",
            params![zone_id],
        )?;
        drop(conn);

        let mut cache = self.cache.lock().unwrap();
        let mut freed = 0usize;
        for name in names {
            if let Some(removed) = cache.remove(&(zone_id.to_string(), name)) {
                freed += removed.cached_size_bytes;
            }
        }
        if freed > 0 {
            let mut total = self.cache_total_bytes.lock().unwrap();
            *total = total.saturating_sub(freed);
        }

        Ok(())
    }

    /// Get file metadata. Returns None if file doesn't exist.
    pub fn stat(&self, zone_id: &str, name: &str) -> Result<Option<WaveFile>, StoreError> {
        // Check cache first
        let key = (zone_id.to_string(), name.to_string());
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(entry) = cache.get_mut(&key) {
                entry.last_access_ms = Self::now_ms();
                return Ok(entry.file.clone());
            }
        }

        // Load from DB
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT zoneid, name, size, createdts, modts, opts, meta FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
            |row| {
                let opts_str: String = row.get(5)?;
                let meta_str: String = row.get(6)?;
                Ok(WaveFile {
                    zoneid: row.get(0)?,
                    name: row.get(1)?,
                    size: row.get(2)?,
                    createdts: row.get(3)?,
                    modts: row.get(4)?,
                    opts: serde_json::from_str(&opts_str).unwrap_or_default(),
                    meta: serde_json::from_str(&meta_str).unwrap_or_default(),
                })
            },
        );

        match result {
            Ok(file) => Ok(Some(file)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Write (replace) entire file contents.
    pub fn write_file(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<(), StoreError> {
        let key = (zone_id.to_string(), name.to_string());
        let now = Self::now_ms();

        // Split data into parts
        let parts = Self::split_into_parts(data);

        // Write directly to DB (write-through for full writes, matching Go's WriteFile)
        let conn = self.conn.lock().unwrap();

        // Verify file exists
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                params![zone_id, name],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !exists {
            return Err(StoreError::NotFound);
        }

        // Update file size
        conn.execute(
            "UPDATE db_wave_file SET size = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
            params![data.len() as i64, now, zone_id, name],
        )?;

        // Replace all data parts
        conn.execute(
            "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
        )?;
        for (idx, part_data) in parts.iter().enumerate() {
            conn.execute(
                "INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, idx as i32, part_data],
            )?;
        }
        drop(conn);

        // Update cache (metadata only — data parts are already in DB, read_file loads from DB)
        {
            let new_size = data.len().max(64);
            let mut cache = self.cache.lock().unwrap();
            if let Some(entry) = cache.get_mut(&key) {
                let old_size = entry.cached_size_bytes;
                if let Some(ref mut file) = entry.file {
                    file.size = data.len() as i64;
                    file.modts = now;
                }
                entry.last_access_ms = now;
                entry.cached_size_bytes = new_size;
                let delta = new_size as i64 - old_size as i64;
                let mut total = self.cache_total_bytes.lock().unwrap();
                if delta >= 0 {
                    *total += delta as usize;
                } else {
                    *total = total.saturating_sub((-delta) as usize);
                }
            }
        }
        self.evict_to_cap();

        Ok(())
    }

    /// Read entire file contents.
    pub fn read_file(&self, zone_id: &str, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        // Get file metadata
        let file = match self.stat(zone_id, name)? {
            Some(f) => f,
            None => return Ok(None),
        };

        if file.size == 0 {
            return Ok(Some(Vec::new()));
        }

        let data_len = file.data_length();
        let start_idx = file.data_start_idx();
        let num_parts = ((start_idx + data_len - 1) / PART_DATA_SIZE as i64 + 1) as i32;
        let start_part = (start_idx / PART_DATA_SIZE as i64) as i32;

        // Load parts from DB
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT partidx, data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 ORDER BY partidx",
        )?;
        let rows = stmt.query_map(params![zone_id, name], |row| {
            Ok((row.get::<_, i32>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        let mut parts_map: HashMap<i32, Vec<u8>> = HashMap::new();
        for row in rows {
            let (idx, data) = row?;
            parts_map.insert(idx, data);
        }
        drop(stmt);
        drop(conn);

        // Assemble data
        let mut result = Vec::with_capacity(data_len as usize);
        for part_idx in start_part..start_part + num_parts {
            if let Some(part_data) = parts_map.get(&part_idx) {
                let part_start = part_idx as i64 * PART_DATA_SIZE as i64;
                let skip = if part_start < start_idx {
                    (start_idx - part_start) as usize
                } else {
                    0
                };
                let remaining = data_len as usize - result.len();
                let take = remaining.min(part_data.len() - skip);
                result.extend_from_slice(&part_data[skip..skip + take]);
            }
        }

        let _ = (num_parts, start_part); // used in loop above
        Ok(Some(result))
    }

    /// Append data to the end of a file.
    pub fn append_data(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<(), StoreError> {
        if data.is_empty() {
            return Ok(());
        }
        self.append_data_at(zone_id, name, data).map(|_| ())
    }

    /// Append data to the end of a file and return the byte offset the
    /// batch actually landed at. Unlike a caller-side stat-then-append,
    /// the size read and the part writes happen under ONE connection
    /// lock, so the returned offset is exact even when concurrent
    /// appenders interleave — codex P2 on PR #2508: the `output.tsidx`
    /// sidecar keys batch receive-times by offset, and a racy pre-append
    /// stat could stamp a batch with another batch's position.
    pub fn append_data_at(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<i64, StoreError> {
        let key = (zone_id.to_string(), name.to_string());
        let now = Self::now_ms();

        // Size read + writes under the same lock (self.stat would
        // re-acquire this non-reentrant mutex, hence the direct query).
        let conn = self.conn.lock().unwrap();
        let file_size: i64 = match conn.query_row(
            "SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
            |row| row.get(0),
        ) {
            Ok(s) => s,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(StoreError::NotFound),
            Err(e) => return Err(e.into()),
        };
        if data.is_empty() {
            return Ok(file_size);
        }
        let new_size = file_size + data.len() as i64;

        // Figure out which part to start writing at
        let start_offset = file_size;
        let start_part = (start_offset / PART_DATA_SIZE as i64) as i32;
        let offset_in_part = (start_offset % PART_DATA_SIZE as i64) as usize;
        let mut data_offset = 0usize;
        let mut current_part = start_part;

        if offset_in_part > 0 {
            // Load existing partial part
            let existing: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                    params![zone_id, name, start_part],
                    |row| row.get(0),
                )
                .ok();

            let mut part_data = existing.unwrap_or_default();
            let space = PART_DATA_SIZE - part_data.len();
            let to_copy = space.min(data.len());
            part_data.extend_from_slice(&data[..to_copy]);
            data_offset = to_copy;

            conn.execute(
                "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, current_part, part_data],
            )?;
            current_part += 1;
        }

        // Write remaining full parts
        while data_offset < data.len() {
            let end = (data_offset + PART_DATA_SIZE).min(data.len());
            let part_data = &data[data_offset..end];
            conn.execute(
                "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, current_part, part_data],
            )?;
            data_offset = end;
            current_part += 1;
        }

        // Update file size
        conn.execute(
            "UPDATE db_wave_file SET size = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
            params![new_size, now, zone_id, name],
        )?;
        drop(conn);

        // Update cache
        {
            let new_size_bytes = (new_size as usize).max(64);
            let mut cache = self.cache.lock().unwrap();
            if let Some(entry) = cache.get_mut(&key) {
                let old_size = entry.cached_size_bytes;
                if let Some(ref mut f) = entry.file {
                    f.size = new_size;
                    f.modts = now;
                }
                entry.last_access_ms = now;
                entry.cached_size_bytes = new_size_bytes;
                let delta = new_size_bytes as i64 - old_size as i64;
                let mut total = self.cache_total_bytes.lock().unwrap();
                if delta >= 0 {
                    *total += delta as usize;
                } else {
                    *total = total.saturating_sub((-delta) as usize);
                }
            }
        }
        self.evict_to_cap();

        Ok(start_offset)
    }

    /// Write metadata. If `merge` is true, only specified keys are updated;
    /// otherwise the entire metadata map is replaced.
    pub fn write_meta(
        &self,
        zone_id: &str,
        name: &str,
        meta: FileMeta,
        merge: bool,
    ) -> Result<(), StoreError> {
        let key = (zone_id.to_string(), name.to_string());
        let now = Self::now_ms();

        let file = self.stat(zone_id, name)?.ok_or(StoreError::NotFound)?;

        let new_meta = if merge {
            let mut merged = file.meta.clone();
            for (k, v) in meta {
                if v.is_null() {
                    merged.remove(&k);
                } else {
                    merged.insert(k, v);
                }
            }
            merged
        } else {
            meta
        };

        let meta_json = serde_json::to_string(&new_meta)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_wave_file SET meta = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
            params![meta_json, now, zone_id, name],
        )?;
        drop(conn);

        // Update cache (metadata write doesn't change file.size, so cached_size_bytes unchanged)
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(&key) {
            if let Some(ref mut f) = entry.file {
                f.meta = new_meta;
                f.modts = now;
            }
            entry.last_access_ms = now;
        }

        Ok(())
    }

    /// List all files in a zone.
    #[allow(dead_code)]
    pub fn list_files(&self, zone_id: &str) -> Result<Vec<WaveFile>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT zoneid, name, size, createdts, modts, opts, meta FROM db_wave_file WHERE zoneid = ?1",
        )?;
        let rows = stmt.query_map(params![zone_id], |row| {
            let opts_str: String = row.get(5)?;
            let meta_str: String = row.get(6)?;
            Ok(WaveFile {
                zoneid: row.get(0)?,
                name: row.get(1)?,
                size: row.get(2)?,
                createdts: row.get(3)?,
                modts: row.get(4)?,
                opts: serde_json::from_str(&opts_str).unwrap_or_default(),
                meta: serde_json::from_str(&meta_str).unwrap_or_default(),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Sqlite)
    }

    /// Get all zone IDs that have files.
    #[allow(dead_code)]
    pub fn get_all_zone_ids(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT DISTINCT zoneid FROM db_wave_file")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Sqlite)
    }

    /// Flush dirty cache entries to the database and evict stale clean entries.
    /// Returns (files_flushed, parts_flushed).
    #[allow(dead_code)]
    pub fn flush_cache(&self) -> Result<(usize, usize), StoreError> {
        let ttl_ms = (CACHE_TTL_SECS * 1000) as i64;
        let now = Self::now_ms();
        let cutoff_ms = now - ttl_ms;

        let (dirty_keys, stale_keys): (Vec<_>, Vec<_>) = {
            let cache = self.cache.lock().unwrap();
            let dirty = cache
                .iter()
                .filter(|(_, e)| e.dirty)
                .map(|(k, _)| k.clone())
                .collect();
            let stale = cache
                .iter()
                .filter(|(_, e)| !e.dirty && e.last_access_ms < cutoff_ms)
                .map(|(k, _)| k.clone())
                .collect();
            (dirty, stale)
        };

        // Evict stale clean entries — they're already persisted in DB.
        if !stale_keys.is_empty() {
            let mut freed = 0usize;
            let mut cache = self.cache.lock().unwrap();
            for key in &stale_keys {
                if let Some(removed) = cache.remove(key) {
                    freed += removed.cached_size_bytes;
                }
            }
            if freed > 0 {
                let mut total = self.cache_total_bytes.lock().unwrap();
                *total = total.saturating_sub(freed);
            }
            tracing::debug!("filestore cache: evicted {} stale entries ({} bytes)", stale_keys.len(), freed);
        }

        let mut files_flushed = 0;
        let mut parts_flushed = 0;

        for key in dirty_keys {
            let entry = {
                let mut cache = self.cache.lock().unwrap();
                let entry = cache.remove(&key);
                if let Some(ref e) = entry {
                    let mut total = self.cache_total_bytes.lock().unwrap();
                    *total = total.saturating_sub(e.cached_size_bytes);
                }
                entry
            };

            if let Some(entry) = entry {
                if let Some(ref file) = entry.file {
                    let conn = self.conn.lock().unwrap();
                    let meta_json = serde_json::to_string(&file.meta)?;
                    conn.execute(
                        "UPDATE db_wave_file SET size = ?1, modts = ?2, meta = ?3 WHERE zoneid = ?4 AND name = ?5",
                        params![file.size, file.modts, meta_json, file.zoneid, file.name],
                    )?;

                    for data_entry in entry.data_entries.values() {
                        conn.execute(
                            "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                            params![file.zoneid, file.name, data_entry.part_idx, data_entry.data],
                        )?;
                        parts_flushed += 1;
                    }
                    files_flushed += 1;
                }
            }
        }

        Ok((files_flushed, parts_flushed))
    }

    /// Split data into PART_DATA_SIZE chunks.
    fn split_into_parts(data: &[u8]) -> Vec<Vec<u8>> {
        if data.is_empty() {
            return Vec::new();
        }
        data.chunks(PART_DATA_SIZE)
            .map(|chunk| chunk.to_vec())
            .collect()
    }

    /// Start background flusher (call from async context).
    #[allow(dead_code)]
    pub fn start_flusher(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let store = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(DEFAULT_FLUSH_SECS));
            loop {
                interval.tick().await;
                match store.flush_cache() {
                    Ok((files, parts)) => {
                        if files > 0 {
                            tracing::debug!(
                                "filestore flush: {} files, {} parts",
                                files,
                                parts
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("filestore flush error: {}", e);
                    }
                }
            }
        })
    }
}

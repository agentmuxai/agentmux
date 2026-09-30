// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! FileStore struct and CRUD operations.


use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::cache::CacheEntry;
use super::counter::{new_gen, AppendMode, AppendPos, DROP_EPOCH_SQL};
use super::lines::count_all;
use super::types::{FileMeta, FileOpts, MuxFile};
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
    /// Background WAL checkpoints for a file-backed store (checkpointer.rs);
    /// `None` in memory, or if it couldn't start (SQLite's automatic
    /// checkpoints stay on then).
    pub(super) _checkpointer: Option<super::checkpointer::Checkpointer>,
}

impl FileStore {
    /// Open a FileStore backed by a file on disk.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        let mut store = Self::configure_and_migrate(conn)?;
        // Checkpoints off the write path: a thread with its own connection
        // runs them, and this connection stops running them inside whichever
        // write crosses the threshold (checkpointer.rs). Only if the thread
        // started — otherwise SQLite's automatic checkpoints stay.
        if let Some(checkpointer) = super::checkpointer::Checkpointer::start(path) {
            store.conn.lock().unwrap().execute_batch("PRAGMA wal_autocheckpoint=0;")?;
            store._checkpointer = Some(checkpointer);
        }
        Ok(store)
    }

    /// Open an existing filestore **read-only** — the `Store::open_read_only`
    /// twin: `SQLITE_OPEN_READ_ONLY`, no pragma that writes, no migrations, no
    /// version stamp. For doctor checks that need `stat`/`read_file` against
    /// a filestore they must not touch (codex P2 on #3070). Fails if the file
    /// does not exist.
    pub fn open_read_only(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        Ok(Self {
            conn: Mutex::new(conn),
            cache: Mutex::new(HashMap::new()),
            cache_total_bytes: Mutex::new(0),
            cache_max_bytes: MAX_CACHE_BYTES,
            _checkpointer: None,
        })
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

    fn configure_and_migrate(mut conn: Connection) -> Result<Self, StoreError> {
        // The global transcript store is opened by every srv instance on the
        // machine, possibly at the same moment. busy_timeout comes first so
        // every lock below waits instead of failing.
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        // Switching a rollback-journal database (new, or created by an old
        // build) to WAL upgrades a read lock to an exclusive one. Two
        // connections doing that at once deadlock, and SQLite fails one at
        // once with SQLITE_BUSY rather than calling the busy handler, so
        // retry. It can't run inside a transaction.
        const WAL_ATTEMPTS: u32 = 100;
        for attempt in 1..=WAL_ATTEMPTS {
            match conn.execute_batch("PRAGMA journal_mode=WAL;") {
                Ok(()) => break,
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::DatabaseBusy && attempt < WAL_ATTEMPTS =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => return Err(e.into()),
            }
        }
        // WAL + NORMAL, as the object store and saga log already are: a commit
        // no longer waits for an fsync (the WAL is synced at checkpoints), so a
        // transcript event — which waits for its writes since 5a-3 — isn't held
        // ~2.5 ms per commit on Windows. It cannot corrupt the database; an OS
        // crash or power cut (not an app crash) can lose the last commits.
        // Chosen by the user over full fsync durability (2026-09-24, measured
        // in SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7).
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        // Safety lock BEFORE migrations — same discipline as mstore /
        // sagas: refuse to touch a newer-schema DB on disk before any
        // mutating step runs. See `check_schema_compat` doc. All three in
        // one IMMEDIATE transaction: it takes the write lock up front (with
        // the busy handler), so a concurrent opener's migration can't
        // deadlock with this one on a lock upgrade.
        {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            check_schema_compat(&tx, FILESTORE_SCHEMA_VERSION, "filestore.db")?;
            run_filestore_migrations(&tx)?;
            stamp_version(&tx, FILESTORE_SCHEMA_VERSION)?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
            cache: Mutex::new(HashMap::new()),
            cache_total_bytes: Mutex::new(0),
            cache_max_bytes: MAX_CACHE_BYTES,
            _checkpointer: None,
        })
    }

    pub(super) fn now_ms() -> i64 {
        agentmux_common::time::now_ms()
    }

    /// Run `f` as one `BEGIN IMMEDIATE` transaction: committed if it returns
    /// `Ok`, rolled back otherwise. Every mutation goes through here.
    ///
    /// The in-process mutex only serializes this process. The global
    /// transcript store is one database file opened by every srv instance on
    /// the machine, so a multi-statement write (read the size, write the parts,
    /// update the size) outside a transaction can interleave with another
    /// process's and overwrite its bytes, and a failure part-way leaves a torn
    /// file. `IMMEDIATE` takes the write lock up front, so reads inside `f` see
    /// the state `f` writes on top of; `busy_timeout` makes another process
    /// wait rather than fail. Callers update the in-process cache only after
    /// this returns `Ok`.
    pub(super) fn write_txn<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Run `f` as one read transaction: in WAL mode every read inside it sees
    /// one consistent snapshot of the database, however other connections
    /// write meanwhile. For a decision that combines several reads (a size, a
    /// header, a label) that must describe the same moment.
    pub(super) fn read_txn<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
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
        let file = MuxFile {
            zoneid: zone_id.to_string(),
            name: name.to_string(),
            size: 0,
            createdts: now,
            modts: now,
            opts,
            meta,
        };

        let opts_json = serde_json::to_string(&file.opts)?;
        let meta_json = serde_json::to_string(&file.meta)?;
        // Check and insert in one transaction: two processes creating the same
        // file must see one success and one AlreadyExists, not a raw
        // primary-key error (which callers treat as fatal and skip the write).
        self.write_txn(|tx| {
            let exists = tx
                .query_row(
                    "SELECT 1 FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                    params![zone_id, name],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if exists {
                return Err(StoreError::AlreadyExists);
            }
            // A new, empty file starts a counted epoch (counter.rs).
            tx.execute(
                "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts, opts, meta,
                     gen, lines, lines_size, lines_tail, lines_modts, lines_rev)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 0, 0, ?5, 0)",
                params![file.zoneid, file.name, file.size, file.createdts, file.modts, opts_json, meta_json, new_gen()],
            )?;
            Ok(())
        })?;

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
        self.write_txn(|tx| {
            tx.execute(
                "DELETE FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                params![zone_id, name],
            )?;
            tx.execute(
                "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2",
                params![zone_id, name],
            )?;
            Ok(())
        })?;

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
        // The names (for cache cleanup) and both deletes in one transaction,
        // so a file another process creates meanwhile is either deleted and
        // listed, or neither.
        let names: Vec<String> = self.write_txn(|tx| {
            let names = {
                let mut stmt = tx.prepare("SELECT name FROM db_wave_file WHERE zoneid = ?1")?;
                let rows = stmt.query_map(params![zone_id], |row| row.get(0))?;
                rows.collect::<Result<Vec<String>, _>>()?
            };
            tx.execute(
                "DELETE FROM db_wave_file WHERE zoneid = ?1",
                params![zone_id],
            )?;
            tx.execute(
                "DELETE FROM db_file_data WHERE zoneid = ?1",
                params![zone_id],
            )?;
            Ok(names)
        })?;

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
    pub fn stat(&self, zone_id: &str, name: &str) -> Result<Option<MuxFile>, StoreError> {
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
                Ok(MuxFile {
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

    /// Replace an existing row's content with `data` inside `tx`. Replaced
    /// content starts a new counted epoch with a fresh `gen` (counter.rs).
    pub(super) fn replace_content_in(
        tx: &Transaction<'_>,
        zone_id: &str,
        name: &str,
        data: &[u8],
        now: i64,
    ) -> Result<(), StoreError> {
        tx.execute(
            "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
        )?;
        for (idx, part_data) in Self::split_into_parts(data).iter().enumerate() {
            tx.execute(
                "INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, idx as i32, part_data],
            )?;
        }
        // The new epoch is recorded at the `rev` the part writes above left
        // (the delete trigger bumps it).
        let count = count_all(data);
        tx.execute(
            "UPDATE db_wave_file SET size = ?1, modts = ?2,
                 gen = ?3, lines = ?4, lines_size = ?1, lines_tail = ?5, lines_modts = ?2,
                 lines_rev = COALESCE(rev, 0)
             WHERE zoneid = ?6 AND name = ?7",
            params![data.len() as i64, now, new_gen(), count.lines as i64, count.tail_start as i64, zone_id, name],
        )?;
        Ok(())
    }

    /// Drop this process's cached rows for `names` (after a write that the
    /// cache can't be patched to follow).
    pub(super) fn forget_cached(&self, zone_id: &str, names: &[&str]) {
        let mut cache = self.cache.lock().unwrap();
        let mut freed = 0usize;
        for name in names {
            if let Some(removed) = cache.remove(&(zone_id.to_string(), name.to_string())) {
                freed += removed.cached_size_bytes;
            }
        }
        if freed > 0 {
            let mut total = self.cache_total_bytes.lock().unwrap();
            *total = total.saturating_sub(freed);
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

        // Write directly to DB (write-through for full writes, matching Go's
        // WriteFile), in one transaction: a failure part-way keeps the old
        // content instead of a new size over half-written parts.
        self.write_txn(|tx| {
            let exists = tx
                .query_row(
                    "SELECT 1 FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                    params![zone_id, name],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Err(StoreError::NotFound);
            }
            Self::replace_content_in(tx, zone_id, name, data, now)
        })?;

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
    /// the size read and the part writes happen in ONE transaction, so the
    /// returned offset is exact even when concurrent appenders interleave —
    /// codex P2 on PR #2508: the `output.tsidx` sidecar keys batch
    /// receive-times by offset, and a racy pre-append stat could stamp a
    /// batch with another batch's position.
    pub fn append_data_at(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<i64, StoreError> {
        self.append_data_pos(zone_id, name, data).map(|pos| pos.offset)
    }

    /// [`Self::append_data_at`], also reporting the line counter's position
    /// (`filestore/counter.rs`).
    pub fn append_data_pos(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
    ) -> Result<AppendPos, StoreError> {
        let now = Self::now_ms();
        let (pos, new_size) = self.append_inner(zone_id, name, data, AppendMode::Raw, now)?;
        // Nothing written, nothing for the cache to follow (its modts must
        // keep matching the database's).
        if new_size > pos.offset {
            self.note_appended(zone_id, name, new_size, now);
        }
        Ok(pos)
    }

    /// Bring this process's cached row up to an append that just committed
    /// at `now` (the modts the append wrote, so the cache matches it).
    pub(super) fn note_appended(&self, zone_id: &str, name: &str, new_size: i64, now: i64) {
        let key = (zone_id.to_string(), name.to_string());
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

        // Merge base read from the database in the same transaction as the
        // update, not from `stat` — this process's cached row can predate
        // another process's write, and merging onto it would drop that write.
        let new_meta = self.write_txn(|tx| {
            let current: String = tx
                .query_row(
                    "SELECT meta FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                    params![zone_id, name],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(StoreError::NotFound)?;
            let new_meta = if merge {
                let mut merged: FileMeta = serde_json::from_str(&current).unwrap_or_default();
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
            // A metadata write doesn't change content: a valid epoch follows
            // the new modts (the right-hand sides see the old row).
            tx.execute(
                "UPDATE db_wave_file SET meta = ?1, modts = ?2,
                     lines_modts = CASE WHEN lines_modts = modts AND lines_size = size THEN ?2 ELSE lines_modts END
                 WHERE zoneid = ?3 AND name = ?4",
                params![meta_json, now, zone_id, name],
            )?;
            Ok(new_meta)
        })?;

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
    pub fn list_files(&self, zone_id: &str) -> Result<Vec<MuxFile>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT zoneid, name, size, createdts, modts, opts, meta FROM db_wave_file WHERE zoneid = ?1",
        )?;
        let rows = stmt.query_map(params![zone_id], |row| {
            let opts_str: String = row.get(5)?;
            let meta_str: String = row.get(6)?;
            Ok(MuxFile {
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
                    let meta_json = serde_json::to_string(&file.meta)?;
                    self.write_txn(|tx| {
                        // Rewrites content from the cache: the counter can't
                        // vouch for it, so the epoch is dropped (counter.rs).
                        tx.execute(
                            &format!(
                                "UPDATE db_wave_file SET size = ?1, modts = ?2, meta = ?3, {DROP_EPOCH_SQL}
                                 WHERE zoneid = ?4 AND name = ?5"
                            ),
                            params![file.size, file.modts, meta_json, file.zoneid, file.name],
                        )?;
                        for data_entry in entry.data_entries.values() {
                            tx.execute(
                                "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                                params![file.zoneid, file.name, data_entry.part_idx, data_entry.data],
                            )?;
                        }
                        Ok(())
                    })?;
                    parts_flushed += entry.data_entries.len();
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

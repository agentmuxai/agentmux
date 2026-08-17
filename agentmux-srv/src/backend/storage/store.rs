// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Store: generic OID-based CRUD for StoreObj types.
//! Port of Go's pkg/wstore/wstore_dbops.go + wstore_dbsetup.go.
//!
//! Uses `Mutex<Connection>` matching Go's `MaxOpenConns(1)`.
//! SQLite WAL mode + 5s busy timeout (same as Go).


use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::backend::obj::{wave_obj_from_json, wave_obj_to_json, StoreObj};
use crate::registry::{DefinitionStore, Registry};

use super::error::StoreError;
use super::migrations::{
    check_schema_compat, run_identity_store_schema, run_object_schema, run_shared_store_schema,
    stamp_version, IDENTITY_STORE_SCHEMA_VERSION, OBJECT_SCHEMA_VERSION,
    SHARED_STORE_SCHEMA_VERSION,
};

/// SQLite-backed object store for StoreObj types.
pub struct Store {
    /// `pub(super)` so sibling subsystem modules (e.g. `memory_bundles`)
    /// can take the lock. Each per-subsystem file adds methods to `Store`
    /// via `impl Store {}` and needs the connection.
    pub(super) conn: Mutex<Connection>,
    /// Cross-version named-agent registry. `None` for in-memory test
    /// stores; `Some` for production srv. Mutations to
    /// `db_agent_instances` parallel-write to this registry when set.
    /// See `docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md`.
    registry: Mutex<Option<Arc<Registry>>>,
    /// GLOBAL (cross-channel) agent-definition store. `None` for in-memory
    /// test stores and when the shared dir can't be resolved; `Some` for
    /// production srv. Definition mutations mirror to it so an agent created
    /// in one channel is visible in every channel (cross-channel agent
    /// persistence, `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`).
    def_registry: Mutex<Option<Arc<DefinitionStore>>>,
    /// Base directory that named-instance `working_directory` values are
    /// expressed **relative to** in the instance registry (write side:
    /// `registry_mirror`; read side: the `listnamedagents` handler).
    ///
    /// This is the **current channel's** agents dir (`channels/<ch>/agents`,
    /// i.e. `AGENTMUX_AGENTS_DIR`). It must be tracked separately from the
    /// registry's own file location because P0.3 re-roots the registry to the
    /// global `~/.agentmux/shared/agents/registry/` — once the registry no
    /// longer sits under `channels/<ch>/agents/`, its parent (`agents_root()`)
    /// stops coinciding with the channel agents dir, and using it to strip /
    /// re-join `working_directory` would drop every live instance.
    ///
    /// In production it is wired from `AGENTMUX_AGENTS_DIR` in `main.rs`
    /// (P0.3b), atomically with the re-root to the global shared registry —
    /// which is why the wiring waited for the re-root: setting it earlier would
    /// diverge in dev mode, where `AGENTMUX_AGENTS_DIR` ≠ the (then
    /// channel-local) registry parent. `None` only for in-memory test stores
    /// and odd envs where the var is unset; the accessor then falls back to the
    /// registry's parent (which equals the channel agents dir in the pre-re-root
    /// layout), so existing mirror tests are unchanged. When set, it is passed
    /// in explicitly — never read from ambient env inside the Store — so tests
    /// running inside an AgentMux pane don't pick up the host's
    /// `AGENTMUX_AGENTS_DIR`. See
    /// `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` (P0.3).
    registry_agents_base: Mutex<Option<PathBuf>>,
    /// Serializes `muxbus_save` end-to-end (keychain read, keychain write,
    /// SQL write, and any rollback) — reagent P1 on #2260: two concurrent
    /// `muxbus_save` calls (the `muxbus.login` RPC handler and the broker's
    /// registered refresh closure can each call it independently) would
    /// otherwise race on the keychain's "previous state" snapshot; if the
    /// second call's SQL write then failed, its rollback could restore a
    /// stale pre-first-call blob, silently reverting a credential the first
    /// call already committed. `self.conn`'s own lock only covers the SQL
    /// portion, not the keychain read/write either side of it, so a
    /// dedicated lock is needed rather than reusing that one. `pub(super)`
    /// like `conn` — `muxbus.rs` is a sibling module under `backend::storage`.
    pub(super) muxbus_save_lock: Mutex<()>,
}

impl Store {
    /// Open a Store backed by a file on disk.
    /// Configures WAL mode and 5s busy timeout (matching Go).
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory Store for testing.
    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    /// Open the GLOBAL shared store at `path` (`~/.agentmux/shared/store.db`).
    ///
    /// Uses the same WAL + busy-timeout config as `open()` but runs
    /// `run_shared_store_schema` instead of `run_object_schema` so only
    /// the durable-user-content tables are created (identity, memory, drone
    /// definitions, muxbus creds). Per-channel session tables are intentionally
    /// absent — calling per-channel CRUD methods on this store will error.
    pub fn open_shared(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        check_schema_compat(&conn, SHARED_STORE_SCHEMA_VERSION, "store.db")?;
        run_shared_store_schema(&conn)?;
        stamp_version(&conn, SHARED_STORE_SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
            registry: Mutex::new(None),
            def_registry: Mutex::new(None),
            registry_agents_base: Mutex::new(None),
            muxbus_save_lock: Mutex::new(()),
        })
    }

    /// Open the permanently-global identity store at `path`
    /// (`~/.agentmux/shared/identity-store.db`) — see
    /// `docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`.
    ///
    /// Same WAL + busy-timeout config as [`open_shared`](Self::open_shared),
    /// but runs [`run_identity_store_schema`] — a strict subset of
    /// [`open_shared`](Self::open_shared)'s tables (everything except
    /// `db_accounts`) — instead of `run_shared_store_schema`. Crucially,
    /// this store's PATH is never gated by `isolated_auth_enabled()` (see
    /// its resolver, `registry::resolve_identity_store_path`): unlike
    /// `open_shared`, there is no isolated/per-channel variant of this
    /// store at all.
    pub fn open_identity_store(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        check_schema_compat(&conn, IDENTITY_STORE_SCHEMA_VERSION, "identity-store.db")?;
        run_identity_store_schema(&conn)?;
        stamp_version(&conn, IDENTITY_STORE_SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
            registry: Mutex::new(None),
            def_registry: Mutex::new(None),
            registry_agents_base: Mutex::new(None),
            muxbus_save_lock: Mutex::new(()),
        })
    }

    /// Open a sibling `objects.db` file for read-only backfill access.
    ///
    /// Does NOT run schema migrations or stamp a version — the source DB is
    /// never modified (spec §3.1). WAL mode is not set either: the file is
    /// opened with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` so concurrent
    /// writers on the same DB are unaffected. Tables absent in older schemas
    /// (e.g. `db_drone_definitions`) return empty results via the normal
    /// `StoreError` path; callers use `.unwrap_or_default()`.
    pub fn open_source_readonly(path: &Path) -> Result<Self, StoreError> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_millis(500))?;
        Ok(Self {
            conn: Mutex::new(conn),
            registry: Mutex::new(None),
            def_registry: Mutex::new(None),
            registry_agents_base: Mutex::new(None),
            muxbus_save_lock: Mutex::new(()),
        })
    }

    /// Crate-internal accessor for sibling modules that maintain their
    /// own per-table CRUD via the `DroneStore` extension trait
    /// pattern (see `agentmux-srv/src/drone/storage.rs`). Outside
    /// callers must use the typed methods on this impl.
    pub(crate) fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    /// Run the `db_agents` consolidation backfill under the wstore's
    /// exclusive connection lock. Idempotent — gated by a marker file in
    /// `data_dir` (skip with `None` for tests).
    pub fn run_agents_consolidate(
        &self,
        data_dir: Option<&Path>,
    ) -> Result<super::agents_consolidate::ConsolidateStats, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        super::agents_consolidate::run_consolidate_migration(&mut conn, data_dir)
    }

    /// Run `PRAGMA wal_checkpoint(TRUNCATE)` to fold the WAL back into the
    /// main DB file. Called periodically and on clean shutdown. The 5s
    /// `busy_timeout` set at open handles transient reader contention — if it
    /// can't fully truncate it returns partial progress (safe; picked up on
    /// the next pass).
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        self.conn()
            .lock()
            .unwrap()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Backfill any `db_agent_definitions` rows that are missing from
    /// `db_agents`. Not marker-gated — runs cheaply on every startup.
    /// See `agents_consolidate::repair_def_gaps` for details.
    pub fn repair_agent_def_gaps(&self) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        super::agents_consolidate::repair_def_gaps(&mut *conn)
    }

    /// True when `db_agent_definitions`/`db_agent_instances` have rows but
    /// `db_agents` doesn't — i.e. the consolidation backfill's marker/stamp
    /// can't be trusted as proof it actually ran. See
    /// `agents_consolidate::consolidate_looks_incomplete`.
    pub fn agents_consolidate_looks_incomplete(&self) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        super::agents_consolidate::consolidate_looks_incomplete(&conn)
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(
            // `foreign_keys=ON` is per-connection and defaults to OFF in
            // SQLite. The v6 schema (`db_agent_identity_links`,
            // `db_agent_instances`) relies on `ON DELETE CASCADE` to clean
            // up junction rows and instances when a parent agent or
            // identity is removed. Without this pragma on the production
            // connection, cascades silently no-op; migration tests set it
            // explicitly, which would have masked the gap.
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        // Safety lock BEFORE any migration side effects — the legacy-
        // table rename + seed-insert steps in `run_object_schema` are
        // mutating, so we must refuse to open a newer-schema DB before
        // we touch it. (codex P1 on #1029 — fixes the original PR's
        // check-after-migrate order that let a downgraded binary
        // partially mutate a newer DB before the error fired.)
        check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db")?;
        run_object_schema(&conn)?;
        stamp_version(&conn, OBJECT_SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
            registry: Mutex::new(None),
            def_registry: Mutex::new(None),
            registry_agents_base: Mutex::new(None),
            muxbus_save_lock: Mutex::new(()),
        })
    }

    /// Attach a shared cross-version agent registry. Called once on
    /// srv startup after `Store::open` and before the store is
    /// wrapped in `Arc`. Mutations to `db_agent_instances` will then
    /// parallel-write to the registry; the SQLite table remains the
    /// authoritative read path for PR A.
    pub fn set_registry(&self, registry: Arc<Registry>) {
        *self.registry.lock().unwrap_or_else(|e| e.into_inner()) = Some(registry);
    }

    pub(super) fn registry(&self) -> Option<Arc<Registry>> {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Set the channel agents dir that instance `working_directory` values
    /// are stored relative to (see the `registry_agents_base` field). Wired
    /// from `AGENTMUX_AGENTS_DIR` in `main.rs` in P0.3b (atomically with the
    /// registry re-root); until then it is exercised only by tests.
    pub fn set_registry_agents_base(&self, base: PathBuf) {
        *self
            .registry_agents_base
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(base);
    }

    /// The base dir for instance `working_directory` relative paths.
    ///
    /// Returns the explicitly-set channel agents dir
    /// ([`set_registry_agents_base`]) when present; otherwise falls back to
    /// the registry's parent (`agents_root()`), which equals the channel
    /// agents dir in the pre-re-root layout. Used symmetrically by the write
    /// mirror and the read handler so the two never disagree on the anchor.
    pub fn registry_agents_base(&self) -> Option<PathBuf> {
        if let Some(base) = self
            .registry_agents_base
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Some(base);
        }
        self.registry()
            .and_then(|r| r.agents_root().map(|p| p.to_path_buf()))
    }

    /// Public accessor for the cross-version named-agent registry.
    /// Returns `None` when the registry couldn't be resolved at
    /// startup (CI / unusual envs); callers must handle the absent
    /// case by falling back to SQLite.
    pub fn shared_agent_registry(&self) -> Option<Arc<Registry>> {
        self.registry()
    }

    /// Attach the GLOBAL (cross-channel) agent-definition store. Called
    /// once on srv startup after `Store::open`, before the store is
    /// wrapped in `Arc`. Definition mutations then mirror to it.
    pub fn set_def_registry(&self, def_registry: Arc<DefinitionStore>) {
        *self.def_registry.lock().unwrap_or_else(|e| e.into_inner()) = Some(def_registry);
    }

    /// Public accessor for the global agent-definition store. `None` when
    /// it couldn't be resolved at startup (CI / unusual envs / in-memory
    /// tests); callers fall back to SQLite.
    pub fn shared_def_registry(&self) -> Option<Arc<DefinitionStore>> {
        self.def_registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Table name for a StoreObj type: `db_<otype>`.
    fn table_name<T: StoreObj>() -> String {
        format!("db_{}", T::get_otype())
    }

    /// Get a single object by OID. Returns `None` if not found.
    pub fn get<T: StoreObj>(&self, oid: &str) -> Result<Option<T>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        let mut stmt =
            conn.prepare(&format!("SELECT version, data FROM {table} WHERE oid = ?1"))?;

        let result = stmt.query_row(params![oid], |row| {
            let version: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok((version, data))
        });

        match result {
            Ok((version, data)) => {
                let mut obj: T = wave_obj_from_json(&data)?;
                obj.set_version(version);
                Ok(Some(obj))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Get a single object, returning `StoreError::NotFound` if missing.
    pub fn must_get<T: StoreObj>(&self, oid: &str) -> Result<T, StoreError> {
        self.get::<T>(oid)?.ok_or(StoreError::NotFound)
    }

    /// Get a single object as raw JSON Value by otype and OID.
    /// Used by GetObject/GetObjects to return data without strict struct deserialization.
    pub fn get_raw(&self, otype: &str, oid: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{}", otype);
        let mut stmt =
            conn.prepare(&format!("SELECT version, data FROM {table} WHERE oid = ?1"))?;

        let result = stmt.query_row(params![oid], |row| {
            let version: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok((version, data))
        });

        match result {
            Ok((version, data)) => {
                let mut val: serde_json::Value = serde_json::from_slice(&data)
                    .map_err(|e| StoreError::Json(e))?;
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("version".to_string(), serde_json::json!(version));
                    obj.insert("otype".to_string(), serde_json::json!(otype));
                }
                Ok(Some(val))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Check if an object exists (by otype and OID).
    #[allow(dead_code)]
    pub fn exists_raw(&self, otype: &str, oid: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{}", otype);
        let count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE oid = ?1"),
            params![oid],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Insert a new object. Sets version to 1.
    pub fn insert<T: StoreObj>(&self, obj: &mut T) -> Result<(), StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        obj.set_version(1);
        let data = wave_obj_to_json(obj)?;

        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        conn.execute(
            &format!("INSERT INTO {table} (oid, version, data) VALUES (?1, 1, ?2)"),
            params![oid, data],
        )?;

        Ok(())
    }

    /// Update an existing object. Increments version atomically.
    /// Returns the new version number.
    pub fn update<T: StoreObj>(&self, obj: &mut T) -> Result<i64, StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        let data = wave_obj_to_json(obj)?;

        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();

        // Optimistic locking: increment version and return new value.
        // Matches Go: `UPDATE ... SET version = version+1 ... RETURNING version`
        let new_version: i64 = conn.query_row(
            &format!(
                "UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2 RETURNING version"
            ),
            params![data, oid],
            |row| row.get(0),
        )?;

        obj.set_version(new_version);
        Ok(new_version)
    }

    /// Update an object using raw JSON (bypasses struct deserialization).
    /// Used by UpdateObject where the frontend sends the full replacement object.
    /// This matches Go's generic map-based UpdateObject behavior.
    pub fn update_raw(&self, otype: &str, oid: &str, value: &serde_json::Value) -> Result<i64, StoreError> {
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }
        let data = serde_json::to_vec(value)?;
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{}", otype);
        let new_version: i64 = conn.query_row(
            &format!(
                "UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2 RETURNING version"
            ),
            params![data, oid],
            |row| row.get(0),
        )?;
        Ok(new_version)
    }

    /// Delete an object by OID.
    #[allow(dead_code)]
    pub fn delete<T: StoreObj>(&self, oid: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }

    /// Delete by otype string and OID (for dynamic dispatch).
    /// Validates `otype` against `VALID_OTYPES` to prevent SQL injection.
    #[allow(dead_code)]
    pub fn delete_by_otype(&self, otype: &str, oid: &str) -> Result<(), StoreError> {
        if !crate::backend::obj::VALID_OTYPES.contains(&otype) {
            return Err(StoreError::Other(format!("unknown otype: {otype:?}")));
        }
        let conn = self.conn.lock().unwrap();
        let table = format!("db_{otype}");
        conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }

    /// Get all objects of a given type.
    pub fn get_all<T: StoreObj>(&self) -> Result<Vec<T>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        let mut stmt = conn.prepare(&format!("SELECT oid, version, data FROM {table}"))?;
        let rows = stmt.query_map([], |row| {
            let version: i64 = row.get(1)?;
            let data: Vec<u8> = row.get(2)?;
            Ok((version, data))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (version, data) = row?;
            let mut obj: T = wave_obj_from_json(&data)?;
            obj.set_version(version);
            result.push(obj);
        }
        Ok(result)
    }

    /// Count objects of a given type.
    #[allow(dead_code)]
    pub fn count<T: StoreObj>(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let table = Self::table_name::<T>();
        let count: i64 =
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        Ok(count)
    }

    /// Execute multiple operations in a single SQLite transaction.
    /// Acquires the Mutex once, wraps all operations in BEGIN/COMMIT.
    /// On error, rolls back and returns the error.
    ///
    /// This is the key performance primitive — reduces N lock acquisitions
    /// and N fsyncs to 1 each.
    pub fn with_tx<F, R>(&self, f: F) -> Result<R, StoreError>
    where
        F: FnOnce(&StoreTx) -> Result<R, StoreError>,
    {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN")?;
        let tx = StoreTx { conn: &conn };
        match f(&tx) {
            Ok(result) => {
                conn.execute_batch("COMMIT")?;
                Ok(result)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ── Migration state (db_migrations — shared store only) ──────────────

    pub fn migration_is_applied(&self, id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM db_migrations WHERE id = ?1",
            [id],
            |_| Ok(true),
        )
        .unwrap_or(false)
    }

    pub fn migration_mark_applied(
        &self,
        id: &str,
        scope: &str,
        duration_ms: u64,
    ) -> Result<(), StoreError> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_migrations (id, applied_at, duration_ms, scope)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, now, duration_ms as i64, scope],
        )?;
        Ok(())
    }

    pub fn migrations_list_applied(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id FROM db_migrations ORDER BY id")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }
}

/// A borrowed connection handle for use inside [`Store::with_tx`].
/// Provides the same CRUD methods as `Store` but operates on the
/// already-locked connection without additional Mutex acquisition.
pub struct StoreTx<'a> {
    conn: &'a Connection,
}

impl<'a> StoreTx<'a> {
    fn table_name<T: StoreObj>() -> String {
        format!("db_{}", T::get_otype())
    }

    pub fn get<T: StoreObj>(&self, oid: &str) -> Result<Option<T>, StoreError> {
        let table = Self::table_name::<T>();
        let mut stmt =
            self.conn.prepare(&format!("SELECT version, data FROM {table} WHERE oid = ?1"))?;

        let result = stmt.query_row(params![oid], |row| {
            let version: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok((version, data))
        });

        match result {
            Ok((version, data)) => {
                let mut obj: T = wave_obj_from_json(&data)?;
                obj.set_version(version);
                Ok(Some(obj))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    pub fn must_get<T: StoreObj>(&self, oid: &str) -> Result<T, StoreError> {
        self.get::<T>(oid)?.ok_or(StoreError::NotFound)
    }

    pub fn insert<T: StoreObj>(&self, obj: &mut T) -> Result<(), StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        obj.set_version(1);
        let data = wave_obj_to_json(obj)?;

        let table = Self::table_name::<T>();
        self.conn.execute(
            &format!("INSERT INTO {table} (oid, version, data) VALUES (?1, 1, ?2)"),
            params![oid, data],
        )?;

        Ok(())
    }

    pub fn update<T: StoreObj>(&self, obj: &mut T) -> Result<i64, StoreError> {
        let oid = obj.get_oid().to_string();
        if oid.is_empty() {
            return Err(StoreError::EmptyOID);
        }

        let data = wave_obj_to_json(obj)?;

        let table = Self::table_name::<T>();
        let new_version: i64 = self.conn.query_row(
            &format!(
                "UPDATE {table} SET data = ?1, version = version + 1 WHERE oid = ?2 RETURNING version"
            ),
            params![data, oid],
            |row| row.get(0),
        )?;

        obj.set_version(new_version);
        Ok(new_version)
    }

    pub fn get_all<T: StoreObj>(&self) -> Result<Vec<T>, StoreError> {
        let table = Self::table_name::<T>();
        let mut stmt = self.conn.prepare(&format!("SELECT oid, version, data FROM {table}"))?;
        let rows = stmt.query_map([], |row| {
            let version: i64 = row.get(1)?;
            let data: Vec<u8> = row.get(2)?;
            Ok((version, data))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (version, data) = row?;
            let mut obj: T = wave_obj_from_json(&data)?;
            obj.set_version(version);
            result.push(obj);
        }
        Ok(result)
    }

    #[allow(dead_code)]
    pub fn delete<T: StoreObj>(&self, oid: &str) -> Result<(), StoreError> {
        let table = Self::table_name::<T>();
        self.conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }
}

// Re-exports so existing `storage::store::*` imports keep working.
pub use super::agents::{derive_slug, AgentDefinition, AgentInstance, InstanceStatus};
pub use super::memory_bundles::Memory;
pub use super::content::AgentContent;
pub use super::history::AgentHistory;
pub use super::skills::AgentSkill;

// Identity system types.
pub use super::identities::{AgentIdentityLink, IdentityAccount, SecretRef};




// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests;

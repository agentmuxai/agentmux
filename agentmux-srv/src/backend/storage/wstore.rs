// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WaveStore: generic OID-based CRUD for WaveObj types.
//! Port of Go's pkg/wstore/wstore_dbops.go + wstore_dbsetup.go.
//!
//! Uses `Mutex<Connection>` matching Go's `MaxOpenConns(1)`.
//! SQLite WAL mode + 5s busy timeout (same as Go).


use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::backend::obj::{wave_obj_from_json, wave_obj_to_json, WaveObj};

use super::error::StoreError;
use super::migrations::{run_forge_migrations, run_wstore_migrations};

/// SQLite-backed object store for WaveObj types.
pub struct WaveStore {
    conn: Mutex<Connection>,
}

impl WaveStore {
    /// Open a WaveStore backed by a file on disk.
    /// Configures WAL mode and 5s busy timeout (matching Go).
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory WaveStore for testing.
    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    /// Crate-internal accessor for sibling modules that maintain their
    /// own per-table CRUD via the `WorkflowStore` extension trait
    /// pattern (see `agentmux-srv/src/workflows/storage.rs`). Outside
    /// callers must use the typed methods on this impl.
    pub(crate) fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(
            // `foreign_keys=ON` is per-connection and defaults to OFF in
            // SQLite. The v6 schema (`db_forge_agent_identities`,
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
        run_wstore_migrations(&conn)?;
        run_forge_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Table name for a WaveObj type: `db_<otype>`.
    fn table_name<T: WaveObj>() -> String {
        format!("db_{}", T::get_otype())
    }

    /// Get a single object by OID. Returns `None` if not found.
    pub fn get<T: WaveObj>(&self, oid: &str) -> Result<Option<T>, StoreError> {
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
    pub fn must_get<T: WaveObj>(&self, oid: &str) -> Result<T, StoreError> {
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
    pub fn insert<T: WaveObj>(&self, obj: &mut T) -> Result<(), StoreError> {
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
    pub fn update<T: WaveObj>(&self, obj: &mut T) -> Result<i64, StoreError> {
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
    pub fn delete<T: WaveObj>(&self, oid: &str) -> Result<(), StoreError> {
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
    pub fn get_all<T: WaveObj>(&self) -> Result<Vec<T>, StoreError> {
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
    pub fn count<T: WaveObj>(&self) -> Result<i64, StoreError> {
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
}

/// A borrowed connection handle for use inside [`WaveStore::with_tx`].
/// Provides the same CRUD methods as `WaveStore` but operates on the
/// already-locked connection without additional Mutex acquisition.
pub struct StoreTx<'a> {
    conn: &'a Connection,
}

impl<'a> StoreTx<'a> {
    fn table_name<T: WaveObj>() -> String {
        format!("db_{}", T::get_otype())
    }

    pub fn get<T: WaveObj>(&self, oid: &str) -> Result<Option<T>, StoreError> {
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

    pub fn must_get<T: WaveObj>(&self, oid: &str) -> Result<T, StoreError> {
        self.get::<T>(oid)?.ok_or(StoreError::NotFound)
    }

    pub fn insert<T: WaveObj>(&self, obj: &mut T) -> Result<(), StoreError> {
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

    pub fn update<T: WaveObj>(&self, obj: &mut T) -> Result<i64, StoreError> {
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

    pub fn get_all<T: WaveObj>(&self) -> Result<Vec<T>, StoreError> {
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
    pub fn delete<T: WaveObj>(&self, oid: &str) -> Result<(), StoreError> {
        let table = Self::table_name::<T>();
        self.conn.execute(
            &format!("DELETE FROM {table} WHERE oid = ?1"),
            params![oid],
        )?;
        Ok(())
    }
}

// ====================================================================
// ForgeAgent CRUD
// ====================================================================

/// A user-defined AI agent managed by the Forge widget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeAgent {
    pub id: String,
    /// Stable, filesystem-safe identifier. Drives working directory,
    /// env var keys (AGENTMUX_AGENT_ID), and cross-references.
    /// NEVER changes after creation — distinct from `name` which is
    /// the renameable display. See
    /// specs/SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md.
    #[serde(default)]
    pub slug: String,
    pub name: String,
    pub icon: String,
    pub provider: String,
    pub description: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub provider_flags: String,
    #[serde(default)]
    pub auto_start: i64,
    #[serde(default)]
    pub restart_on_crash: i64,
    #[serde(default)]
    pub idle_timeout_minutes: i64,
    pub created_at: i64,
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
    #[serde(default)]
    pub is_seeded: i64,
    /// JSON-encoded per-provider account references.
    /// **Deprecated in v6** — superseded by the `db_forge_agent_identities`
    /// junction table. Still populated on read for backward compat with
    /// any existing DB rows; new writes leave it empty. See
    /// specs/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md.
    #[serde(default)]
    pub accounts: String,
    /// Parent definition id (forge_agents.id). Empty string = root
    /// definition; non-empty = this agent was forked from another.
    /// Added in v6. See spec §Phase 1.
    #[serde(default)]
    pub parent_id: String,
    /// Label describing the branch (e.g. `"pr-422-review"`,
    /// `"experiment-refactor"`). Empty for root definitions and for
    /// branches that didn't set a label. Added in v6.
    #[serde(default)]
    pub branch_label: String,
}

/// Derive a filesystem-safe slug from a display name. Lowercase,
/// ASCII alphanumeric + dash/underscore, consecutive dashes collapsed,
/// trimmed to 64 chars. Returns `"agent"` if the input has no valid
/// characters (defensive fallback).
pub fn derive_slug(name: &str) -> String {
    let filtered: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let collapsed: String = filtered
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let trimmed: String = collapsed.chars().take(64).collect();
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed
    }
}

fn default_agent_type() -> String {
    "standalone".to_string()
}

/// A content blob associated with a forge agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeContent {
    pub agent_id: String,
    pub content_type: String,
    pub content: String,
    pub updated_at: i64,
}

/// A reusable skill/capability attached to a forge agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeSkill {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub trigger: String,
    pub skill_type: String,
    pub description: String,
    pub content: String,
    pub created_at: i64,
}

/// An append-only session history entry for a forge agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeHistory {
    pub id: i64,
    pub agent_id: String,
    pub session_date: String,
    pub entry: String,
    pub timestamp: i64,
}

impl WaveStore {
    /// List all forge agents, ordered by created_at ascending.
    pub fn forge_list(&self) -> Result<Vec<ForgeAgent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description, working_directory, shell,
                    provider_flags, auto_start, restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded, accounts,
                    parent_id, branch_label
             FROM db_forge_agents ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ForgeAgent {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                icon: row.get(3)?,
                provider: row.get(4)?,
                description: row.get(5)?,
                working_directory: row.get(6)?,
                shell: row.get(7)?,
                provider_flags: row.get(8)?,
                auto_start: row.get(9)?,
                restart_on_crash: row.get(10)?,
                idle_timeout_minutes: row.get(11)?,
                created_at: row.get(12)?,
                agent_type: row.get(13)?,
                environment: row.get(14)?,
                agent_bus_id: row.get(15)?,
                is_seeded: row.get(16)?,
                accounts: row.get(17)?,
                parent_id: row.get(18)?,
                branch_label: row.get(19)?,
            })
        })?;
        let mut agents = Vec::new();
        for row in rows {
            agents.push(row?);
        }
        Ok(agents)
    }

    /// Count forge agents (used by seed engine to check if seeding is needed).
    pub fn forge_count(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_forge_agents",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Delete all seeded agents (is_seeded=1). Used by reseed to clear built-in agents.
    pub fn forge_delete_seeded(&self) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_forge_agents WHERE is_seeded=1", [])?;
        Ok(rows)
    }

    /// Insert a new forge agent. Auto-derives slug from name if empty,
    /// resolves collisions by appending `-2`, `-3`, etc., and mutates
    /// `agent.slug` so the caller sees the resolved value (important
    /// for handlers that serialize the struct back to the frontend
    /// after insert).
    ///
    /// The collision check + insert run under a single mutex lock,
    /// so this is race-safe against concurrent inserts on the same
    /// connection.
    pub fn forge_insert(&self, agent: &mut ForgeAgent) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let base = if agent.slug.is_empty() {
            derive_slug(&agent.name)
        } else {
            agent.slug.clone()
        };
        // Collision-resolve: scan for existing slugs matching base or
        // base-N. The migration backfill does the same dance for
        // pre-existing rows (see run_forge_v4_migrations).
        let mut candidate = base.clone();
        let mut n: u32 = 2;
        loop {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM db_forge_agents WHERE slug = ?1",
                params![candidate],
                |row| row.get(0),
            )?;
            if count == 0 {
                break;
            }
            candidate = format!("{}-{}", base, n);
            n += 1;
        }
        agent.slug = candidate;
        conn.execute(
            "INSERT INTO db_forge_agents (id, slug, name, icon, provider, description,
             working_directory, shell, provider_flags, auto_start, restart_on_crash,
             idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
             is_seeded, accounts, parent_id, branch_label)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20)",
            params![
                agent.id,
                agent.slug,
                agent.name,
                agent.icon,
                agent.provider,
                agent.description,
                agent.working_directory,
                agent.shell,
                agent.provider_flags,
                agent.auto_start,
                agent.restart_on_crash,
                agent.idle_timeout_minutes,
                agent.created_at,
                agent.agent_type,
                agent.environment,
                agent.agent_bus_id,
                agent.is_seeded,
                agent.accounts,
                agent.parent_id,
                agent.branch_label,
            ],
        )?;
        Ok(())
    }

    /// Update an existing forge agent (all fields except id, created_at, is_seeded).
    /// `parent_id` and `branch_label` are NOT updatable post-insert — they
    /// describe the agent's provenance; renaming or re-branching is done by
    /// creating a new fork, not mutating the original.
    pub fn forge_update(&self, agent: &ForgeAgent) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_forge_agents SET name=?1, icon=?2, provider=?3, description=?4,
             working_directory=?5, shell=?6, provider_flags=?7, auto_start=?8,
             restart_on_crash=?9, idle_timeout_minutes=?10,
             agent_type=?11, environment=?12, agent_bus_id=?13, accounts=?14
             WHERE id=?15",
            params![
                agent.name,
                agent.icon,
                agent.provider,
                agent.description,
                agent.working_directory,
                agent.shell,
                agent.provider_flags,
                agent.auto_start,
                agent.restart_on_crash,
                agent.idle_timeout_minutes,
                agent.agent_type,
                agent.environment,
                agent.agent_bus_id,
                agent.accounts,
                agent.id
            ],
        )?;
        Ok(rows > 0)
    }

    /// Delete a forge agent by id. Returns true if a row was deleted.
    pub fn forge_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_forge_agents WHERE id=?1",
            params![id],
        )?;
        Ok(rows > 0)
    }

    // ---- ForgeContent CRUD ----

    /// Get a single content blob for an agent.
    pub fn forge_get_content(&self, agent_id: &str, content_type: &str) -> Result<Option<ForgeContent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, content_type, content, updated_at
             FROM db_forge_content WHERE agent_id=?1 AND content_type=?2",
        )?;
        let result = stmt.query_row(params![agent_id, content_type], |row| {
            Ok(ForgeContent {
                agent_id: row.get(0)?,
                content_type: row.get(1)?,
                content: row.get(2)?,
                updated_at: row.get(3)?,
            })
        });
        match result {
            Ok(content) => Ok(Some(content)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Upsert a content blob for an agent.
    pub fn forge_set_content(&self, content: &ForgeContent) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_forge_content (agent_id, content_type, content, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, content_type) DO UPDATE SET content=?3, updated_at=?4",
            params![
                content.agent_id,
                content.content_type,
                content.content,
                content.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Get all content blobs for an agent.
    pub fn forge_get_all_content(&self, agent_id: &str) -> Result<Vec<ForgeContent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, content_type, content, updated_at
             FROM db_forge_content WHERE agent_id=?1 ORDER BY content_type ASC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(ForgeContent {
                agent_id: row.get(0)?,
                content_type: row.get(1)?,
                content: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        let mut contents = Vec::new();
        for row in rows {
            contents.push(row?);
        }
        Ok(contents)
    }

    /// Delete a specific content blob. Returns true if a row was deleted.
    #[allow(dead_code)]
    pub fn forge_delete_content(&self, agent_id: &str, content_type: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_forge_content WHERE agent_id=?1 AND content_type=?2",
            params![agent_id, content_type],
        )?;
        Ok(rows > 0)
    }

    // ---- ForgeSkill CRUD ----

    /// List all skills for an agent, ordered by created_at ascending.
    pub fn forge_list_skills(&self, agent_id: &str) -> Result<Vec<ForgeSkill>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, name, trigger, skill_type, description, content, created_at
             FROM db_forge_skills WHERE agent_id=?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(ForgeSkill {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                name: row.get(2)?,
                trigger: row.get(3)?,
                skill_type: row.get(4)?,
                description: row.get(5)?,
                content: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        let mut skills = Vec::new();
        for row in rows {
            skills.push(row?);
        }
        Ok(skills)
    }

    /// Get a single skill by id.
    pub fn forge_get_skill(&self, id: &str) -> Result<Option<ForgeSkill>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, name, trigger, skill_type, description, content, created_at
             FROM db_forge_skills WHERE id=?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            Ok(ForgeSkill {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                name: row.get(2)?,
                trigger: row.get(3)?,
                skill_type: row.get(4)?,
                description: row.get(5)?,
                content: row.get(6)?,
                created_at: row.get(7)?,
            })
        });
        match result {
            Ok(skill) => Ok(Some(skill)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Insert a new skill.
    pub fn forge_insert_skill(&self, skill: &ForgeSkill) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_forge_skills (id, agent_id, name, trigger, skill_type, description, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                skill.id,
                skill.agent_id,
                skill.name,
                skill.trigger,
                skill.skill_type,
                skill.description,
                skill.content,
                skill.created_at
            ],
        )?;
        Ok(())
    }

    /// Update an existing skill (all fields except id, agent_id, created_at).
    pub fn forge_update_skill(&self, skill: &ForgeSkill) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_forge_skills SET name=?1, trigger=?2, skill_type=?3, description=?4, content=?5
             WHERE id=?6",
            params![
                skill.name,
                skill.trigger,
                skill.skill_type,
                skill.description,
                skill.content,
                skill.id
            ],
        )?;
        Ok(rows > 0)
    }

    /// Delete a skill by id. Returns true if a row was deleted.
    pub fn forge_delete_skill(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_forge_skills WHERE id=?1",
            params![id],
        )?;
        Ok(rows > 0)
    }

    // ---- ForgeHistory methods ----

    /// Append a history entry for an agent. Auto-sets session_date (today) and timestamp.
    pub fn forge_append_history(&self, agent_id: &str, entry: &str) -> Result<ForgeHistory, StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // session_date as YYYY-MM-DD
        let secs = (now / 1000) as u64;
        let days = secs / 86400;
        // Simple date calculation (no chrono dependency needed)
        let session_date = format_epoch_date(days);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_forge_history (agent_id, session_date, entry, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, session_date, entry, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(ForgeHistory {
            id,
            agent_id: agent_id.to_string(),
            session_date,
            entry: entry.to_string(),
            timestamp: now,
        })
    }

    /// List history entries for an agent, with optional date filter and pagination.
    pub fn forge_list_history(
        &self,
        agent_id: &str,
        session_date: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ForgeHistory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match session_date {
            Some(date) => (
                "SELECT id, agent_id, session_date, entry, timestamp
                 FROM db_forge_history WHERE agent_id=?1 AND session_date=?2
                 ORDER BY timestamp DESC LIMIT ?3 OFFSET ?4".to_string(),
                vec![
                    Box::new(agent_id.to_string()),
                    Box::new(date.to_string()),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
            None => (
                "SELECT id, agent_id, session_date, entry, timestamp
                 FROM db_forge_history WHERE agent_id=?1
                 ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3".to_string(),
                vec![
                    Box::new(agent_id.to_string()),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
        };
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(ForgeHistory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                session_date: row.get(2)?,
                entry: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Search history entries for an agent using LIKE-based matching.
    pub fn forge_search_history(
        &self,
        agent_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ForgeHistory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, session_date, entry, timestamp
             FROM db_forge_history WHERE agent_id=?1 AND entry LIKE ?2
             ORDER BY timestamp DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![agent_id, pattern, limit], |row| {
            Ok(ForgeHistory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                session_date: row.get(2)?,
                entry: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }
}

/// Format days-since-epoch as YYYY-MM-DD string.
/// Simple implementation without chrono dependency.
fn format_epoch_date(days_since_epoch: u64) -> String {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

// ====================================================================
// Identity Accounts + Agent Instances + Junction
// (Phase 2 of SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md)
// ====================================================================

/// Provider-specific credential reference. Stored as JSON in
/// `db_identity_accounts.secret_ref`. `backend` is the discriminator.
/// The actual secret value is NEVER stored in the DB — only how to
/// look it up at launch time (env var, secrets-manager path, etc.).
/// `PlaintextDev` exists for local dev convenience and must never be
/// the default path in production builds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum SecretRef {
    Env {
        env_var: String,
    },
    SecretsManager {
        sm_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sm_json_path: Option<String>,
    },
    PlaintextDev {
        plaintext_dev: String,
    },
}

/// An identity account (reusable credential, linked to agents via the
/// `db_forge_agent_identities` junction). Replaces the browser
/// localStorage identity store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityAccount {
    pub id: String,
    pub name: String,
    pub provider: String, // "github" | "aws" | "anthropic" | "custom"
    pub kind: String,     // "pat" | "role" | "api_key" | "env_ref"
    #[serde(default)]
    pub display_name: String,
    pub secret_ref: SecretRef,
    /// Free-form JSON context (username, scopes, role ARN, etc.). Stored
    /// verbatim; frontend types it by `provider`.
    #[serde(default = "default_context_json")]
    pub context: serde_json::Value,
    #[serde(default = "default_identity_status")]
    pub status: String, // "unknown" | "ok" | "expired" | "invalid"
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_context_json() -> serde_json::Value {
    serde_json::json!({})
}

fn default_identity_status() -> String {
    "unknown".to_string()
}

/// Junction row: which identity an agent uses for a given provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeAgentIdentity {
    pub agent_id: String,
    pub account_id: String,
    pub provider: String,
}

// ====================================================================
// v7 — Identity bundles (named credential bundles) + Memory bundles
// ====================================================================

/// A named credential bundle. Contains zero or more accounts via the
/// `db_identity_bindings` junction. `is_blank` tags the seeded singleton
/// row that the launch UI uses as the default option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_blank: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Junction row binding an account to an identity for a given provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityBinding {
    pub identity_id: String,
    pub provider: String,
    pub account_id: String,
}

/// A Memory bundle — the agent's personality and capability stack.
/// Provider, model, instructions, and JSON-encoded arrays of context
/// files / MCP servers / skills. Forge agents shadow-migrate into this
/// table during the v7 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_blank: bool,
    /// "claude" | "codex" | "gemini" | empty string
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub instructions: String,
    /// JSON-encoded array; the renderer types it as `[{path, content}]`.
    #[serde(default = "default_json_array_string")]
    pub context_files: String,
    /// JSON-encoded array of MCP server configs.
    #[serde(default = "default_json_array_string")]
    pub mcp_servers: String,
    /// JSON-encoded array of skill IDs.
    #[serde(default = "default_json_array_string")]
    pub skills: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_json_array_string() -> String {
    "[]".to_string()
}

/// Instance lifecycle status. Serialised lowercase to match the DB text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus {
    Running,
    Paused,
    Stopped,
    Crashed,
    Detached,
}

impl InstanceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Crashed => "crashed",
            Self::Detached => "detached",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "stopped" => Some(Self::Stopped),
            "crashed" => Some(Self::Crashed),
            "detached" => Some(Self::Detached),
            _ => None,
        }
    }
}

/// Optional context describing which GitHub-side unit of work a specific
/// instance is operating on. Stored as JSON in
/// `db_agent_instances.github_context` (empty string when unset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubContext {
    pub repo: String, // "owner/repo"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<u64>,
}

/// One row per running/historical execution of an agent definition.
/// `block_id` / `session_id` / `github_context` are modelled as empty
/// strings on the wire rather than `Option<String>` to match the
/// existing forge schema conventions (`NOT NULL DEFAULT ''`). Callers
/// that need structured absence can use `.is_empty()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: String,
    pub definition_id: String,
    #[serde(default)]
    pub parent_instance_id: String,
    #[serde(default)]
    pub block_id: String,
    #[serde(default)]
    pub session_id: String,
    pub status: String,
    /// JSON-encoded `GitHubContext`, or empty string.
    #[serde(default)]
    pub github_context: String,
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: i64,
    pub created_at: i64,
    /// FK to `db_identities.id`. Empty string means "use the blank
    /// singleton" (= ambient creds, no env-var injection). Set at
    /// instantiation via the launch modal's Identity dropdown.
    #[serde(default)]
    pub identity_id: String,
    /// FK to `db_memories.id`. Empty string means "use the blank
    /// singleton" (= vanilla CLI, no instructions). Set at
    /// instantiation via the launch modal's Memory dropdown.
    #[serde(default)]
    pub memory_id: String,
}

impl WaveStore {
    // ---- Identity account CRUD ----

    /// List identity accounts. If `provider` is `Some`, filter to that
    /// provider; otherwise return every account, ordered by most recent
    /// update first (so the identity panel shows live accounts on top).
    pub fn identity_list(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<IdentityAccount>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut rows_vec = Vec::new();
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<IdentityAccount> {
            let secret_ref_json: String = row.get(5)?;
            let context_json: String = row.get(6)?;
            Ok(IdentityAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                display_name: row.get(4)?,
                secret_ref: serde_json::from_str(&secret_ref_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                context: serde_json::from_str(&context_json).unwrap_or_else(|_| serde_json::json!({})),
                status: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        };
        match provider {
            Some(p) => {
                let mut stmt = conn.prepare(
                    "SELECT id, name, provider, kind, display_name, secret_ref, context,
                            status, created_at, updated_at
                     FROM db_identity_accounts
                     WHERE provider = ?1
                     ORDER BY updated_at DESC",
                )?;
                let iter = stmt.query_map(params![p], map_row)?;
                for r in iter {
                    rows_vec.push(r?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, name, provider, kind, display_name, secret_ref, context,
                            status, created_at, updated_at
                     FROM db_identity_accounts
                     ORDER BY updated_at DESC",
                )?;
                let iter = stmt.query_map([], map_row)?;
                for r in iter {
                    rows_vec.push(r?);
                }
            }
        }
        Ok(rows_vec)
    }

    pub fn identity_get(&self, id: &str) -> Result<Option<IdentityAccount>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, provider, kind, display_name, secret_ref, context,
                    status, created_at, updated_at
             FROM db_identity_accounts WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            let secret_ref_json: String = row.get(5)?;
            let context_json: String = row.get(6)?;
            Ok(IdentityAccount {
                id: row.get(0)?,
                name: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                display_name: row.get(4)?,
                secret_ref: serde_json::from_str(&secret_ref_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                context: serde_json::from_str(&context_json).unwrap_or_else(|_| serde_json::json!({})),
                status: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        });
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert an identity account. If `account.id` is empty the caller
    /// must generate one first (we don't silently mint ids here — callers
    /// should know whether they're creating vs updating).
    pub fn identity_upsert(&self, account: &IdentityAccount) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let secret_ref_json = serde_json::to_string(&account.secret_ref)?;
        let context_json = serde_json::to_string(&account.context)?;
        conn.execute(
            "INSERT INTO db_identity_accounts
                (id, name, provider, kind, display_name, secret_ref, context,
                 status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                provider = excluded.provider,
                kind = excluded.kind,
                display_name = excluded.display_name,
                secret_ref = excluded.secret_ref,
                context = excluded.context,
                status = excluded.status,
                updated_at = excluded.updated_at",
            params![
                account.id,
                account.name,
                account.provider,
                account.kind,
                account.display_name,
                secret_ref_json,
                context_json,
                account.status,
                account.created_at,
                account.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn identity_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_identity_accounts WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    // ---- Agent ↔ Identity junction ----

    /// Link an agent to an identity for a given provider. Overwrites any
    /// existing link for the same (agent_id, provider) — each agent has
    /// at most one account per provider.
    pub fn agent_identity_link(
        &self,
        agent_id: &str,
        account_id: &str,
        provider: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_forge_agent_identities (agent_id, account_id, provider)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, provider) DO UPDATE SET account_id = excluded.account_id",
            params![agent_id, account_id, provider],
        )?;
        Ok(())
    }

    /// Remove the identity link for a given (agent_id, provider).
    /// Returns true iff a link existed.
    pub fn agent_identity_unlink(
        &self,
        agent_id: &str,
        provider: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_forge_agent_identities WHERE agent_id = ?1 AND provider = ?2",
            params![agent_id, provider],
        )?;
        Ok(rows > 0)
    }

    /// List all (agent_id, account_id, provider) triples for an agent.
    pub fn agent_identity_list_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Vec<ForgeAgentIdentity>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, account_id, provider
             FROM db_forge_agent_identities
             WHERE agent_id = ?1
             ORDER BY provider",
        )?;
        let iter = stmt.query_map(params![agent_id], |row| {
            Ok(ForgeAgentIdentity {
                agent_id: row.get(0)?,
                account_id: row.get(1)?,
                provider: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    // ---- Agent instance CRUD ----

    /// List instances. Both filters are optional — pass `None` to scan all
    /// instances. Ordered by `created_at` descending (most recent first).
    pub fn instance_list(
        &self,
        definition_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Build the query dynamically. The parameter count varies, so we
        // can't reuse a single prepared statement across filter combos.
        let mut sql = String::from(
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id
             FROM db_agent_instances",
        );
        let mut clauses: Vec<&str> = Vec::new();
        if definition_id.is_some() {
            clauses.push("definition_id = ?");
        }
        if status.is_some() {
            clauses.push("status = ?");
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = conn.prepare(&sql)?;
        // Build a Vec<String> so parameter lifetimes outlive the query call.
        // Borrowing the `&str` args directly caused E0597 because they're
        // bound to the match arms, not the outer scope.
        let mut param_vals: Vec<String> = Vec::new();
        if let Some(d) = definition_id {
            param_vals.push(d.to_string());
        }
        if let Some(s) = status {
            param_vals.push(s.to_string());
        }
        let iter = stmt.query_map(rusqlite::params_from_iter(param_vals.iter()), map_instance_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn instance_get(&self, id: &str) -> Result<Option<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id
             FROM db_agent_instances WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], map_instance_row);
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Insert a new instance row. Caller is responsible for the id (UUID).
    pub fn instance_create(&self, inst: &AgentInstance) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_instances
                (id, definition_id, parent_instance_id, block_id, session_id, status,
                 github_context, started_at, ended_at, created_at,
                 identity_id, memory_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                inst.id,
                inst.definition_id,
                inst.parent_instance_id,
                inst.block_id,
                inst.session_id,
                inst.status,
                inst.github_context,
                inst.started_at,
                inst.ended_at,
                inst.created_at,
                inst.identity_id,
                inst.memory_id,
            ],
        )?;
        Ok(())
    }

    /// Update mutable instance fields. `id`, `definition_id`,
    /// `parent_instance_id`, `started_at`, `created_at` are immutable
    /// after insert (they describe provenance, not state).
    pub fn instance_update(&self, inst: &AgentInstance) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_agent_instances SET
                block_id = ?1,
                session_id = ?2,
                status = ?3,
                github_context = ?4,
                ended_at = ?5
             WHERE id = ?6",
            params![
                inst.block_id,
                inst.session_id,
                inst.status,
                inst.github_context,
                inst.ended_at,
                inst.id,
            ],
        )?;
        Ok(rows > 0)
    }

    pub fn instance_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_agent_instances WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Look up the most-recently-created **active** instance for a
    /// block — `active` = status in (running, paused). Stopped and
    /// crashed instances are skipped: when a user closes a pane and
    /// re-opens it (creating a fresh instance), the resolver should
    /// see the NEW row's identity_id / memory_id, not bleed creds
    /// from the prior stopped one. Reagent P2 (PR #751).
    pub fn instance_get_active_for_block(
        &self,
        block_id: &str,
    ) -> Result<Option<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id
             FROM db_agent_instances
             WHERE block_id = ?1 AND status IN ('running', 'paused')
             ORDER BY created_at DESC
             LIMIT 1",
        )?;
        let result = stmt.query_row(params![block_id], map_instance_row);
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // ====================================================================
    // v7 — Identity bundles (named credential bundles) + Memory bundles
    // ====================================================================

    // ---- Identity bundle CRUD ----

    /// List all Identity bundles, blank singleton last so the picker shows
    /// user-defined bundles first.
    pub fn bundle_identity_list(&self) -> Result<Vec<Identity>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, created_at, updated_at
             FROM db_identities
             ORDER BY is_blank ASC, updated_at DESC",
        )?;
        let iter = stmt.query_map([], |row| {
            Ok(Identity {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_blank: row.get::<_, i64>(3)? != 0,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn bundle_identity_get(&self, id: &str) -> Result<Option<Identity>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, created_at, updated_at
             FROM db_identities WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            Ok(Identity {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_blank: row.get::<_, i64>(3)? != 0,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        });
        match result {
            Ok(i) => Ok(Some(i)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert an Identity bundle. Caller mints the id (no silent generation).
    /// The `is_blank` flag is reserved for the seeded singleton — callers
    /// should pass `false` for user-created identities.
    pub fn bundle_identity_upsert(&self, identity: &Identity) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_identities
                (id, name, description, is_blank, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                updated_at = excluded.updated_at",
            params![
                identity.id,
                identity.name,
                identity.description,
                identity.is_blank as i64,
                identity.created_at,
                identity.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Delete an Identity bundle. Refuses to delete the blank singleton —
    /// the launch UI depends on it as the always-present default option.
    pub fn bundle_identity_delete(&self, id: &str) -> Result<bool, StoreError> {
        if id == "blank" {
            return Err(StoreError::Other(
                "cannot delete the blank Identity singleton".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_identities WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    // ---- Identity bundle bindings (junction with accounts) ----

    /// Set the account for `(identity_id, provider)`. Overwrites any
    /// existing binding for the same (identity, provider).
    pub fn bundle_identity_bind(
        &self,
        identity_id: &str,
        provider: &str,
        account_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_identity_bindings (identity_id, provider, account_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(identity_id, provider) DO UPDATE SET account_id = excluded.account_id",
            params![identity_id, provider, account_id],
        )?;
        Ok(())
    }

    /// Remove the binding for `(identity_id, provider)`. Returns whether a
    /// row was deleted.
    pub fn bundle_identity_unbind(
        &self,
        identity_id: &str,
        provider: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_identity_bindings WHERE identity_id = ?1 AND provider = ?2",
            params![identity_id, provider],
        )?;
        Ok(rows > 0)
    }

    /// List bindings for an Identity bundle.
    pub fn bundle_identity_bindings(
        &self,
        identity_id: &str,
    ) -> Result<Vec<IdentityBinding>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT identity_id, provider, account_id
             FROM db_identity_bindings
             WHERE identity_id = ?1
             ORDER BY provider ASC",
        )?;
        let iter = stmt.query_map(params![identity_id], |row| {
            Ok(IdentityBinding {
                identity_id: row.get(0)?,
                provider: row.get(1)?,
                account_id: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    // ---- Memory bundle CRUD ----

    pub fn bundle_memory_list(&self) -> Result<Vec<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, provider, model, instructions,
                    context_files, mcp_servers, skills, created_at, updated_at
             FROM db_memories
             ORDER BY is_blank ASC, updated_at DESC",
        )?;
        let iter = stmt.query_map([], map_memory_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn bundle_memory_get(&self, id: &str) -> Result<Option<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, provider, model, instructions,
                    context_files, mcp_servers, skills, created_at, updated_at
             FROM db_memories WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], map_memory_row);
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn bundle_memory_upsert(&self, memory: &Memory) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_memories
                (id, name, description, is_blank, provider, model, instructions,
                 context_files, mcp_servers, skills, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                provider = excluded.provider,
                model = excluded.model,
                instructions = excluded.instructions,
                context_files = excluded.context_files,
                mcp_servers = excluded.mcp_servers,
                skills = excluded.skills,
                updated_at = excluded.updated_at",
            params![
                memory.id,
                memory.name,
                memory.description,
                memory.is_blank as i64,
                memory.provider,
                memory.model,
                memory.instructions,
                memory.context_files,
                memory.mcp_servers,
                memory.skills,
                memory.created_at,
                memory.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Delete a Memory bundle. Refuses to delete the blank singleton.
    pub fn bundle_memory_delete(&self, id: &str) -> Result<bool, StoreError> {
        if id == "blank" {
            return Err(StoreError::Other(
                "cannot delete the blank Memory singleton".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_memories WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }
}

fn map_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        is_blank: row.get::<_, i64>(3)? != 0,
        provider: row.get(4)?,
        model: row.get(5)?,
        instructions: row.get(6)?,
        context_files: row.get(7)?,
        mcp_servers: row.get(8)?,
        skills: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn map_instance_row(row: &rusqlite::Row) -> rusqlite::Result<AgentInstance> {
    Ok(AgentInstance {
        id: row.get(0)?,
        definition_id: row.get(1)?,
        parent_instance_id: row.get(2)?,
        block_id: row.get(3)?,
        session_id: row.get(4)?,
        status: row.get(5)?,
        github_context: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        created_at: row.get(9)?,
        identity_id: row.get(10)?,
        memory_id: row.get(11)?,
    })
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::*;

    fn make_store() -> WaveStore {
        WaveStore::open_in_memory().unwrap()
    }

    #[test]
    fn test_insert_and_get_client() {
        let store = make_store();
        let mut client = Client {
            oid: "test-client-oid".to_string(),
            version: 0,
            windowids: vec!["w1".to_string()],
            meta: MetaMapType::new(),
            tosagreed: 1700000000000,
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert_eq!(client.get_version(), 1);

        let loaded = store.must_get::<Client>("test-client-oid").unwrap();
        assert_eq!(loaded.oid, "test-client-oid");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.windowids, vec!["w1"]);
        assert_eq!(loaded.tosagreed, 1700000000000);
    }

    #[test]
    fn test_insert_and_get_window() {
        let store = make_store();
        let mut win = Window {
            oid: "win-1".to_string(),
            workspaceid: "ws-1".to_string(),
            pos: Point { x: 10, y: 20 },
            winsize: WinSize {
                width: 800,
                height: 600,
            },
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut win).unwrap();

        let loaded = store.must_get::<Window>("win-1").unwrap();
        assert_eq!(loaded.workspaceid, "ws-1");
        assert_eq!(loaded.pos.x, 10);
        assert_eq!(loaded.winsize.width, 800);
    }

    #[test]
    fn test_insert_and_get_workspace() {
        let store = make_store();
        let mut ws = Workspace {
            oid: "ws-1".to_string(),
            name: "Test WS".to_string(),
            tabids: vec!["t1".to_string()],
            activetabid: "t1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut ws).unwrap();

        let loaded = store.must_get::<Workspace>("ws-1").unwrap();
        assert_eq!(loaded.name, "Test WS");
        assert_eq!(loaded.tabids, vec!["t1"]);
    }

    #[test]
    fn test_insert_and_get_tab() {
        let store = make_store();
        let mut tab = Tab {
            oid: "tab-1".to_string(),
            name: "Shell".to_string(),
            layoutstate: "ls-1".to_string(),
            blockids: vec!["b1".to_string()],
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut tab).unwrap();

        let loaded = store.must_get::<Tab>("tab-1").unwrap();
        assert_eq!(loaded.name, "Shell");
    }

    #[test]
    fn test_insert_and_get_block() {
        let store = make_store();
        let mut block = Block {
            oid: "blk-1".to_string(),
            parentoref: "tab:tab-1".to_string(),
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".into(), serde_json::json!("term"));
                m
            },
            ..Default::default()
        };
        store.insert(&mut block).unwrap();

        let loaded = store.must_get::<Block>("blk-1").unwrap();
        assert_eq!(loaded.parentoref, "tab:tab-1");
        assert_eq!(loaded.meta.get("view").unwrap(), "term");
    }

    #[test]
    fn test_insert_and_get_layout_state() {
        let store = make_store();
        // Phase E.4.B Phase 2 — uses typed LayoutNode (was a junk JSON blob).
        let mut ls = LayoutState {
            oid: "ls-1".to_string(),
            rootnode: Some(crate::backend::obj::LayoutNode {
                id: "n1".into(),
                flex_direction: crate::backend::obj::FlexDirection::Row,
                size: 1.0,
                children: Vec::new(),
                data: None,
                ..Default::default()
            }),
            magnifiednodeid: "n1".to_string(),
            ..Default::default()
        };
        store.insert(&mut ls).unwrap();

        let loaded = store.must_get::<LayoutState>("ls-1").unwrap();
        assert_eq!(loaded.magnifiednodeid, "n1");
        assert!(loaded.rootnode.is_some());
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let store = make_store();
        let result = store.get::<Client>("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_must_get_nonexistent_returns_error() {
        let store = make_store();
        let result = store.must_get::<Client>("nonexistent");
        assert!(matches!(result, Err(StoreError::NotFound)));
    }

    #[test]
    fn test_update_increments_version() {
        let store = make_store();
        let mut client = Client {
            oid: "c1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert_eq!(client.version, 1);

        client.windowids = vec!["w-new".to_string()];
        let v2 = store.update(&mut client).unwrap();
        assert_eq!(v2, 2);
        assert_eq!(client.version, 2);

        let v3 = store.update(&mut client).unwrap();
        assert_eq!(v3, 3);
    }

    #[test]
    fn test_delete() {
        let store = make_store();
        let mut client = Client {
            oid: "del-me".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut client).unwrap();
        assert!(store.get::<Client>("del-me").unwrap().is_some());

        store.delete::<Client>("del-me").unwrap();
        assert!(store.get::<Client>("del-me").unwrap().is_none());
    }

    #[test]
    fn test_get_all() {
        let store = make_store();
        for i in 0..3 {
            let mut tab = Tab {
                oid: format!("tab-{i}"),
                name: format!("Tab {i}"),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            store.insert(&mut tab).unwrap();
        }

        let all = store.get_all::<Tab>().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_count() {
        let store = make_store();
        assert_eq!(store.count::<Client>().unwrap(), 0);

        let mut c = Client {
            oid: "c1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut c).unwrap();
        assert_eq!(store.count::<Client>().unwrap(), 1);
    }

    #[test]
    fn test_insert_empty_oid_fails() {
        let store = make_store();
        let mut client = Client {
            oid: String::new(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        let result = store.insert(&mut client);
        assert!(matches!(result, Err(StoreError::EmptyOID)));
    }

    #[test]
    fn test_with_tx_commits_on_success() {
        let store = make_store();
        store
            .with_tx(|tx| {
                let mut ws = Workspace {
                    oid: "ws-tx".to_string(),
                    name: "TX Workspace".to_string(),
                    meta: MetaMapType::new(),
                    ..Default::default()
                };
                tx.insert(&mut ws)?;

                let mut tab = Tab {
                    oid: "tab-tx".to_string(),
                    // tabN naming convention per SPEC_TAB_GAPS_AND_NAMING_2026_04_25.
                    name: "tab1".to_string(),
                    layoutstate: "ls-tx".to_string(),
                    meta: MetaMapType::new(),
                    ..Default::default()
                };
                tx.insert(&mut tab)?;

                // Update workspace to reference tab
                ws.tabids.push("tab-tx".to_string());
                tx.update(&mut ws)?;

                Ok(())
            })
            .unwrap();

        // Verify everything committed
        let ws = store.must_get::<Workspace>("ws-tx").unwrap();
        assert_eq!(ws.name, "TX Workspace");
        assert_eq!(ws.tabids, vec!["tab-tx"]);
        assert_eq!(ws.version, 2); // insert=v1, update=v2

        let tab = store.must_get::<Tab>("tab-tx").unwrap();
        assert_eq!(tab.name, "tab1");
    }

    #[test]
    fn test_with_tx_rollbacks_on_error() {
        let store = make_store();
        let result: Result<(), StoreError> = store.with_tx(|tx| {
            let mut ws = Workspace {
                oid: "ws-rollback".to_string(),
                name: "Should Not Exist".to_string(),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            tx.insert(&mut ws)?;

            // Force an error
            Err(StoreError::Other("intentional failure".to_string()))
        });
        assert!(result.is_err());

        // Verify the insert was rolled back
        let ws = store.get::<Workspace>("ws-rollback").unwrap();
        assert!(ws.is_none());
    }

    #[test]
    fn test_forge_insert_collision_resolves_at_runtime() {
        // Two agents whose names derive to the same slug must both
        // insert successfully, with the second getting a `-2` suffix.
        // This exercises the runtime collision-resolution path in
        // forge_insert (separate from the migration backfill path
        // tested in migrations.rs).
        let store = make_store();

        let mut a1 = ForgeAgent {
            id: "id-a".to_string(),
            slug: String::new(),
            name: "Agent X".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        store.forge_insert(&mut a1).unwrap();
        // "Agent X" → "agent-x"
        assert_eq!(a1.slug, "agent-x");

        let mut a2 = ForgeAgent {
            id: "id-b".to_string(),
            // Different surface form, derives to the same slug
            name: "agent x".to_string(),
            ..a1.clone()
        };
        a2.slug = String::new();
        store.forge_insert(&mut a2).unwrap();
        assert_eq!(a2.slug, "agent-x-2");

        let mut a3 = ForgeAgent {
            id: "id-c".to_string(),
            name: "AGENT-X".to_string(),
            ..a1.clone()
        };
        a3.slug = String::new();
        store.forge_insert(&mut a3).unwrap();
        assert_eq!(a3.slug, "agent-x-3");

        // Verify the underlying rows actually got written
        let listed = store.forge_list().unwrap();
        let slugs: Vec<&str> = listed.iter().map(|a| a.slug.as_str()).collect();
        assert!(slugs.contains(&"agent-x"));
        assert!(slugs.contains(&"agent-x-2"));
        assert!(slugs.contains(&"agent-x-3"));
    }

    #[test]
    fn test_forge_insert_explicit_slug_collision_resolves() {
        // When a caller passes an explicit (non-empty) slug that
        // already exists, forge_insert still resolves the collision
        // via suffixing — guards against the seed pre-loading the
        // same slug twice or any other "I know the slug" path.
        let store = make_store();

        let mut a1 = ForgeAgent {
            id: "id-a".to_string(),
            slug: "explicit".to_string(),
            name: "First".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        };
        store.forge_insert(&mut a1).unwrap();
        assert_eq!(a1.slug, "explicit");

        let mut a2 = ForgeAgent {
            id: "id-b".to_string(),
            ..a1.clone()
        };
        a2.slug = "explicit".to_string();
        store.forge_insert(&mut a2).unwrap();
        assert_eq!(a2.slug, "explicit-2");
    }

    // ---- v6 identity / instance CRUD ----

    fn v6_test_store() -> WaveStore {
        WaveStore::open_in_memory().unwrap()
    }

    fn sample_account(id: &str, provider: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.to_string(),
            name: format!("asaf-{provider}"),
            provider: provider.to_string(),
            kind: "pat".to_string(),
            display_name: "".to_string(),
            secret_ref: SecretRef::Env { env_var: format!("{}_TOKEN", provider.to_uppercase()) },
            context: serde_json::json!({"username": "asaf"}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn sample_agent(id: &str, slug: &str) -> ForgeAgent {
        ForgeAgent {
            id: id.to_string(),
            slug: slug.to_string(),
            name: id.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "".to_string(),
            working_directory: "".to_string(),
            shell: "".to_string(),
            provider_flags: "".to_string(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: "".to_string(),
            agent_bus_id: "".to_string(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
        }
    }

    #[test]
    fn test_identity_upsert_round_trip() {
        let store = v6_test_store();
        let acct = sample_account("id-gh", "github");
        store.identity_upsert(&acct).unwrap();

        let fetched = store.identity_get("id-gh").unwrap().expect("row");
        assert_eq!(fetched.name, "asaf-github");
        assert_eq!(fetched.provider, "github");
        assert!(matches!(fetched.secret_ref, SecretRef::Env { ref env_var } if env_var == "GITHUB_TOKEN"));
        assert_eq!(fetched.context["username"], "asaf");
    }

    #[test]
    fn test_identity_list_filtered_by_provider() {
        let store = v6_test_store();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        store.identity_upsert(&sample_account("id-aws", "aws")).unwrap();
        store.identity_upsert(&sample_account("id-gh2", "github")).unwrap();

        let all = store.identity_list(None).unwrap();
        assert_eq!(all.len(), 3);
        let gh = store.identity_list(Some("github")).unwrap();
        assert_eq!(gh.len(), 2);
        assert!(gh.iter().all(|a| a.provider == "github"));
    }

    #[test]
    fn test_identity_delete() {
        let store = v6_test_store();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        assert!(store.identity_delete("id-gh").unwrap());
        assert!(store.identity_get("id-gh").unwrap().is_none());
        // Second delete is a no-op.
        assert!(!store.identity_delete("id-gh").unwrap());
    }

    #[test]
    fn test_agent_identity_link_and_unlink() {
        let store = v6_test_store();
        let mut agent = sample_agent("ag1", "agent-x");
        store.forge_insert(&mut agent).unwrap();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();

        store.agent_identity_link("ag1", "id-gh", "github").unwrap();
        let links = store.agent_identity_list_for_agent("ag1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].account_id, "id-gh");

        // Re-link with a different account overwrites (one account per provider per agent)
        store.identity_upsert(&sample_account("id-gh2", "github")).unwrap();
        store.agent_identity_link("ag1", "id-gh2", "github").unwrap();
        let links = store.agent_identity_list_for_agent("ag1").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].account_id, "id-gh2");

        assert!(store.agent_identity_unlink("ag1", "github").unwrap());
        assert!(store.agent_identity_list_for_agent("ag1").unwrap().is_empty());
    }

    #[test]
    fn test_agent_identity_cascade_on_agent_delete() {
        let store = v6_test_store();
        let mut agent = sample_agent("ag1", "agent-x");
        store.forge_insert(&mut agent).unwrap();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        store.agent_identity_link("ag1", "id-gh", "github").unwrap();

        store.forge_delete("ag1").unwrap();
        assert!(store.agent_identity_list_for_agent("ag1").unwrap().is_empty());
    }

    #[test]
    fn test_instance_create_update_filter() {
        let store = v6_test_store();
        let mut agent = sample_agent("def1", "agent-x");
        store.forge_insert(&mut agent).unwrap();

        let inst = AgentInstance {
            id: "inst1".to_string(),
            definition_id: "def1".to_string(),
            parent_instance_id: String::new(),
            block_id: "block-abc".to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
        };
        store.instance_create(&inst).unwrap();

        let fetched = store.instance_get("inst1").unwrap().expect("row");
        assert_eq!(fetched.block_id, "block-abc");
        assert_eq!(fetched.status, "running");

        // Update status → stopped
        let mut updated = fetched.clone();
        updated.status = InstanceStatus::Stopped.as_str().to_string();
        updated.ended_at = 2000;
        assert!(store.instance_update(&updated).unwrap());
        assert_eq!(store.instance_get("inst1").unwrap().unwrap().status, "stopped");

        // Filter queries
        let all = store.instance_list(None, None).unwrap();
        assert_eq!(all.len(), 1);
        let by_def = store.instance_list(Some("def1"), None).unwrap();
        assert_eq!(by_def.len(), 1);
        let running = store.instance_list(None, Some("running")).unwrap();
        assert_eq!(running.len(), 0);
        let stopped = store.instance_list(None, Some("stopped")).unwrap();
        assert_eq!(stopped.len(), 1);
    }

    #[test]
    fn test_instance_cascade_on_definition_delete() {
        let store = v6_test_store();
        let mut agent = sample_agent("def1", "agent-x");
        store.forge_insert(&mut agent).unwrap();
        let inst = AgentInstance {
            id: "inst1".to_string(),
            definition_id: "def1".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 0,
            ended_at: 0,
            created_at: 0,
            identity_id: String::new(),
            memory_id: String::new(),
        };
        store.instance_create(&inst).unwrap();

        store.forge_delete("def1").unwrap();
        assert!(store.instance_get("inst1").unwrap().is_none());
    }

    #[test]
    fn test_instance_status_enum_roundtrip() {
        for s in &[
            InstanceStatus::Running,
            InstanceStatus::Paused,
            InstanceStatus::Stopped,
            InstanceStatus::Crashed,
            InstanceStatus::Detached,
        ] {
            assert_eq!(Some(*s), InstanceStatus::parse(s.as_str()));
        }
        assert_eq!(None, InstanceStatus::parse("nonsense"));
    }

    // ── v7 — Identity bundle accessors ───────────────────────────────────

    #[test]
    fn test_bundle_identity_lifecycle() {
        let store = make_store();

        // Blank singleton always present.
        let initial = store.bundle_identity_list().unwrap();
        assert_eq!(initial.len(), 1);
        assert!(initial[0].is_blank);
        assert_eq!(initial[0].id, "blank");

        // Upsert a user identity.
        let work = Identity {
            id: "id-work".to_string(),
            name: "Work".to_string(),
            description: "Office laptop credentials".to_string(),
            is_blank: false,
            created_at: 100,
            updated_at: 100,
        };
        store.bundle_identity_upsert(&work).unwrap();

        // List orders user identities first, blank last.
        let listed = store.bundle_identity_list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "id-work");
        assert_eq!(listed[1].id, "blank");

        // Get round-trip.
        let fetched = store.bundle_identity_get("id-work").unwrap().unwrap();
        assert_eq!(fetched.name, "Work");

        // Refuse to delete the blank singleton.
        let blank_delete = store.bundle_identity_delete("blank");
        assert!(blank_delete.is_err());

        // Delete the user identity.
        assert!(store.bundle_identity_delete("id-work").unwrap());
        assert_eq!(store.bundle_identity_list().unwrap().len(), 1);
    }

    #[test]
    fn test_bundle_identity_bindings_round_trip() {
        let store = make_store();

        // Need an account to bind.
        let acct = IdentityAccount {
            id: "acct-1".to_string(),
            name: "asaf-github".to_string(),
            provider: "github".to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::Env {
                env_var: "GITHUB_TOKEN".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&acct).unwrap();

        let identity = Identity {
            id: "id-work".to_string(),
            name: "Work".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();

        // Bind, list, unbind.
        store
            .bundle_identity_bind("id-work", "github", "acct-1")
            .unwrap();
        let bindings = store.bundle_identity_bindings("id-work").unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].provider, "github");
        assert_eq!(bindings[0].account_id, "acct-1");

        // Re-bind same provider replaces the account.
        let acct2 = IdentityAccount {
            id: "acct-2".to_string(),
            name: "asaf-github-2".to_string(),
            provider: "github".to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::Env {
                env_var: "GITHUB_TOKEN_ALT".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&acct2).unwrap();
        store
            .bundle_identity_bind("id-work", "github", "acct-2")
            .unwrap();
        let rebound = store.bundle_identity_bindings("id-work").unwrap();
        assert_eq!(rebound.len(), 1);
        assert_eq!(rebound[0].account_id, "acct-2");

        // Unbind.
        assert!(store.bundle_identity_unbind("id-work", "github").unwrap());
        assert_eq!(
            store.bundle_identity_bindings("id-work").unwrap().len(),
            0
        );
    }

    #[test]
    fn test_bundle_identity_bindings_cascade_on_account_delete() {
        let store = make_store();

        let acct = IdentityAccount {
            id: "acct-1".to_string(),
            name: "asaf-github".to_string(),
            provider: "github".to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::Env {
                env_var: "GITHUB_TOKEN".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        store.identity_upsert(&acct).unwrap();

        let identity = Identity {
            id: "id-work".to_string(),
            name: "Work".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        store.bundle_identity_upsert(&identity).unwrap();
        store
            .bundle_identity_bind("id-work", "github", "acct-1")
            .unwrap();

        // Deleting the account cascades the binding row.
        store.identity_delete("acct-1").unwrap();
        assert_eq!(
            store.bundle_identity_bindings("id-work").unwrap().len(),
            0
        );
    }

    // ── v7 — Memory bundle accessors ─────────────────────────────────────

    #[test]
    fn test_bundle_memory_lifecycle() {
        let store = make_store();

        // Blank singleton always present.
        let initial = store.bundle_memory_list().unwrap();
        assert_eq!(initial.len(), 1);
        assert!(initial[0].is_blank);
        assert_eq!(initial[0].id, "blank");

        // Upsert a user memory.
        let coder = Memory {
            id: "mem-coder".to_string(),
            name: "Claude-coder".to_string(),
            description: "Pair-programming setup".to_string(),
            is_blank: false,
            provider: "claude".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            instructions: "You are a careful refactorer.".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            created_at: 100,
            updated_at: 100,
        };
        store.bundle_memory_upsert(&coder).unwrap();

        let listed = store.bundle_memory_list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "mem-coder");
        assert_eq!(listed[1].id, "blank");

        let fetched = store.bundle_memory_get("mem-coder").unwrap().unwrap();
        assert_eq!(fetched.provider, "claude");
        assert_eq!(fetched.instructions, "You are a careful refactorer.");

        // Refuse to delete the blank singleton.
        assert!(store.bundle_memory_delete("blank").is_err());

        // Delete the user memory.
        assert!(store.bundle_memory_delete("mem-coder").unwrap());
        assert_eq!(store.bundle_memory_list().unwrap().len(), 1);
    }
}

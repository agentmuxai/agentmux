// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Store: generic OID-based CRUD for StoreObj types.
//! Port of Go's pkg/wstore/wstore_dbops.go + wstore_dbsetup.go.
//!
//! Uses `Mutex<Connection>` matching Go's `MaxOpenConns(1)`.
//! SQLite WAL mode + 5s busy timeout (same as Go).


use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::backend::obj::{wave_obj_from_json, wave_obj_to_json, StoreObj};
use crate::registry::Registry;

use super::error::StoreError;
use super::migrations::{check_schema_compat, run_object_schema, stamp_version, OBJECT_SCHEMA_VERSION};

/// SQLite-backed object store for StoreObj types.
pub struct Store {
    conn: Mutex<Connection>,
    /// Cross-version named-agent registry. `None` for in-memory test
    /// stores; `Some` for production srv. Mutations to
    /// `db_agent_instances` parallel-write to this registry when set.
    /// See `docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md`.
    registry: Mutex<Option<Arc<Registry>>>,
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

    /// Crate-internal accessor for sibling modules that maintain their
    /// own per-table CRUD via the `DroneStore` extension trait
    /// pattern (see `agentmux-srv/src/drone/storage.rs`). Outside
    /// callers must use the typed methods on this impl.
    pub(crate) fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    /// Run the Phase 3a `db_agents` consolidation backfill under the
    /// wstore's exclusive connection lock. Idempotent — gated by a
    /// marker file in `data_dir` (skip with `None` for tests).
    pub fn run_agents_consolidate(
        &self,
        data_dir: Option<&Path>,
    ) -> Result<super::agents_consolidate::ConsolidateStats, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        super::agents_consolidate::run_consolidate_migration(&mut conn, data_dir)
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

    fn registry(&self) -> Option<Arc<Registry>> {
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Public accessor for the cross-version named-agent registry.
    /// Returns `None` when the registry couldn't be resolved at
    /// startup (CI / unusual envs); callers must handle the absent
    /// case by falling back to SQLite.
    pub fn shared_agent_registry(&self) -> Option<Arc<Registry>> {
        self.registry()
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

// ====================================================================
// AgentDefinition CRUD
// ====================================================================

/// A user-defined AI agent in the user's agent-definition catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
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
    /// JSON-encoded per-provider account assignments
    /// (`{"github":"acct-id", …}`). Written by the Agent pane's Identity
    /// tab (`AgentIdentityPanel`) via `updateagent`, read back by
    /// `parseAgentAccounts` and consumed by startup credential
    /// resolution. (An older v6 comment called this deprecated in favour
    /// of `db_agent_identity_links`; that migration never completed — the
    /// JSON blob is still the live store, so the schema flatten keeps the
    /// column.)
    #[serde(default)]
    pub accounts: String,
    /// Parent definition id (db_agent_definitions.id). Empty string = root
    /// definition; non-empty = this agent was forked from another.
    /// Added in v6. See spec §Phase 1.
    #[serde(default)]
    pub parent_id: String,
    /// Label describing the branch (e.g. `"pr-422-review"`,
    /// `"experiment-refactor"`). Empty for root definitions and for
    /// branches that didn't set a label. Added in v6.
    #[serde(default)]
    pub branch_label: String,
    /// Last-modified timestamp (epoch ms). Set to `created_at` on insert
    /// and refreshed on every `agent_def_update`. Schema v2. `0` for
    /// rows written before v2 (until next update).
    #[serde(default)]
    pub updated_at: i64,
    /// Per-user hide flag for seeded templates. `1` = the user clicked
    /// "Hide template" on the picker's `+ New from template` tier; the
    /// row stays on disk (templates are manifest-managed; deletion would
    /// fight re-seed) but the default `listagents` view filters it out.
    /// Reset to `0` by the agent-seed re-sync flow for any NEW template
    /// id newly added to the manifest, so a fresh template surfaces once
    /// even if a same-named one was previously hidden. Schema v3 (Phase
    /// 2 of `SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md` — Q2 Decision Y).
    /// User-owned rows (`is_seeded = 0`) MUST stay at `0` here; their
    /// removal path is `deleteagent`, not hide.
    #[serde(default)]
    pub user_hidden: i64,
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

/// A content blob associated with a agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContent {
    pub agent_id: String,
    pub content_type: String,
    pub content: String,
    pub updated_at: i64,
}

/// A reusable skill/capability attached to a agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub trigger: String,
    pub skill_type: String,
    pub description: String,
    pub content: String,
    pub created_at: i64,
}

/// An append-only session history entry for a agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHistory {
    pub id: i64,
    pub agent_id: String,
    pub session_date: String,
    pub entry: String,
    pub timestamp: i64,
}

impl Store {
    /// List all agent definitions, **most-recently-used first**.
    ///
    /// Phase 3b: read from the consolidated `db_agents` table. Order is
    /// most-recently-touched first (`updated_at DESC`) then stable on
    /// creation (`created_at ASC`). Dual-write keeps `db_agents.updated_at`
    /// fresh on every definition mutation AND every instance lifecycle
    /// touch, so this approximates the old "MAX(started_at)" ordering
    /// without joining the instances table — recency on a row tracks the
    /// last time the agent was either edited or launched.
    ///
    /// Result-set shape: every row in `db_agents` is returned. Under the
    /// fold rule (`agents_consolidate.rs`):
    ///   - Templates (`is_template = 1`) appear once each.
    ///   - User-cloned defs (`is_template = 0`, id matches old `db_agent_definitions.id`)
    ///     appear once each — instance bindings, when present, are folded
    ///     into this same row.
    ///   - Template-instances (`is_template = 0`, id matches old `db_agent_instances.id`)
    ///     appear once each as first-class user agents.
    /// `parent_id` on the returned struct is sourced from
    /// `db_agents.parent_template_id` — the dual-write preserves the
    /// legacy semantics (def.parent_id for user-clones, template id for
    /// instance projections), so existing handlers that look up the parent
    /// template by id continue to work.
    /// Find user-clone definitions for a given seeded template (rows
    /// in `db_agent_definitions` with `is_seeded = 0` and
    /// `parent_id = <template.id>`). Returns the most-recent-first.
    ///
    /// Reads `db_agent_definitions` directly — NOT the `db_agents`
    /// consolidated view — because the latter surfaces template-
    /// instance projection rows under the same
    /// `is_template = 0 AND parent_template_id = <tpl>` shape as
    /// user-clone defs, which would conflate two distinct things.
    ///
    /// Sole production caller today is the `template_promote`
    /// migration's "did the user delete the deterministic-id
    /// clone?" diagnostic logging and its tests. Kept public so
    /// follow-up callers (e.g. a cleanup pass that GCs orphaned
    /// pre-deterministic-id clones from earlier migration code)
    /// can use it without re-deriving the schema.
    pub fn user_clone_defs_for_template(
        &self,
        template_id: &str,
    ) -> Result<Vec<AgentDefinition>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_id, branch_label, updated_at,
                    user_hidden
             FROM db_agent_definitions
             WHERE is_seeded = 0 AND parent_id = ?1
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map(params![template_id], map_agent_definition_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Fetch a single agent definition by primary key. Reads
    /// `db_agent_definitions` directly (not the `db_agents`
    /// consolidated view that `agent_def_list` reads), so it
    /// returns user-clone definitions and seeded templates, never
    /// template-instance projection rows.
    ///
    /// Used by the `template_promote` migration's deterministic-id
    /// idempotency check (see
    /// `migrate_promote_template_sessions_v1`): every retry asks
    /// "does the promote-target clone for this template already
    /// exist?" and either reuses it or inserts it.
    pub fn agent_def_get(&self, id: &str) -> Result<Option<AgentDefinition>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_id, branch_label, updated_at,
                    user_hidden
             FROM db_agent_definitions
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_agent_definition_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn agent_def_list(&self) -> Result<Vec<AgentDefinition>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_template_id, branch_label, updated_at,
                    user_hidden
             FROM db_agents
             ORDER BY updated_at DESC, created_at ASC",
        )?;
        let rows = stmt.query_map([], map_agent_definition_row)?;
        let mut agents = Vec::new();
        for row in rows {
            agents.push(row?);
        }
        Ok(agents)
    }

    /// Count agent rows (used by seed engine to check if seeding is needed).
    /// Phase 3b: reads from the consolidated `db_agents` table — every
    /// definition AND every template-instance projection counts, mirroring
    /// the new shape of `agent_def_list`. The seed engine only cares about
    /// `== 0` to decide "fresh database, seed templates", so the broader
    /// count doesn't false-positive that branch (an empty db_agents
    /// guarantees an empty db_agent_definitions).
    pub fn agent_def_count(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_agents",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Delete all seeded agents (is_seeded=1). Used by reseed to clear built-in agents.
    pub fn agent_def_delete_seeded(&self) -> Result<usize, StoreError> {
        // Reagent P1 round 4 on #1013: capture the cascaded instance ids
        // BEFORE the FK cascade fires. The bulk delete on
        // `db_agent_definitions` triggers `ON DELETE CASCADE` on
        // `db_agent_instances` for every instance keyed off those
        // templates; once the cascade runs, we can't query them anymore,
        // and `db_agents` would be left holding orphaned instance
        // projections. Capture → delete → drop projections.
        let (rows, cascaded_inst_ids) = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT i.id FROM db_agent_instances i
                 INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                 WHERE d.is_seeded = 1",
            )?;
            let ids: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            let rows = conn.execute("DELETE FROM db_agent_definitions WHERE is_seeded=1", [])?;
            (rows, ids)
        };
        // Phase 3a dual-write (Phase 3b: errors propagate): drop template
        // projections AND any cascaded instance projections. User-clone
        // DEFINITION projections (`is_template = 0`, `id` is a def_id)
        // are NOT touched here — those persist as long as the underlying
        // user-clone def row in `db_agent_definitions` does.
        self.agents_dual_write_seeded_delete(&cascaded_inst_ids)?;
        Ok(rows)
    }

    /// Insert a new agent definition. Auto-derives slug from name if empty,
    /// resolves collisions by appending `-2`, `-3`, etc., and mutates
    /// `agent.slug` so the caller sees the resolved value (important
    /// for handlers that serialize the struct back to the frontend
    /// after insert).
    ///
    /// The collision check + insert run under a single mutex lock,
    /// so this is race-safe against concurrent inserts on the same
    /// connection.
    pub fn agent_def_insert(&self, agent: &mut AgentDefinition) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let base = if agent.slug.is_empty() {
            derive_slug(&agent.name)
        } else {
            agent.slug.clone()
        };
        // Collision-resolve: scan for existing slugs matching base or
        // base-N. Phase 3b reads slug uniqueness from `db_agents` — the
        // dual-write keeps every definition's slug mirrored there, and
        // the consolidated table also surfaces template-instance
        // projections, so a slug collision against an instance-derived
        // row is caught now too (under the legacy schema, instances
        // didn't have slugs at all, so this is a strict superset).
        let mut candidate = base.clone();
        let mut n: u32 = 2;
        loop {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM db_agents WHERE slug = ?1",
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
            "INSERT INTO db_agent_definitions (id, slug, name, icon, provider, description,
             working_directory, shell, provider_flags, auto_start, restart_on_crash,
             idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
             is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21, ?22)",
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
                // New definitions: updated_at == created_at.
                agent.created_at,
                // Phase 2 (hide templates): new rows start visible. The
                // user only hides via the explicit `agent_def_hide` RPC,
                // and the agent-seed re-sync forces user_hidden = 0 on
                // any newly-added template id anyway, so honouring the
                // caller-supplied value here is safe even when a stray
                // 1 sneaks through.
                agent.user_hidden,
            ],
        )?;
        // Persist the stamped updated_at before we leave the lock so the
        // dual-write helper sees the same value the SQL row carries.
        let stamped_updated_at = agent.created_at;
        drop(conn);
        let mut snapshot = agent.clone();
        snapshot.updated_at = stamped_updated_at;
        // Phase 3a dual-write (Phase 3b: errors propagate): mirror to
        // db_agents so readers see the new row immediately.
        self.agents_dual_write_definition_upsert(&snapshot)?;
        // Reagent P1 on #1013 round 2: do NOT mutate the caller's
        // `&mut AgentDefinition` here. The PR is supposed to be
        // zero-behaviour-change; the previous version reflected
        // `stamped_updated_at` back onto the caller, which is an
        // observable mutation downstream callers may rely on remaining
        // untouched. Callers that need the freshly-stamped value can
        // re-fetch the row via the normal read path.
        let _ = stamped_updated_at;
        Ok(())
    }

    /// Set the `user_hidden` flag on a single agent definition. Phase 2
    /// of the two-tier picker (Q2 Decision Y). Returns:
    ///   `Ok(true)`  — row updated.
    ///   `Ok(false)` — no row with that id exists.
    ///   `Err(...)`  — the row exists but is NOT a seeded template
    ///                 (`is_seeded != 1`). User-owned definitions go
    ///                 through `agent_def_delete`, not hide.
    ///
    /// Does NOT bump `updated_at`: hide is a per-user view-state flag,
    /// not a definition-content edit. Keeps `updated_at` faithful to the
    /// agent's payload (the manifest re-sync compares `description` etc.
    /// against the canonical row).
    pub fn agent_def_set_hidden(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Phase 3b: precondition check reads `is_template` from `db_agents`.
        // Templates carry `is_template = 1` and are the only rows allowed
        // to flip the hide flag; folded user-clone-def projections and
        // template-instance projections (both `is_template = 0`) reject.
        let is_template: i64 = match conn.query_row(
            "SELECT is_template FROM db_agents WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ) {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        if is_template != 1 {
            return Err(StoreError::Other(format!(
                "agent_def_set_hidden: {id} is not a seeded template (is_template={is_template}); \
                 user-owned definitions must use delete/archive paths, not hide"
            )));
        }
        // The hide flag is per-row; the write must still hit the legacy
        // table (cascade source) — dual-write mirrors into `db_agents`
        // for the next read.
        let rows = conn.execute(
            "UPDATE db_agent_definitions SET user_hidden = ?1 WHERE id = ?2",
            params![if hidden { 1_i64 } else { 0_i64 }, id],
        )?;
        if rows > 0 {
            // Mirror the flag into `db_agents` so the next read of the
            // template projection sees the update without waiting for a
            // round-trip through definition_upsert.
            if let Err(e) = conn.execute(
                "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 1",
                params![if hidden { 1_i64 } else { 0_i64 }, id],
            ) {
                tracing::error!(
                    id = %id,
                    hidden,
                    error = %e,
                    "db_agents dual-write: template hide flag mirror failed",
                );
            }
        }
        Ok(rows > 0)
    }

    /// Update an existing agent definition (all fields except id, created_at, is_seeded).
    /// `parent_id` and `branch_label` are NOT updatable post-insert — they
    /// describe the agent's provenance; renaming or re-branching is done by
    /// creating a new fork, not mutating the original.
    ///
    /// Self-stamps `updated_at` with the current time and writes it back into
    /// `agent.updated_at`, so the caller's struct (e.g. an RPC response body)
    /// reflects exactly what landed in the database.
    pub fn agent_def_update(&self, agent: &mut AgentDefinition) -> Result<bool, StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agent_definitions SET name=?1, icon=?2, provider=?3, description=?4,
                 working_directory=?5, shell=?6, provider_flags=?7, auto_start=?8,
                 restart_on_crash=?9, idle_timeout_minutes=?10,
                 agent_type=?11, environment=?12, agent_bus_id=?13, accounts=?14, updated_at=?15
                 WHERE id=?16",
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
                    now,
                    agent.id
                ],
            )?
        };
        // Reflect the persisted timestamp back to the caller's struct so an
        // RPC response carries the fresh value, not the pre-update one.
        agent.updated_at = now;
        // Phase 3a dual-write (Phase 3b: errors propagate): mirror to
        // db_agents so the next read sees the new name/payload.
        if rows > 0 {
            self.agents_dual_write_definition_upsert(agent)?;
        }
        Ok(rows > 0)
    }

    /// Delete a agent definition by id. Returns true if a row was deleted.
    pub fn agent_def_delete(&self, id: &str) -> Result<bool, StoreError> {
        // Snapshot cascaded instance ids AND issue the parent DELETE
        // under one lock acquisition so no thread can `instance_create`
        // a new row for this definition between the two steps. The
        // SQL DELETE's FK cascade fires inside SQLite while we hold
        // the mutex, so the snapshot exactly matches the rows that
        // got removed by the cascade.
        let (cascaded_instance_ids, rows) = {
            let conn = self.conn.lock().unwrap();
            let cascaded_instance_ids: Vec<String> = {
                let mut stmt = conn
                    .prepare("SELECT id FROM db_agent_instances WHERE definition_id = ?1")?;
                let iter = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
                iter.collect::<Result<Vec<_>, _>>()?
            };
            let rows = conn.execute(
                "DELETE FROM db_agent_definitions WHERE id=?1",
                params![id],
            )?;
            (cascaded_instance_ids, rows)
        };
        if rows > 0 {
            if let Some(reg) = self.registry() {
                for instance_id in &cascaded_instance_ids {
                    if let Err(e) = reg.hard_delete(instance_id) {
                        tracing::warn!(
                            instance_id = %instance_id,
                            agent_def_id = %id,
                            error = %e,
                            "registry: failed to mirror agent_def_delete cascade"
                        );
                    }
                }
            }
            // Phase 3a dual-write (Phase 3b: errors propagate): drop
            // the definition's projection + every cascaded user-clone
            // (instance) projection.
            self.agents_dual_write_definition_delete(id)?;
            for instance_id in &cascaded_instance_ids {
                self.agents_dual_write_instance_delete(instance_id)?;
            }
        }
        Ok(rows > 0)
    }

    // ---- AgentContent CRUD ----

    /// Get a single content blob for an agent.
    pub fn agent_content_get(&self, agent_id: &str, content_type: &str) -> Result<Option<AgentContent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, content_type, content, updated_at
             FROM db_agent_content WHERE agent_id=?1 AND content_type=?2",
        )?;
        let result = stmt.query_row(params![agent_id, content_type], |row| {
            Ok(AgentContent {
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
    pub fn agent_content_set(&self, content: &AgentContent) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_content (agent_id, content_type, content, updated_at)
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
    pub fn agent_content_get_all(&self, agent_id: &str) -> Result<Vec<AgentContent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, content_type, content, updated_at
             FROM db_agent_content WHERE agent_id=?1 ORDER BY content_type ASC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(AgentContent {
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
    pub fn agent_content_delete(&self, agent_id: &str, content_type: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_agent_content WHERE agent_id=?1 AND content_type=?2",
            params![agent_id, content_type],
        )?;
        Ok(rows > 0)
    }

    // ---- AgentSkill CRUD ----

    /// List all skills for an agent, ordered by created_at ascending.
    pub fn agent_skill_list(&self, agent_id: &str) -> Result<Vec<AgentSkill>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, name, trigger, skill_type, description, content, created_at
             FROM db_agent_skills WHERE agent_id=?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(AgentSkill {
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
    pub fn agent_skill_get(&self, id: &str) -> Result<Option<AgentSkill>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, name, trigger, skill_type, description, content, created_at
             FROM db_agent_skills WHERE id=?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            Ok(AgentSkill {
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
    pub fn agent_skill_insert(&self, skill: &AgentSkill) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_skills (id, agent_id, name, trigger, skill_type, description, content, created_at)
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
    pub fn agent_skill_update(&self, skill: &AgentSkill) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_agent_skills SET name=?1, trigger=?2, skill_type=?3, description=?4, content=?5
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
    pub fn agent_skill_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_agent_skills WHERE id=?1",
            params![id],
        )?;
        Ok(rows > 0)
    }

    // ---- AgentHistory methods ----

    /// Append a history entry for an agent. Auto-sets session_date (today) and timestamp.
    pub fn agent_history_append(&self, agent_id: &str, entry: &str) -> Result<AgentHistory, StoreError> {
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
            "INSERT INTO db_agent_history (agent_id, session_date, entry, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, session_date, entry, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(AgentHistory {
            id,
            agent_id: agent_id.to_string(),
            session_date,
            entry: entry.to_string(),
            timestamp: now,
        })
    }

    /// List history entries for an agent, with optional date filter and pagination.
    pub fn agent_history_list(
        &self,
        agent_id: &str,
        session_date: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AgentHistory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match session_date {
            Some(date) => (
                "SELECT id, agent_id, session_date, entry, timestamp
                 FROM db_agent_history WHERE agent_id=?1 AND session_date=?2
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
                 FROM db_agent_history WHERE agent_id=?1
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
            Ok(AgentHistory {
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
    pub fn agent_history_search(
        &self,
        agent_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<AgentHistory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, session_date, entry, timestamp
             FROM db_agent_history WHERE agent_id=?1 AND entry LIKE ?2
             ORDER BY timestamp DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![agent_id, pattern, limit], |row| {
            Ok(AgentHistory {
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
    /// **OAuth credentials stored as a filesystem pointer.** The CLI
    /// (Claude Code, codex, openclaw, …) reads its OAuth tokens from
    /// this directory at spawn time — agentmux only holds the path,
    /// never the tokens themselves. Token refresh is the CLI's job;
    /// the path stays stable across refreshes. Used by oauth-class
    /// providers; the resolver (PR B) dispatches to a config-dir
    /// env-var injection mode rather than the api-key env-var path.
    /// See `docs/specs/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`.
    OAuthConfigDir {
        /// Absolute path to the per-bundle, per-provider config
        /// directory — e.g. `~/.agentmux/shared/identities/<id>/claude/`,
        /// or the legacy `~/.claude/` for the Default migration bundle
        /// (PR E).
        dir: String,
    },
}

/// An identity account (reusable credential, linked to agents via the
/// `db_agent_identity_links` junction). Replaces the browser
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
pub struct AgentIdentityLink {
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
/// files / MCP servers / skills. Agent definitions shadow-migrate into this
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
/// existing schema conventions (`NOT NULL DEFAULT ''`). Callers
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
    /// FK to `db_identity_bundles.id`. Empty string means "use the blank
    /// singleton" (= ambient creds, no env-var injection). Set at
    /// instantiation via the launch modal's Identity dropdown.
    #[serde(default)]
    pub identity_id: String,
    /// FK to `db_memory_bundles.id`. Empty string means "use the blank
    /// singleton" (= vanilla CLI, no instructions). Set at
    /// instantiation via the launch modal's Memory dropdown.
    #[serde(default)]
    pub memory_id: String,
    /// User-chosen instance name (becomes `AGENTMUX_AGENT_ID` in the
    /// spawn env). Empty for pre-v8 rows and for un-named launches.
    /// Drives the "Continue agent" dropdown in the launch modal.
    #[serde(default)]
    pub instance_name: String,
    /// Absolute path returned by `allocate_agent_workdir` at spawn.
    /// Stored explicitly (rather than re-derived from the slug at
    /// continue-time) so the continue flow is robust against
    /// slug-rule changes and user-side renames.
    #[serde(default)]
    pub working_directory: String,
    /// Soft-delete flag for the "Forget agent" affordance. Hidden
    /// rows stay on disk for audit + recovery.
    #[serde(default)]
    pub display_hidden: bool,
}

/// Build a registry record from an `AgentInstance`. Returns an error
/// if the working directory can't be expressed as a path relative to
/// the canonical shared agents root (e.g. user pointed an agent at
/// `~/projects/foo`, which would also fail a naive `"agents"`
/// segment-scan that happened to match `~/projects/agents/foo`).
/// Caller logs + skips — agent stays in SQLite, just not in the
/// cross-version dropdown.
fn agent_instance_to_record(
    inst: &AgentInstance,
    agents_root: &Path,
) -> Result<crate::registry::NamedAgentRecord, String> {
    use crate::registry::{NamedAgentRecord, NamedAgentRecordV1, MAX_SUPPORTED_SCHEMA};
    let rel = relative_workdir(&inst.working_directory, agents_root).ok_or_else(|| {
        format!(
            "working_directory {:?} is not under {:?}",
            inst.working_directory,
            agents_root.display()
        )
    })?;
    let version = env!("CARGO_PKG_VERSION").to_string();
    Ok(NamedAgentRecord {
        schema_version: MAX_SUPPORTED_SCHEMA,
        data: NamedAgentRecordV1 {
            instance_id: inst.id.clone(),
            instance_name: inst.instance_name.clone(),
            definition_id: inst.definition_id.clone(),
            identity_id: empty_to_none(&inst.identity_id),
            memory_id: empty_to_none(&inst.memory_id),
            working_dir: rel,
            created_at_ms: inst.created_at,
            last_launched_at_ms: inst.started_at,
            created_by_version: version.clone(),
            last_launched_by_version: version,
        },
    })
}

fn empty_to_none(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}

/// Express `abs` as a path relative to `agents_root`. Returns `None`
/// when `abs` is empty, not under `agents_root`, or after stripping
/// resolves to an empty path. Anchors against the **resolved** shared
/// root (passed in by the caller) — never scans for a path segment
/// named "agents", which would match unrelated user directories like
/// `/home/me/projects/agents/foo`.
fn relative_workdir(abs: &str, agents_root: &Path) -> Option<String> {
    if abs.is_empty() {
        return None;
    }
    let p = std::path::Path::new(abs);
    let rel = p.strip_prefix(agents_root).ok()?;
    // Reject empties + traversals (defense in depth — strip_prefix
    // already rules out `..` escapes, but the registry's own validator
    // re-checks).
    let s = rel.to_string_lossy().to_string();
    if s.is_empty() || s == "." {
        return None;
    }
    Some(s)
}

impl Store {
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
            "INSERT INTO db_agent_identity_links (agent_id, account_id, provider)
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
            "DELETE FROM db_agent_identity_links WHERE agent_id = ?1 AND provider = ?2",
            params![agent_id, provider],
        )?;
        Ok(rows > 0)
    }

    /// List all (agent_id, account_id, provider) triples for an agent.
    pub fn agent_identity_list_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentIdentityLink>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, account_id, provider
             FROM db_agent_identity_links
             WHERE agent_id = ?1
             ORDER BY provider",
        )?;
        let iter = stmt.query_map(params![agent_id], |row| {
            Ok(AgentIdentityLink {
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
                    identity_id, memory_id, instance_name, working_directory,
                    display_hidden
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
                    identity_id, memory_id, instance_name, working_directory,
                    display_hidden
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
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO db_agent_instances
                    (id, definition_id, parent_instance_id, block_id, session_id, status,
                     github_context, started_at, ended_at, created_at,
                     identity_id, memory_id,
                     instance_name, working_directory, display_hidden)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                         ?13, ?14, ?15)",
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
                    inst.instance_name,
                    inst.working_directory,
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                ],
            )?;
        }
        self.registry_upsert_if_named(inst);
        // Phase 3a dual-write (Phase 3b: errors propagate): mirror to
        // db_agents so the next read sees the new instance.
        self.agents_dual_write_instance_create(inst)?;
        Ok(())
    }

    /// Set the `display_hidden` flag on an existing instance row. Used
    /// by the "Forget agent" affordance — soft-delete only; the row +
    /// working directory remain on disk for audit + recovery.
    ///
    /// Cross-version case: an agent migrated into the registry from
    /// another version's SQLite won't have a row in the current
    /// version's SQLite. The UPDATE returns 0 rows, but the registry
    /// still needs to flip — otherwise "Forget agent" silently no-ops
    /// on cross-version entries. Returns `true` if either side acted.
    pub fn instance_set_hidden(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agent_instances SET display_hidden = ?1 WHERE id = ?2",
                params![if hidden { 1_i64 } else { 0_i64 }, id],
            )?
        };
        let mut registry_acted = false;
        if let Some(reg) = self.registry() {
            // Only act on the registry side if a record exists there
            // (in either active or retired). Avoids logging spurious
            // "failed to retire" warnings on a no-op for unrelated ids.
            if reg.exists_anywhere(id) {
                let res = if hidden {
                    reg.retire(id)
                } else {
                    reg.unretire(id)
                };
                match res {
                    Ok(()) => registry_acted = true,
                    Err(e) => tracing::warn!(
                        instance_id = %id,
                        hidden,
                        error = %e,
                        "registry: failed to mirror instance_set_hidden"
                    ),
                }
            }
        }
        // Phase 3a dual-write (Phase 3b: errors propagate): flip the
        // hidden bit on db_agents.
        self.agents_dual_write_instance_set_hidden(id, hidden)?;
        Ok(rows > 0 || registry_acted)
    }

    /// List named instances for the launch-modal "Continue agent"
    /// dropdown (`include_continuations = false`) or the picker
    /// "My Agents" surface (`include_continuations = true`). Filters
    /// to non-hidden + named rows, sorted by `started_at DESC`,
    /// capped by `limit`.
    ///
    /// `definition_id`, when provided, restricts the result to
    /// instances of that definition. Server-side filtering is
    /// necessary because the launch modal opens per-definition: a
    /// user with 200+ named agents across many definitions could
    /// have the current definition's older instances cut off by a
    /// purely global limit otherwise.
    ///
    /// `include_continuations` controls whether rows with
    /// `parent_instance_id != ''` (continuation chains) are
    /// returned:
    ///
    /// - **`false`** (legacy "head-of-chain only"). Pre-Option-E
    ///   semantics: hides continuation rows so the launch-modal
    ///   dropdown shows one entry per chain root. `listnamedagents`
    ///   ALSO uses this mode for its same-version SQLite enrichment
    ///   of registry-sourced rows — the registry mirror filter at
    ///   `registry_upsert_if_named` excludes continuations
    ///   symmetrically, and breaking that symmetry under a `limit`
    ///   truncation would let continuation rows displace
    ///   registry-head rows in the top-N and miss the
    ///   merge-by-id enrichment. (Codex P1 on PR #1016 first cut:
    ///   regresses running-state badges and focus-existing-pane
    ///   hints for any user whose latest instance is a continuation.)
    ///
    /// - **`true`** (Option-E "include continuations"). For the
    ///   picker's `listrecentsessions` flow and the
    ///   `template_promote` migration's instance-name lookup. Under
    ///   Option E the session zone is anchored on `definition_id`,
    ///   so a continuation row is simply the most-recent named
    ///   instance of an agent the user actively used — exactly
    ///   what those callers want visible. Excluding them hides
    ///   real agents (the original 2026-05-24 "Maks doesn't appear
    ///   under My Agents" report) and makes `template_promote`'s
    ///   name lookup miss the real `instance_name`, falling back
    ///   to the template name.
    pub fn instance_list_named(
        &self,
        limit: usize,
        definition_id: Option<&str>,
        identity_id: Option<&str>,
        include_continuations: bool,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();

        // Dynamic bind-index assignment. We accumulate optional
        // string params into `extra_params` in the same order they
        // appear in the SQL, then bind the limit last.
        let mut extra_params: Vec<&str> = Vec::with_capacity(2);

        if !include_continuations {
            // Legacy "dropdown" mode: only chain heads (no parent).
            // Used by the launch modal's `Continue agent` dropdown +
            // the registry-enrichment path. Symmetric with
            // `registry_upsert_if_named` — chains show up as one
            // head entry, not N entries per resume.
            let mut sql = String::from(
                "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                        status, github_context, started_at, ended_at, created_at,
                        identity_id, memory_id, instance_name, working_directory,
                        display_hidden
                 FROM db_agent_instances
                 WHERE display_hidden = 0
                   AND instance_name <> ''
                   AND parent_instance_id = ''",
            );
            let mut next_idx = 1usize;
            if let Some(id) = identity_id {
                sql.push_str(&format!("\n                   AND identity_id = ?{}", next_idx));
                extra_params.push(id);
                next_idx += 1;
            }
            if let Some(def) = definition_id {
                sql.push_str(&format!("\n                   AND definition_id = ?{}", next_idx));
                extra_params.push(def);
                next_idx += 1;
            }
            sql.push_str(&format!(
                "\n                 ORDER BY started_at DESC\n                 LIMIT ?{}",
                next_idx
            ));
            let mut stmt = conn.prepare(&sql)?;
            let limit_i64 = limit as i64;
            let mut bindings: Vec<&dyn rusqlite::ToSql> =
                Vec::with_capacity(extra_params.len() + 1);
            for s in &extra_params {
                bindings.push(s);
            }
            bindings.push(&limit_i64);
            let iter = stmt.query_map(bindings.as_slice(), map_instance_row)?;
            let mut out = Vec::new();
            for r in iter {
                out.push(r?);
            }
            return Ok(out);
        }

        // Picker ("My Agents") mode: dedupe continuation chains so
        // each logical agent surfaces as exactly one row — the most
        // recent in its chain (by `started_at`, tiebreaker `id`).
        //
        // Before this dedup, a user with 5 continuations of one
        // logical agent saw 5 entries in My Agents. See discussion
        // #1095 / `docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md`
        // Phase 3b.1.
        //
        // Mechanics:
        //   - A recursive CTE walks `parent_instance_id` from each
        //     head (parent_instance_id = '') down to its
        //     descendants, stamping every row with the head's
        //     `root_id`.
        //   - `ROW_NUMBER() OVER (PARTITION BY root_id ORDER BY
        //     started_at DESC, id DESC)` picks the latest row per
        //     chain. The id tiebreaker keeps the ordering
        //     deterministic when two rows share `started_at` (only
        //     happens in tests / on adjacent inserts).
        //   - **Hidden filter must run AFTER ranking.** Otherwise
        //     `hidenamedagent` becomes a no-op for any chain with
        //     older visible siblings: the SQL excludes the hidden
        //     row before ranking, so the next-newest visible row
        //     inherits `rn = 1` and the "forgotten" agent
        //     immediately reappears in the picker. Codex P2 on PR
        //     #1096. By ranking first and filtering
        //     `display_hidden` last, hiding the surfaced row
        //     suppresses the whole chain — exactly what the user's
        //     forget action means.
        //   - The unnamed-row filter (`instance_name <> ''`) stays
        //     pre-rank: an unnamed continuation row should never
        //     win, but also shouldn't influence chain ranking — it
        //     simply isn't a candidate.
        let mut sql = String::from(
            r#"WITH RECURSIVE
            roots(id, definition_id, parent_instance_id, block_id, session_id,
                  status, github_context, started_at, ended_at, created_at,
                  identity_id, memory_id, instance_name, working_directory,
                  display_hidden, root_id) AS (
                -- Anchor: a row is its own root if it has no parent OR
                -- its parent no longer exists in the table. The latter
                -- case (orphan continuation) happens when
                -- `deleteagentinstance` hard-deletes a chain head —
                -- there's no FK cascade, so descendant rows remain. If
                -- we seeded only from `parent_instance_id = ''`,
                -- orphans would be unreachable by the recursive walk
                -- and disappear from My Agents even though they're
                -- recoverable sessions. Codex P2 on PR #1096
                -- bbe897cc → orphan-as-root anchor.
                SELECT id, definition_id, parent_instance_id, block_id, session_id,
                       status, github_context, started_at, ended_at, created_at,
                       identity_id, memory_id, instance_name, working_directory,
                       display_hidden,
                       id
                FROM db_agent_instances p
                WHERE p.parent_instance_id = ''
                   OR NOT EXISTS (
                       SELECT 1 FROM db_agent_instances q
                       WHERE q.id = p.parent_instance_id
                   )
                UNION ALL
                SELECT c.id, c.definition_id, c.parent_instance_id, c.block_id,
                       c.session_id, c.status, c.github_context, c.started_at,
                       c.ended_at, c.created_at, c.identity_id, c.memory_id,
                       c.instance_name, c.working_directory, c.display_hidden,
                       r.root_id
                FROM db_agent_instances c
                JOIN roots r ON c.parent_instance_id = r.id
            ),
            ranked AS (
                SELECT id, definition_id, parent_instance_id, block_id, session_id,
                       status, github_context, started_at, ended_at, created_at,
                       identity_id, memory_id, instance_name, working_directory,
                       display_hidden, root_id,
                       ROW_NUMBER() OVER (
                           PARTITION BY root_id
                           ORDER BY started_at DESC, id DESC
                       ) AS rn
                FROM roots
                WHERE instance_name <> ''"#
                .to_string(),
        );
        // Identity filter MUST run inside the `ranked` CTE (i.e.,
        // before ROW_NUMBER) so the newest row matching the requested
        // identity per chain wins. If we filtered identity in the
        // outer SELECT instead, a chain whose newest row uses a
        // different identity would be dropped even if an older row in
        // the chain matched. Codex P2 #3 on PR #1096 0c4c8c46.
        let mut next_idx = 1usize;
        if let Some(id) = identity_id {
            sql.push_str(&format!("\n                  AND identity_id = ?{}", next_idx));
            extra_params.push(id);
            next_idx += 1;
        }
        sql.push_str(
            r#"
            )
            SELECT id, definition_id, parent_instance_id, block_id, session_id,
                   status, github_context, started_at, ended_at, created_at,
                   identity_id, memory_id, instance_name, working_directory,
                   display_hidden
            FROM ranked
            WHERE rn = 1
              AND display_hidden = 0"#,
        );
        if let Some(def) = definition_id {
            sql.push_str(&format!("\n              AND definition_id = ?{}", next_idx));
            extra_params.push(def);
            next_idx += 1;
        }
        sql.push_str(&format!(
            "\n            ORDER BY started_at DESC\n            LIMIT ?{}",
            next_idx
        ));
        let mut stmt = conn.prepare(&sql)?;
        let limit_i64 = limit as i64;
        let mut bindings: Vec<&dyn rusqlite::ToSql> =
            Vec::with_capacity(extra_params.len() + 1);
        for s in &extra_params {
            bindings.push(s);
        }
        bindings.push(&limit_i64);
        let iter = stmt.query_map(bindings.as_slice(), map_instance_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Look up the canonical named-agent row matching `instance_name`.
    /// Used by the launch modal to detect name collisions ("did you
    /// mean to continue?") and by `ContinueNamedAgentCommand` to
    /// resolve the consolidated row when the caller only knows the
    /// name. Hidden rows are excluded.
    ///
    /// Phase 3b.2: reads from the consolidated `db_agents` table —
    /// `is_template = 0` (named user agent), `instance_name` matches,
    /// `user_hidden = 0`. Continuation chains are pre-collapsed in
    /// `db_agents` (one row per logical agent), so this returns the
    /// canonical agent regardless of how many launches its chain has.
    /// MRU tiebreak is by `updated_at` (the dual-write touches it on
    /// every continuation), then `created_at` for stable order.
    ///
    /// The legacy `db_agent_instances` carried per-launch runtime
    /// state (`block_id`, `session_id`, `status`, `started_at`,
    /// `ended_at`, `parent_instance_id`) that has no analog in
    /// `db_agents` — those fields are returned as their `AgentInstance`
    /// defaults (empty strings, 0). Callers wanting transient state
    /// should consult runtime sources (the controller, the block
    /// row); none of the documented use cases need it (collision
    /// detection only cares about identity / cwd; ContinueNamed only
    /// cares about `id` + bindings).
    /// Spec: docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md §3b.
    pub fn instance_get_by_name(
        &self,
        instance_name: &str,
    ) -> Result<Option<AgentInstance>, StoreError> {
        if instance_name.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        // `definition_id` mapping: db_agents folds the def and the
        // user-clone instance into ONE row (keyed by def.id) for
        // is_seeded=0 agents. For seeded-template projections,
        // parent_template_id points at the template. Either way, the
        // consolidated row's "definition" is `COALESCE(parent_template_id,
        // id)` — the legacy `definition_id` semantics from the caller's
        // perspective. Empty parent_template_id (folded user-clone)
        // resolves to the row's own id.
        let mut stmt = conn.prepare(
            "SELECT id,
                    CASE WHEN parent_template_id = '' THEN id ELSE parent_template_id END AS def_id,
                    github_context, created_at,
                    identity_id, memory_id, instance_name, working_directory,
                    user_hidden
             FROM db_agents
             WHERE instance_name = ?1
               AND is_template = 0
               AND user_hidden = 0
             ORDER BY updated_at DESC, created_at DESC
             LIMIT 1",
        )?;
        let result = stmt.query_row(params![instance_name], |row| {
            Ok(AgentInstance {
                id: row.get(0)?,
                definition_id: row.get(1)?,
                parent_instance_id: String::new(),
                block_id: String::new(),
                session_id: String::new(),
                status: String::new(),
                github_context: row.get(2)?,
                started_at: row.get(3)?, // created_at — best proxy for the consolidated row
                ended_at: 0,
                created_at: row.get(3)?,
                identity_id: row.get(4)?,
                memory_id: row.get(5)?,
                instance_name: row.get(6)?,
                working_directory: row.get(7)?,
                display_hidden: row.get::<_, i64>(8)? != 0,
            })
        });
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update mutable instance fields. `id`, `definition_id`,
    /// `parent_instance_id`, `started_at`, `created_at` are immutable
    /// after insert (they describe provenance, not state).
    pub fn instance_update(&self, inst: &AgentInstance) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
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
            )?
        };
        if rows > 0 {
            // Refresh the registry from the post-update authoritative row.
            // `inst` is the caller's pre-update view; SQL UPDATE only
            // touches a subset of fields, so we reload to keep registry
            // mirror exact.
            if let Ok(Some(fresh)) = self.instance_get(&inst.id) {
                self.registry_upsert_if_named(&fresh);
                // Phase 3a dual-write (Phase 3b: errors propagate):
                // mirror the fields the consolidation cares about
                // (github_context, updated_at).
                self.agents_dual_write_instance_update(&fresh)?;
            }
        }
        Ok(rows > 0)
    }

    /// Repoint every instance currently referencing `old_def_id` to
    /// `new_def_id`. Used by the Phase 1 two-tier-picker migration
    /// (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md): when a seeded
    /// template has been used directly (carries an `agent:<id>:current`
    /// zone), the migration clones the template into a user agent and
    /// repoints any instances so the existing reattach flow
    /// (`continueOfInstanceId`) keeps working against the new
    /// definition_id. Returns the number of rows updated.
    ///
    /// `definition_id` is declared immutable post-insert on the normal
    /// `instance_update` path. This is the migration escape hatch.
    pub fn instance_repoint_definition(
        &self,
        old_def_id: &str,
        new_def_id: &str,
    ) -> Result<usize, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agent_instances SET definition_id = ?1 WHERE definition_id = ?2",
                params![new_def_id, old_def_id],
            )?
        };
        // Phase 3a dual-write (Phase 3b: errors propagate): re-aim
        // parent_template_id on the user-clone projection rows that
        // were pointing at old_def_id.
        if rows > 0 {
            self.agents_dual_write_instance_repoint(old_def_id, new_def_id)?;
        }
        Ok(rows)
    }

    pub fn instance_delete(&self, id: &str) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM db_agent_instances WHERE id = ?1", params![id])?
        };
        if rows > 0 {
            if let Some(reg) = self.registry() {
                if let Err(e) = reg.hard_delete(id) {
                    tracing::warn!(
                        instance_id = %id,
                        error = %e,
                        "registry: failed to mirror instance_delete"
                    );
                }
            }
            // Phase 3a dual-write (Phase 3b: errors propagate): drop
            // the user-clone projection.
            self.agents_dual_write_instance_delete(id)?;
        }
        Ok(rows > 0)
    }

    /// Back-fill `db_agent_instances.identity_id` for legacy rows that
    /// have either the empty string (post-v7 default before the launch
    /// modal required Identity) or the literal `"blank"` sentinel
    /// (pre-v8 placeholder for "use ambient creds"). Both shapes map
    /// to "no Identity bundle assigned" and the OAuth-bundles startup
    /// migration (PR E, spec §5) routes them to the newly-seeded
    /// Default bundle so the resolver can inject env vars from the
    /// captured ambient credentials at the next spawn.
    ///
    /// Returns the number of rows touched. Caller must verify that
    /// `new_identity_id` is a real `db_identity_bundles.id` — this
    /// method does NOT enforce FK validity (the column has no FK
    /// constraint per the v7 migration). Mis-use would orphan the
    /// rows to a non-existent bundle; the OAuth-bundles migration
    /// guards against this by only calling here when it just upserted
    /// the bundle row.
    pub fn instance_backfill_identity_id(
        &self,
        new_identity_id: &str,
    ) -> Result<usize, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agent_instances
                 SET identity_id = ?1
                 WHERE identity_id = '' OR identity_id = 'blank'",
                params![new_identity_id],
            )?
        };
        // Phase 3a dual-write (Phase 3b: errors propagate): same
        // backfill on db_agents user-clone rows.
        self.agents_dual_write_backfill_identity(new_identity_id)?;
        Ok(rows)
    }

    /// Mirror a `db_agent_instances` mutation into the cross-version
    /// registry. Only fires for **named** rows. Routes by
    /// `display_hidden` so the registry file ends up in the tree
    /// matching SQLite's dropdown filter:
    ///
    /// - hidden = true  → upsert (atomic write to active/) then
    ///   retire (atomic rename to retired/). Net: file lives in
    ///   `retired/<id>.json` with the freshest content. Prevents
    ///   `instance_update` on a previously-hidden row from
    ///   resurrecting an active registry file, AND keeps the
    ///   retired tombstone's content current.
    /// - hidden = false → unretire (no-op if not retired) then
    ///   upsert. Net: file in `active/<id>.json`, no orphan retired.
    ///
    /// Failures are logged, never propagated: SQLite remains
    /// authoritative.
    fn registry_upsert_if_named(&self, inst: &AgentInstance) {
        // Mirror filter: only registers named rows. Pre-Option-E this
        // also excluded continuation rows (parent_instance_id != '')
        // so the registry-sourced read path wouldn't surface chained
        // resumes as duplicate dropdown rows. Under Option E, the
        // session zone is anchored on definition_id, so a continuation
        // row IS the most-recent named instance — exactly what we want
        // visible. `instance_list_named` (the SQLite-sourced read
        // path) dropped its parent_instance_id filter in the
        // 2026-05-24 picker-visibility fix; the registry mirror keeps
        // its filter here for now since the registry-sourced read path
        // doesn't have the dedup-by-(definition_id, instance_name)
        // affordance the SQLite ORDER BY/LIMIT provides. Follow-up
        // PR will land cross-version dedup so this filter can drop too.
        if inst.instance_name.is_empty() || !inst.parent_instance_id.is_empty() {
            return;
        }
        let Some(reg) = self.registry() else { return };
        let Some(agents_root) = reg.agents_root() else {
            tracing::warn!("registry: agents_root has no parent — skipping mirror");
            return;
        };
        let rec = match agent_instance_to_record(inst, agents_root) {
            Ok(rec) => rec,
            Err(e) => {
                tracing::warn!(
                    instance_id = %inst.id,
                    error = %e,
                    "registry: instance not representable as record, skipping mirror"
                );
                return;
            }
        };

        // If the row was previously hidden, the file lives in
        // `retired/`. Move it back to active before upserting so
        // upsert's merge-preserves-unknown-fields path operates on
        // the right file (and we never leave dangling retired files).
        if let Err(e) = reg.unretire(&inst.id) {
            tracing::warn!(
                instance_id = %inst.id,
                error = %e,
                "registry: failed to unretire row before upsert"
            );
        }

        if let Err(e) = reg.upsert(&rec) {
            tracing::warn!(
                instance_id = %inst.id,
                error = %e,
                "registry: failed to mirror instance_create/update"
            );
            return;
        }

        // After the upsert, move into retired/ if the row is hidden.
        // Combined: hidden row's tombstone always has up-to-date
        // content, and active/ never carries a hidden row.
        if inst.display_hidden {
            if let Err(e) = reg.retire(&inst.id) {
                tracing::warn!(
                    instance_id = %inst.id,
                    error = %e,
                    "registry: failed to retire hidden row post-upsert"
                );
            }
        }
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
                    identity_id, memory_id, instance_name, working_directory,
                    display_hidden
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
    // Phase 3a — db_agents dual-write helpers
    //
    // Every mutation on `db_agent_definitions` / `db_agent_instances`
    // mirrors into `db_agents` so a later read-migration PR (Phase 3b)
    // can flip readers over with confidence. Dual-write failures LOG +
    // CONTINUE — the old tables remain authoritative until Phase 3b.
    // See docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md.
    // ====================================================================

    /// Mirror a `db_agent_definitions` row into `db_agents` as the
    /// canonical row for that definition.
    ///
    /// - `is_seeded = 1` rows become `is_template = 1` (canonical
    ///   template; bindings stay empty).
    /// - `is_seeded = 0` rows become `is_template = 0` with
    ///   `parent_template_id = parent_id` (user-cloned from a template;
    ///   bindings come from the matching instance, if any — handled by
    ///   `agents_dual_write_instance_create` updating in place).
    ///
    /// Idempotent: uses `INSERT … ON CONFLICT(id) DO UPDATE`. Existing
    /// bindings on the row (written previously by an instance dual-write)
    /// are preserved — only definition-side fields are overwritten.
    ///
    /// Phase 3b: returns `Err` on failure (previously logged + continued).
    /// Phase 3b readers see `db_agents`, so a silent dual-write failure
    /// would leak stale data into the picker.
    pub(crate) fn agents_dual_write_definition_upsert(&self, def: &AgentDefinition) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let is_template = if def.is_seeded == 1 { 1_i64 } else { 0_i64 };
        let parent_template_id = if def.is_seeded == 1 {
            String::new()
        } else {
            def.parent_id.clone()
        };
        // Phase 3b: carry the caller's `user_hidden` into the INSERT
        // (previously hardcoded to 0). The legacy `db_agent_definitions`
        // INSERT honours it, and now that Phase 3b readers look at
        // `db_agents`, the projection must agree from row creation —
        // not just after a subsequent `agent_def_set_hidden` flip.
        // ON CONFLICT deliberately does NOT update `user_hidden`: hide
        // is a per-user view-state flag that survives definition
        // payload edits.
        conn.execute(
            "INSERT INTO db_agents (
                id, name, icon, description,
                is_template, parent_template_id,
                provider, provider_flags, shell, environment,
                agent_type, agent_bus_id, accounts,
                auto_start, restart_on_crash, idle_timeout_minutes,
                slug, branch_label,
                created_at, updated_at, is_seeded, user_hidden
             ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15, ?16,
                ?17, ?18,
                ?19, ?20, ?21, ?22
             )
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                icon = excluded.icon,
                description = excluded.description,
                is_template = excluded.is_template,
                parent_template_id = excluded.parent_template_id,
                provider = excluded.provider,
                provider_flags = excluded.provider_flags,
                shell = excluded.shell,
                environment = excluded.environment,
                agent_type = excluded.agent_type,
                agent_bus_id = excluded.agent_bus_id,
                accounts = excluded.accounts,
                auto_start = excluded.auto_start,
                restart_on_crash = excluded.restart_on_crash,
                idle_timeout_minutes = excluded.idle_timeout_minutes,
                slug = excluded.slug,
                branch_label = excluded.branch_label,
                updated_at = excluded.updated_at,
                is_seeded = excluded.is_seeded",
            params![
                def.id,
                def.name,
                def.icon,
                def.description,
                is_template,
                parent_template_id,
                def.provider,
                def.provider_flags,
                def.shell,
                def.environment,
                def.agent_type,
                def.agent_bus_id,
                def.accounts,
                def.auto_start,
                def.restart_on_crash,
                def.idle_timeout_minutes,
                def.slug,
                def.branch_label,
                def.created_at,
                def.updated_at,
                def.is_seeded,
                def.user_hidden,
            ],
        )?;
        Ok(())
    }

    /// Mirror a `db_agent_definitions` DELETE into `db_agents`. The
    /// definition row itself is removed; any user-cloned children (rows
    /// with `parent_template_id = old_id`) are left intact because the
    /// FK cascade on the OLD schema only deletes instances, not other
    /// definitions.
    pub(crate) fn agents_dual_write_definition_delete(&self, def_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Reagent P2 on #1013 round 3: `id` is the PK so two DELETE
        // statements scoped by `id = ?1 AND is_template = N` add nothing
        // over a single PK delete (only one row can match either, and
        // an early return on the first error would skip the second
        // cleanup unnecessarily). Collapsed to a single direct PK
        // delete that handles both template and user-clone projections.
        conn.execute(
            "DELETE FROM db_agents WHERE id = ?1",
            params![def_id],
        )?;
        Ok(())
    }

    /// Bulk dual-write: mirror `agent_def_delete_seeded`. Deletes:
    ///   1. every `is_template = 1` row (the template projections), AND
    ///   2. every `db_agents` row whose `id` is in the
    ///      `cascaded_inst_ids` set (template-instance projections that
    ///      were just removed by the FK cascade on `db_agent_instances`).
    ///
    /// User-clone DEFINITION projections (`is_template = 0`, `id` is a
    /// def_id in `db_agent_definitions`) are NOT touched — those rows
    /// represent persistent user agents and live or die with their
    /// `db_agent_definitions` row, not with the seeded-template bulk
    /// delete. Reagent P1 round 4 on #1013: the previous version
    /// scoped by `parent_template_id` and over-deleted user-clone DEF
    /// projections too. Idempotent.
    pub(crate) fn agents_dual_write_seeded_delete(&self, cascaded_inst_ids: &[String]) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM db_agents WHERE is_template = 1",
            [],
        )?;
        // Delete cascaded instance projections one by one. Could batch
        // via `id IN (?1, ?2, ...)` but the seeded delete is rare and
        // the typical set is small (< 100); per-id loop keeps the SQL
        // simple and avoids dynamic-parameter expansion.
        for inst_id in cascaded_inst_ids {
            conn.execute(
                "DELETE FROM db_agents WHERE id = ?1 AND is_template = 0",
                params![inst_id],
            )?;
        }
        Ok(())
    }

    /// Mirror a `db_agent_instances` INSERT into `db_agents`. Always
    /// creates a NEW row in `db_agents` whose id == instance id.
    ///
    /// The row's identity comes from the instance (id, name, bindings).
    /// Its template-config fields are copied from the parent definition;
    /// `parent_template_id` points at that definition.
    ///
    /// Continuations (`parent_instance_id` non-empty) mirror their
    /// new bindings into the chain's existing `db_agents` row rather
    /// than creating a separate one. The chain root is resolved via
    /// `agents_projection_key_for_inst`. Codex P2 on PR #1110 — without
    /// this, a named-agent rebind (continue with different identity,
    /// memory, cwd, or github context) never reaches the consolidated
    /// row and Phase 3b readers see stale data.
    pub(crate) fn agents_dual_write_instance_create(&self, inst: &AgentInstance) -> Result<(), StoreError> {
        let is_continuation = !inst.parent_instance_id.is_empty();
        let conn = self.conn.lock().unwrap();
        // Pull the parent definition for the cmd-config fields.
        let def = match Self::load_definition_for_dual_write(&conn, &inst.definition_id)? {
            Some(d) => d,
            None => {
                // Orphan instance — no matching definition. The legacy
                // table accepted the row (no FK in the old test stores),
                // but with nothing to project there's nothing this
                // helper can mirror. Surfacing this as Err would block
                // a write the legacy path tolerated; log at error level
                // and continue with no projection.
                tracing::error!(
                    instance_id = %inst.id,
                    definition_id = %inst.definition_id,
                    "db_agents dual-write: instance has no matching definition; skipping mirror",
                );
                return Ok(());
            }
        };
        let name = if inst.instance_name.is_empty() {
            def.name.clone()
        } else {
            inst.instance_name.clone()
        };
        // Reagent P1 on #1013 round 2: match the backfill rule
        // (`agents_consolidate.rs::backfill_instances`) exactly. When the
        // parent def is a user-clone (`is_seeded = 0`), the existing
        // `db_agents` row keyed by `def.id` already represents this
        // agent — FOLD the instance's bindings into it instead of
        // inserting a new row keyed by `inst.id` with
        // `parent_template_id = def.id` (which would point at a
        // non-template row and produce a shape inconsistent with what
        // backfill produced for identical data).
        // Monotonic-bump helper. See the original comment below for
        // the full reasoning; extracted into a closure so both the
        // user-clone path and the new continuation-mirror path can
        // reuse it. Successive fast-fire launches (e.g. test loops on
        // Windows millisecond-resolution clocks) need the strict
        // global ordering to survive ORDER BY updated_at.
        //
        // Reagent P2 round 3 on #1013: don't write `inst.created_at`
        // here — the user-clone def may already have a fresher
        // `updated_at` from a prior `agent_def_update`. Wall-clock
        // now() is the right monotonic stamp.
        let now_ms = {
            let wall_now: i64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(inst.created_at);
            let global_prior: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(updated_at), 0) FROM db_agents",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
            std::cmp::max(wall_now, global_prior.saturating_add(1))
        };

        let res = if def.is_seeded == 0 {
            // User-clone def — one db_agents row per def, keyed by
            // def.id. Continuations and head launches both UPDATE the
            // same row. (The chain's identity has nothing to do with
            // which row gets touched here; it's always def.id.)
            conn.execute(
                "UPDATE db_agents SET
                    name = ?2,
                    identity_id = ?3,
                    memory_id = ?4,
                    working_directory = ?5,
                    github_context = ?6,
                    instance_name = ?7,
                    updated_at = ?8,
                    user_hidden = ?9
                 WHERE id = ?1",
                params![
                    def.id,
                    name,
                    inst.identity_id,
                    inst.memory_id,
                    inst.working_directory,
                    inst.github_context,
                    inst.instance_name,
                    now_ms,
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                ],
            )
        } else if is_continuation {
            // Template-instance continuation — UPDATE the chain head's
            // row with the new bindings. The head's id is found via
            // `agents_projection_key_for_inst` (which walks parent
            // _instance_id up to the root). No new row is created;
            // there's exactly one db_agents row per logical agent.
            // Codex P2 on PR #1110.
            let root_id = match Self::agents_projection_key_for_inst(&conn, &inst.id) {
                Some((k, _)) => k,
                None => {
                    tracing::warn!(
                        instance_id = %inst.id,
                        parent_instance_id = %inst.parent_instance_id,
                        "db_agents dual-write: continuation chain has no resolvable root; skipping mirror",
                    );
                    return Ok(());
                }
            };
            conn.execute(
                "UPDATE db_agents SET
                    name = ?2,
                    identity_id = ?3,
                    memory_id = ?4,
                    working_directory = ?5,
                    github_context = ?6,
                    instance_name = ?7,
                    updated_at = ?8,
                    user_hidden = ?9
                 WHERE id = ?1 AND is_template = 0",
                params![
                    root_id,
                    name,
                    inst.identity_id,
                    inst.memory_id,
                    inst.working_directory,
                    inst.github_context,
                    inst.instance_name,
                    now_ms,
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                ],
            )
        } else {
            // Template-instance head — INSERT a new row keyed by inst.id.
            conn.execute(
                "INSERT INTO db_agents (
                    id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    identity_id, memory_id, working_directory, github_context,
                    instance_name,
                    created_at, updated_at, is_seeded, user_hidden
                 ) VALUES (
                    ?1, ?2, ?3, ?4,
                    0, ?5,
                    ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12,
                    ?13, ?14, ?15,
                    ?16, ?17,
                    ?18, ?19, ?20, ?21,
                    ?22,
                    ?23, ?24, 0, ?25
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    identity_id = excluded.identity_id,
                    memory_id = excluded.memory_id,
                    working_directory = excluded.working_directory,
                    github_context = excluded.github_context,
                    instance_name = excluded.instance_name,
                    updated_at = excluded.updated_at,
                    user_hidden = excluded.user_hidden",
                params![
                    inst.id,
                    name,
                    def.icon,
                    def.description,
                    def.id,                  // parent_template_id = definition_id
                    def.provider,
                    def.provider_flags,
                    def.shell,
                    def.environment,
                    def.agent_type,
                    def.agent_bus_id,
                    def.accounts,
                    def.auto_start,
                    def.restart_on_crash,
                    def.idle_timeout_minutes,
                    def.slug,
                    def.branch_label,
                    inst.identity_id,
                    inst.memory_id,
                    inst.working_directory,
                    inst.github_context,
                    inst.instance_name,
                    inst.created_at,
                    inst.created_at,          // updated_at = created_at on insert
                    if inst.display_hidden { 1_i64 } else { 0_i64 },
                ],
            )
        };
        res?;
        Ok(())
    }

    /// Mirror a `db_agent_instances` UPDATE into `db_agents`. Touches
    /// only the fields that `instance_update` writes (block + session +
    /// status + github_context + ended_at) — name/bindings come from
    /// the original create.
    ///
    /// Continuations flow through here too — `agents_projection_key_for_inst`
    /// walks up the chain to the head's id, so a continuation's
    /// `github_context` refresh lands on the canonical row (codex P2 on
    /// PR #1110, paired with the create-path fix in
    /// `agents_dual_write_instance_create`).
    pub(crate) fn agents_dual_write_instance_update(&self, inst: &AgentInstance) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // Reagent P1 round 4 on #1013: route by projection key so the
        // UPDATE actually hits the folded user-clone-def row when this
        // instance was folded at create. The previous version keyed by
        // `inst.id` always and silently no-op'd on every folded
        // instance's lifecycle event.
        let key = match Self::agents_projection_key_for_inst(&conn, &inst.id) {
            Some((k, _)) => k,
            None => return Ok(()),
        };
        // `instance_update` only touches block_id/session_id/status/
        // github_context/ended_at. Of those, only `github_context` lands
        // on db_agents (block/session/status/ended_at are not modelled
        // on the consolidated row — they're block/session-machine
        // concerns the consolidation deliberately drops). We DO refresh
        // updated_at so a Phase-3b reader can sort by recency. Apply
        // the same monotonic-floor trick as the fold branch in
        // `agents_dual_write_instance_create` — wall clock alone
        // collides under millisecond resolution on fast successive
        // mutations.
        let global_prior: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(updated_at), 0) FROM db_agents",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        let now_monotonic = std::cmp::max(now, global_prior.saturating_add(1));
        conn.execute(
            "UPDATE db_agents SET
                github_context = ?1,
                updated_at = ?2
             WHERE id = ?3 AND is_template = 0",
            params![inst.github_context, now_monotonic, key],
        )?;
        Ok(())
    }

    /// Mirror `instance_set_hidden` into `db_agents.user_hidden`.
    pub(crate) fn agents_dual_write_instance_set_hidden(&self, id: &str, hidden: bool) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Reagent P1 round 4 on #1013: route by projection key (see
        // `agents_dual_write_instance_update` for context).
        let key = match Self::agents_projection_key_for_inst(&conn, id) {
            Some((k, _)) => k,
            None => return Ok(()),
        };
        conn.execute(
            "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 0",
            params![if hidden { 1_i64 } else { 0_i64 }, key],
        )?;
        Ok(())
    }

    /// Mirror `instance_repoint_definition` into `db_agents`. The
    /// `parent_template_id` of every user-clone row that pointed at
    /// `old_def_id` is updated to `new_def_id`.
    pub(crate) fn agents_dual_write_instance_repoint(
        &self,
        old_def_id: &str,
        new_def_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            // Reagent P1 round 3 on #1013: `instance_repoint_definition`
            // only rewrites `db_agent_instances.definition_id` — it
            // does NOT rewrite the parent of a sibling user-cloned
            // definition that happens to share the same parent template.
            // Restrict the projection update to rows whose `id` is an
            // ACTUALLY repointed instance id (i.e. the rows whose
            // `db_agent_instances.definition_id` was just rewritten to
            // `new_def_id`). User-clone definition projections, whose
            // `id` lives in `db_agent_definitions` not `db_agent_instances`,
            // are untouched.
            "UPDATE db_agents
             SET parent_template_id = ?1
             WHERE is_template = 0
               AND parent_template_id = ?2
               AND id IN (SELECT id FROM db_agent_instances WHERE definition_id = ?1)",
            params![new_def_id, old_def_id],
        )?;
        Ok(())
    }

    /// Mirror `instance_delete` into `db_agents`.
    pub(crate) fn agents_dual_write_instance_delete(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Reagent P1 round 4 on #1013: route by projection key. For a
        // template-instance projection (`is_folded = false`), the row
        // lives at `id` and goes when the instance goes. For a
        // user-clone-def fold (`is_folded = true`), the projection at
        // `def.id` represents the DEF — the def persists when its
        // instance ends, so NO-OP. `agents_dual_write_definition_delete`
        // is the right entry point to remove that row.
        //
        // Race fallback: `instance_delete` runs the DELETE on
        // `db_agent_instances` BEFORE calling this helper, so by now
        // the instance row is gone and `agents_projection_key_for_inst`
        // returns None. Fall back to checking `db_agents` directly: if
        // there's a row at `id` with `is_template = 0`, that's a
        // non-folded projection — delete it. (Folded projections live
        // at `def.id`, never at `inst.id`, so this is safe.)
        let key_info = Self::agents_projection_key_for_inst(&conn, id);
        let (key, is_folded) = match key_info {
            Some(info) => info,
            None => (id.to_string(), false),
        };
        if is_folded {
            return Ok(());
        }
        conn.execute(
            "DELETE FROM db_agents WHERE id = ?1 AND is_template = 0",
            params![key],
        )?;
        Ok(())
    }

    /// Mirror `instance_backfill_identity_id` into `db_agents`. Same
    /// filter (empty or `"blank"` identity_id) restricted to user-clone
    /// rows.
    pub(crate) fn agents_dual_write_backfill_identity(&self, new_identity_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_agents
             SET identity_id = ?1
             WHERE is_template = 0 AND (identity_id = '' OR identity_id = 'blank')",
            params![new_identity_id],
        )?;
        Ok(())
    }

    /// Reagent P1 round 4 on #1013 — fold-aware projection key lookup.
    /// Resolves "for an instance with this id, which `db_agents` row
    /// represents it post-create?". Returns `Some((key, is_folded))`:
    ///   - `is_folded = true`  → key is the parent definition id
    ///                            (the user-clone-def projection absorbs
    ///                            the instance's bindings).
    ///   - `is_folded = false` → key is the instance id (template-instance
    ///                            projection is its own row).
    /// Returns `None` if the instance no longer exists in
    /// `db_agent_instances` (e.g. already deleted by FK cascade).
    fn agents_projection_key_for_inst(
        conn: &Connection,
        inst_id: &str,
    ) -> Option<(String, bool)> {
        // Codex P2 on PR #1110: continuations of template-instances
        // must resolve to the chain HEAD's id, not the continuation's
        // own id — there's exactly one db_agents row per logical
        // agent, keyed by the head. Walk up `parent_instance_id` via
        // a recursive CTE; the row whose parent is `''` (or whose
        // parent no longer exists) is the root.
        //
        // For user-clone defs (`is_seeded = 0`) the projection key is
        // the def.id regardless of where in the chain we are — the
        // existing one-row-per-def behavior is unchanged. Only the
        // is_seeded=1 (template) branch needs the chain walk.
        conn.query_row(
            "WITH RECURSIVE chain(id, parent_instance_id, definition_id) AS (
                SELECT id, parent_instance_id, definition_id
                FROM db_agent_instances WHERE id = ?1
                UNION ALL
                SELECT p.id, p.parent_instance_id, p.definition_id
                FROM db_agent_instances p
                JOIN chain c ON p.id = c.parent_instance_id
             )
             SELECT c.id, c.definition_id, COALESCE(d.is_seeded, 1) AS is_seeded
             FROM chain c
             LEFT JOIN db_agent_definitions d ON c.definition_id = d.id
             WHERE c.parent_instance_id = ''
                OR NOT EXISTS (
                    SELECT 1 FROM db_agent_instances q
                    WHERE q.id = c.parent_instance_id
                )
             LIMIT 1",
            params![inst_id],
            |row| {
                let root_id: String = row.get(0)?;
                let def_id: String = row.get(1)?;
                let is_seeded: i64 = row.get(2)?;
                Ok(if is_seeded == 0 {
                    (def_id, true)      // folded into def-projection
                } else {
                    (root_id, false)    // chain head's row
                })
            },
        ).ok()
    }

    /// Helper: re-read a definition row from inside an active connection
    /// lock (used by `agents_dual_write_instance_create` to avoid
    /// re-locking the mutex recursively).
    fn load_definition_for_dual_write(
        conn: &Connection,
        id: &str,
    ) -> rusqlite::Result<Option<AgentDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_id, branch_label, updated_at,
                    user_hidden
             FROM db_agent_definitions WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| {
            Ok(AgentDefinition {
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
                updated_at: row.get(20)?,
                user_hidden: row.get(21)?,
            })
        });
        match result {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
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
             FROM db_identity_bundles
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
             FROM db_identity_bundles WHERE id = ?1",
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
            "INSERT INTO db_identity_bundles
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
        let rows = conn.execute("DELETE FROM db_identity_bundles WHERE id = ?1", params![id])?;
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
             FROM db_memory_bundles
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
             FROM db_memory_bundles WHERE id = ?1",
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
            "INSERT INTO db_memory_bundles
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
        let rows = conn.execute("DELETE FROM db_memory_bundles WHERE id = ?1", params![id])?;
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
    let display_hidden_int: i64 = row.get(14)?;
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
        instance_name: row.get(12)?,
        working_directory: row.get(13)?,
        display_hidden: display_hidden_int != 0,
    })
}

/// Phase 3b — row mapper for `db_agents` rows projected back into the
/// `AgentDefinition` shape. The column order MUST match the SELECT in
/// `agent_def_list`. `parent_template_id` maps to `parent_id` because
/// the consolidated table renamed the field to clarify its semantics
/// (template lineage), but the wire shape kept the old name.
fn map_agent_definition_row(row: &rusqlite::Row) -> rusqlite::Result<AgentDefinition> {
    Ok(AgentDefinition {
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
        updated_at: row.get(20)?,
        user_hidden: row.get(21)?,
    })
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::*;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
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
    fn test_agent_def_insert_collision_resolves_at_runtime() {
        // Two agents whose names derive to the same slug must both
        // insert successfully, with the second getting a `-2` suffix.
        // This exercises the runtime collision-resolution path in
        // agent_def_insert (separate from the migration backfill path
        // tested in migrations.rs).
        let store = make_store();

        let mut a1 = AgentDefinition {
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
            updated_at: 0,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut a1).unwrap();
        // "Agent X" → "agent-x"
        assert_eq!(a1.slug, "agent-x");

        let mut a2 = AgentDefinition {
            id: "id-b".to_string(),
            // Different surface form, derives to the same slug
            name: "agent x".to_string(),
            ..a1.clone()
        };
        a2.slug = String::new();
        store.agent_def_insert(&mut a2).unwrap();
        assert_eq!(a2.slug, "agent-x-2");

        let mut a3 = AgentDefinition {
            id: "id-c".to_string(),
            name: "AGENT-X".to_string(),
            ..a1.clone()
        };
        a3.slug = String::new();
        store.agent_def_insert(&mut a3).unwrap();
        assert_eq!(a3.slug, "agent-x-3");

        // Verify the underlying rows actually got written
        let listed = store.agent_def_list().unwrap();
        let slugs: Vec<&str> = listed.iter().map(|a| a.slug.as_str()).collect();
        assert!(slugs.contains(&"agent-x"));
        assert!(slugs.contains(&"agent-x-2"));
        assert!(slugs.contains(&"agent-x-3"));
    }

    #[test]
    fn test_agent_def_insert_explicit_slug_collision_resolves() {
        // When a caller passes an explicit (non-empty) slug that
        // already exists, agent_def_insert still resolves the collision
        // via suffixing — guards against the seed pre-loading the
        // same slug twice or any other "I know the slug" path.
        let store = make_store();

        let mut a1 = AgentDefinition {
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
            updated_at: 0,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut a1).unwrap();
        assert_eq!(a1.slug, "explicit");

        let mut a2 = AgentDefinition {
            id: "id-b".to_string(),
            ..a1.clone()
        };
        a2.slug = "explicit".to_string();
        store.agent_def_insert(&mut a2).unwrap();
        assert_eq!(a2.slug, "explicit-2");
    }

    // ---- v6 identity / instance CRUD ----

    fn v6_test_store() -> Store {
        Store::open_in_memory().unwrap()
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

    fn sample_agent(id: &str, slug: &str) -> AgentDefinition {
        AgentDefinition {
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
            updated_at: 0,
            user_hidden: 0,
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
        store.agent_def_insert(&mut agent).unwrap();
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
        store.agent_def_insert(&mut agent).unwrap();
        store.identity_upsert(&sample_account("id-gh", "github")).unwrap();
        store.agent_identity_link("ag1", "id-gh", "github").unwrap();

        store.agent_def_delete("ag1").unwrap();
        assert!(store.agent_identity_list_for_agent("ag1").unwrap().is_empty());
    }

    #[test]
    fn test_instance_create_update_filter() {
        let store = v6_test_store();
        let mut agent = sample_agent("def1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();

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
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
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
    fn test_agent_def_list_orders_by_last_used() {
        let store = v6_test_store();
        // Three definitions; none launched yet.
        let mut a = sample_agent("def-a", "agent-a");
        let mut b = sample_agent("def-b", "agent-b");
        let mut c = sample_agent("def-c", "agent-c");
        store.agent_def_insert(&mut a).unwrap();
        store.agent_def_insert(&mut b).unwrap();
        store.agent_def_insert(&mut c).unwrap();

        let mk = |id: &str, def: &str, started: i64| AgentInstance {
            id: id.to_string(),
            definition_id: def.to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: started,
            ended_at: 0,
            created_at: started,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        // Launch def-a, then def-b later. def-c is never launched.
        store.instance_create(&mk("i-a", "def-a", 500)).unwrap();
        store.instance_create(&mk("i-b", "def-b", 600)).unwrap();

        let ids = |s: &Store| -> Vec<String> {
            s.agent_def_list().unwrap().into_iter().map(|d| d.id).collect()
        };
        // Most-recently-launched first; never-launched (def-c) last.
        assert_eq!(ids(&store), vec!["def-b", "def-a", "def-c"]);

        // A newer launch of def-a flips it above def-b (MAX(started_at)).
        store.instance_create(&mk("i-a2", "def-a", 700)).unwrap();
        assert_eq!(ids(&store), vec!["def-a", "def-b", "def-c"]);
    }

    #[test]
    fn test_instance_cascade_on_definition_delete() {
        let store = v6_test_store();
        let mut agent = sample_agent("def1", "agent-x");
        store.agent_def_insert(&mut agent).unwrap();
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
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        store.agent_def_delete("def1").unwrap();
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

    // ---- Registry parallel-write mirror (PR A) ----

    fn make_named_inst(id: &str, name: &str, agents_root: &Path) -> AgentInstance {
        // working_directory must sit under <agents_root>/<slug> so the
        // relative-path resolver picks it up.
        let wd = agents_root.join(format!("{name}-fixture")).to_string_lossy().to_string();
        AgentInstance {
            id: id.to_string(),
            definition_id: "def-mirror".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 1_000,
            ended_at: 0,
            created_at: 900,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: name.to_string(),
            working_directory: wd,
            display_hidden: false,
        }
    }

    fn store_with_registry() -> (tempfile::TempDir, Store, Arc<crate::registry::Registry>) {
        let tmp = tempfile::tempdir().unwrap();
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let reg_root = agents_root.join("registry");
        let reg = Arc::new(crate::registry::Registry::open(reg_root).unwrap());
        let store = Store::open_in_memory().unwrap();
        store.set_registry(reg.clone());
        // Satisfy the FK from db_agent_instances.definition_id.
        let mut agent = sample_agent("def-mirror", "mirror");
        store.agent_def_insert(&mut agent).unwrap();
        (tmp, store, reg)
    }

    #[test]
    fn instance_create_named_writes_registry_file() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-1", "demo", &agents_root);
        store.instance_create(&inst).unwrap();
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data.instance_id, "inst-1");
        assert_eq!(records[0].data.instance_name, "demo");
        assert_eq!(records[0].data.identity_id, None);
        assert_eq!(records[0].data.memory_id, None);
        assert_eq!(records[0].data.working_dir, "demo-fixture");
    }

    #[test]
    fn instance_create_unnamed_does_not_mirror() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-2", "demo2", &agents_root);
        inst.instance_name = String::new(); // unnamed
        store.instance_create(&inst).unwrap();
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn instance_set_hidden_retires_then_unretires() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-3", "demo3", &agents_root);
        store.instance_create(&inst).unwrap();
        store.instance_set_hidden("inst-3", true).unwrap();
        assert!(reg.list_active().unwrap().is_empty());
        store.instance_set_hidden("inst-3", false).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);
    }

    #[test]
    fn instance_update_refreshes_registry_record() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-4", "demo4", &agents_root);
        store.instance_create(&inst).unwrap();
        let mut updated = inst.clone();
        updated.status = "paused".to_string();
        updated.session_id = "sess-xyz".to_string();
        store.instance_update(&updated).unwrap();
        // instance_update doesn't bump last_launched_at_ms (started_at is
        // immutable in the SQL update), so we just verify the record
        // still exists and is reachable.
        let records = reg.list_active().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data.instance_id, "inst-4");
    }

    #[test]
    fn instance_delete_removes_registry_file() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-5", "demo5", &agents_root);
        store.instance_create(&inst).unwrap();
        store.instance_delete("inst-5").unwrap();
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn instance_create_outside_agents_dir_skips_mirror() {
        let (tmp, store, reg) = store_with_registry();
        let mut inst = make_named_inst("inst-6", "demo6", tmp.path());
        // Override working_dir to live outside any "agents/" segment.
        inst.working_directory = tmp.path().join("projects").join("myrepo").to_string_lossy().to_string();
        store.instance_create(&inst).unwrap();
        // SQL row was written, but mirror is skipped because the working
        // dir can't be expressed as a relative subpath under agents/.
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn instance_create_user_path_with_agents_segment_is_skipped() {
        // Anchored-prefix check: a user-owned workspace at
        // `/home/user/code/agents/myproject` must NOT be mirrored. The
        // pre-fix scan-for-segment logic matched the inner "agents",
        // producing `working_dir = "myproject"` that would resolve to
        // `<shared>/agents/myproject` (wrong) when PR B reads the row.
        let (tmp, store, reg) = store_with_registry();
        // tmp is NOT under the registry's agents root, so this is a
        // user path that happens to include an "agents" component.
        let outside = tmp.path().join("code").join("agents").join("myproject");
        let mut inst = make_named_inst("inst-pathconfuse", "confuse", tmp.path());
        inst.working_directory = outside.to_string_lossy().to_string();
        store.instance_create(&inst).unwrap();
        assert!(reg.list_active().unwrap().is_empty(),
            "user path with inner 'agents' segment must not be mirrored");
    }

    #[test]
    fn instance_create_continuation_row_does_not_mirror() {
        // Registry mirror filter (per the doc comment in
        // registry_upsert_if_named) intentionally lags the SQLite
        // dropdown filter: continuation rows are NOT mirrored to
        // registry, even though `instance_list_named` does return
        // them under Option E. Cross-version dedup is the planned
        // follow-up; until then the registry-sourced read path
        // doesn't have the SQLite ORDER BY/LIMIT affordance and so
        // continues to gate on parent_instance_id == ''.
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let parent = make_named_inst("inst-parent", "demoP", &agents_root);
        store.instance_create(&parent).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);

        let mut child = make_named_inst("inst-child", "demoP", &agents_root);
        child.parent_instance_id = "inst-parent".to_string();
        store.instance_create(&child).unwrap();
        // Still only one record — the continuation row is NOT mirrored.
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.instance_id, "inst-parent");
    }

    #[test]
    fn instance_list_named_picker_mode_dedupes_continuation_chain() {
        // Discussion #1095 / SPEC_AGENT_ARCHITECTURE Phase 3b.1.
        // Before this dedup, a user with N continuations of one
        // logical agent saw N rows in "My Agents" (the user-visible
        // "4 Claudes" bug). Picker mode now collapses every chain
        // to its most-recent row.
        //
        // Test shape: head + one continuation, same name. Picker
        // returns ONE row — the continuation (latest started_at).
        // The chain's identity is preserved via `parent_instance_id`
        // on the surviving row.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Maks", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        cont.started_at = 200;
        store.instance_create(&cont).unwrap();

        let picker_rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(
            picker_rows.len(),
            1,
            "picker mode must collapse continuation chain to ONE entry"
        );
        assert_eq!(picker_rows[0].id, "inst-cont");
        // The surviving row keeps its real parent_instance_id —
        // callers needing to reconstruct the chain can still do so
        // by walking up from this row.
        assert_eq!(picker_rows[0].parent_instance_id, "inst-head");

        // Definition-scoped picker mode — same dedup behavior.
        let scoped = store
            .instance_list_named(10, Some("def-mirror"), None, true)
            .unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, "inst-cont");
    }

    #[test]
    fn instance_list_named_picker_mode_dedupes_long_chain() {
        // Regression for the 2026-05-27 "4 Claudes" report
        // (discussion #1095). The user's `db_agent_instances` had
        // 1 head + 4 continuations of the same agent. Picker mode
        // must return exactly 1 row — the most recent.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-root", "Claude", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        // 4 continuations chaining linearly off the head.
        for (i, parent) in [("c1", "inst-root"), ("c2", "c1"), ("c3", "c2"), ("c4", "c3")] {
            let mut c = make_named_inst(i, "Claude", &agents_root);
            c.parent_instance_id = parent.to_string();
            c.started_at = 100 + (i.chars().last().unwrap().to_digit(10).unwrap() as i64) * 100;
            store.instance_create(&c).unwrap();
        }

        let picker_rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(
            picker_rows.len(),
            1,
            "five-row chain (1 head + 4 continuations) must collapse to one entry"
        );
        assert_eq!(picker_rows[0].id, "c4", "newest continuation wins");
    }

    #[test]
    fn instance_get_by_name_reads_from_db_agents() {
        // Phase 3b.2 — the consolidated `db_agents` table is the new
        // authority for named-agent lookups. After `instance_create`'s
        // dual-write, the helper must surface the agent by name with
        // the bindings populated from `db_agents`. Transient runtime
        // fields (block_id, session_id, status, started_at as launch
        // moment, ended_at, parent_instance_id) have no analog in the
        // consolidated row and come back as their type defaults.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-1", "Maks", &agents_root);
        head.identity_id = "id-1".to_string();
        head.memory_id = "mem-1".to_string();
        head.github_context = "ghctx".to_string();
        store.instance_create(&head).unwrap();

        let got = store
            .instance_get_by_name("Maks")
            .unwrap()
            .expect("should find by name");
        // Folded user-clone: the def and the instance share ONE
        // db_agents row keyed by def.id (def-mirror is_seeded=0 in
        // the test fixture). The caller still sees `definition_id`
        // populated — via the COALESCE in the query, an empty
        // parent_template_id resolves to the row's own id.
        assert_eq!(got.id, "def-mirror");
        assert_eq!(got.definition_id, "def-mirror");
        assert_eq!(got.instance_name, "Maks");
        assert_eq!(got.identity_id, "id-1");
        assert_eq!(got.memory_id, "mem-1");
        assert_eq!(got.github_context, "ghctx");
        assert!(!got.display_hidden);
        // Transient fields default to empty / 0 — see doc comment.
        assert_eq!(got.parent_instance_id, "");
        assert_eq!(got.block_id, "");
        assert_eq!(got.session_id, "");
        assert_eq!(got.status, "");
        assert_eq!(got.ended_at, 0);
    }

    #[test]
    fn instance_get_by_name_returns_none_for_missing_name() {
        let (_tmp, store, _reg) = store_with_registry();
        assert!(store.instance_get_by_name("does-not-exist").unwrap().is_none());
    }

    #[test]
    fn instance_get_by_name_excludes_hidden_rows() {
        // user_hidden = 1 (via display_hidden) must filter out — both
        // the launch modal collision detect and ContinueNamed depend
        // on "forgotten" agents being invisible.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-hidden", "Ghost", &agents_root);
        inst.display_hidden = true;
        store.instance_create(&inst).unwrap();
        assert!(store.instance_get_by_name("Ghost").unwrap().is_none());
    }

    #[test]
    fn instance_get_by_name_empty_input_returns_none() {
        let (_tmp, store, _reg) = store_with_registry();
        assert!(store.instance_get_by_name("").unwrap().is_none());
    }

    #[test]
    fn continuation_mirrors_bindings_into_db_agents_user_clone_path() {
        // Codex P2 on PR #1110: when a named agent is continued with
        // different identity/memory/cwd/github_context, those bindings
        // must reach db_agents — otherwise `instance_get_by_name`
        // (which reads from db_agents) returns the head's stale data.
        // For user-clone defs (is_seeded=0), the projection is keyed
        // by def.id and the existing UPDATE handles both head and
        // continuation.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Original launch with one set of bindings.
        let mut head = make_named_inst("inst-head", "Maks", &agents_root);
        head.identity_id = "id-original".to_string();
        head.memory_id = "mem-original".to_string();
        store.instance_create(&head).unwrap();

        // Continuation with NEW bindings.
        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        cont.identity_id = "id-NEW".to_string();
        cont.memory_id = "mem-NEW".to_string();
        cont.github_context = "ghctx-NEW".to_string();
        store.instance_create(&cont).unwrap();

        // The folded db_agents row reflects the continuation's bindings.
        let got = store.instance_get_by_name("Maks").unwrap().expect("found");
        assert_eq!(got.id, "def-mirror");
        assert_eq!(
            got.identity_id, "id-NEW",
            "continuation bindings must overwrite the head's"
        );
        assert_eq!(got.memory_id, "mem-NEW");
        assert_eq!(got.github_context, "ghctx-NEW");
    }

    #[test]
    fn instance_get_by_name_collapses_continuation_chain_to_one_row() {
        // Continuations live in `db_agent_instances` (each launch =
        // one row). The Phase 3a dual-write projects them into ONE
        // canonical row in `db_agents` (keyed on the original head's
        // id, with bindings updated each continuation). So a 4-deep
        // chain with the same name surfaces as exactly one row here —
        // no MRU-row tie-breaking needed at this layer.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let head = make_named_inst("inst-head", "Maks", &agents_root);
        store.instance_create(&head).unwrap();

        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        store.instance_create(&cont).unwrap();

        let got = store.instance_get_by_name("Maks").unwrap().expect("found");
        // Only one db_agents row exists for the whole chain (the
        // folded user-clone row keyed by def-mirror); both
        // continuations updated its bindings.
        assert_eq!(got.id, "def-mirror");
        assert_eq!(got.instance_name, "Maks");
    }

    #[test]
    fn instance_list_named_picker_mode_keeps_distinct_agents_separate() {
        // Two unrelated chains (different agents, different names)
        // remain as two rows. The dedup is per-chain, not per-name.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head_a = make_named_inst("a-head", "AgentA", &agents_root);
        head_a.started_at = 100;
        store.instance_create(&head_a).unwrap();
        let mut cont_a = make_named_inst("a-cont", "AgentA", &agents_root);
        cont_a.parent_instance_id = "a-head".to_string();
        cont_a.started_at = 150;
        store.instance_create(&cont_a).unwrap();

        let mut head_b = make_named_inst("b-head", "AgentB", &agents_root);
        head_b.started_at = 200;
        store.instance_create(&head_b).unwrap();

        let rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(rows.len(), 2);
        // MRU order: b-head (200) before a-cont (150).
        assert_eq!(rows[0].id, "b-head");
        assert_eq!(rows[1].id, "a-cont");
    }

    #[test]
    fn instance_list_named_picker_mode_orphan_continuation_surfaces() {
        // Regression for codex P2 on PR #1096 bbe897cc: when a chain
        // head is hard-deleted via `deleteagentinstance` (no FK
        // cascade on `parent_instance_id`), descendant continuation
        // rows are orphaned — `parent_instance_id` points at an id
        // that no longer exists.
        //
        // The recursive CTE anchor must seed from BOTH (a) real
        // heads (`parent_instance_id = ''`) and (b) orphans (parent
        // doesn't exist in the table). Without the orphan anchor,
        // the recursive walk can't reach them and they disappear
        // from My Agents — even though they're recoverable sessions
        // the previous (buggy) query surfaced.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Seed a chain: head + 2 continuations.
        let mut head = make_named_inst("inst-deleted-head", "Claude", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont1 = make_named_inst("inst-orphan-cont1", "Claude", &agents_root);
        cont1.parent_instance_id = "inst-deleted-head".to_string();
        cont1.started_at = 200;
        store.instance_create(&cont1).unwrap();

        let mut cont2 = make_named_inst("inst-orphan-cont2", "Claude", &agents_root);
        cont2.parent_instance_id = "inst-orphan-cont1".to_string();
        cont2.started_at = 300;
        store.instance_create(&cont2).unwrap();

        // Hard-delete the head — no cascade, so cont1 + cont2 are
        // now orphaned (cont1.parent_instance_id points at the
        // deleted head; cont2 still has a valid parent).
        store.instance_delete("inst-deleted-head").unwrap();

        let rows = store.instance_list_named(10, None, None, true).unwrap();
        // The orphan chain (cont1 → cont2) must surface as ONE row:
        // cont1 becomes a root (its parent is gone); cont2 chains
        // off cont1. Newest in chain (cont2) wins.
        assert_eq!(
            rows.len(),
            1,
            "orphan chain must remain reachable after head deletion"
        );
        assert_eq!(rows[0].id, "inst-orphan-cont2");
    }

    #[test]
    fn instance_list_named_picker_mode_forget_suppresses_whole_chain() {
        // Regression for codex P2 on PR #1096: when the user clicks
        // "Forget" on a continuation row that's currently the picker's
        // surfaced entry, `hidenamedagent` flips `display_hidden=1`
        // only on that one row. If the dedup query filtered hidden
        // BEFORE ranking, the next-newest visible row in the same
        // chain would inherit `rn = 1` and the "forgotten" agent
        // would immediately reappear — making forget a no-op.
        //
        // Correct behavior: filter hidden AFTER ranking. When the
        // surfaced row is hidden, the entire chain disappears from
        // the picker until the user explicitly unhides one.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Claude", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont1 = make_named_inst("inst-cont1", "Claude", &agents_root);
        cont1.parent_instance_id = "inst-head".to_string();
        cont1.started_at = 200;
        store.instance_create(&cont1).unwrap();

        let mut cont2 = make_named_inst("inst-cont2", "Claude", &agents_root);
        cont2.parent_instance_id = "inst-cont1".to_string();
        cont2.started_at = 300;
        store.instance_create(&cont2).unwrap();

        // Before forget: chain surfaces as cont2 (newest).
        let before = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].id, "inst-cont2");

        // User clicks "Forget" on the surfaced row.
        store.instance_set_hidden("inst-cont2", true).unwrap();

        // After forget: the whole chain must stay forgotten — older
        // visible rows in the chain (head, cont1) must NOT bubble up.
        let after = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(
            after.len(),
            0,
            "hiding the surfaced row must suppress the entire chain — \
             older visible siblings must NOT be promoted to rn=1"
        );
    }

    #[test]
    fn instance_list_named_picker_mode_skips_hidden_chains() {
        // A hidden continuation should not win the ranking; its
        // sibling (if any) does. If the entire chain is hidden,
        // it disappears entirely.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Chain 1: head + cont, both visible.
        let mut head = make_named_inst("v-head", "Visible", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();
        let mut cont = make_named_inst("v-cont", "Visible", &agents_root);
        cont.parent_instance_id = "v-head".to_string();
        cont.started_at = 200;
        store.instance_create(&cont).unwrap();

        // Chain 2: head + cont, both hidden.
        let mut hidden_head = make_named_inst("h-head", "Hidden", &agents_root);
        hidden_head.started_at = 50;
        hidden_head.display_hidden = true;
        store.instance_create(&hidden_head).unwrap();
        let mut hidden_cont = make_named_inst("h-cont", "Hidden", &agents_root);
        hidden_cont.parent_instance_id = "h-head".to_string();
        hidden_cont.started_at = 60;
        hidden_cont.display_hidden = true;
        store.instance_create(&hidden_cont).unwrap();

        let rows = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "v-cont");
    }

    #[test]
    fn instance_list_named_picker_mode_identity_filter_in_ranking() {
        // Codex P2 #3 on PR #1096 0c4c8c46: identity_id filter must
        // participate in the dedup ranking. If we returned the newest
        // row in a chain and then filtered identity, a chain whose
        // newest row used a different identity would disappear from
        // the picker — even if an older row in the chain matched the
        // requested identity. Push the filter INTO the CTE so the
        // newest IDENTITY-MATCHING row per chain wins.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        // Chain: head with identity-a, cont with identity-b, then
        // another cont with identity-a (the user switched back).
        let mut head = make_named_inst("inst-head", "Claude", &agents_root);
        head.identity_id = "identity-a".to_string();
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont_b = make_named_inst("inst-cont-b", "Claude", &agents_root);
        cont_b.parent_instance_id = "inst-head".to_string();
        cont_b.identity_id = "identity-b".to_string();
        cont_b.started_at = 200;
        store.instance_create(&cont_b).unwrap();

        let mut cont_a2 = make_named_inst("inst-cont-a2", "Claude", &agents_root);
        cont_a2.parent_instance_id = "inst-cont-b".to_string();
        cont_a2.identity_id = "identity-a".to_string();
        cont_a2.started_at = 300;
        store.instance_create(&cont_a2).unwrap();

        // Filter by identity-a → newest identity-a row wins (cont-a2).
        let rows_a = store
            .instance_list_named(10, None, Some("identity-a"), true)
            .unwrap();
        assert_eq!(rows_a.len(), 1);
        assert_eq!(rows_a[0].id, "inst-cont-a2");

        // Filter by identity-b → only cont-b matches.
        let rows_b = store
            .instance_list_named(10, None, Some("identity-b"), true)
            .unwrap();
        assert_eq!(rows_b.len(), 1);
        assert_eq!(rows_b[0].id, "inst-cont-b");

        // No filter → newest in chain wins (cont-a2, started_at=300).
        let rows_all = store.instance_list_named(10, None, None, true).unwrap();
        assert_eq!(rows_all.len(), 1);
        assert_eq!(rows_all[0].id, "inst-cont-a2");
    }

    #[test]
    fn instance_list_named_picker_mode_identity_filter_recovers_older_match() {
        // Concrete repro for the bug codex described: chain where the
        // newest row uses identity-b but only older rows match the
        // requested identity-a. Without the in-ranking filter, the
        // chain would disappear when filtering by identity-a (newest
        // row is identity-b, gets ranked first, then post-filter
        // drops it). With the in-ranking filter, identity-a's older
        // row survives because it's the only candidate.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Claude", &agents_root);
        head.identity_id = "identity-a".to_string();
        head.started_at = 100;
        store.instance_create(&head).unwrap();

        let mut cont_newer_b = make_named_inst("inst-cont-newer-b", "Claude", &agents_root);
        cont_newer_b.parent_instance_id = "inst-head".to_string();
        cont_newer_b.identity_id = "identity-b".to_string();
        cont_newer_b.started_at = 200;
        store.instance_create(&cont_newer_b).unwrap();

        let rows = store
            .instance_list_named(10, None, Some("identity-a"), true)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "older identity-a row must survive even though the newest row uses identity-b"
        );
        assert_eq!(rows[0].id, "inst-head");
    }

    #[test]
    fn instance_list_named_dropdown_mode_excludes_continuations() {
        // Launch-modal "Continue agent" dropdown / `listnamedagents`
        // registry-enrichment path: `include_continuations = false`.
        // Symmetric with `registry_upsert_if_named`'s mirror filter —
        // a chain shows up as ONE entry (the head), not N entries
        // for every resume. Codex P1 on PR #1016 first cut: when the
        // enrichment path lost this filter, continuation rows could
        // displace registry-head rows under the `limit` truncation
        // and miss the merge-by-id enrichment.
        let (tmp, store, _reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");

        let mut head = make_named_inst("inst-head", "Maks", &agents_root);
        head.started_at = 100;
        store.instance_create(&head).unwrap();
        let mut cont = make_named_inst("inst-cont", "Maks", &agents_root);
        cont.parent_instance_id = "inst-head".to_string();
        cont.started_at = 200;
        store.instance_create(&cont).unwrap();

        let dropdown_rows = store.instance_list_named(10, None, None, false).unwrap();
        assert_eq!(
            dropdown_rows.len(),
            1,
            "legacy dropdown mode must drop continuation rows"
        );
        assert_eq!(dropdown_rows[0].id, "inst-head");

        // Definition-scoped dropdown mode — head only.
        let scoped_dropdown = store
            .instance_list_named(10, Some("def-mirror"), None, false)
            .unwrap();
        assert_eq!(scoped_dropdown.len(), 1);
        assert_eq!(scoped_dropdown[0].id, "inst-head");
    }

    #[test]
    fn instance_update_does_not_resurrect_hidden_row() {
        // Sequence: create (active) → set_hidden(true) → update.
        // The update must NOT move the file from retired/ back to active.
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-resurrect", "demoR", &agents_root);
        store.instance_create(&inst).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);

        store.instance_set_hidden("inst-resurrect", true).unwrap();
        assert!(reg.list_active().unwrap().is_empty(),
            "after set_hidden(true), record must be in retired/");
        assert!(reg.root().join("retired").join("inst-resurrect.json").exists());

        // SQLite still has the row (display_hidden=1). instance_update
        // would refresh it — the mirror must NOT re-add to active.
        let mut updated = inst.clone();
        updated.status = "stopped".to_string();
        updated.ended_at = 9999;
        store.instance_update(&updated).unwrap();

        assert!(reg.list_active().unwrap().is_empty(),
            "instance_update on a hidden row must NOT resurrect it");
        assert!(reg.root().join("retired").join("inst-resurrect.json").exists());
    }

    #[test]
    fn instance_create_with_display_hidden_writes_retired() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let mut inst = make_named_inst("inst-bornhidden", "demoH", &agents_root);
        inst.display_hidden = true;
        store.instance_create(&inst).unwrap();
        assert!(reg.list_active().unwrap().is_empty());
        assert!(reg.root().join("retired").join("inst-bornhidden.json").exists());
    }

    #[test]
    fn instance_update_toggling_hidden_off_unretires() {
        // Sequence: create → set_hidden(true) → set_hidden(false) →
        // update. After the toggle off, the file should be in active.
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst = make_named_inst("inst-toggle", "demoT", &agents_root);
        store.instance_create(&inst).unwrap();
        store.instance_set_hidden("inst-toggle", true).unwrap();
        store.instance_set_hidden("inst-toggle", false).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);

        // A subsequent update should preserve active state.
        let mut updated = inst.clone();
        updated.status = "paused".to_string();
        store.instance_update(&updated).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);
        assert!(!reg.root().join("retired").join("inst-toggle.json").exists(),
            "no orphan retired file alongside active");
    }

    #[test]
    fn instance_set_hidden_acts_on_registry_only_row() {
        // Cross-version case: a registry record exists (e.g. migrated
        // from another version's SQLite) but the current version's
        // SQLite has no matching row. `instance_set_hidden` must still
        // flip the registry file and report success.
        let (tmp, store, reg) = store_with_registry();
        // Seed a registry record directly — no SQLite row.
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("cross-ver");
        std::fs::create_dir_all(&wd).unwrap();
        reg.upsert(&crate::registry::NamedAgentRecord {
            schema_version: crate::registry::MAX_SUPPORTED_SCHEMA,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-crossver".to_string(),
                instance_name: "crossver".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                working_dir: "cross-ver".to_string(),
                created_at_ms: 100,
                last_launched_at_ms: 100,
                created_by_version: "0.33.821".to_string(),
                last_launched_by_version: "0.33.821".to_string(),
            },
        })
        .unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 1);
        assert!(store.instance_get("inst-crossver").unwrap().is_none(),
            "precondition: no SQLite row for cross-version agent");

        let result = store.instance_set_hidden("inst-crossver", true).unwrap();
        assert!(result, "must report success even when only registry was affected");
        assert!(reg.list_active().unwrap().is_empty(),
            "registry record must be retired");
        assert!(reg.root().join("retired").join("inst-crossver.json").exists());
    }

    #[test]
    fn agent_def_delete_cascade_removes_registry_files() {
        let (tmp, store, reg) = store_with_registry();
        let agents_root = tmp.path().join("agents");
        let inst_a = make_named_inst("inst-cascade-a", "demoA", &agents_root);
        let inst_b = make_named_inst("inst-cascade-b", "demoB", &agents_root);
        store.instance_create(&inst_a).unwrap();
        store.instance_create(&inst_b).unwrap();
        assert_eq!(reg.list_active().unwrap().len(), 2);

        // Delete the agent definition — SQLite FK cascades both instance
        // rows; the mirror must also drop both registry files.
        store.agent_def_delete("def-mirror").unwrap();
        assert!(reg.list_active().unwrap().is_empty(),
            "agent_def_delete cascade must remove all child instance registry files");
    }

    // ----------------------------------------------------------------
    // Phase 3a — db_agents dual-write coverage
    // ----------------------------------------------------------------

    fn count_agents(store: &Store, where_clause: &str) -> i64 {
        let conn = store.conn.lock().unwrap();
        let sql = format!("SELECT COUNT(*) FROM db_agents WHERE {where_clause}");
        conn.query_row(&sql, [], |row| row.get(0)).unwrap()
    }

    fn read_agent_field(store: &Store, id: &str, field: &str) -> Option<String> {
        let conn = store.conn.lock().unwrap();
        let sql = format!("SELECT {field} FROM db_agents WHERE id = ?1");
        let mut stmt = conn.prepare(&sql).unwrap();
        let r = stmt.query_row(params![id], |row| row.get::<_, String>(0));
        match r {
            Ok(s) => Some(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => panic!("query failed: {e}"),
        }
    }

    fn read_agent_int(store: &Store, id: &str, field: &str) -> Option<i64> {
        let conn = store.conn.lock().unwrap();
        let sql = format!("SELECT {field} FROM db_agents WHERE id = ?1");
        let mut stmt = conn.prepare(&sql).unwrap();
        let r = stmt.query_row(params![id], |row| row.get::<_, i64>(0));
        match r {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => panic!("query failed: {e}"),
        }
    }

    #[test]
    fn dual_write_agent_def_insert_seeded_creates_template_row() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "tpl-dw-seeded".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "desc".to_string(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut def).unwrap();
        // db_agents row exists, projected as template.
        assert_eq!(read_agent_int(&store, "tpl-dw-seeded", "is_template"), Some(1));
        assert_eq!(read_agent_field(&store, "tpl-dw-seeded", "parent_template_id"), Some(String::new()));
        assert_eq!(read_agent_field(&store, "tpl-dw-seeded", "name"), Some("Coder".to_string()));
        assert_eq!(read_agent_field(&store, "tpl-dw-seeded", "provider"), Some("claude".to_string()));
    }

    #[test]
    fn dual_write_agent_def_insert_user_clone_carries_parent() {
        let store = make_store();
        // Seed the template first so the FK exists in the old table.
        let mut tpl = AgentDefinition {
            id: "tpl-parent".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();
        // User-cloned def has is_seeded=0 + parent_id pointing at template.
        let mut user_def = tpl.clone();
        user_def.id = "def-user".to_string();
        user_def.slug = String::new();
        user_def.is_seeded = 0;
        user_def.parent_id = "tpl-parent".to_string();
        store.agent_def_insert(&mut user_def).unwrap();
        assert_eq!(read_agent_int(&store, "def-user", "is_template"), Some(0));
        assert_eq!(
            read_agent_field(&store, "def-user", "parent_template_id"),
            Some("tpl-parent".to_string())
        );
    }

    #[test]
    fn dual_write_agent_def_update_refreshes_name_in_db_agents() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "tpl-update".to_string(),
            slug: String::new(),
            name: "Old Name".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut def).unwrap();
        def.name = "New Name".to_string();
        assert!(store.agent_def_update(&mut def).unwrap());
        assert_eq!(
            read_agent_field(&store, "tpl-update", "name"),
            Some("New Name".to_string())
        );
    }

    #[test]
    fn dual_write_agent_def_delete_removes_db_agents_row() {
        let store = make_store();
        let mut def = AgentDefinition {
            id: "tpl-del".to_string(),
            slug: String::new(),
            name: "Goner".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut def).unwrap();
        assert_eq!(count_agents(&store, "id = 'tpl-del'"), 1);
        store.agent_def_delete("tpl-del").unwrap();
        assert_eq!(count_agents(&store, "id = 'tpl-del'"), 0);
    }

    #[test]
    fn dual_write_instance_create_inserts_user_clone_row() {
        let store = make_store();
        // Seed template.
        let mut tpl = AgentDefinition {
            id: "tpl-for-inst".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "desc".to_string(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();

        let inst = AgentInstance {
            id: "inst-dw".to_string(),
            definition_id: "tpl-for-inst".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: "id-1".to_string(),
            memory_id: "mem-1".to_string(),
            instance_name: "Maks".to_string(),
            working_directory: "/wd/maks".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        // Projected row: id == inst.id, is_template = 0, parent = tpl id,
        // bindings copied.
        assert_eq!(read_agent_int(&store, "inst-dw", "is_template"), Some(0));
        assert_eq!(
            read_agent_field(&store, "inst-dw", "parent_template_id"),
            Some("tpl-for-inst".to_string())
        );
        assert_eq!(read_agent_field(&store, "inst-dw", "name"), Some("Maks".to_string()));
        assert_eq!(read_agent_field(&store, "inst-dw", "identity_id"), Some("id-1".to_string()));
        assert_eq!(read_agent_field(&store, "inst-dw", "memory_id"), Some("mem-1".to_string()));
        assert_eq!(read_agent_field(&store, "inst-dw", "working_directory"), Some("/wd/maks".to_string()));
        // Continuation rows skipped.
        let cont = AgentInstance {
            id: "inst-cont".to_string(),
            parent_instance_id: "inst-dw".to_string(),
            ..inst.clone()
        };
        store.instance_create(&cont).unwrap();
        assert_eq!(count_agents(&store, "id = 'inst-cont'"), 0);
    }

    /// Reagent P1 + P2 on #1013 round 2 — pins the user-cloned-def
    /// branch of `agents_dual_write_instance_create` so it matches
    /// the backfill rule (`agents_consolidate.rs::backfill_instances`
    /// folds the instance's bindings into the EXISTING `db_agents`
    /// row keyed by `def.id`, NOT a fresh row keyed by `inst.id`).
    /// Round-1 test only covered the seeded-template branch.
    #[test]
    fn dual_write_instance_create_folds_into_user_clone_def() {
        let store = make_store();
        // Seed a template.
        let mut tpl = AgentDefinition {
            id: "tpl-folded".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: "desc".to_string(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();

        // User-clone of the template (is_seeded = 0, parent_id = tpl id).
        let mut clone = AgentDefinition {
            id: "user-clone-1".to_string(),
            slug: String::new(),
            name: "Maks".to_string(),
            is_seeded: 0,
            parent_id: "tpl-folded".to_string(),
            created_at: 1500,
            updated_at: 1500,
            ..tpl.clone()
        };
        store.agent_def_insert(&mut clone).unwrap();
        // The user-clone projection in db_agents starts with empty bindings.
        assert_eq!(read_agent_field(&store, "user-clone-1", "identity_id"), Some(String::new()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "memory_id"), Some(String::new()));

        // Create an instance ON the user-clone def. Per backfill rule,
        // this must FOLD the instance's bindings into the existing
        // user-clone-1 row — NOT create a separate inst-fold-1 row
        // with parent_template_id pointing at a non-template row.
        let inst = AgentInstance {
            id: "inst-fold-1".to_string(),
            definition_id: "user-clone-1".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: "gh-ctx-A".to_string(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: "id-folded".to_string(),
            memory_id: "mem-folded".to_string(),
            instance_name: "Maks v2".to_string(),
            working_directory: "/wd/folded".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        // No new row keyed by inst.id — backfill never creates one for
        // user-clone-def instances, and dual-write must match.
        assert_eq!(count_agents(&store, "id = 'inst-fold-1'"), 0);

        // Bindings folded onto the user-clone-1 row.
        assert_eq!(read_agent_field(&store, "user-clone-1", "identity_id"), Some("id-folded".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "memory_id"), Some("mem-folded".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "working_directory"), Some("/wd/folded".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "github_context"), Some("gh-ctx-A".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "instance_name"), Some("Maks v2".to_string()));
        assert_eq!(read_agent_field(&store, "user-clone-1", "name"), Some("Maks v2".to_string()));
        // is_template stays 0, parent_template_id untouched (still empty
        // since user-clone insert leaves it blank).
        assert_eq!(read_agent_int(&store, "user-clone-1", "is_template"), Some(0));
    }

    #[test]
    fn dual_write_instance_set_hidden_flips_user_hidden_bit() {
        let store = make_store();
        let mut tpl = AgentDefinition {
            id: "tpl-hide".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();
        let inst = AgentInstance {
            id: "inst-hide".to_string(),
            definition_id: "tpl-hide".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "H".to_string(),
            working_directory: "/wd/h".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        assert_eq!(read_agent_int(&store, "inst-hide", "user_hidden"), Some(0));
        store.instance_set_hidden("inst-hide", true).unwrap();
        assert_eq!(read_agent_int(&store, "inst-hide", "user_hidden"), Some(1));
        store.instance_set_hidden("inst-hide", false).unwrap();
        assert_eq!(read_agent_int(&store, "inst-hide", "user_hidden"), Some(0));
    }

    #[test]
    fn dual_write_instance_delete_drops_db_agents_row() {
        let store = make_store();
        let mut tpl = AgentDefinition {
            id: "tpl-instdel".to_string(),
            slug: String::new(),
            name: "Coder".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();
        let inst = AgentInstance {
            id: "inst-del".to_string(),
            definition_id: "tpl-instdel".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "D".to_string(),
            working_directory: "/wd/d".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        assert_eq!(count_agents(&store, "id = 'inst-del'"), 1);
        store.instance_delete("inst-del").unwrap();
        assert_eq!(count_agents(&store, "id = 'inst-del'"), 0);
    }

    #[test]
    fn dual_write_instance_repoint_updates_parent_template_id() {
        let store = make_store();
        let mut tpl_a = AgentDefinition {
            id: "tpl-A".to_string(),
            slug: String::new(),
            name: "A".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl_a).unwrap();
        let mut tpl_b = tpl_a.clone();
        tpl_b.id = "tpl-B".to_string();
        tpl_b.slug = String::new();
        store.agent_def_insert(&mut tpl_b).unwrap();

        let inst = AgentInstance {
            id: "inst-rp".to_string(),
            definition_id: "tpl-A".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: "R".to_string(),
            working_directory: "/wd/r".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        assert_eq!(
            read_agent_field(&store, "inst-rp", "parent_template_id"),
            Some("tpl-A".to_string())
        );
        store.instance_repoint_definition("tpl-A", "tpl-B").unwrap();
        assert_eq!(
            read_agent_field(&store, "inst-rp", "parent_template_id"),
            Some("tpl-B".to_string())
        );
    }

    #[test]
    fn dual_write_agent_def_delete_seeded_drops_all_template_rows() {
        let store = make_store();
        for id in &["s1", "s2", "s3"] {
            let mut d = AgentDefinition {
                id: id.to_string(),
                slug: String::new(),
                name: id.to_string(),
                icon: "✦".to_string(),
                provider: "claude".to_string(),
                description: String::new(),
                working_directory: String::new(),
                shell: "bash".to_string(),
                provider_flags: String::new(),
                auto_start: 0,
                restart_on_crash: 0,
                idle_timeout_minutes: 0,
                created_at: 1000,
                agent_type: "standalone".to_string(),
                environment: String::new(),
                agent_bus_id: String::new(),
                is_seeded: 1,
                accounts: String::new(),
                parent_id: String::new(),
                branch_label: String::new(),
                updated_at: 1000,
                user_hidden: 0,
            };
            store.agent_def_insert(&mut d).unwrap();
        }
        assert_eq!(count_agents(&store, "is_template = 1"), 3);
        store.agent_def_delete_seeded().unwrap();
        assert_eq!(count_agents(&store, "is_template = 1"), 0);
    }

    /// Reagent P2 round 4 on #1013 — pins the seeded-bulk-delete
    /// scope: templates + cascaded INSTANCE projections go;
    /// user-clone DEF projections survive.
    #[test]
    fn dual_write_seeded_delete_preserves_user_clone_def_projections() {
        let store = make_store();
        // Seeded template.
        let mut tpl = AgentDefinition {
            id: "tpl-keep-check".to_string(),
            slug: String::new(),
            name: "TplCheck".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();
        // User-clone DEF of that template (Phase 1 created this).
        let mut clone = AgentDefinition {
            id: "user-clone-keep".to_string(),
            slug: String::new(),
            name: "MaksKeeper".to_string(),
            is_seeded: 0,
            parent_id: "tpl-keep-check".to_string(),
            created_at: 1500,
            updated_at: 1500,
            ..tpl.clone()
        };
        store.agent_def_insert(&mut clone).unwrap();
        // Instance ON the seeded template (cascaded instance projection).
        let inst_on_tpl = AgentInstance {
            id: "inst-on-tpl-keep".to_string(),
            definition_id: "tpl-keep-check".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: String::new(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst_on_tpl).unwrap();
        // Now delete seeded → template + cascaded instance go, user-clone survives.
        store.agent_def_delete_seeded().unwrap();
        assert_eq!(count_agents(&store, "id = 'tpl-keep-check'"), 0, "template projection gone");
        assert_eq!(count_agents(&store, "id = 'inst-on-tpl-keep'"), 0, "cascaded instance projection gone");
        assert_eq!(count_agents(&store, "id = 'user-clone-keep'"), 1, "user-clone def projection survives");
    }

    /// Reagent P2 round 4 on #1013 — pins instance_update/hide/delete
    /// routing through the projection key. The previous version keyed
    /// everything on `inst.id` and silently no-op'd on folded rows.
    #[test]
    fn dual_write_instance_lifecycle_on_user_clone_def_routes_to_folded_row() {
        let store = make_store();
        // Template, user-clone def of it, instance on the user-clone.
        let mut tpl = AgentDefinition {
            id: "tpl-rt".to_string(),
            slug: String::new(),
            name: "Tpl".to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
        };
        store.agent_def_insert(&mut tpl).unwrap();
        let mut clone = AgentDefinition {
            id: "user-rt".to_string(),
            slug: String::new(),
            name: "Maks".to_string(),
            is_seeded: 0,
            parent_id: "tpl-rt".to_string(),
            created_at: 1500,
            updated_at: 1500,
            ..tpl.clone()
        };
        store.agent_def_insert(&mut clone).unwrap();
        let inst = AgentInstance {
            id: "inst-rt".to_string(),
            definition_id: "user-rt".to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "running".to_string(),
            github_context: "gh-initial".to_string(),
            started_at: 2000,
            ended_at: 0,
            created_at: 2000,
            identity_id: "id-init".to_string(),
            memory_id: "mem-init".to_string(),
            instance_name: "Maks v1".to_string(),
            working_directory: "/wd".to_string(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();
        // Sanity: no inst-rt row (folded).
        assert_eq!(count_agents(&store, "id = 'inst-rt'"), 0);
        assert_eq!(read_agent_field(&store, "user-rt", "github_context"), Some("gh-initial".to_string()));

        // instance_update: github_context flows through to the folded row.
        let updated = AgentInstance {
            github_context: "gh-updated".to_string(),
            ..inst.clone()
        };
        store.instance_update(&updated).unwrap();
        assert_eq!(
            read_agent_field(&store, "user-rt", "github_context"),
            Some("gh-updated".to_string()),
            "instance_update on user-clone-def routes to folded row",
        );

        // instance_set_hidden: flips user_hidden on the folded row.
        store.instance_set_hidden("inst-rt", true).unwrap();
        assert_eq!(
            read_agent_int(&store, "user-rt", "user_hidden"),
            Some(1),
            "instance_set_hidden routes to folded row",
        );

        // instance_delete: NO-OP on folded row (the def projection persists).
        store.instance_delete("inst-rt").unwrap();
        assert_eq!(
            count_agents(&store, "id = 'user-rt'"),
            1,
            "instance_delete on user-clone-def is a no-op (def projection persists)",
        );
    }
}

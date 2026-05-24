// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WaveStore: generic OID-based CRUD for WaveObj types.
//! Port of Go's pkg/wstore/wstore_dbops.go + wstore_dbsetup.go.
//!
//! Uses `Mutex<Connection>` matching Go's `MaxOpenConns(1)`.
//! SQLite WAL mode + 5s busy timeout (same as Go).


use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::backend::obj::{wave_obj_from_json, wave_obj_to_json, WaveObj};
use crate::registry::Registry;

use super::error::StoreError;
use super::migrations::{run_object_schema, stamp_and_check_version, OBJECT_SCHEMA_VERSION};

/// SQLite-backed object store for WaveObj types.
pub struct WaveStore {
    conn: Mutex<Connection>,
    /// Cross-version named-agent registry. `None` for in-memory test
    /// stores; `Some` for production srv. Mutations to
    /// `db_agent_instances` parallel-write to this registry when set.
    /// See `docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md`.
    registry: Mutex<Option<Arc<Registry>>>,
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
    /// own per-table CRUD via the `DroneStore` extension trait
    /// pattern (see `agentmux-srv/src/drone/storage.rs`). Outside
    /// callers must use the typed methods on this impl.
    pub(crate) fn conn(&self) -> &Mutex<Connection> {
        &self.conn
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(
            // `foreign_keys=ON` is per-connection and defaults to OFF in
            // SQLite. The schema (`db_agent_identity_links` etc.) relies
            // on `ON DELETE CASCADE` to clean up junction rows when a
            // parent agent is removed. Without this pragma on the
            // production connection, cascades silently no-op; migration
            // tests set it explicitly, which would have masked the gap.
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;",
        )?;
        run_object_schema(&conn)?;
        stamp_and_check_version(&conn, OBJECT_SCHEMA_VERSION, "objects.db")?;
        Ok(Self {
            conn: Mutex::new(conn),
            registry: Mutex::new(None),
        })
    }

    /// Attach a shared cross-version agent registry. Called once on
    /// srv startup after `WaveStore::open` and before the store is
    /// wrapped in `Arc`. Mutations to named agent rows will then
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

impl WaveStore {
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
    ///
    /// Phase 3c: writes directly to `db_agents`. The deletion targets:
    ///   1. Every template row (`is_template = 1`) — the canonical
    ///      seeded templates.
    ///   2. Every template-instance projection row keyed off a template
    ///      that was just removed (i.e. `is_template = 0` rows whose
    ///      `parent_template_id` matched one of the templates we
    ///      deleted). Under the old schema the FK cascade on
    ///      `db_agent_instances.definition_id` did this automatically;
    ///      with `db_agents` having no parent FK on
    ///      `parent_template_id`, we drop those rows explicitly.
    ///
    /// User-clone DEFINITION rows (created via "+ New from template" —
    /// `is_template = 0`, id NOT in `db_agent_instances`) are NOT
    /// touched: those persist as long as the underlying def row does.
    pub fn agent_def_delete_seeded(&self) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Capture the template ids BEFORE the delete so we can scope
        // the cascaded-instance cleanup. The instance projections to
        // drop are exactly those rows whose `parent_template_id`
        // matched one of the just-deleted templates AND whose `id`
        // was an actual instance id (i.e. has a non-empty
        // `definition_id` field; user-clone DEF rows leave
        // definition_id empty).
        let template_ids: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT id FROM db_agents WHERE is_template = 1",
            )?;
            let ids = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            ids
        };
        // Drop the cascaded template-instance projections first, while
        // we still have the parent ids. An instance row in db_agents
        // carries `definition_id` (the runtime field — non-empty for
        // instances, empty for user-clone DEF rows).
        for tpl_id in &template_ids {
            conn.execute(
                "DELETE FROM db_agents
                 WHERE is_template = 0
                   AND parent_template_id = ?1
                   AND definition_id <> ''",
                params![tpl_id],
            )?;
        }
        // Drop the templates themselves.
        let rows = conn.execute(
            "DELETE FROM db_agents WHERE is_template = 1",
            [],
        )?;
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
    ///
    /// Phase 3c: writes directly to the consolidated `db_agents` table.
    /// Templates (`is_seeded = 1`) land with `is_template = 1` and
    /// empty bindings; user-clone definitions land with `is_template
    /// = 0` + `parent_template_id = parent_id`.
    pub fn agent_def_insert(&self, agent: &mut AgentDefinition) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let base = if agent.slug.is_empty() {
            derive_slug(&agent.name)
        } else {
            agent.slug.clone()
        };
        // Collision-resolve against `db_agents.slug`. The consolidated
        // table surfaces template-instance projections too (which under
        // the old schema didn't have slugs), so this is a strict
        // superset of the legacy uniqueness check.
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
        let is_template = if agent.is_seeded == 1 { 1_i64 } else { 0_i64 };
        let parent_template_id = if agent.is_seeded == 1 {
            String::new()
        } else {
            agent.parent_id.clone()
        };
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
             )",
            params![
                agent.id,
                agent.name,
                agent.icon,
                agent.description,
                is_template,
                parent_template_id,
                agent.provider,
                agent.provider_flags,
                agent.shell,
                agent.environment,
                agent.agent_type,
                agent.agent_bus_id,
                agent.accounts,
                agent.auto_start,
                agent.restart_on_crash,
                agent.idle_timeout_minutes,
                agent.slug,
                agent.branch_label,
                agent.created_at,
                // New rows: updated_at == created_at.
                agent.created_at,
                agent.is_seeded,
                // Phase 2 (hide templates): new rows start visible.
                // The agent-seed re-sync forces user_hidden = 0 on any
                // newly-added template id, so honouring the caller-
                // supplied value here is safe even when a stray 1
                // sneaks through.
                agent.user_hidden,
            ],
        )?;
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
        // Phase 3c: read + write both target `db_agents`. Templates
        // carry `is_template = 1` and are the only rows allowed to
        // flip the hide flag; user-owned rows (`is_template = 0`)
        // reject — their removal path is `agent_def_delete`, not hide.
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
        let rows = conn.execute(
            "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 1",
            params![if hidden { 1_i64 } else { 0_i64 }, id],
        )?;
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
            // Phase 3c: writes the consolidated `db_agents` row in
            // place. `working_directory` is preserved on the agent row
            // (it was tracked on `db_agent_instances` previously, but
            // we keep it on db_agents for user-owned rows so the
            // existing AgentDefinition wire shape stays intact).
            conn.execute(
                "UPDATE db_agents
                 SET name = ?1,
                     icon = ?2,
                     provider = ?3,
                     description = ?4,
                     working_directory = ?5,
                     shell = ?6,
                     provider_flags = ?7,
                     auto_start = ?8,
                     restart_on_crash = ?9,
                     idle_timeout_minutes = ?10,
                     agent_type = ?11,
                     environment = ?12,
                     agent_bus_id = ?13,
                     accounts = ?14,
                     updated_at = ?15
                 WHERE id = ?16",
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
                    agent.id,
                ],
            )?
        };
        // Reflect the persisted timestamp back to the caller's struct so an
        // RPC response carries the fresh value, not the pre-update one.
        agent.updated_at = now;
        Ok(rows > 0)
    }

    /// Delete an agent by id. Returns true if a row was deleted.
    ///
    /// Phase 3c: writes directly to `db_agents`. Cascades through any
    /// template-instance projection rows whose `parent_template_id`
    /// matches `id` (the old FK on `db_agent_instances.definition_id`
    /// did this automatically). The cross-version registry mirror is
    /// kept in sync: any registry record whose `definition_id` field
    /// references this agent is removed too, so "Forget agent"
    /// cascades correctly even for folded instances (whose db_agents
    /// row already merged into the def and so doesn't appear in the
    /// cascade SELECT).
    pub fn agent_def_delete(&self, id: &str) -> Result<bool, StoreError> {
        let (cascaded_instance_ids, rows) = {
            let conn = self.conn.lock().unwrap();
            // Collect template-instance projection ids so we can drop
            // them + their registry tombstones in one pass.
            let cascaded_instance_ids: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM db_agents
                     WHERE is_template = 0
                       AND parent_template_id = ?1
                       AND id <> ?1",
                )?;
                let iter = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
                let v = iter.collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                v
            };
            // Drop the cascaded template-instance projection rows.
            for instance_id in &cascaded_instance_ids {
                conn.execute(
                    "DELETE FROM db_agents WHERE id = ?1",
                    params![instance_id],
                )?;
            }
            // Drop the agent row itself.
            let rows = conn.execute(
                "DELETE FROM db_agents WHERE id = ?1",
                params![id],
            )?;
            (cascaded_instance_ids, rows)
        };
        if rows > 0 {
            if let Some(reg) = self.registry() {
                // Cascade explicit template-instance ids.
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
                // Cascade any registry record whose `definition_id`
                // points at this agent — covers the folded-instance
                // case (where the inst.id never had a separate
                // db_agents row, so the SELECT above didn't see it).
                if let Ok(records) = reg.list_active() {
                    for rec in records {
                        if rec.data.definition_id == id {
                            if let Err(e) = reg.hard_delete(&rec.data.instance_id) {
                                tracing::warn!(
                                    instance_id = %rec.data.instance_id,
                                    agent_def_id = %id,
                                    error = %e,
                                    "registry: failed to mirror agent_def_delete cascade (folded)"
                                );
                            }
                        }
                    }
                }
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
    //
    // Phase 3c retired `db_agent_instances`. Every "instance" is now a
    // row in `db_agents` with `is_template = 0` and a non-empty
    // `definition_id` (set when `instance_create` runs against the
    // agent row — empty for user-clone DEF rows that never spawned).
    //
    // Two row shapes carry instance semantics:
    //   - Template-instance projection: id = old instance.id,
    //     parent_template_id = template id, definition_id = template id.
    //   - User-clone-def fold: id = def.id, parent_template_id = the
    //     template the user cloned from, definition_id = def.id.
    // Both shape resolve to `is_template = 0`. Callers that need to
    // distinguish "has run before" from "fresh user-clone" check
    // `definition_id <> ''`.

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
                    user_hidden
             FROM db_agents
             WHERE is_template = 0
               AND definition_id <> ''",
        );
        let mut clauses: Vec<&str> = Vec::new();
        if definition_id.is_some() {
            clauses.push("definition_id = ?");
        }
        if status.is_some() {
            clauses.push("status = ?");
        }
        if !clauses.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = conn.prepare(&sql)?;
        // Build a Vec<String> so parameter lifetimes outlive the query call.
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
                    user_hidden
             FROM db_agents
             WHERE id = ?1 AND is_template = 0 AND definition_id <> ''",
        )?;
        let result = stmt.query_row(params![id], map_instance_row);
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Create a new instance row on the consolidated `db_agents`.
    /// Caller is responsible for the id (UUID).
    ///
    /// Phase 3c routing:
    ///   - If the parent definition row in `db_agents` is a user-clone
    ///     DEF (`is_template = 0`, `definition_id = ''`), FOLD the
    ///     instance's bindings + lifecycle into that row in place
    ///     (no new row).
    ///   - Otherwise (template parent), INSERT a new template-instance
    ///     projection row keyed by `inst.id`.
    ///   - Continuation rows (`parent_instance_id` non-empty) are a
    ///     pre-Option-E concept the consolidation retires: they don't
    ///     get their own db_agents row.
    pub fn instance_create(&self, inst: &AgentInstance) -> Result<(), StoreError> {
        if !inst.parent_instance_id.is_empty() {
            // Continuation row — no db_agents projection. Registry
            // mirror is intentionally skipped too (matches the legacy
            // `registry_upsert_if_named` filter: continuation rows
            // would otherwise duplicate the dropdown entry).
            return Ok(());
        }
        {
            let conn = self.conn.lock().unwrap();
            // Pull the parent agent row (the "definition" the instance
            // is being created against).
            let parent = match Self::load_agent_row_for_instance(&conn, &inst.definition_id)? {
                Some(p) => p,
                None => {
                    // Orphan instance — no matching agent row in
                    // db_agents. Log and skip; the legacy schema
                    // tolerated this case (no FK enforcement on test
                    // stores).
                    tracing::error!(
                        instance_id = %inst.id,
                        definition_id = %inst.definition_id,
                        "instance_create: parent agent row missing; skipping projection",
                    );
                    return Ok(());
                }
            };

            let name = if inst.instance_name.is_empty() {
                parent.name.clone()
            } else {
                inst.instance_name.clone()
            };

            // Compute a monotonic updated_at past the table-wide max
            // so successive fast-fire creates produce a strict total
            // order on the sort key. SQLite millisecond resolution on
            // Windows collides under fast test loops.
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
            let now_ms = std::cmp::max(wall_now, global_prior.saturating_add(1));

            if parent.is_seeded == 0 {
                // User-clone DEF parent — fold the instance into the
                // def row. This preserves the consolidation rule (one
                // agent row per user agent) even on subsequent launch
                // events.
                conn.execute(
                    "UPDATE db_agents SET
                        name = ?2,
                        identity_id = ?3,
                        memory_id = ?4,
                        working_directory = ?5,
                        github_context = ?6,
                        instance_name = ?7,
                        definition_id = ?8,
                        parent_instance_id = ?9,
                        block_id = ?10,
                        session_id = ?11,
                        status = ?12,
                        started_at = ?13,
                        ended_at = ?14,
                        updated_at = ?15,
                        user_hidden = ?16
                     WHERE id = ?1",
                    params![
                        parent.id,
                        name,
                        inst.identity_id,
                        inst.memory_id,
                        inst.working_directory,
                        inst.github_context,
                        inst.instance_name,
                        inst.definition_id,
                        inst.parent_instance_id,
                        inst.block_id,
                        inst.session_id,
                        inst.status,
                        inst.started_at,
                        inst.ended_at,
                        now_ms,
                        if inst.display_hidden { 1_i64 } else { 0_i64 },
                    ],
                )?;
            } else {
                // Template parent — fresh template-instance projection
                // row keyed by `inst.id`. Copy provider/cmd config from
                // the parent so the row is self-describing.
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
                        definition_id, parent_instance_id,
                        block_id, session_id, status,
                        started_at, ended_at,
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
                        ?23, ?24,
                        ?25, ?26, ?27,
                        ?28, ?29,
                        ?30, ?31, 0, ?32
                     )
                     ON CONFLICT(id) DO UPDATE SET
                        name = excluded.name,
                        identity_id = excluded.identity_id,
                        memory_id = excluded.memory_id,
                        working_directory = excluded.working_directory,
                        github_context = excluded.github_context,
                        instance_name = excluded.instance_name,
                        definition_id = excluded.definition_id,
                        block_id = excluded.block_id,
                        session_id = excluded.session_id,
                        status = excluded.status,
                        started_at = excluded.started_at,
                        ended_at = excluded.ended_at,
                        updated_at = excluded.updated_at,
                        user_hidden = excluded.user_hidden",
                    params![
                        inst.id,
                        name,
                        parent.icon,
                        parent.description,
                        parent.id, // parent_template_id = template def id
                        parent.provider,
                        parent.provider_flags,
                        parent.shell,
                        parent.environment,
                        parent.agent_type,
                        parent.agent_bus_id,
                        parent.accounts,
                        parent.auto_start,
                        parent.restart_on_crash,
                        parent.idle_timeout_minutes,
                        parent.slug, // slug carried for completeness; collision-protected by index
                        parent.branch_label,
                        inst.identity_id,
                        inst.memory_id,
                        inst.working_directory,
                        inst.github_context,
                        inst.instance_name,
                        inst.definition_id,
                        inst.parent_instance_id,
                        inst.block_id,
                        inst.session_id,
                        inst.status,
                        inst.started_at,
                        inst.ended_at,
                        inst.created_at,
                        now_ms,
                        if inst.display_hidden { 1_i64 } else { 0_i64 },
                    ],
                )?;
            }
        }
        self.registry_upsert_if_named(inst);
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
    ///
    /// Phase 3c: routes by the projection key so a folded user-clone-def
    /// instance flips the bit on its def row.
    pub fn instance_set_hidden(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            // Locate the row this instance projects onto. A
            // template-instance projection lives at `id`; a folded
            // user-clone-def projection lives at `def.id` but the
            // instance's `definition_id` column carries that key —
            // and since folds write definition_id = def.id, the row
            // we want IS the row at `id`. Practical net: the WHERE
            // clause hits the right row in either shape (the fold
            // row has id == def.id; the template-instance projection
            // row has id == inst.id).
            //
            // For folded rows, instance.id != def.id — we need to
            // resolve the routing. Look up the definition_id from
            // the row keyed by `id`; if that row is a fold (id ==
            // definition_id), flip the user_hidden on it. Otherwise
            // (template-instance projection or freestanding row),
            // also flip user_hidden on the same row.
            conn.execute(
                "UPDATE db_agents SET user_hidden = ?1
                 WHERE is_template = 0
                   AND (
                        id = ?2
                        OR (definition_id = ?2 AND id = definition_id)
                   )",
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
        Ok(rows > 0 || registry_acted)
    }

    /// List all instances that the launch modal's "Continue agent"
    /// dropdown should surface: have a non-empty `instance_name`, not
    /// hidden, sorted by most-recent start. Capped to keep the
    /// dropdown's wire payload bounded.
    ///
    /// `definition_id`, when provided, restricts the result to
    /// instances of that definition.
    pub fn instance_list_named(
        &self,
        limit: usize,
        definition_id: Option<&str>,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let sql = if definition_id.is_some() {
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id, instance_name, working_directory,
                    user_hidden
             FROM db_agents
             WHERE is_template = 0
               AND definition_id <> ''
               AND user_hidden = 0
               AND instance_name <> ''
               AND parent_instance_id = ''
               AND definition_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2"
        } else {
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id, instance_name, working_directory,
                    user_hidden
             FROM db_agents
             WHERE is_template = 0
               AND definition_id <> ''
               AND user_hidden = 0
               AND instance_name <> ''
               AND parent_instance_id = ''
             ORDER BY started_at DESC
             LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql)?;
        let iter = match definition_id {
            Some(def) => stmt.query_map(params![def, limit as i64], map_instance_row)?,
            None => stmt.query_map(params![limit as i64], map_instance_row)?,
        };
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Look up the most recent (by `started_at`) named instance that
    /// matches the given `instance_name`. Used by the launch modal to
    /// detect name collisions ("did you mean to continue?") and by
    /// `ContinueNamedAgentCommand` to resolve the canonical row when
    /// the caller only knows the name. Hidden rows are excluded.
    pub fn instance_get_by_name(
        &self,
        instance_name: &str,
    ) -> Result<Option<AgentInstance>, StoreError> {
        if instance_name.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id, instance_name, working_directory,
                    user_hidden
             FROM db_agents
             WHERE is_template = 0
               AND definition_id <> ''
               AND user_hidden = 0
               AND instance_name = ?1
             ORDER BY started_at DESC
             LIMIT 1",
        )?;
        let result = stmt.query_row(params![instance_name], map_instance_row);
        match result {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update mutable instance fields on the consolidated `db_agents`
    /// row. `id`, `definition_id`, `parent_instance_id`, `started_at`,
    /// `created_at` are immutable after insert.
    pub fn instance_update(&self, inst: &AgentInstance) -> Result<bool, StoreError> {
        // Continuation rows don't have their own projection — no-op.
        if !inst.parent_instance_id.is_empty() {
            return Ok(false);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let rows = {
            let conn = self.conn.lock().unwrap();
            // Monotonic floor on updated_at so the picker's recency
            // sort sees a strict total order even under fast loops.
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
                    block_id = ?1,
                    session_id = ?2,
                    status = ?3,
                    github_context = ?4,
                    ended_at = ?5,
                    updated_at = ?6
                 WHERE is_template = 0
                   AND (
                        id = ?7
                        OR (definition_id = ?7 AND id = definition_id)
                   )",
                params![
                    inst.block_id,
                    inst.session_id,
                    inst.status,
                    inst.github_context,
                    inst.ended_at,
                    now_monotonic,
                    inst.id,
                ],
            )?
        };
        if rows > 0 {
            // Refresh the registry from the post-update authoritative row.
            if let Ok(Some(fresh)) = self.instance_get(&inst.id) {
                self.registry_upsert_if_named(&fresh);
            }
        }
        Ok(rows > 0)
    }

    /// Repoint every instance currently referencing `old_def_id` to
    /// `new_def_id`. Used by the Phase 1 two-tier-picker migration
    /// (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
    ///
    /// Phase 3c: writes directly to `db_agents`. The instance's
    /// `definition_id` column is rewritten, and `parent_template_id`
    /// on template-instance projections is repointed too so the
    /// reattach flow stays consistent.
    pub fn instance_repoint_definition(
        &self,
        old_def_id: &str,
        new_def_id: &str,
    ) -> Result<usize, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            // Repoint the runtime definition_id pointer + the template
            // lineage pointer on template-instance projections, all in
            // one UPDATE. The WHERE filter restricts to rows whose
            // `definition_id` matches (instances), so user-clone DEF
            // rows (definition_id empty until first instance_create)
            // and template rows (is_template = 1) are untouched.
            conn.execute(
                "UPDATE db_agents
                 SET definition_id = ?1,
                     parent_template_id = CASE
                        WHEN parent_template_id = ?2 THEN ?1
                        ELSE parent_template_id
                     END
                 WHERE is_template = 0
                   AND definition_id = ?2",
                params![new_def_id, old_def_id],
            )? as usize
        };
        Ok(rows)
    }

    /// Delete an instance from `db_agents`.
    ///
    /// For a template-instance projection (the row's id IS the instance
    /// id), the row is dropped. For a folded user-clone-def projection
    /// (the def row carries instance state), the delete is a NO-OP at
    /// the row level — the def itself persists when its instance ends.
    /// `agent_def_delete` is the right entry point for removing the
    /// folded def row.
    ///
    /// The cross-version registry file is cleaned up in both shapes:
    /// the registry tracks instance lifecycle, not agent lifecycle,
    /// so an instance "ending" must remove its named entry regardless
    /// of whether the db_agents row persists.
    pub fn instance_delete(&self, id: &str) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            // For a template-instance projection (id == inst.id), the
            // row drops. For a folded user-clone-def instance, the row
            // lives at def.id and the DELETE-by-inst.id matches
            // nothing — that's the NO-OP at the row level. Either way
            // we always clean the registry file (the instance
            // lifecycle is ending; the registry tracks instances, not
            // agents).
            conn.execute(
                "DELETE FROM db_agents
                 WHERE id = ?1 AND is_template = 0",
                params![id],
            )?
        };
        // Registry mirror runs unconditionally — `hard_delete` is
        // NotFound-tolerant, so it's safe to call when no file exists
        // (the folded-instance case, or stale id).
        if let Some(reg) = self.registry() {
            if let Err(e) = reg.hard_delete(id) {
                tracing::warn!(
                    instance_id = %id,
                    error = %e,
                    "registry: failed to mirror instance_delete"
                );
            }
        }
        Ok(rows > 0)
    }

    /// Back-fill `identity_id` for legacy instance rows that have
    /// either the empty string or the literal `"blank"` sentinel.
    /// Both shapes map to "no Identity bundle assigned" and the
    /// OAuth-bundles startup migration (PR E, spec §5) routes them
    /// to the newly-seeded Default bundle so the resolver can inject
    /// env vars from the captured ambient credentials.
    ///
    /// Phase 3c: writes directly to `db_agents`. Scope restricted to
    /// instance rows (is_template = 0 AND definition_id <> '') —
    /// template rows leave identity_id empty by design.
    pub fn instance_backfill_identity_id(
        &self,
        new_identity_id: &str,
    ) -> Result<usize, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agents
                 SET identity_id = ?1
                 WHERE is_template = 0
                   AND definition_id <> ''
                   AND (identity_id = '' OR identity_id = 'blank')",
                params![new_identity_id],
            )?
        };
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
        // Mirror filter must match SQLite's dropdown filter
        // (instance_list_named): non-empty instance_name AND
        // parent_instance_id == ''. Continuation rows (created when
        // the user resumes an existing named agent) carry the prior
        // row in parent_instance_id and would otherwise produce a
        // duplicate registry entry that the registry-sourced read
        // path would surface as a separate dropdown row.
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
                    user_hidden
             FROM db_agents
             WHERE is_template = 0
               AND definition_id <> ''
               AND block_id = ?1
               AND status IN ('running', 'paused')
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
    // Phase 3c — internal helpers
    //
    // The dual-write helpers from Phase 3a are gone; mutations now write
    // directly to `db_agents`. The only remaining internal helper is a
    // narrow read of the parent agent row used by `instance_create` to
    // decide between INSERT (template parent) and UPDATE-fold (user-
    // clone-def parent). See docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md.
    // ====================================================================

    /// Re-read the parent agent row from inside an active connection
    /// lock — used by `instance_create` to route between fresh
    /// projection (template parent) and fold-into-def (user-clone-def
    /// parent) without re-locking the mutex recursively.
    fn load_agent_row_for_instance(
        conn: &Connection,
        id: &str,
    ) -> rusqlite::Result<Option<AgentDefinition>> {
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_template_id, branch_label, updated_at,
                    user_hidden
             FROM db_agents WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], map_agent_definition_row);
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

/// Phase 3c — row mapper for `db_agents` rows projected into the
/// `AgentInstance` vestigial shape. Column order MUST match every
/// SELECT that calls this mapper (instance_list, instance_get,
/// instance_list_named, instance_get_by_name, instance_get_active_for_block).
/// The final column is `user_hidden` (renamed from the retired
/// `db_agent_instances.display_hidden`); we coerce the i64 into the
/// struct's `bool` field.
fn map_instance_row(row: &rusqlite::Row) -> rusqlite::Result<AgentInstance> {
    let hidden_int: i64 = row.get(14)?;
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
        display_hidden: hidden_int != 0,
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
        // Phase 3c: `instance_create` on a user-clone-def parent folds
        // into the def row (no fresh `inst1` row). Use a template
        // parent so the projection lives at `inst1` and this test's
        // read-by-instance-id semantics hold.
        let mut agent = sample_agent("def1", "agent-x");
        agent.is_seeded = 1;
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

        let ids = |s: &WaveStore| -> Vec<String> {
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

    fn store_with_registry() -> (tempfile::TempDir, WaveStore, Arc<crate::registry::Registry>) {
        let tmp = tempfile::tempdir().unwrap();
        let agents_root = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let reg_root = agents_root.join("registry");
        let reg = Arc::new(crate::registry::Registry::open(reg_root).unwrap());
        let store = WaveStore::open_in_memory().unwrap();
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
        // SQLite's dropdown filter excludes rows where
        // parent_instance_id != ''. The mirror must agree, otherwise
        // continuation rows duplicate their parent in the registry.
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
}

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
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
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
            "SELECT id, name, icon, provider, description, working_directory, shell,
                    provider_flags, auto_start, restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded
             FROM db_forge_agents ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ForgeAgent {
                id: row.get(0)?,
                name: row.get(1)?,
                icon: row.get(2)?,
                provider: row.get(3)?,
                description: row.get(4)?,
                working_directory: row.get(5)?,
                shell: row.get(6)?,
                provider_flags: row.get(7)?,
                auto_start: row.get(8)?,
                restart_on_crash: row.get(9)?,
                idle_timeout_minutes: row.get(10)?,
                created_at: row.get(11)?,
                agent_type: row.get(12)?,
                environment: row.get(13)?,
                agent_bus_id: row.get(14)?,
                is_seeded: row.get(15)?,
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

    /// Insert a new forge agent.
    pub fn forge_insert(&self, agent: &ForgeAgent) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_forge_agents (id, name, icon, provider, description,
             working_directory, shell, provider_flags, auto_start, restart_on_crash,
             idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id, is_seeded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                agent.id,
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
                agent.is_seeded
            ],
        )?;
        Ok(())
    }

    /// Update an existing forge agent (all fields except id, created_at, is_seeded).
    pub fn forge_update(&self, agent: &ForgeAgent) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_forge_agents SET name=?1, icon=?2, provider=?3, description=?4,
             working_directory=?5, shell=?6, provider_flags=?7, auto_start=?8,
             restart_on_crash=?9, idle_timeout_minutes=?10,
             agent_type=?11, environment=?12, agent_bus_id=?13
             WHERE id=?14",
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
        let mut ls = LayoutState {
            oid: "ls-1".to_string(),
            rootnode: Some(serde_json::json!({"type": "split"})),
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
                    name: "Untitled1".to_string(),
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
        assert_eq!(tab.name, "Untitled1");
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
}

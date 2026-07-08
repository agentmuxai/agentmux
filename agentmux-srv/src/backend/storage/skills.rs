// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent skills — reusable capability records attached to an agent
//! definition.
//!
//! Extracted from `store.rs` in Phase R.4 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::agent_skill_*` still
//! lives on `Store` via this `impl` block; callers stay on
//! `storage::store::AgentSkill` thanks to the re-export.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

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

impl Store {
    /// List all skills for an agent, ordered by created_at ascending.
    /// LOCAL channel's skills only — NO cross-channel fallback. Used by the
    /// def-registry mirror (which always operates on a local agent) and by
    /// `agent_skill_list`.
    pub(super) fn agent_skill_list_local(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentSkill>, StoreError> {
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

    pub fn agent_skill_list(&self, agent_id: &str) -> Result<Vec<AgentSkill>, StoreError> {
        let local = self.agent_skill_list_local(agent_id)?;
        if !local.is_empty() {
            return Ok(local);
        }
        // Fall back to the global record ONLY for a cross-channel agent
        // (absent from local SQLite); a locally-known agent with genuinely no
        // skills must return empty, not resurrect them. (reagent P1 on #1385.)
        if self.agent_def_exists_local(agent_id)? {
            return Ok(local);
        }
        if let Some(reg) = self.shared_def_registry() {
            if let Ok(Some(rec)) = reg.get(agent_id) {
                return Ok(rec
                    .data
                    .skills
                    .iter()
                    .map(|s| AgentSkill {
                        id: s.id.clone(),
                        agent_id: agent_id.to_string(),
                        name: s.name.clone(),
                        trigger: s.trigger.clone(),
                        skill_type: s.skill_type.clone(),
                        description: s.description.clone(),
                        content: s.content.clone(),
                        created_at: rec.data.created_at,
                    })
                    .collect());
            }
        }
        Ok(local)
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
        {
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
        }
        // Re-mirror the owning definition (no-op for seeded templates). (P0.2b.)
        self.registry_def_upsert(&skill.agent_id);
        Ok(())
    }

    /// Update an existing skill (all fields except id, agent_id, created_at).
    pub fn agent_skill_update(&self, skill: &AgentSkill) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
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
            )?
        };
        // conn dropped — re-mirror the definition (no-op for seeded). (P0.2b.)
        if rows > 0 {
            self.registry_def_upsert(&skill.agent_id);
        }
        Ok(rows > 0)
    }

    /// Delete a legacy skill by id. Returns true if a row was deleted.
    pub fn agent_skill_delete(&self, id: &str) -> Result<bool, StoreError> {
        let (rows, agent_id) = {
            let conn = self.conn.lock().unwrap();
            // Capture the owning agent_id before the delete so we can
            // re-mirror its definition afterwards.
            let agent_id: Option<String> = match conn.query_row(
                "SELECT agent_id FROM db_agent_skills WHERE id=?1",
                params![id],
                |row| row.get(0),
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(StoreError::Sqlite(e)),
            };
            let rows = conn.execute("DELETE FROM db_agent_skills WHERE id=?1", params![id])?;
            (rows, agent_id)
        };
        // conn dropped — re-mirror the definition so the global record drops
        // the removed skill (no-op for seeded templates). (P0.2b.)
        if rows > 0 {
            if let Some(aid) = agent_id {
                self.registry_def_upsert(&aid);
            }
        }
        Ok(rows > 0)
    }
}

// ---------------------------------------------------------------------------
// v1 standalone Skill primitive
// ---------------------------------------------------------------------------

/// Standalone skill primitive (v1 composable model).
/// Not bound to a specific agent at rest — agents reference via db_agent_skills_ref.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub trigger: String,
    pub skill_type: String,
    pub description: String,
    pub content: String,
    pub is_global: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `skill_list`'s response shape: the skill plus whether the requesting agent
/// specifically holds a `db_agent_skills_ref` row for it. Mirrors
/// `McpServerListItem` — see its doc comment for why `is_global` alone isn't
/// enough to render bind/unbind as a stateful toggle (tracked in #1960).
#[derive(Debug, Clone, Serialize)]
pub struct SkillListItem {
    #[serde(flatten)]
    pub skill: Skill,
    pub bound_to_agent: bool,
}

/// `skill_list_global`'s response shape: the skill plus how many agents
/// currently hold a `db_agent_skills_ref` to it — the Armory catalog's
/// "used by N agents" count (gap #2 of #1960).
#[derive(Debug, Clone, Serialize)]
pub struct SkillCatalogItem {
    #[serde(flatten)]
    pub skill: Skill,
    pub bound_count: i64,
}

impl Store {
    /// List all skills visible to an agent: own (referenced) + global, each
    /// annotated with whether this specific agent holds the bind ref.
    pub fn skill_list(&self, agent_id: &str) -> Result<Vec<SkillListItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.trigger, s.skill_type, s.description, s.content, s.is_global, s.created_at, s.updated_at,
                    EXISTS(SELECT 1 FROM db_agent_skills_ref r WHERE r.skill_id = s.id AND r.agent_id = ?1) AS bound_to_agent
             FROM db_skills s
             WHERE s.is_global = 1
                OR s.id IN (SELECT skill_id FROM db_agent_skills_ref WHERE agent_id = ?1)
             ORDER BY s.is_global DESC, s.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(SkillListItem {
                skill: Skill {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    trigger: row.get(2)?,
                    skill_type: row.get(3)?,
                    description: row.get(4)?,
                    content: row.get(5)?,
                    is_global: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                },
                bound_to_agent: row.get::<_, i64>(9)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// List every GLOBAL skill — the Armory catalog view. Unlike
    /// `skill_list`, this takes no `agent_id` and never includes an agent's
    /// private skills; it backs the window-scoped `skill.catalog.*` App API
    /// (no `check_s1`, so there is no agent context to scope by). Each row
    /// carries `bound_count` — how many agents currently hold a
    /// `db_agent_skills_ref` to it — per
    /// SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md §8 ("used by N agents"),
    /// tracked as gap #2 of #1960.
    pub fn skill_list_global(&self) -> Result<Vec<SkillCatalogItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.trigger, s.skill_type, s.description, s.content, s.is_global, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM db_agent_skills_ref r WHERE r.skill_id = s.id) AS bound_count
             FROM db_skills s
             WHERE s.is_global = 1
             ORDER BY s.updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SkillCatalogItem {
                skill: Skill {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    trigger: row.get(2)?,
                    skill_type: row.get(3)?,
                    description: row.get(4)?,
                    content: row.get(5)?,
                    is_global: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                },
                bound_count: row.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Get a standalone skill by id.
    pub fn skill_get(&self, id: &str) -> Result<Option<Skill>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, name, trigger, skill_type, description, content, is_global, created_at, updated_at
             FROM db_skills WHERE id = ?1",
            params![id],
            |row| {
                Ok(Skill {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    trigger: row.get(2)?,
                    skill_type: row.get(3)?,
                    description: row.get(4)?,
                    content: row.get(5)?,
                    is_global: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Upsert a standalone skill. Caller must have already stripped is_global escalation.
    pub fn skill_upsert(&self, skill: &Skill) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_skills (id, name, trigger, skill_type, description, content, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, trigger=excluded.trigger, skill_type=excluded.skill_type,
               description=excluded.description, content=excluded.content,
               updated_at=excluded.updated_at",
            params![
                skill.id,
                skill.name,
                skill.trigger,
                skill.skill_type,
                skill.description,
                skill.content,
                if skill.is_global { 1i64 } else { 0i64 },
                skill.created_at,
                skill.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Delete a standalone skill and purge its ref rows. Returns true if deleted.
    pub fn skill_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Purge ref rows explicitly (FK cascades may be off on some builds).
        conn.execute("DELETE FROM db_agent_skills_ref WHERE skill_id = ?1", params![id])?;
        let rows = conn.execute("DELETE FROM db_skills WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Bind a skill to an agent (insert ref row). Idempotent.
    pub fn skill_bind(&self, agent_id: &str, skill_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_skills_ref (agent_id, skill_id) VALUES (?1, ?2)",
            params![agent_id, skill_id],
        )?;
        Ok(())
    }

    /// Unbind a skill from an agent. Returns true if a row was removed.
    pub fn skill_unbind(&self, agent_id: &str, skill_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_agent_skills_ref WHERE agent_id = ?1 AND skill_id = ?2",
            params![agent_id, skill_id],
        )?;
        Ok(rows > 0)
    }

    /// Atomically upsert a skill enforcing per-agent name uniqueness, and (when
    /// `bind_new`) bind it to the agent — all in one transaction so concurrent
    /// `skill.upsert` calls for the same name can't both pass a separate check
    /// and insert duplicates (the check+write is not split across lock releases).
    /// Returns `NameConflict` if another skill visible to the agent (bound or
    /// global) already uses the name.
    pub fn skill_upsert_unique(
        &self,
        agent_id: &str,
        skill: &Skill,
        bind_new: bool,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM db_skills
             WHERE name = ?1 AND id <> ?2 AND (is_global = 1 OR id IN (
               SELECT skill_id FROM db_agent_skills_ref WHERE agent_id = ?3
             ))",
            params![skill.name, skill.id, agent_id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "skill name '{}' already bound to this agent",
                skill.name
            )));
        }
        tx.execute(
            "INSERT INTO db_skills (id, name, trigger, skill_type, description, content, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, trigger=excluded.trigger, skill_type=excluded.skill_type,
               description=excluded.description, content=excluded.content,
               updated_at=excluded.updated_at",
            params![
                skill.id, skill.name, skill.trigger, skill.skill_type,
                skill.description, skill.content,
                if skill.is_global { 1i64 } else { 0i64 },
                skill.created_at, skill.updated_at,
            ],
        )?;
        if bind_new {
            tx.execute(
                "INSERT OR IGNORE INTO db_agent_skills_ref (agent_id, skill_id) VALUES (?1, ?2)",
                params![agent_id, skill.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically upsert a GLOBAL skill enforcing catalog-wide name
    /// uniqueness (no `agent_id` — unlike `skill_upsert_unique`, this checks
    /// for a duplicate name among *every* global row, not just those visible
    /// to one agent). Same defense-in-depth as
    /// `McpServer::mcp_server_upsert_unique_global` (reagent P1 on #1948) —
    /// two same-named global skills would at minimum produce a confusing
    /// duplicate bullet in the assembled CLAUDE.md skills index.
    /// `skill.is_global` must already be `true`; caller's job.
    pub fn skill_upsert_unique_global(&self, skill: &Skill) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM db_skills WHERE name = ?1 AND id <> ?2 AND is_global = 1",
            params![skill.name, skill.id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "a global skill named '{}' already exists",
                skill.name
            )));
        }
        tx.execute(
            "INSERT INTO db_skills (id, name, trigger, skill_type, description, content, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, trigger=excluded.trigger, skill_type=excluded.skill_type,
               description=excluded.description, content=excluded.content,
               updated_at=excluded.updated_at",
            params![
                skill.id, skill.name, skill.trigger, skill.skill_type,
                skill.description, skill.content,
                skill.created_at, skill.updated_at,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Return true if the given skill is accessible to the agent (global or bound).
    /// Used for read and delete access checks.
    pub fn skill_is_accessible_to(&self, agent_id: &str, skill_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_skills
             WHERE id = ?1 AND (is_global = 1 OR id IN (
               SELECT skill_id FROM db_agent_skills_ref WHERE agent_id = ?2
             ))",
            rusqlite::params![skill_id, agent_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Return true if the agent has a direct ref binding to this skill (for delete/mutation).
    /// Global skills are excluded — use the is_global guard separately.
    pub fn skill_is_bound_to(&self, agent_id: &str, skill_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_agent_skills_ref WHERE agent_id = ?1 AND skill_id = ?2",
            rusqlite::params![agent_id, skill_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

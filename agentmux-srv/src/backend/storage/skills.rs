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

    /// Delete a skill by id. Returns true if a row was deleted.
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

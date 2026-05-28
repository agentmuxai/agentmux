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
}

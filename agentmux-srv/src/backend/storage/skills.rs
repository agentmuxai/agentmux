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

/// `bundle_skill_list`'s response shape — mirrors `SkillListItem`, but
/// "bound" means a `db_bundle_skills_ref` row for this bundle, not an
/// agent's `db_agent_skills_ref` row. Composable model v2,
/// docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md.
#[derive(Debug, Clone, Serialize)]
pub struct SkillBundleListItem {
    #[serde(flatten)]
    pub skill: Skill,
    pub bound_to_bundle: bool,
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

    /// Effective skills for an agent at launch/config-materialization time:
    /// this agent's own ref-bound skills (via `skill_list`) take over
    /// *entirely* when present (globals still included), otherwise fall back
    /// to the legacy `db_agent_skills` table with globals layered on top. The
    /// fallback decision is gated on *own* refs only — not the
    /// global-inclusive list — so adding a global skill never discards a
    /// legacy-only agent's skills.
    ///
    /// Single source of truth for two call sites that must stay consistent:
    /// `write_agent_config_files` (the authoritative Rust materialization
    /// path) and the `listagentskills` RPC handler, which the frontend's
    /// pre-launch `ListAgentSkillsCommand` call depends on for its own
    /// `buildConfigFiles` mirror. Before this was extracted, `listagentskills`
    /// returned only legacy skills — silently hiding every standalone/Armory-
    /// catalog skill (including any Agent-Skills-format one, see
    /// SKILL_TYPE_AGENT_SKILL) from the actual "click Launch" flow, since
    /// that RPC is window-scoped (no `check_s1`) and can't call the
    /// agent-scoped `skill.list` RPC the way an already-running, authenticated
    /// agent connection could (reagent P0 on PR #2322 — the launch UI is not
    /// an authenticated agent connection, so it was never able to reach the
    /// standalone Skill primitive via the RPC layer at all).
    pub fn effective_skills(&self, agent_id: &str) -> Vec<AgentSkill> {
        let legacy_skills = self.agent_skill_list(agent_id).unwrap_or_default();
        let mut visible_skills: Vec<Skill> = self
            .skill_list(agent_id)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.skill)
            .collect();
        // reagentx P1 on PR #2639: `has_own_skill_refs` must be decided
        // BEFORE unioning in the bundle's referenced skills below — it
        // reflects only the agent's OWN direct binds. Mandatory ABF means
        // every agent has a bundle, so if a bundle's private skill alone
        // could flip this flag, a legacy-only agent bound to any bundle
        // that happens to reference one private skill would silently lose
        // every `db_agent_skills` entry, with no action taken on the agent
        // itself. The "own refs are authoritative" path is meant strictly
        // for the agent's own binds (see `own_refs_take_over_entirely_and_discard_legacy_skills`).
        let has_own_skill_refs = visible_skills.iter().any(|s| !s.is_global);
        // Composable model v2 (docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md,
        // GH issue #2024 item 3): union in the agent's bound bundle's own
        // referenced skills too — without this, db_bundle_skills_ref would
        // be exactly as inert at launch as the bundle's old inline `skills`
        // JSON column always was. Deduped by id since a global skill is
        // already visible via both `skill_list` above and
        // `bundle_skill_list` below. Happens AFTER the has_own decision —
        // bundle-referenced skills still appear in the final list, they
        // just never trigger the "discard legacy" path on their own.
        if let Ok(Some(def)) = self.agent_def_get(agent_id) {
            if !def.memory_id.is_empty() {
                for item in self.bundle_skill_list(&def.memory_id).unwrap_or_default() {
                    if !visible_skills.iter().any(|s| s.id == item.skill.id) {
                        visible_skills.push(item.skill);
                    }
                }
            }
        }
        if has_own_skill_refs {
            crate::backend::agent_config::skills_to_agent_skills(&visible_skills, agent_id)
        } else {
            let mut merged = legacy_skills;
            merged.extend(crate::backend::agent_config::skills_to_agent_skills(&visible_skills, agent_id));
            merged
        }
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

    /// Catalog-tier sibling of `skill_list` (above) — same `bound_to_agent`
    /// shape, but deliberately GLOBAL ROWS ONLY, unlike `skill_list`'s UNION
    /// with `agent_id`'s own private skills. Backs `skill.catalog.list_for_agent`,
    /// which — like every other `skill.catalog.*` command — has no
    /// `check_s1`, so `agent_id` here is caller-supplied and unverified.
    /// Returning private skill rows (whose `content`/`description`/`trigger`
    /// can carry sensitive agent-authored material) for an arbitrary
    /// caller-chosen `agent_id` would let any window connection read any
    /// agent's private skills. Global rows carry nothing per-agent-secret —
    /// they're already fully visible via `skill_list_global` (the Armory
    /// catalog) — so exposing them alongside a caller-chosen agent's bind
    /// status is safe. reagentx P0 on PR #2329.
    pub fn skill_list_global_for_agent(&self, agent_id: &str) -> Result<Vec<SkillListItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.trigger, s.skill_type, s.description, s.content, s.is_global, s.created_at, s.updated_at,
                    EXISTS(SELECT 1 FROM db_agent_skills_ref r WHERE r.skill_id = s.id AND r.agent_id = ?1) AS bound_to_agent
             FROM db_skills s
             WHERE s.is_global = 1
             ORDER BY s.updated_at DESC",
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

    /// Delete a standalone skill and purge its ref rows (both agent- and
    /// bundle-level). Returns true if deleted.
    pub fn skill_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Purge ref rows explicitly (FK cascades may be off on some builds).
        conn.execute("DELETE FROM db_agent_skills_ref WHERE skill_id = ?1", params![id])?;
        conn.execute("DELETE FROM db_bundle_skills_ref WHERE skill_id = ?1", params![id])?;
        let rows = conn.execute("DELETE FROM db_skills WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Bind a skill to an agent (insert ref row). Idempotent — binding an
    /// already-bound pair is a silent no-op success.
    ///
    /// Errors if `agent_id` isn't a LOCAL agent definition:
    /// `db_agent_skills_ref.agent_id` has an ON-enforced FK to
    /// `db_agent_definitions(id)` (store.rs), but the Armory's agent
    /// picker (`ListAgentDefinitionsCommand` → `agent_def_list()`) also
    /// lists cross-channel agents that only exist in another channel's
    /// local database. Binding one of those would otherwise have the FK
    /// silently swallow the `INSERT OR IGNORE` — indistinguishable, by
    /// affected-row-count alone, from the equally-silent "already bound"
    /// case — reporting success while creating nothing. reagentx P1, PR
    /// #2315.
    pub fn skill_bind(&self, agent_id: &str, skill_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let agent_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_agent_definitions WHERE id = ?1)",
            params![agent_id],
            |row| row.get(0),
        )?;
        if !agent_exists {
            return Err(StoreError::Other(format!(
                "agent {agent_id} not found in this channel's local registry — cross-channel skill binding is not supported"
            )));
        }
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

    // ── Bundle-level references (composable model v2) ──────────────────
    // docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
    // item 3. Mirror the agent-level methods above exactly, keyed by
    // bundle_id via db_bundle_skills_ref instead of agent_id/db_agent_skills_ref.

    /// Bundle-level sibling of `skill_list` — this bundle's own (referenced)
    /// and global skills, each annotated with whether this specific bundle
    /// holds the `db_bundle_skills_ref` row.
    pub fn bundle_skill_list(&self, bundle_id: &str) -> Result<Vec<SkillBundleListItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.trigger, s.skill_type, s.description, s.content, s.is_global, s.created_at, s.updated_at,
                    EXISTS(SELECT 1 FROM db_bundle_skills_ref r WHERE r.skill_id = s.id AND r.bundle_id = ?1) AS bound_to_bundle
             FROM db_skills s
             WHERE s.is_global = 1
                OR s.id IN (SELECT skill_id FROM db_bundle_skills_ref WHERE bundle_id = ?1)
             ORDER BY s.is_global DESC, s.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![bundle_id], |row| {
            Ok(SkillBundleListItem {
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
                bound_to_bundle: row.get::<_, i64>(9)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Bind a skill to a bundle (insert ref row). Idempotent.
    ///
    /// `id_store` (NOT `self`) is where bundle existence is checked — see
    /// `Store::bundle_mcp_bind`'s doc comment (mcp_servers.rs) for the full
    /// reasoning (reagentx P0 review on PR #2639).
    pub fn bundle_skill_bind(&self, id_store: &Store, bundle_id: &str, skill_id: &str) -> Result<(), StoreError> {
        let bundle_exists: bool = {
            let conn = id_store.conn.lock().unwrap();
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM db_bundles WHERE id = ?1)",
                params![bundle_id],
                |row| row.get(0),
            )?
        };
        if !bundle_exists {
            return Err(StoreError::Other(format!(
                "bundle {bundle_id} not found — cannot bind a skill to a nonexistent bundle"
            )));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_bundle_skills_ref (bundle_id, skill_id) VALUES (?1, ?2)",
            params![bundle_id, skill_id],
        )?;
        Ok(())
    }

    /// Atomically create a NEW, PRIVATE (never global) skill scoped and
    /// bound directly to a bundle, enforcing bundle-scoped name uniqueness
    /// — the bundle-level analog of `skill_upsert_unique`. See
    /// `Store::bundle_mcp_upsert_unique`'s doc comment (mcp_servers.rs) for
    /// the full reasoning (reagentx P1 review on PR #2639).
    pub fn bundle_skill_upsert_unique(
        &self,
        id_store: &Store,
        bundle_id: &str,
        skill: &Skill,
        bind_new: bool,
    ) -> Result<(), StoreError> {
        let bundle_exists: bool = {
            let conn = id_store.conn.lock().unwrap();
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM db_bundles WHERE id = ?1)",
                params![bundle_id],
                |row| row.get(0),
            )?
        };
        if !bundle_exists {
            return Err(StoreError::Other(format!(
                "bundle {bundle_id} not found — cannot create a skill for a nonexistent bundle"
            )));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM db_skills
             WHERE name = ?1 AND id <> ?2 AND (is_global = 1 OR id IN (
               SELECT skill_id FROM db_bundle_skills_ref WHERE bundle_id = ?3
             ))",
            params![skill.name, skill.id, bundle_id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "skill name '{}' already bound to this bundle",
                skill.name
            )));
        }
        // is_global hardcoded to 0 — see bundle_mcp_upsert_unique's
        // identical comment.
        tx.execute(
            "INSERT INTO db_skills (id, name, trigger, skill_type, description, content, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)
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
        if bind_new {
            tx.execute(
                "INSERT OR IGNORE INTO db_bundle_skills_ref (bundle_id, skill_id) VALUES (?1, ?2)",
                params![bundle_id, skill.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Unbind a skill from a bundle. Returns true if a row was removed.
    pub fn bundle_skill_unbind(&self, bundle_id: &str, skill_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_bundle_skills_ref WHERE bundle_id = ?1 AND skill_id = ?2",
            params![bundle_id, skill_id],
        )?;
        Ok(rows > 0)
    }

    /// Return true if the given skill is accessible to the bundle (global or
    /// bundle-bound). Mirrors `skill_is_accessible_to`.
    pub fn bundle_skill_is_accessible_to(&self, bundle_id: &str, skill_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_skills
             WHERE id = ?1 AND (is_global = 1 OR id IN (
               SELECT skill_id FROM db_bundle_skills_ref WHERE bundle_id = ?2
             ))",
            rusqlite::params![skill_id, bundle_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Return true if the bundle has a direct ref binding to this skill
    /// (global excluded). Mirrors `skill_is_bound_to` — used for
    /// edit/delete-ownership checks, as opposed to
    /// `bundle_skill_is_accessible_to`'s broader read-access check.
    pub fn bundle_skill_is_bound_to(&self, bundle_id: &str, skill_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_bundle_skills_ref WHERE bundle_id = ?1 AND skill_id = ?2",
            rusqlite::params![bundle_id, skill_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod effective_skills_tests {
    use super::*;
    use crate::backend::storage::store::AgentDefinition;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn insert_agent(store: &Store, id: &str) {
        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: String::new(),
            name: "Test Agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();
    }

    fn global_skill(id: &str, name: &str) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: format!("{name} description"),
            content: format!("{name} content"),
            is_global: true,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn falls_back_to_legacy_plus_globals_when_agent_has_no_own_refs() {
        let store = make_store();
        insert_agent(&store, "agent-1");

        store.agent_skill_insert(&AgentSkill {
            id: "legacy-1".to_string(),
            agent_id: "agent-1".to_string(),
            name: "Legacy Skill".to_string(),
            trigger: "legacy".to_string(),
            skill_type: "prompt".to_string(),
            description: "legacy description".to_string(),
            content: "legacy content".to_string(),
            created_at: 1_700_000_000_000,
        }).unwrap();

        store.skill_upsert_unique_global(&global_skill("global-1", "Global Skill")).unwrap();
        // Not bound to agent-1 -- has_own_skill_refs must stay false.

        let effective = store.effective_skills("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 2, "expected legacy + global, got: {names:?}");
        assert!(names.contains(&"Legacy Skill"));
        assert!(names.contains(&"Global Skill"));
    }

    #[test]
    fn own_refs_take_over_entirely_and_discard_legacy_skills() {
        let store = make_store();
        insert_agent(&store, "agent-1");

        store.agent_skill_insert(&AgentSkill {
            id: "legacy-1".to_string(),
            agent_id: "agent-1".to_string(),
            name: "Legacy Skill".to_string(),
            trigger: "legacy".to_string(),
            skill_type: "prompt".to_string(),
            description: "legacy description".to_string(),
            content: "legacy content".to_string(),
            created_at: 1_700_000_000_000,
        }).unwrap();

        store.skill_upsert_unique_global(&global_skill("global-1", "Global Skill")).unwrap();

        // Bind a NEW (non-global) skill to agent-1 -- this must flip
        // has_own_skill_refs to true and discard the legacy skill entirely.
        let own_skill = Skill {
            id: "own-1".to_string(),
            name: "Own Skill".to_string(),
            trigger: String::new(),
            skill_type: "agent-skill".to_string(),
            description: "own description".to_string(),
            content: "own content".to_string(),
            is_global: false,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        store.skill_upsert_unique("agent-1", &own_skill, true).unwrap();

        let effective = store.effective_skills("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 2, "expected own + global, legacy discarded, got: {names:?}");
        assert!(names.contains(&"Own Skill"));
        assert!(names.contains(&"Global Skill"));
        assert!(!names.contains(&"Legacy Skill"), "legacy skill must be discarded once own refs exist");
    }

    #[test]
    fn agent_skill_format_survives_the_merge_with_correct_skill_type() {
        // Regression for PR #2322: an agent-skill-format skill materialized
        // via effective_skills must retain skill_type == "agent-skill" so
        // build_config_files still branches it to SKILL.md, not a slash
        // command.
        let store = make_store();
        insert_agent(&store, "agent-1");

        let own_skill = Skill {
            id: "own-1".to_string(),
            name: "Deploy Checklist".to_string(),
            trigger: String::new(),
            skill_type: "agent-skill".to_string(),
            description: "checklist".to_string(),
            content: "1. test\n2. deploy".to_string(),
            is_global: false,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        store.skill_upsert_unique("agent-1", &own_skill, true).unwrap();

        let effective = store.effective_skills("agent-1");
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].skill_type, "agent-skill");
    }

    fn insert_agent_with_bundle(store: &Store, id: &str, memory_id: &str) {
        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: String::new(),
            name: "Test Agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: memory_id.to_string(),
        };
        store.agent_def_insert(&mut def).unwrap();
    }

    fn insert_test_bundle(store: &Store, id: &str) {
        store
            .bundle_memory_upsert(&crate::backend::storage::memory_bundles::Memory {
                id: id.to_string(),
                name: format!("Bundle {id}"),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: "claude".to_string(),
                model: "anthropic".to_string(),
                instructions: String::new(),
                instructions_by_provider: "{}".to_string(),
                context_files: "[]".to_string(),
                mcp_servers: "[]".to_string(),
                skills: "[]".to_string(),
                sort_order: 0,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_000,
            })
            .unwrap();
    }

    /// Composable model v2 (docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md,
    /// GH issue #2024 item 3): a skill referenced by the agent's OWN bound
    /// bundle (not the agent itself) must show up in effective_skills —
    /// otherwise the new bundle-level ref tables are inert at launch, same
    /// as the bug this whole feature exists to fix (see
    /// `write_agent_config_files`'s doc comment on why this must be a
    /// single source of truth shared with the RPC handler).
    #[test]
    fn effective_skills_includes_the_bound_bundles_referenced_skills() {
        let store = make_store();
        insert_test_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");

        let bundle_skill = Skill {
            id: "bundle-skill-1".to_string(),
            name: "Bundle Skill".to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: "content".to_string(),
            is_global: false,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        // Upserted under an unrelated agent context, then bound to the
        // BUNDLE (not to agent-1 directly) — simulates a skill added via
        // the bundle editor, not the agent's own Stash.
        store.skill_upsert_unique("some-other-context", &bundle_skill, false).unwrap();
        store.bundle_skill_bind(&store, "bundle-1", "bundle-skill-1").unwrap();

        let effective = store.effective_skills("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Bundle Skill"),
            "a skill referenced only by the agent's bound bundle must still be effective: {names:?}"
        );
    }

    /// End-to-end proof of the reagentx P1 fix, mirrored from
    /// mcp_servers.rs's identical test: a skill created via
    /// `bundle_skill_upsert_unique` reaches a spawned agent's effective
    /// skill list.
    #[test]
    fn effective_skills_includes_a_bundle_owned_private_skill_created_via_upsert_unique() {
        let store = make_store();
        insert_test_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");

        let bundle_skill = Skill {
            id: "bundle-own-skill".to_string(),
            name: "Bundle-Owned Skill".to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: "content".to_string(),
            is_global: true, // ignored — upsert_unique forces is_global=false
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        store.bundle_skill_upsert_unique(&store, "bundle-1", &bundle_skill, true).unwrap();

        let effective = store.effective_skills("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Bundle-Owned Skill"),
            "a bundle-owned private skill created via bundle_skill_upsert_unique must reach the agent's effective skills: {names:?}"
        );
    }

    /// reagentx P1 on PR #2639: `has_own_skill_refs` must reflect ONLY the
    /// agent's own direct binds, never a bundle-referenced skill — mandatory
    /// ABF means every agent has a bundle, so if a bundle's private skill
    /// alone could flip this flag, a legacy-only agent bound to ANY bundle
    /// that happens to reference one private skill would silently lose all
    /// of its `db_agent_skills` entries the moment someone edits that
    /// bundle, with no action taken on the agent itself. The bundle's
    /// referenced skill must still appear in the result (it's not excluded)
    /// — it just must not trigger the "own refs are authoritative, discard
    /// legacy" path on its own.
    #[test]
    fn a_bundle_referenced_private_skill_does_not_discard_the_agents_own_legacy_skills() {
        let store = make_store();
        insert_test_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");

        store.agent_skill_insert(&AgentSkill {
            id: "legacy-1".to_string(),
            agent_id: "agent-1".to_string(),
            name: "Legacy Skill".to_string(),
            trigger: "legacy".to_string(),
            skill_type: "prompt".to_string(),
            description: "legacy description".to_string(),
            content: "legacy content".to_string(),
            created_at: 1_700_000_000_000,
        }).unwrap();

        let bundle_private_skill = Skill {
            id: "bundle-private-1".to_string(),
            name: "Bundle Private Skill".to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: "content".to_string(),
            is_global: false,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        };
        store.skill_upsert_unique("some-other-context", &bundle_private_skill, false).unwrap();
        store.bundle_skill_bind(&store, "bundle-1", "bundle-private-1").unwrap();
        // agent-1 itself has NO own db_skills ref — only its bundle does.

        let effective = store.effective_skills("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Legacy Skill"),
            "a bundle-referenced private skill must not silently discard the agent's own legacy skills: {names:?}"
        );
        assert!(
            names.contains(&"Bundle Private Skill"),
            "the bundle-referenced skill must still be included, just not treated as agent-authoritative: {names:?}"
        );
    }

    #[test]
    fn effective_skills_does_not_duplicate_a_global_skill_visible_via_both_agent_and_bundle() {
        let store = make_store();
        insert_test_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");
        store.skill_upsert_unique_global(&global_skill("global-1", "Global Skill")).unwrap();

        let effective = store.effective_skills("agent-1");
        let matches: Vec<_> = effective.iter().filter(|s| s.name == "Global Skill").collect();
        assert_eq!(matches.len(), 1, "a global skill visible via both the agent and its bundle must not be duplicated: {effective:?}");
    }
}

#[cfg(test)]
mod bundle_ref_tests {
    use super::*;
    use crate::backend::storage::memory_bundles::Memory;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn insert_bundle(store: &Store, id: &str) {
        store
            .bundle_memory_upsert(&Memory {
                id: id.to_string(),
                name: format!("Bundle {id}"),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: "claude".to_string(),
                model: "anthropic".to_string(),
                instructions: String::new(),
                instructions_by_provider: "{}".to_string(),
                context_files: "[]".to_string(),
                mcp_servers: "[]".to_string(),
                skills: "[]".to_string(),
                sort_order: 0,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_000,
            })
            .unwrap();
    }

    fn skill(id: &str, name: &str, is_global: bool) -> Skill {
        Skill {
            id: id.to_string(),
            name: name.to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: "content".to_string(),
            is_global,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn bind_makes_a_private_skill_visible_in_bundle_skill_list() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store.skill_upsert_unique_global(&skill("skill-global", "Global", true)).unwrap();
        store
            .skill_upsert_unique("some-other-agent-context", &skill("skill-private", "Private", false), false)
            .unwrap_or(());

        let before = store.bundle_skill_list("bundle-1").unwrap();
        assert_eq!(before.len(), 1, "only the global skill should be visible before any bind: {before:?}");

        store.bundle_skill_bind(&store, "bundle-1", "skill-private").unwrap();
        let after = store.bundle_skill_list("bundle-1").unwrap();
        assert_eq!(after.len(), 2, "private skill must now be visible after binding: {after:?}");
        let private_item = after.iter().find(|i| i.skill.id == "skill-private").expect("private skill present");
        assert!(private_item.bound_to_bundle);
    }

    #[test]
    fn bind_is_not_visible_to_a_different_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        insert_bundle(&store, "bundle-2");
        store
            .skill_upsert_unique("ctx", &skill("skill-private", "Private", false), false)
            .unwrap_or(());
        store.bundle_skill_bind(&store, "bundle-1", "skill-private").unwrap();

        let bundle2_list = store.bundle_skill_list("bundle-2").unwrap();
        assert!(
            bundle2_list.is_empty(),
            "a private skill bound to bundle-1 must not leak into bundle-2's list: {bundle2_list:?}"
        );
    }

    #[test]
    fn bind_errors_when_the_bundle_does_not_exist() {
        let store = make_store();
        store.skill_upsert_unique_global(&skill("skill-1", "S", true)).unwrap();
        let result = store.bundle_skill_bind(&store, "no-such-bundle", "skill-1");
        assert!(result.is_err(), "binding to a nonexistent bundle must error, not silently no-op");
    }

    #[test]
    fn unbind_removes_the_ref_and_is_idempotent() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store
            .skill_upsert_unique("ctx", &skill("skill-private", "Private", false), false)
            .unwrap_or(());
        store.bundle_skill_bind(&store, "bundle-1", "skill-private").unwrap();

        let removed = store.bundle_skill_unbind("bundle-1", "skill-private").unwrap();
        assert!(removed);
        assert!(store.bundle_skill_list("bundle-1").unwrap().is_empty());

        let removed_again = store.bundle_skill_unbind("bundle-1", "skill-private").unwrap();
        assert!(!removed_again, "unbinding an already-unbound pair returns false, not an error");
    }

    #[test]
    fn deleting_a_skill_purges_its_bundle_ref_too() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store
            .skill_upsert_unique("ctx", &skill("skill-private", "Private", false), false)
            .unwrap_or(());
        store.bundle_skill_bind(&store, "bundle-1", "skill-private").unwrap();
        assert_eq!(store.bundle_skill_list("bundle-1").unwrap().len(), 1);

        store.skill_delete("skill-private").unwrap();
        assert!(
            store.bundle_skill_list("bundle-1").unwrap().is_empty(),
            "the bundle ref row must be purged when the underlying skill is deleted"
        );
    }

    /// reagentx P0 review on PR #2639: bundle existence must be checked
    /// against `id_store`, not `self` — see the identical test in
    /// mcp_servers.rs for the full reasoning, mirrored here for skills.
    #[test]
    fn bind_checks_bundle_existence_in_id_store_not_self() {
        let wstore = make_store();
        let id_store = make_store();
        insert_bundle(&id_store, "bundle-1");
        wstore.skill_upsert_unique_global(&skill("skill-1", "S", true)).unwrap();

        let result = wstore.bundle_skill_bind(&id_store, "bundle-1", "skill-1");
        assert!(result.is_ok(), "must check bundle existence against id_store, not self: {result:?}");
    }

    #[test]
    fn bind_fails_when_bundle_exists_only_in_self_not_id_store() {
        let wstore = make_store();
        let id_store = make_store();
        insert_bundle(&wstore, "bundle-1");
        wstore.skill_upsert_unique_global(&skill("skill-1", "S", true)).unwrap();

        let result = wstore.bundle_skill_bind(&id_store, "bundle-1", "skill-1");
        assert!(
            result.is_err(),
            "a bundle only present in self's non-authoritative copy must not satisfy the id_store check: {result:?}"
        );
    }

    /// reagentx P1 review on PR #2639: the missing "give this bundle its
    /// own skill" path — see the identical test in mcp_servers.rs.
    #[test]
    fn upsert_unique_creates_a_new_private_skill_bound_to_the_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");

        store
            .bundle_skill_upsert_unique(&store, "bundle-1", &skill("new-skill", "Bundle-Only Skill", true), true)
            .unwrap();

        let list = store.bundle_skill_list("bundle-1").unwrap();
        let item = list.iter().find(|i| i.skill.id == "new-skill").expect("newly created skill present");
        assert!(item.bound_to_bundle);
        assert!(!item.skill.is_global, "upsert_unique must force is_global=false regardless of the input struct");
    }

    #[test]
    fn upsert_unique_rejects_a_duplicate_name_already_bound_to_the_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store.bundle_skill_upsert_unique(&store, "bundle-1", &skill("skill-a", "Dup Name", true), true).unwrap();

        let result = store.bundle_skill_upsert_unique(&store, "bundle-1", &skill("skill-b", "Dup Name", true), true);
        assert!(result.is_err(), "a second skill with the same name bound to the same bundle must be rejected");
    }

    #[test]
    fn is_accessible_to_reflects_global_and_bound_state() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store.skill_upsert_unique_global(&skill("skill-global", "Global", true)).unwrap();
        store
            .skill_upsert_unique("ctx", &skill("skill-private", "Private", false), false)
            .unwrap_or(());

        assert!(store.bundle_skill_is_accessible_to("bundle-1", "skill-global").unwrap());
        assert!(!store.bundle_skill_is_accessible_to("bundle-1", "skill-private").unwrap());
        store.bundle_skill_bind(&store, "bundle-1", "skill-private").unwrap();
        assert!(store.bundle_skill_is_accessible_to("bundle-1", "skill-private").unwrap());
    }
}

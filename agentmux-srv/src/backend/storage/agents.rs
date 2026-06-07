// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent subsystem — definitions, instances, and their lifecycle CRUD.
//!
//! This is the largest of the storage subsystems: 8 `agent_def_*`
//! methods (template + user-clone definitions), 12 `instance_*`
//! methods (per-launch instance rows, named-agent continuation,
//! identity-bound active-for-block resolution), the
//! `AgentDefinition` / `AgentInstance` structs, and the
//! `InstanceStatus` enum.
//!
//! Extracted from `store.rs` in Phase R.1 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`) — the
//! final phase of that plan. The method surface is unchanged —
//! `Store::agent_def_*` and `Store::instance_*` still live on `Store`
//! via the two `impl` blocks below; callers stay on
//! `storage::store::AgentDefinition` / `AgentInstance` /
//! `InstanceStatus` thanks to the re-exports.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

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

/// Partial-update payload for [`Store::instance_update_partial`]. Each
/// `Some` field is written; `None` leaves the column untouched. Mirrors
/// the mutable subset of `CommandUpdateAgentInstanceData`
/// (`block_id`/`session_id`/`status`/`github_context`/`ended_at`) — the
/// only columns `instance_update` ever wrote. `Some("")` for a string
/// field explicitly clears it.
#[derive(Debug, Clone, Default)]
pub struct InstanceUpdate {
    pub block_id: Option<String>,
    pub session_id: Option<String>,
    pub status: Option<String>,
    pub github_context: Option<String>,
    pub ended_at: Option<i64>,
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

    /// Atomic check-then-insert for `agent.define`.
    ///
    /// Looks up an existing user-owned definition by name (case-insensitive)
    /// or derived slug under the SAME mutex guard that protects the INSERT —
    /// preventing TOCTOU when two concurrent `agent.define` calls arrive for
    /// the same name.
    ///
    /// Returns:
    /// - `Ok(Some(def))` — an existing row matched; `agent` was NOT inserted.
    /// - `Ok(None)` — no match; `agent` was inserted and `agent.slug` now
    ///   holds the collision-resolved slug.
    pub fn agent_def_find_or_insert(
        &self,
        agent: &mut AgentDefinition,
    ) -> Result<Option<AgentDefinition>, StoreError> {
        let name_lower = agent.name.trim().to_lowercase();
        let derived_slug = derive_slug(agent.name.trim());

        let stamped_updated_at = {
            let conn = self.conn.lock().unwrap();

            // Check under the same lock to close the TOCTOU window.
            let mut stmt = conn.prepare(
                "SELECT id, slug, name, icon, provider, description,
                        working_directory, shell, provider_flags, auto_start,
                        restart_on_crash, idle_timeout_minutes, created_at,
                        agent_type, environment, agent_bus_id, is_seeded,
                        accounts, parent_id, branch_label, updated_at,
                        user_hidden
                 FROM db_agent_definitions
                 WHERE (lower(trim(name)) = ?1 OR slug = ?2)
                   AND is_seeded = 0
                 ORDER BY CASE WHEN lower(trim(name)) = ?1 THEN 0 ELSE 1 END
                 LIMIT 1",
            )?;
            let mut rows = stmt.query_map(
                params![name_lower, derived_slug],
                map_agent_definition_row,
            )?;
            if let Some(row) = rows.next() {
                return Ok(Some(row?));
            }
            // Drop borrows on `conn` before proceeding to the insert.
            drop(rows);
            drop(stmt);

            // Not found — insert under the same lock.
            let base = if agent.slug.is_empty() {
                derive_slug(&agent.name)
            } else {
                agent.slug.clone()
            };
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
                "INSERT INTO db_agent_definitions
                   (id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start, restart_on_crash,
                    idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
                    is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden)
                 VALUES
                   (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    agent.id, agent.slug, agent.name, agent.icon, agent.provider,
                    agent.description, agent.working_directory, agent.shell,
                    agent.provider_flags, agent.auto_start, agent.restart_on_crash,
                    agent.idle_timeout_minutes, agent.created_at, agent.agent_type,
                    agent.environment, agent.agent_bus_id, agent.is_seeded,
                    agent.accounts, agent.parent_id, agent.branch_label,
                    agent.created_at, // updated_at = created_at for new rows
                    agent.user_hidden,
                ],
            )?;
            agent.created_at
            // conn guard drops here; dual-write acquires the lock again below
        };

        let mut snapshot = agent.clone();
        snapshot.updated_at = stamped_updated_at;
        self.agents_dual_write_definition_upsert(&snapshot)?;

        Ok(None)
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

    // AgentContent / AgentSkill / AgentHistory CRUD moved to
    // `super::content` / `super::skills` / `super::history` in Phase R.4
    // (SPEC_STORE_MODULARIZATION_2026_05_27.md). The `impl Store {}`
    // blocks there add the same methods to this type. `format_epoch_date`
    // moves with `agent_history_append`.
}

impl Store {

    // ---- Agent instance CRUD ----

    /// List instances. Both filters are optional — pass `None` to scan
    /// all instances. Ordered by `updated_at` descending, with
    /// `created_at` as a tiebreaker (most recent activity first; the
    /// dual-write bumps `updated_at` on every launch / continuation,
    /// so a continued older agent ranks ahead of a brand-new untouched
    /// one).
    ///
    /// Phase 3b.3a (no-status case): reads from the consolidated
    /// `db_agents` table (`is_template = 0`, `user_hidden = 0`).
    /// Continuation chains pre-collapse — one row per logical agent.
    /// The `definition_id` filter, when supplied, matches the agent's
    /// own `id` only (templates aren't agents and user-clones derived
    /// from a template are SEPARATE agents — see the implementation
    /// note below for why `parent_template_id` traversal was dropped).
    ///
    /// Field mapping for fields with no consolidated-row analog:
    /// - `block_id`, `session_id`, `status`, `ended_at`,
    ///   `parent_instance_id` → type defaults (`""` / `0`). Truly
    ///   transient per-launch state; not modelled on `db_agents`.
    /// - `started_at` → `db_agents.created_at`. Same proxy used by
    ///   `instance_get_by_name` (3b.2); the consolidated row's
    ///   creation IS the agent's launch moment in the new model.
    /// - `display_hidden` → always `false`. The WHERE clause filters
    ///   `user_hidden = 0`, so hidden rows never surface here.
    ///
    /// Phase 3b.3b (deferred — status filter case): callers passing a
    /// `status` filter need transient runtime state that `db_agents`
    /// doesn't model. Route those to the legacy `db_agent_instances`
    /// path so existing semantics are preserved until the
    /// updateagentinstance handler's "fetch + merge transient fields"
    /// pattern is refactored. (Currently no production caller passes
    /// `status` — `listagentinstances` RPC frontends call with empty
    /// filters — so the legacy path is exercised only by tests.)
    /// Spec: docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md §3b.3.
    pub fn instance_list(
        &self,
        definition_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        if status.is_some() {
            return self.instance_list_legacy(definition_id, status);
        }
        let conn = self.conn.lock().unwrap();
        // `definition_id` filter: match the agent's own `id` only.
        //
        // In the consolidated model every `is_template = 0` row IS an
        // agent identifiable by its `id`:
        //   - User-clone def projection: `id` = the clone def's id.
        //   - Template-instance projection: `id` = the original inst.id.
        //
        // The legacy `definition_id` filter conflated "agent identity"
        // with "parent template" because the old schema split them
        // across two tables. The new model has no such split. A
        // template-id filter would over-match (user-clones of X share
        // `parent_template_id` with template-instances of X — see
        // codex P2 on PR #1111), so we route template-id callers to
        // an empty result. No live caller exercises that path; the
        // only frontend consumer (`listagentinstances` RPC via
        // swarm-model) passes empty filters.
        let mut filter_clause = String::new();
        let mut param_vals: Vec<String> = Vec::new();
        if let Some(d) = definition_id {
            filter_clause.push_str("\n               AND id = ?1");
            param_vals.push(d.to_string());
        }
        // Projection for `definition_id`: use the row's own id.
        //
        // Reagent P2 on PR #1111 round 2: the earlier "parent_template_id
        // with id fallback" projection diverged from legacy for
        // user-clones derived from a template — those rows have
        // `parent_template_id` SET (the lineage), but their legacy
        // `db_agent_instances.definition_id` was the clone's own def
        // id, not the template's. Template-instance projections and
        // user-clone projections aren't schema-distinguishable in
        // db_agents (both `is_template = 0`, `is_seeded = 0`), so any
        // consistent projection must pick one rule. The consolidated
        // model treats the row's `id` as the agent's identity (== legacy
        // `definition_id` for the user-clone case), so use that. The
        // template-instance case yields the inst.id instead of the
        // template id, but no live caller depends on this field — it's
        // a back-compat surface for the AgentInstance struct shape.
        //
        // `ORDER BY updated_at DESC`: reagent P2 on PR #1111 round 2.
        // Continuation chains keep the head's `created_at` (the row is
        // never re-inserted) while the dual-write bumps `updated_at` on
        // every launch / continuation. So `created_at DESC` would rank
        // a brand-new agent ahead of an actively-continued older one,
        // violating "most recent activity first". `updated_at` tracks
        // recency correctly and matches the ordering `agent_def_list`
        // uses elsewhere in this file.
        let mut sql = String::from(
            "SELECT id, id AS def_id,
                    github_context, created_at,
                    identity_id, memory_id, instance_name, working_directory
             FROM db_agents
             WHERE is_template = 0
               AND user_hidden = 0",
        );
        sql.push_str(&filter_clause);
        sql.push_str("\n             ORDER BY updated_at DESC, created_at DESC");
        let mut stmt = conn.prepare(&sql)?;
        let iter = stmt.query_map(rusqlite::params_from_iter(param_vals.iter()), |row| {
            Ok(AgentInstance {
                id: row.get(0)?,
                definition_id: row.get(1)?,
                parent_instance_id: String::new(),
                block_id: String::new(),
                session_id: String::new(),
                status: String::new(),
                github_context: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: 0,
                created_at: row.get(3)?,
                identity_id: row.get(4)?,
                memory_id: row.get(5)?,
                instance_name: row.get(6)?,
                working_directory: row.get(7)?,
                // Filter above guarantees the row is visible; column
                // omitted from SELECT to match. Reagent P2 on PR #1111.
                display_hidden: false,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Legacy `db_agent_instances` read — preserved for the
    /// status-filter case (transient state). Will retire when the
    /// updateagentinstance handler's fetch-and-merge pattern is
    /// refactored (Phase 3b.3b). Do NOT add new callers; use
    /// `instance_list` instead.
    fn instance_list_legacy(
        &self,
        definition_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
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
        // `definition_id` projection: use the row's own `id`.
        //
        // The earlier "parent_template_id with id fallback" rule
        // misrendered the legacy semantics for user-clones derived
        // from a template — those rows have `parent_template_id` SET
        // (lineage record) but their legacy `definition_id` was the
        // clone's own def id, NOT the template's. Template-instance
        // and user-clone projections aren't schema-distinguishable in
        // db_agents, so any consistent rule must pick one. The
        // consolidated model treats `id` as the agent's identity, so
        // that's what we expose — matches `instance_list` (3b.3a)
        // after the same fix. Reagent P2 on PR #1111 round 2.
        let mut stmt = conn.prepare(
            "SELECT id, id AS def_id,
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

    /// Partial update of an instance's mutable runtime fields. Only
    /// `Some` fields are written; `None` leaves that column untouched.
    ///
    /// Replaces the `updateagentinstance` handler's fetch-and-merge
    /// (read the full row → fill the unspecified fields → write the
    /// whole struct back). That read was the only production caller
    /// that needed `instance_get`'s transient per-launch fields, which
    /// pinned `instance_get` to the legacy `db_agent_instances` table.
    /// With a partial write the handler no longer reads the row at all.
    /// See docs/specs/SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29.md.
    ///
    /// Returns the post-update row so callers that need `definition_id`
    /// for an event scope — or want to echo the row back — get it from
    /// the same authoritative reload this method already runs to refresh
    /// the registry mirror + Phase-3a dual-write. Those consumers read
    /// only non-transient fields, so the reload survives a future
    /// `instance_get` → db_agents flip (Phase 3b.3c).
    ///
    /// `None` is reserved for **not-found** (the id doesn't exist). An
    /// all-`None` update on an existing id is a no-op that returns the
    /// unchanged row — so callers can distinguish "nothing to change"
    /// from "no such instance".
    pub fn instance_update_partial(
        &self,
        id: &str,
        upd: &InstanceUpdate,
    ) -> Result<Option<AgentInstance>, StoreError> {
        use rusqlite::types::ToSql;

        let mut sets: Vec<&str> = Vec::new();
        let mut vals: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(v) = &upd.block_id {
            sets.push("block_id = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = &upd.session_id {
            sets.push("session_id = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = &upd.status {
            sets.push("status = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = &upd.github_context {
            // `Some("")` explicitly clears (matches the command contract).
            sets.push("github_context = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = upd.ended_at {
            sets.push("ended_at = ?");
            vals.push(Box::new(v));
        }
        if sets.is_empty() {
            // No fields to write. Return the current row unchanged (or
            // `None` if the id genuinely doesn't exist) so the caller can
            // still tell a no-op apart from not-found — rather than
            // conflating both as `None`. No write, no registry/dual-write
            // reload needed since nothing changed.
            return self.instance_get(id);
        }

        let rows = {
            let conn = self.conn.lock().unwrap();
            let sql = format!(
                "UPDATE db_agent_instances SET {} WHERE id = ?",
                sets.join(", ")
            );
            vals.push(Box::new(id.to_string()));
            let params: Vec<&dyn ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, params.as_slice())?
        };
        if rows == 0 {
            return Ok(None);
        }
        // Reload the authoritative post-update row and refresh the
        // registry mirror + dual-write — identical to `instance_update`.
        let fresh = self.instance_get(id)?;
        if let Some(f) = &fresh {
            self.registry_upsert_if_named(f);
            // Phase 3a dual-write (Phase 3b: errors propagate).
            self.agents_dual_write_instance_update(f)?;
        }
        Ok(fresh)
    }

    /// Update mutable instance fields. `id`, `definition_id`,
    /// `parent_instance_id`, `started_at`, `created_at` are immutable
    /// after insert (they describe provenance, not state).
    ///
    /// Retained as a full-struct convenience for store tests + internal
    /// callers; the `updateagentinstance` handler now uses
    /// [`Self::instance_update_partial`] so it no longer reads the row
    /// to merge.
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

    // `registry_upsert_if_named` moved to `super::registry_mirror`
    // in Phase R.6 (SPEC_STORE_MODULARIZATION_2026_05_27.md).

    /// Resolve the agent bindings tied to a block.
    ///
    /// Phase 3b.4: resolve through `block.meta.agentId` (or legacy
    /// `agent:id`) against `db_agents` for the user-clone case;
    /// fall back to the legacy `db_agent_instances` lookup by
    /// `block_id` for seeded-template launches and any other block
    /// the consolidated path can't satisfy.
    ///
    /// We deliberately do NOT consult `block.meta.agentInstanceId`:
    /// codex P1 on PR #1114 round 3 surfaced that pane reuse
    /// (`backToPicker` clears `agentId` but not `agentInstanceId`)
    /// and quick-launch (no instance-id stamp) leave the key stale.
    /// Trusting it would silently bleed the prior agent's identity
    /// across reopens — exactly the regression the legacy active-
    /// instance query avoided. The agentId-then-legacy path covers
    /// every launch shape without needing instance-id stamping:
    ///   - User-clone: db_agents.id == def.id, hits is_template=0.
    ///   - Template direct-launch: agentId points at a template
    ///     (is_template=1, filtered out). Legacy fallback finds the
    ///     active instance row keyed on block_id.
    ///   - Pane reuse: stale `agentInstanceId` is ignored; current
    ///     `agentId` wins.
    ///
    /// Replaces the legacy "find most recent active instance for
    /// this block" as the PRIMARY path for user-clones; the legacy
    /// query remains as fallback for templates + edge cases.
    /// Retires fully when Phase 3c drops the legacy table.
    /// Spec: docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md §3b.
    ///
    /// `user_hidden` is NOT filtered — hiding a named agent
    /// ("forget") is a picker-visibility concept; the pane bound to
    /// that agent must keep resolving credentials. Codex P2 on PR
    /// #1114 round 2.
    ///
    /// Used by the identity resolver to pull `identity_id` /
    /// `memory_id` for environment injection on every command
    /// dispatch. Caller only reads `identity_id` from the returned
    /// `AgentInstance` — transient per-launch fields (status,
    /// session_id, started_at, ended_at, parent_instance_id) come
    /// back as type defaults. `block_id` echoes back the caller's
    /// argument.
    pub fn instance_get_active_for_block(
        &self,
        block_id: &str,
    ) -> Result<Option<AgentInstance>, StoreError> {
        let block: crate::backend::obj::Block = match self.get(block_id)? {
            Some(b) => b,
            None => return Ok(None),
        };
        let agent_id = block
            .meta
            .get("agentId")
            .and_then(|v| v.as_str())
            .or_else(|| block.meta.get("agent:id").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let conn = self.conn.lock().unwrap();
        if !agent_id.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT id, github_context, created_at,
                        identity_id, memory_id, instance_name, working_directory
                 FROM db_agents
                 WHERE id = ?1
                   AND is_template = 0",
            )?;
            let block_id_owned = block_id.to_string();
            let result = stmt.query_row(params![agent_id], |row| {
                Ok(AgentInstance {
                    id: row.get(0)?,
                    definition_id: row.get(0)?, // consolidated model — see 3b.3a
                    parent_instance_id: String::new(),
                    block_id: block_id_owned.clone(),
                    session_id: String::new(),
                    status: String::new(),
                    github_context: row.get(1)?,
                    started_at: row.get(2)?,
                    ended_at: 0,
                    created_at: row.get(2)?,
                    identity_id: row.get(3)?,
                    memory_id: row.get(4)?,
                    instance_name: row.get(5)?,
                    working_directory: row.get(6)?,
                    display_hidden: false,
                })
            });
            match result {
                Ok(a) => return Ok(Some(a)),
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(e.into()),
            }
        }
        // Legacy-block fallback: seeded-template launches and any
        // block whose agentId references a row that doesn't exist
        // in the consolidated view. The active-instance query is
        // robust to pane reuse (it picks the most recent active
        // row keyed on block_id).
        let mut legacy_stmt = conn.prepare(
            "SELECT id, definition_id, parent_instance_id, block_id, session_id,
                    status, github_context, started_at, ended_at, created_at,
                    identity_id, memory_id, instance_name, working_directory,
                    display_hidden
             FROM db_agent_instances
             WHERE block_id = ?1 AND status IN ('running', 'paused')
             ORDER BY created_at DESC
             LIMIT 1",
        )?;
        match legacy_stmt.query_row(params![block_id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // ====================================================================
    // Phase 3a — db_agents dual-write helpers MOVED to `super::dual_write`
    // in Phase R.5 (SPEC_STORE_MODULARIZATION_2026_05_27.md). The whole
    // module retires entirely when Phase 3c drops the legacy tables.
    // ====================================================================
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

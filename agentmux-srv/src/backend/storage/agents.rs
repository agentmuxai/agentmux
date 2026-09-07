// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent subsystem — definitions, instances, and their lifecycle CRUD.
//!
//! Covers `agent_def_*` methods (template + user-clone definitions),
//! `instance_*` methods (per-launch instance rows, named-agent
//! continuation, identity-bound active-for-block resolution), the
//! `AgentDefinition` / `AgentInstance` structs, and the `InstanceStatus` enum.
//! `db_agent_definitions` and `db_agent_instances` remain the write targets
//! with dual-write mirrors into `db_agents` (see `dual_write.rs`). The read
//! side is **not** uniformly flipped to `db_agents` — it is decided per
//! function, and the two states currently coexist on purpose:
//!   - `agent_def_list()` reads the consolidated `db_agents` table (a
//!     partial Phase 3b flip that landed undocumented alongside unrelated
//!     work — see `ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3.1's
//!     2026-08-15 correction).
//!   - `agent_def_get()` / `user_clone_defs_for_template()` deliberately
//!     still read `db_agent_definitions` directly — `db_agents` surfaces
//!     template-instance projection rows under the same shape as real
//!     user-clone definitions, which these two callers must NOT see. See
//!     each function's own doc comment.
//!   - `instance_list()` / `instance_get()` and friends still read
//!     `db_agent_instances` directly — Phase 3b has not touched the
//!     instance side at all.
//! Do not describe this as "Phase 3a" or "Phase 3b" as a single global
//! state in a new comment; say which function you mean. See
//! `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
//! P4, and `agent_def_list_and_agent_def_get_read_different_tables_by_design`
//! below for an executable check of the current split.

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
    /// docs/specs/SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md.
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
    /// Docker image to use when `agent_type == "container"`.
    /// e.g. `"ghcr.io/agentmuxai/agent-claude:latest"`.
    /// Empty string for host agents. Schema v6.
    #[serde(default)]
    pub container_image: String,
    /// JSON array of volume mount specs for container agents.
    /// Each element is a string in Docker bind-mount format:
    /// `"source:target"` or `"source:target:options"`.
    /// Empty JSON array (`"[]"`) for host agents. Schema v6.
    #[serde(default = "default_container_volumes")]
    pub container_volumes: String,
    /// Stable Docker container name managed by the server.
    /// Set to `"agentmux-<slug>"` on first container spawn;
    /// empty for host agents. Schema v6.
    #[serde(default)]
    pub container_name: String,
    /// Explicit per-agent opt-in to the CLI's global (ambient) login when no
    /// oauth-class account resolves at spawn time. `0` (default) = spawn
    /// FAILS when an oauth-class provider the agent is supposed to have
    /// credentials for is missing/unresolvable ("fail by default"); `1` =
    /// the spawn proceeds WITHOUT injecting a config dir so the CLI reads
    /// the user's global login (e.g. `~/.claude`) — surfaced via the
    /// `identity.spawn.ambient:` log line, never silent. Toggled from the
    /// Agent setup modal's Accounts tab. Schema v12; the m0017 migration
    /// grandfathers pre-existing linkless agents to `1`. Layer 3 of
    /// SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md (§2.2-§2.4).
    #[serde(default)]
    pub use_ambient_login: i64,
    /// Redirects this agent's harness (CLI) at a non-default model vendor
    /// backend — e.g. `"https://my-proxy.example.com"` for a `claude`-provider
    /// agent, injected into `ANTHROPIC_BASE_URL` at spawn time. Empty string
    /// (default) = use the harness's default vendor endpoint. Only settable
    /// when the resolved provider declares `ProviderConfig::base_url_env_var`
    /// (validated in `agent_define_core`) — see docs/specs for the harness
    /// vs. model-vendor concept this formalizes. Schema v15. Channel-local:
    /// does not currently survive a cross-channel reopen of the same agent
    /// (known limitation, not wired into the registry mirror).
    #[serde(default)]
    pub model_vendor_base_url: String,
    /// Per-agent opt-in: when non-zero, a running Warden Supervisor watcher
    /// agent is permitted to auto-continue this agent's session on
    /// turn-end (subject to a server-side consecutive-nudge ceiling).
    /// Default 0 = opt-in required, same fail-by-default posture as
    /// `use_ambient_login`. Schema v17. Toggled from the Warden Supervisor
    /// panel. See
    /// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
    #[serde(default)]
    pub auto_continue_enabled: i64,
    /// The agent's own dedicated ABF bundle (`db_bundles.id`). Distinct from
    /// `AgentInstance.memory_id` (a specific *launch*'s bundle, which can
    /// still be pointed at a different bundle on purpose — this is just the
    /// default a launch inherits when it doesn't override). Empty string =
    /// not yet provisioned (legacy row predating this field, or a definition
    /// awaiting `m0021`'s backfill). Deliberately NOT dual-written into
    /// `db_agents` (see `dual_write.rs`'s module doc and
    /// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.1) — that table's
    /// own `memory_id` column is instance-only by existing convention, and
    /// this field predates Phase 3b (the reader flip that would need a
    /// decision about how the two interact). Schema v19.
    #[serde(default)]
    pub memory_id: String,
    /// This agent's own disclosure policy for an incoming cross-tier
    /// `transcript_request` jekt (`muxspect` Phase B/C — LAN/WAN
    /// conversation visibility, `SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`,
    /// jekt tier rules confirmed in `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`).
    /// One of `"private"` (default — auto-deny, fail-closed), `"trusted_peers"`
    /// (auto-approve only a requester in `db_conversation_trust_grants`), or
    /// `"ask"` (force human escalation, never auto-respond). Schema v26.
    /// Channel-local only, like `model_vendor_base_url`/`memory_id` above —
    /// deliberately NOT added to `DefinitionRecordV1`'s cross-channel wire
    /// format yet; a cross-channel-reopened agent starts back at the safe
    /// `"private"` default rather than carrying a stale value. Known
    /// limitation, not a bug — same precedent as those two fields.
    #[serde(default = "default_conversation_visibility")]
    pub conversation_visibility: String,
}

/// `AgentDefinition::conversation_visibility`'s serde default — the safe,
/// fail-closed value for any row/context that doesn't specify one
/// (including every pre-existing row before this column existed).
pub fn default_conversation_visibility() -> String {
    "private".to_string()
}

/// The working directory `agent.open` substitutes when an
/// `AgentDefinition`'s own `working_directory` is blank —
/// `~/.agentmux/agents/<name-derived-slug>`.
///
/// Blank is the DEFAULT for a newly defined agent, so this is the path most
/// agents actually run in. Extracted so `agent.open` (which creates and uses
/// it) and native-memory resolution (which must find the memories written
/// inside it) share one definition and cannot drift — they previously
/// disagreed, and the resolver simply gave up on a blank field, breaking
/// Armory → Memory → Personal for the common case. See
/// `docs/specs/SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md`.
///
/// NOTE: deliberately NOT `derive_slug` above. `agent.open`'s own inline
/// derivation is `name.to_lowercase()` with a Unicode-aware
/// `char::is_alphanumeric()` filter (keeping `-`/`_`), which differs from
/// `derive_slug`'s ASCII-only rule, dash-collapsing, 64-char trim and
/// `"agent"` fallback. Reusing `derive_slug` here would silently point at a
/// DIFFERENT directory than the one the agent is really running in for any
/// non-ASCII or dash-heavy name. This mirrors the real behaviour exactly.
pub fn default_agent_working_dir(agent_name: &str) -> String {
    let slug: String = agent_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    format!("~/.agentmux/agents/{slug}")
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

fn default_container_volumes() -> String {
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
    /// Legacy Identity-bundle id column — `db_identity_bundles` was
    /// dropped in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md.
    /// The launch modal now writes an account_id here instead; credential
    /// resolution and display names both go through
    /// `db_agent_identity_links`/`db_accounts`. Empty string means
    /// "ambient creds, no env-var injection."
    #[serde(default)]
    pub identity_id: String,
    /// FK to `db_bundles.id`. Empty string means "use the blank
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
    /// Reads from the consolidated `db_agents` table, ordered by
    /// `updated_at DESC` then `created_at ASC`. Dual-write keeps
    /// `db_agents.updated_at` fresh on every definition mutation AND every
    /// instance lifecycle touch, so recency on a row tracks the last time
    /// the agent was either edited or launched.
    ///
    /// Result-set shape: every `db_agents` row is returned — templates
    /// (`is_template = 1`) and user-clone projections (`is_template = 0`)
    /// each appear once. `parent_id` is sourced from
    /// `db_agents.parent_template_id`.
    ///
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
        let local = self.agent_def_get_local_only(id)?;
        if local.is_some() {
            return Ok(local);
        }
        // Not in local SQLite — check the global cross-channel registry.
        // agent_def_list() already overlays this; keep agent_def_get consistent.
        let Some(reg) = self.shared_def_registry() else {
            return Ok(None);
        };
        match reg.get(id) {
            Ok(Some(record)) => Ok(Some(super::def_registry_mirror::record_to_agent_definition(&record))),
            Ok(None) => Ok(None),
            Err(e) => {
                tracing::warn!(agent_id = %id, error = %e, "agent_def_get: global registry lookup failed; returning not-found");
                Ok(None)
            }
        }
    }

    /// Local-only lookup — queries `db_agent_definitions` directly, no
    /// registry fallback. Shared by `agent_def_get` (public read path,
    /// unchanged behavior) and `instance_create`'s FK-backfill gate,
    /// which specifically needs to know whether the FK-target table
    /// itself has a row — NOT `agent_def_exists_local`, which queries
    /// the newer, separately-maintained `db_agents` consolidated table
    /// and isn't guaranteed to agree with `db_agent_definitions` in
    /// every code path.
    fn agent_def_get_local_only(&self, id: &str) -> Result<Option<AgentDefinition>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description,
                    working_directory, shell, provider_flags, auto_start,
                    restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded,
                    accounts, parent_id, branch_label, updated_at,
                    user_hidden, container_image, container_volumes, container_name,
                    use_ambient_login, model_vendor_base_url, auto_continue_enabled, memory_id,
                    conversation_visibility
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
        // Local SQLite: this channel's templates (seeded) + its own user agents.
        let local: Vec<AgentDefinition> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(&format!("{AGENT_DEFINITION_SELECT} ORDER BY updated_at DESC, created_at ASC"))?;
            let rows = stmt.query_map([], map_agent_definition_row)?;
            let mut agents = Vec::new();
            for row in rows {
                agents.push(row?);
            }
            agents
        };
        // conn dropped. Overlay the GLOBAL cross-channel user-agent roster so
        // an agent created in another channel appears here too. The global
        // store wins for user rows (it's authoritative and holds cross-channel
        // agents); templates (seeded — never in the global store) come from
        // local SQLite. Falls back to SQLite-only when the global store is
        // absent or unreadable. (P0.2c.)
        let Some(reg) = self.shared_def_registry() else {
            return Ok(local);
        };
        let global = match reg.list_active() {
            Ok(recs) => recs,
            Err(e) => {
                tracing::warn!(error = %e, "def registry: global list failed, using SQLite only");
                return Ok(local);
            }
        };
        let mut by_id: std::collections::HashMap<String, AgentDefinition> =
            local.into_iter().map(|d| (d.id.clone(), d)).collect();
        for rec in &global {
            let mut def = super::def_registry_mirror::record_to_agent_definition(rec);
            // model_vendor_base_url is deliberately channel-local only —
            // DefinitionRecordV1 doesn't carry it, so record_to_agent_definition
            // always returns "" for it. Without this, the global overlay
            // silently wiped a same-channel agent's override on every read
            // (including agent.open's spawn-time resolution), making the
            // whole feature a no-op in default single-instance operation —
            // not just the genuinely-cross-channel case this limitation is
            // documented for. Preserve the local row's value when one
            // exists; only a truly cross-channel agent (no local row) sees
            // the empty default. (reagent P0 on PR #2505.)
            //
            // memory_id needs the IDENTICAL treatment, for the identical
            // reason (P0 fix, ReAgent review on PR #2587 round 6):
            // DefinitionRecordV1 doesn't carry memory_id either (same
            // documented gap, def_registry_mirror.rs), so
            // record_to_agent_definition always returns "" for it too.
            // Every local write auto-mirrors into the global registry
            // (agent_def_insert -> registry_def_upsert), so this overlay
            // fires for virtually every agent whenever a shared store is
            // configured — the "normal case," not an edge case. Without
            // this fix, agent_def_list() would silently zero memory_id on
            // every read, defeating agent_open.rs's spawn-time bundle
            // resolution (falls back to the driftable agent.provider
            // instead of the immutable bundle) and m0021's own
            // memory_id-empty backfill filter (which would re-process
            // every already-bound agent on every migration run, since it
            // reads through this same function).
            if let Some(existing) = by_id.get(&def.id) {
                def.model_vendor_base_url = existing.model_vendor_base_url.clone();
                def.memory_id = existing.memory_id.clone();
                // conversation_visibility is the identical channel-local-only
                // case (SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md
                // Phase B/C, jekt rules SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md)
                // — DefinitionRecordV1 doesn't carry it, so without this the
                // global overlay would silently reset every agent's
                // disclosure policy back to "private" on every list read,
                // even though "private" is a safe fail-closed default (this
                // preserves the ADMINISTRATOR'S actual configured choice,
                // not just avoiding a crash).
                def.conversation_visibility = existing.conversation_visibility.clone();
            }
            by_id.insert(def.id.clone(), def);
        }
        let mut result: Vec<AgentDefinition> = by_id.into_values().collect();
        // Match the SQL ORDER BY: updated_at DESC, then created_at ASC.
        result.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then(a.created_at.cmp(&b.created_at))
        });
        Ok(result)
    }

    /// Count agent rows (used by seed engine to check if seeding is needed).
    /// Reads from the consolidated `db_agents` table. The seed engine only
    /// cares about `== 0` to decide "fresh database, seed templates".
    pub fn agent_def_count(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_agents",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Whether a definition with `id` exists in the LOCAL channel's SQLite
    /// (`db_agents`). Gates the cross-channel content/skills fallback: a
    /// locally-known agent with genuinely empty content/skills must NOT
    /// resurrect them from the global record. (reagent P1 on #1385.)
    pub(super) fn agent_def_exists_local(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_agents WHERE id = ?1)",
            params![id],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    /// Delete all seeded agents (is_seeded=1). Used by reseed to clear built-in agents.
    pub fn agent_def_delete_seeded(&self) -> Result<usize, StoreError> {
        // Consolidation Phase 3b (PR 2): only the template rows go. Every
        // `is_template = 0` row — a user clone OR an agent launched from a
        // template — is an agent in its own right and persists across a
        // reseed, keeping its `parent_template_id` (templates have stable
        // ids, so the lineage survives the re-insert). The legacy FK cascade
        // still clears stale `db_agent_instances` rows on its own.
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM db_agent_definitions WHERE is_seeded=1", [])?
        };
        self.agents_dual_write_seeded_delete()?;
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
        self.agent_def_insert_local_only(agent, None)?;
        // Mirror into the global cross-channel definition store. Content +
        // skills are inserted after the definition (separate calls), so this
        // initial record is content-less; agent_content_set / agent_skill_*
        // re-mirror with the full payload. (P0.2b.)
        self.registry_def_upsert(&agent.id);
        Ok(())
    }

    /// Insert into local `db_agent_definitions` + dual-write `db_agents`
    /// only — does NOT mirror to the global registry. Used directly by
    /// `agent_def_insert` (which mirrors right after) and by
    /// `agent_def_backfill_local_from_registry` (which must NOT mirror —
    /// see that function's doc comment for why re-mirroring immediately
    /// after this call would wipe the registry's real content/skills).
    ///
    /// `updated_at_override`: `None` stamps `updated_at = created_at`
    /// (the original, unchanged behavior for genuinely new definitions).
    /// `Some(ts)` stamps `updated_at = ts` instead — used by the backfill
    /// path to preserve the registry record's real `updated_at` rather
    /// than resetting it.
    fn agent_def_insert_local_only(
        &self,
        agent: &mut AgentDefinition,
        updated_at_override: Option<i64>,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let base = if agent.slug.is_empty() {
            derive_slug(&agent.name)
        } else {
            agent.slug.clone()
        };
        // Collision-resolve: scan for existing slugs matching base or base-N.
        // Reads uniqueness from `db_agents` — the consolidated table surfaces
        // both definition slugs and template-instance projections, so a slug
        // collision against an instance-derived row is caught here too.
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
             is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden,
             container_image, container_volumes, container_name, use_ambient_login,
             model_vendor_base_url, auto_continue_enabled, memory_id, conversation_visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)",
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
                // New definitions: updated_at == created_at, unless the
                // caller supplied an override (backfill path preserving
                // the registry record's real updated_at).
                updated_at_override.unwrap_or(agent.created_at),
                // Phase 2 (hide templates): new rows start visible. The
                // user only hides via the explicit `agent_def_hide` RPC,
                // and the agent-seed re-sync forces user_hidden = 0 on
                // any newly-added template id anyway, so honouring the
                // caller-supplied value here is safe even when a stray
                // 1 sneaks through.
                agent.user_hidden,
                agent.container_image,
                agent.container_volumes,
                agent.container_name,
                agent.use_ambient_login,
                agent.model_vendor_base_url,
                agent.auto_continue_enabled,
                agent.memory_id,
                agent.conversation_visibility,
            ],
        )?;
        // Persist the stamped updated_at before we leave the lock so the
        // dual-write helper sees the same value the SQL row carries.
        let stamped_updated_at = updated_at_override.unwrap_or(agent.created_at);
        drop(conn);
        let mut snapshot = agent.clone();
        snapshot.updated_at = stamped_updated_at;
        // Mirror new definition into db_agents immediately.
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

    /// Materialize a local shadow of a registry-only definition — this
    /// channel's `db_agent_definitions`/`db_agents` row plus a best-
    /// effort local copy of content/skills — so `instance_create`'s FK
    /// can succeed for an agent whose definition exists cross-channel
    /// but was never created in THIS channel's SQLite.
    ///
    /// Deliberately does NOT call `registry_def_upsert` (unlike
    /// `agent_def_insert`, which does): the registry is already
    /// authoritative for this record. `registry_def_upsert` rebuilds
    /// the registry record from `agent_content_get_all_local`/
    /// `agent_skill_list_local` — LOCAL-only reads, by design (see that
    /// function's own comment) — so calling it immediately after this
    /// insert-with-no-content-yet would overwrite the registry's real
    /// content/skills with empty arrays. Content/skill rows are instead
    /// copied here via direct INSERTs (bypassing `agent_content_set`/
    /// `agent_skill_insert`, which each call `registry_def_upsert`
    /// themselves) so nothing re-mirrors mid-backfill.
    ///
    /// Copy failures for content/skills are logged and otherwise
    /// ignored — the definition row alone satisfies the FK, and reads
    /// still resolve content/skills correctly via the existing
    /// cross-channel fallback even if this best-effort local copy is
    /// incomplete. A slug collision against an unrelated local agent
    /// will rename the local slug (existing `agent_def_insert_local_only`
    /// behavior) — low-probability and non-fatal, since slug isn't the
    /// FK target.
    fn agent_def_backfill_local_from_registry(
        &self,
        record: &crate::registry::DefinitionRecord,
    ) -> Result<(), StoreError> {
        let mut def = super::def_registry_mirror::record_to_agent_definition(record);
        self.agent_def_insert_local_only(&mut def, Some(record.data.updated_at))?;

        let conn = self.conn.lock().unwrap();
        for c in &record.data.content {
            if let Err(e) = conn.execute(
                "INSERT INTO db_agent_content (agent_id, content_type, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id, content_type) DO UPDATE SET content=?3, updated_at=?4",
                params![def.id, c.content_type, c.content, record.data.updated_at],
            ) {
                tracing::warn!(
                    def_id = %def.id,
                    content_type = %c.content_type,
                    error = %e,
                    "instance_create backfill: local content copy failed (non-fatal)"
                );
            }
        }
        for s in &record.data.skills {
            if let Err(e) = conn.execute(
                "INSERT INTO db_agent_skills (id, agent_id, name, trigger, skill_type, description, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    s.id,
                    def.id,
                    s.name,
                    s.trigger,
                    s.skill_type,
                    s.description,
                    s.content,
                    record.data.updated_at,
                ],
            ) {
                tracing::warn!(
                    def_id = %def.id,
                    skill = %s.name,
                    error = %e,
                    "instance_create backfill: local skill copy failed (non-fatal)"
                );
            }
        }
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
                        user_hidden, container_image, container_volumes, container_name,
                        use_ambient_login, model_vendor_base_url, auto_continue_enabled, memory_id,
                        conversation_visibility
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
            // Query db_agents (the superset view) for slug uniqueness — matches
            // agent_def_insert at line 440. Template-instance projections add rows to
            // db_agents without a corresponding db_agent_definitions entry; checking
            // only db_agent_definitions would miss those slugs and allow collisions.
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
                    is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden,
                    container_image, container_volumes, container_name, use_ambient_login,
                    model_vendor_base_url, auto_continue_enabled, memory_id, conversation_visibility)
                 VALUES
                   (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                    ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)",
                params![
                    agent.id, agent.slug, agent.name, agent.icon, agent.provider,
                    agent.description, agent.working_directory, agent.shell,
                    agent.provider_flags, agent.auto_start, agent.restart_on_crash,
                    agent.idle_timeout_minutes, agent.created_at, agent.agent_type,
                    agent.environment, agent.agent_bus_id, agent.is_seeded,
                    agent.accounts, agent.parent_id, agent.branch_label,
                    agent.created_at, // updated_at = created_at for new rows
                    agent.user_hidden,
                    agent.container_image, agent.container_volumes, agent.container_name,
                    agent.use_ambient_login,
                    agent.model_vendor_base_url,
                    agent.auto_continue_enabled,
                    agent.memory_id,
                    agent.conversation_visibility,
                ],
            )?;
            agent.created_at
            // conn guard drops here; dual-write acquires the lock again below
        };

        let mut snapshot = agent.clone();
        snapshot.updated_at = stamped_updated_at;
        self.agents_dual_write_definition_upsert(&snapshot)?;
        // Mirror the freshly-defined agent into the global store (agent.define
        // path). Without this a define-created agent stays channel-local until
        // a later edit. (codex P2 on #1385.)
        self.registry_def_upsert(&agent.id);

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
        // Only seeded templates (`is_template = 1`) may flip the hide flag.
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
        // for the next read. (Templates are seeded; they do NOT go to the
        // global cross-channel def store, so no mirror here.)
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

    /// Set `memory_id` on a LOCAL agent definition — but only if it's
    /// currently empty. Exists because `agent_def_update`'s SET clause
    /// deliberately never touches this column (readonly-after-creation, see
    /// its own field doc comment) — this is the one narrow, explicit path
    /// allowed to set it, and only the FIRST time, for `m0021`'s backfill
    /// and definition-time provisioning to use. Returns `Ok(true)` if the
    /// row existed and had an empty `memory_id` (write applied), `Ok(false)`
    /// if the row doesn't exist locally OR already has a non-empty value
    /// (no-op, not an error — matches the "set once" semantic silently
    /// rather than requiring every caller to pre-check).
    ///
    /// LOCAL ONLY, deliberately — a cross-channel (global-registry-only)
    /// definition can't be reached this way today because
    /// `DefinitionRecordV1` doesn't carry `memory_id` yet (same accepted gap
    /// as `model_vendor_base_url`, see `def_registry_mirror.rs`). Callers
    /// backfilling across the whole agent population must check
    /// `agent_def_get_local_only` first and skip (with a log) anything that
    /// only resolves via the global registry.
    pub fn agent_def_set_memory_id_if_empty(
        &self,
        id: &str,
        memory_id: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_agent_definitions SET memory_id = ?1 WHERE id = ?2 AND memory_id = ''",
            params![memory_id, id],
        )?;
        if rows > 0 {
            // Mirror into db_agents' SEPARATE default_memory_id column (see
            // its own CREATE TABLE comment) so agent_def_list — which reads
            // from db_agents, not db_agent_definitions — sees the binding
            // too. Same secondary-mirror pattern as agent_def_set_hidden
            // just above. Best-effort: a mirror failure here would leave
            // db_agents transiently stale, same posture as that function's
            // own error handling (log + continue, not fail the caller).
            if let Err(e) = conn.execute(
                "UPDATE db_agents SET default_memory_id = ?1 WHERE id = ?2",
                params![memory_id, id],
            ) {
                tracing::error!(
                    id = %id,
                    memory_id = %memory_id,
                    error = %e,
                    "db_agents dual-write: default_memory_id mirror failed",
                );
            }
        }
        Ok(rows > 0)
    }

    /// The provider's default vendor, or `"custom"` when the agent has a
    /// non-empty `model_vendor_base_url` override — same rule the
    /// frontend's `resolveEffectiveVendor`
    /// (`frontend/app/view/agent/providers/catalog.ts`) already uses for
    /// the dual-icon vendor badge. Falls back to the provider id itself
    /// if the provider isn't in the registry (e.g. a stale/custom
    /// provider string), same fallback the frontend uses.
    ///
    /// P2 fix (2026-08-15, Codex review on PR #2587): `bundle_provision_
    /// for_new_agent` used to ignore `model_vendor_base_url` entirely and
    /// always pick the provider's bare default — a Claude agent pointed
    /// at a custom endpoint got an immutable bundle claiming
    /// `model="anthropic"`, permanently wrong once
    /// `check_provider_model_immutable` locks it in.
    pub(crate) fn resolve_effective_vendor(provider: &str, model_vendor_base_url: &str) -> String {
        if !model_vendor_base_url.trim().is_empty() {
            return "custom".to_string();
        }
        crate::backend::providers::get_provider(provider)
            .and_then(|p| p.supported_vendors.first().copied())
            .unwrap_or(provider)
            .to_string()
    }

    /// Resolve the effective harness/provider id for `agent`, preferring
    /// its bound ABF bundle's copy over the agent's own `provider`
    /// column. The bundle is the readonly-once-set source of truth
    /// (`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §7.4.1):
    /// `AgentDefinition.provider` can still drift after creation via
    /// `agent.define`'s `if_exists=update` path, while the bundle's own
    /// copy cannot (backend-enforced in `bundle.upsert`, see
    /// `check_provider_model_immutable`). Falls back to `agent.provider`
    /// when there's no bundle to consult (unbound legacy agent, missing
    /// row, or an empty `provider` on the bundle itself) — a caller
    /// never hard-fails on this alone.
    ///
    /// **Call this on the EFFECTIVE identity/memory store
    /// (`AppState.id_store`), never on the per-channel `wstore` directly**
    /// — the bundle lives in the shared store when one is configured (the
    /// normal case), so a `wstore` lookup silently returns the fallback
    /// every time in that configuration. See
    /// `bundle_provision_for_new_agent`'s own doc comment for the write-
    /// side half of this same rule.
    ///
    /// Consolidated here (2026-08-15) after the identical resolution
    /// logic was independently duplicated in `agent_open.rs`'s spawn path
    /// and found to have the exact same wstore-vs-id_store mistake twice
    /// in review (rounds 3 and, for a second call site,
    /// `identity/resolver/inject.rs`'s layer-3 credential gate, found in
    /// a later scoping pass — not by an automated review this time). One
    /// shared implementation both consumers call means they can't drift
    /// on this resolution independently again.
    pub fn resolve_effective_provider_id(&self, agent: &AgentDefinition) -> String {
        if agent.memory_id.is_empty() {
            return agent.provider.clone();
        }
        match self.bundle_memory_get(&agent.memory_id) {
            Ok(Some(b)) if !b.provider.is_empty() => b.provider,
            _ => agent.provider.clone(),
        }
    }

    /// Provision a fresh, dedicated ABF bundle for a NEW agent definition —
    /// the definition-time half of
    /// `docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3.2
    /// (m0021 is the backfill half, for agents that already existed before
    /// this shipped). Callers pass the not-yet-inserted `AgentDefinition`
    /// so its already-known `provider` (harness) carries straight onto the
    /// bundle — unlike `m0021`'s backfill, which can't just hardcode
    /// `claude`/`anthropic` for a legacy agent's unknown-at-migration-time
    /// harness, a brand-new agent's harness is already a required field at
    /// this point, so there's no need to guess.
    ///
    /// **Call this on the EFFECTIVE identity/memory store
    /// (`AppState.id_store`), never on the per-channel `wstore` directly.**
    /// Bundles live in the shared store when one is configured (the normal
    /// case) — every real bundle-read path (`listmemories`/`getmemory`/
    /// the Armory editor/the bundle-summary panel) reads through
    /// `id_store`, so a bundle created via `wstore.bundle_memory_upsert`
    /// directly would be written to a different SQLite file and be
    /// invisible everywhere else (P1 finding, Codex review on PR #2587,
    /// live in the shipped code this fixes: `agent_def_provision_and_
    /// bind_bundle`'s callers all used to invoke this method ON `wstore`).
    ///
    /// Does NOT mutate `agent` or touch `db_agent_definitions` — returns
    /// the new bundle's id; callers set `agent.memory_id` themselves before
    /// their own insert (so the existing `agent_def_insert*` INSERT
    /// statements, which already include `memory_id`, pick it up with no
    /// separate write).
    pub fn bundle_provision_for_new_agent(
        &self,
        agent: &AgentDefinition,
        now: i64,
    ) -> Result<String, StoreError> {
        let vendor = Self::resolve_effective_vendor(&agent.provider, &agent.model_vendor_base_url);
        let bundle_id = uuid::Uuid::new_v4().to_string();
        let name = self.resolve_unique_bundle_name(&format!("{} — ABF", agent.name))?;
        let bundle = super::memory_bundles::Memory {
            id: bundle_id.clone(),
            name,
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: agent.provider.clone(),
            model: vendor,
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: now,
            updated_at: now,
            is_system: false,
        };
        self.bundle_memory_upsert(&bundle)?;
        Ok(bundle_id)
    }

    /// Resolve a `db_bundles.name` guaranteed not to collide with an
    /// existing row — that column is `TEXT NOT NULL UNIQUE`, but only
    /// `AgentDefinition.slug` is guaranteed unique, not the display
    /// `name`, so a naive `"{agent.name} — ABF"` collides whenever two
    /// agents share a display name. Appends a numeric suffix on
    /// collision, same shape `agent_def_insert_local_only` already uses
    /// for slug collisions.
    ///
    /// P1 fix (2026-08-15, ReAgent review on PR #2587 round 4): before
    /// this, a collision here silently left a runtime-provisioned agent
    /// unbound (the caller's best-effort error handling swallowed it) and
    /// — worse — unconditionally ABORTED `m0021`'s entire backfill loop
    /// via `?` on the very first collision, permanently stalling backfill
    /// for every agent processed after it, on every subsequent boot,
    /// until the underlying name collision was manually resolved.
    pub(crate) fn resolve_unique_bundle_name(&self, base_name: &str) -> Result<String, StoreError> {
        let existing_names: std::collections::HashSet<String> =
            self.bundle_memory_list()?.into_iter().map(|b| b.name).collect();
        if !existing_names.contains(base_name) {
            return Ok(base_name.to_string());
        }
        let mut n: u32 = 2;
        loop {
            let candidate = format!("{base_name} ({n})");
            if !existing_names.contains(&candidate) {
                return Ok(candidate);
            }
            n += 1;
        }
    }

    /// Provision + bind a fresh bundle to an ALREADY-INSERTED agent
    /// definition — the two-step, post-insert counterpart to
    /// `bundle_provision_for_new_agent` above. Deliberately separate from
    /// the insert itself (rather than setting `memory_id` on the struct
    /// before calling `agent_def_insert*`/`agent_def_find_or_insert`):
    /// `agent_def_find_or_insert` in particular uses its `AgentDefinition`
    /// argument as BOTH the lookup key and the conditional-insert payload,
    /// so a bundle built up-front would go to waste (leaked, unbound) on
    /// every `if_exists=skip`/`update` call against an already-existing
    /// name — and `agent.define` is meant to be called repeatedly/
    /// idempotently. Binding after the fact, only once the caller has
    /// confirmed a row was genuinely freshly inserted, avoids that leak.
    ///
    /// Best-effort by design (matches `createagent`'s own color-assignment
    /// comment: a failure here shouldn't fail agent creation) — logs and
    /// returns on either step's failure rather than propagating, since the
    /// agent row itself is already durably committed by the time this
    /// runs, and a still-unbound agent is exactly the pre-existing
    /// `memory_id=''` state `m0021` already knows how to backfill later.
    ///
    /// Takes `&mut AgentDefinition` and writes the new bundle id back into
    /// `agent.memory_id` on success — same "caller's struct reflects what
    /// landed" convention `agent_def_update` documents just below — so RPC
    /// handlers that serialize `agent` straight back to the caller (e.g.
    /// `createagent`, `importagentfromclaw`) return the real value instead
    /// of the empty string the struct held before this call.
    ///
    /// `self` (the definition store, i.e. `wstore`) and `bundle_store`
    /// (the effective identity/memory store, i.e. `AppState.id_store`) are
    /// deliberately two SEPARATE parameters, not the same store used for
    /// both writes — see `bundle_provision_for_new_agent`'s own doc
    /// comment for why (P1 fix, Codex review on PR #2587: they used to be
    /// the same store, silently writing every provisioned bundle
    /// somewhere the rest of the app can't see it).
    pub fn agent_def_provision_and_bind_bundle(
        &self,
        bundle_store: &Store,
        agent: &mut AgentDefinition,
        now: i64,
    ) {
        let bundle_id = match bundle_store.bundle_provision_for_new_agent(agent, now) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(agent_id = %agent.id, error = %e, "agent_def_provision_and_bind_bundle: bundle create failed (non-fatal)");
                return;
            }
        };
        match self.agent_def_set_memory_id_if_empty(&agent.id, &bundle_id) {
            Ok(true) => agent.memory_id = bundle_id,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(agent_id = %agent.id, bundle_id = %bundle_id, error = %e, "agent_def_provision_and_bind_bundle: bind failed (non-fatal)");
            }
        }
    }

    /// Update an existing agent definition (all fields except id, created_at, is_seeded, `parent_id`).
    /// `parent_id` is NOT updatable post-insert — it describes the agent's
    /// lineage; re-parenting is done by creating a new fork, not mutating the
    /// original.
    ///
    /// `branch_label` IS written here (unlike `parent_id`) — this storage-
    /// layer function persists whatever is on `agent`. Immutability for most
    /// callers is enforced one layer up, at the RPC handler: `updateagent`
    /// always passes back `old.branch_label.clone()` unchanged, so it's a
    /// no-op for that caller. `renameagentdefinitiontitle` is the one
    /// deliberate exception that supplies a real change — see
    /// `agentmux-srv/src/server/agent_handlers/template.rs` and
    /// docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §4.
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
                 agent_type=?11, environment=?12, agent_bus_id=?13, accounts=?14, updated_at=?15,
                 container_image=?17, container_volumes=?18, container_name=?19,
                 use_ambient_login=?20, branch_label=?21, model_vendor_base_url=?22,
                 auto_continue_enabled=?23, conversation_visibility=?24
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
                    agent.id,
                    agent.container_image,
                    agent.container_volumes,
                    agent.container_name,
                    agent.use_ambient_login,
                    agent.branch_label,
                    agent.model_vendor_base_url,
                    agent.auto_continue_enabled,
                    agent.conversation_visibility,
                ],
            )?
        };
        // Reflect the persisted timestamp back to the caller's struct so an
        // RPC response carries the fresh value, not the pre-update one.
        agent.updated_at = now;
        // Mirror updated definition into db_agents.
        if rows > 0 {
            self.agents_dual_write_definition_upsert(agent)?;
            // Mirror the updated definition into the global store. (P0.2b.)
            self.registry_def_upsert(&agent.id);
        }
        // Cross-channel edit: an agent surfaced only via the global overlay has
        // no local SQLite row, so the UPDATE affected 0 rows. Apply the edit to
        // the global record directly (preserving its content/skills) so editing
        // a cross-channel agent isn't silently dropped with a "not found" error
        // — symmetric with the unconditional cross-channel delete. (reagent P1
        // on #1385.)
        let updated_global = if rows == 0 {
            self.registry_def_update_definition_fields(agent)
        } else {
            false
        };
        Ok(rows > 0 || updated_global)
    }

    /// Delete a agent definition by id. Returns true if a row was deleted.
    pub fn agent_def_delete(&self, id: &str) -> Result<bool, StoreError> {
        // Snapshot cascaded instance ids AND issue the parent DELETE
        // under one lock acquisition so no thread can `instance_create`
        // a new row for this definition between the two steps. The
        // SQL DELETE's FK cascade fires inside SQLite while we hold
        // the mutex, so the snapshot exactly matches the rows that
        // got removed by the cascade.
        // Consolidation Phase 3b (PR 2): the legacy FK cascade still clears
        // stale `db_agent_instances` rows, but launches are no longer rows of
        // their own — a user agent launched from this template keeps its own
        // `db_agents` row (it is an agent, not a launch of the template), the
        // same way user clones always survived deleting their template. The
        // JSON registry mirror for the deleted definition's own row is
        // covered by `registry_def_retire` below.
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM db_agent_definitions WHERE id=?1", params![id])?
        };
        if rows > 0 {
            if let Some(reg) = self.registry() {
                if let Err(e) = reg.hard_delete(id) {
                    tracing::warn!(
                        agent_def_id = %id,
                        error = %e,
                        "registry: failed to mirror agent_def_delete"
                    );
                }
            }
            // Mirror deleted definition out of db_agents.
            self.agents_dual_write_definition_delete(id)?;
        }
        // Tombstone the global definition record so another channel's stale
        // SQLite can't resurrect this deleted user agent — AND so an agent
        // that exists in THIS channel only via the global overlay (no local
        // SQLite row, rows == 0) is actually deletable instead of reappearing
        // on the next agent_def_list. (P0.2b + codex P1 on #1385.)
        let global_retired = self.registry_def_retire(id);
        Ok(rows > 0 || global_retired)
    }

    /// One-shot grandfather pass for the layer-3 ambient-login opt-in
    /// (m0017, spec §2.4 of SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md).
    ///
    /// Agents WITHOUT any `db_agent_identity_links` row at migration time
    /// were de-facto ambient users → `use_ambient_login = 1`; agents WITH
    /// links opted into managed accounts → `0` (honest failure is the new
    /// behavior for them). `linked_agent_ids` comes from the SHARED store
    /// (the live links table); this method writes both the legacy
    /// `db_agent_definitions` and the consolidated `db_agents` projections
    /// of the CURRENT channel store. Returns (rows set to 1, rows set to 0)
    /// across `db_agent_definitions`.
    pub fn agents_grandfather_ambient_login(
        &self,
        linked_agent_ids: &std::collections::HashSet<String>,
    ) -> Result<(usize, usize), StoreError> {
        let conn = self.conn.lock().unwrap();
        // Set everyone ambient first, then flip the linked set back to the
        // fail-by-default 0 — two passes instead of a dynamic IN() list.
        let ambient_defs = conn.execute(
            "UPDATE db_agent_definitions SET use_ambient_login = 1",
            [],
        )?;
        conn.execute("UPDATE db_agents SET use_ambient_login = 1", [])?;
        let mut linked_rows = 0usize;
        for id in linked_agent_ids {
            linked_rows += conn.execute(
                "UPDATE db_agent_definitions SET use_ambient_login = 0 WHERE id = ?1",
                params![id],
            )?;
            conn.execute(
                "UPDATE db_agents SET use_ambient_login = 0 WHERE id = ?1",
                params![id],
            )?;
        }
        Ok((ambient_defs.saturating_sub(linked_rows), linked_rows))
    }

    // AgentContent / AgentSkill / AgentHistory CRUD live in
    // `super::content` / `super::skills` / `super::history` —
    // each adds an `impl Store {}` block to this type.
}

impl Store {

    // ---- Agent instance CRUD ----
    //
    // Agent-concept consolidation Phase 3b, PR 2 of 4
    // (SPEC_AGENT_ARCHITECTURE_2026_05_27.md): every method in this block
    // reads and writes `db_agents` only. `db_agent_instances` is no longer
    // touched by the live instance API; it stays on disk until PR 4 drops it.
    //
    // What an "instance" is now: the launch-side view of a non-template
    // `db_agents` row. One row per agent, so:
    //   - `id` / `definition_id` are both the row's `id` — the agent's
    //     identity (the rule `instance_list` adopted in PR #1111).
    //   - `block_id`/`session_id`/`status`/`started_at`/`ended_at` are the
    //     row's LATEST launch state (schema v29); a continuation moves them
    //     to the new launch instead of adding a row, so `parent_instance_id`
    //     is always empty and chains are pre-collapsed.
    //   - Launching a TEMPLATE creates a new user-agent row keyed by the
    //     launch's instance id (`parent_template_id` = the template);
    //     launching a USER agent folds into its own row.
    //   - `display_hidden` is `user_hidden` (one flag, one row).

    /// List agents as instances. Both filters are optional — pass `None`
    /// to scan all. Ordered by `updated_at` descending, `created_at` as a
    /// tiebreaker (most recent activity first — every launch, continuation
    /// and lifecycle write bumps `updated_at`).
    ///
    /// The `definition_id` filter matches the agent's own `id` only:
    /// templates are not agents and user agents derived from one template
    /// are separate agents (see `instance_list_named` for the one caller
    /// that wants "everything derived from this template"). The `status`
    /// filter reads the row's latest launch status (v29).
    pub fn instance_list(
        &self,
        definition_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents WHERE is_template = 0 AND user_hidden = 0"
        );
        let mut param_vals: Vec<String> = Vec::new();
        if let Some(d) = definition_id {
            sql.push_str(&format!(" AND id = ?{}", param_vals.len() + 1));
            param_vals.push(d.to_string());
        }
        if let Some(s) = status {
            sql.push_str(&format!(" AND status = ?{}", param_vals.len() + 1));
            param_vals.push(s.to_string());
        }
        sql.push_str(" ORDER BY updated_at DESC, created_at DESC");
        let mut stmt = conn.prepare(&sql)?;
        let iter = stmt.query_map(rusqlite::params_from_iter(param_vals.iter()), map_instance_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// One agent by id, as an instance. `None` for a template or an
    /// unknown id.
    pub fn instance_get(&self, id: &str) -> Result<Option<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents WHERE id = ?1 AND is_template = 0"
        ))?;
        match stmt.query_row(params![id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// A single `db_agents` row in the `AgentDefinition` shape, no registry
    /// overlay — the definition a launch resolves against. Templates and
    /// user agents alike.
    fn agent_row_get(&self, id: &str) -> Result<Option<AgentDefinition>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("{AGENT_DEFINITION_SELECT} WHERE id = ?1"))?;
        match stmt.query_row(params![id], map_agent_definition_row) {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Record a launch. Returns the agent row the launch now lives on, as an
    /// instance — **its `id` may differ from `inst.id`**: a launch of a user
    /// agent folds into that agent's row, and a continuation (non-empty
    /// `parent_instance_id` naming an existing row) folds into the row it
    /// continues. Only a fresh launch of a TEMPLATE creates a new row, keyed
    /// by `inst.id`. Callers must persist the returned id, not the one they
    /// passed (the frontend stores it as the block's `agentInstanceId`).
    ///
    /// `inst.definition_id` must name a `db_agents` row; when it only exists
    /// in the global cross-channel definition registry (an agent created in
    /// another channel), the local definition is backfilled first, exactly
    /// as before (`REPORT_AGENT_DEFINITION_DB_GAP_2026_07_27.md`). A
    /// definition that exists nowhere is `StoreError::NotFound` — the error
    /// the legacy FK used to raise, minus the FK.
    pub fn instance_create(&self, inst: &AgentInstance) -> Result<AgentInstance, StoreError> {
        let def = match self.agent_row_get(&inst.definition_id)? {
            Some(d) => d,
            None => {
                if let Some(reg) = self.shared_def_registry() {
                    match reg.get(&inst.definition_id) {
                        Ok(Some(record)) => {
                            if let Err(e) = self.agent_def_backfill_local_from_registry(&record) {
                                tracing::warn!(
                                    definition_id = %inst.definition_id,
                                    error = %e,
                                    "instance_create: registry backfill failed"
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!(
                                definition_id = %inst.definition_id,
                                error = %e,
                                "instance_create: registry lookup failed"
                            );
                        }
                    }
                }
                match self.agent_row_get(&inst.definition_id)? {
                    Some(d) => d,
                    None => {
                        tracing::warn!(
                            definition_id = %inst.definition_id,
                            instance_id = %inst.id,
                            "instance_create: definition exists nowhere; refusing the launch"
                        );
                        return Err(StoreError::NotFound);
                    }
                }
            }
        };

        let display_name = if inst.instance_name.is_empty() { def.name.clone() } else { inst.instance_name.clone() };
        let hidden = if inst.display_hidden { 1_i64 } else { 0_i64 };
        // The agent's working directory is the launch's RESOLVED path when
        // the launch supplies one (`agent.open` resolves it, including slug
        // collision suffixing), falling back to the definition's configured
        // cwd. Phase 3a's dual-write always wrote the definition's, on the
        // reasoning that "db_agents holds durable agent config; the
        // per-launch resolved cwd lives on the block" — that split ends with
        // the instance table: one row per agent means the row holds the
        // agent's real workspace, which is also what the cross-version JSON
        // registry mirror records (and it silently skips any row whose
        // working_directory is not under the agents root).
        let working_directory = if inst.working_directory.is_empty() {
            def.working_directory.clone()
        } else {
            inst.working_directory.clone()
        };
        // The row this launch lands on. A user agent IS its row; a template
        // launch is its own row unless it continues one that already exists.
        let key = if def.is_seeded == 0 {
            def.id.clone()
        } else if !inst.parent_instance_id.is_empty() && self.instance_get(&inst.parent_instance_id)?.is_some() {
            inst.parent_instance_id.clone()
        } else {
            inst.id.clone()
        };
        {
            let conn = self.conn.lock().unwrap();
            let now_ms = Self::monotonic_updated_at(&conn, inst.created_at);
            if key == inst.id {
                // Fresh template launch — its own user-agent row, config
                // copied from the template, bindings + launch state from the
                // launch. ON CONFLICT covers an id the caller reuses on a
                // retry (the App-API stub path).
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
                        created_at, updated_at, is_seeded, user_hidden,
                        last_block_id,
                        container_image, container_volumes, container_name,
                        use_ambient_login, model_vendor_base_url, auto_continue_enabled,
                        conversation_visibility,
                        session_id, status, started_at, ended_at
                     ) VALUES (
                        ?1, ?2, ?3, ?4,
                        0, ?5,
                        ?6, ?7, ?8, ?9,
                        ?10, ?11, ?12,
                        ?13, ?14, ?15,
                        ?16, ?17,
                        ?18, ?19, ?20, ?21,
                        ?22,
                        ?23, ?24, 0, ?25,
                        ?26,
                        ?27, ?28, ?29,
                        ?30, ?31, ?32,
                        ?33,
                        ?34, ?35, ?36, ?37
                     )
                     ON CONFLICT(id) DO UPDATE SET
                        name = excluded.name,
                        identity_id = excluded.identity_id,
                        memory_id = excluded.memory_id,
                        working_directory = excluded.working_directory,
                        github_context = excluded.github_context,
                        instance_name = excluded.instance_name,
                        updated_at = excluded.updated_at,
                        user_hidden = excluded.user_hidden,
                        last_block_id = CASE WHEN excluded.last_block_id = '' THEN db_agents.last_block_id ELSE excluded.last_block_id END,
                        session_id = CASE WHEN excluded.session_id = '' THEN db_agents.session_id ELSE excluded.session_id END,
                        status = excluded.status,
                        started_at = excluded.started_at,
                        ended_at = excluded.ended_at",
                    params![
                        inst.id,
                        display_name,
                        def.icon,
                        def.description,
                        def.id,
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
                        working_directory,
                        inst.github_context,
                        inst.instance_name,
                        inst.created_at,
                        now_ms,
                        hidden,
                        inst.block_id,
                        def.container_image,
                        def.container_volumes,
                        def.container_name,
                        def.use_ambient_login,
                        def.model_vendor_base_url,
                        def.auto_continue_enabled,
                        def.conversation_visibility,
                        inst.session_id,
                        inst.status,
                        inst.started_at,
                        inst.ended_at,
                    ],
                )?;
            } else {
                // Fold: the launch (or continuation) moves the row's bindings
                // and launch state to this launch. Config stays the row's own.
                //
                // An EMPTY `session_id` never clears the row's: a fresh
                // continuation is created session-less and the CLI only
                // reports the real session id later, so overwriting here
                // would drop the pointer `--resume` needs (the same reason
                // `last_block_id` is guarded). A deliberate clear goes
                // through `instance_update_partial(Some(""))`, which does
                // write it — see `poison_resume`.
                conn.execute(
                    "UPDATE db_agents SET
                        name = ?2,
                        identity_id = ?3,
                        memory_id = ?4,
                        working_directory = ?14,
                        github_context = ?5,
                        instance_name = ?6,
                        updated_at = ?7,
                        user_hidden = ?8,
                        last_block_id = CASE WHEN ?9 = '' THEN last_block_id ELSE ?9 END,
                        session_id = CASE WHEN ?10 = '' THEN session_id ELSE ?10 END,
                        status = ?11,
                        started_at = ?12,
                        ended_at = ?13
                     WHERE id = ?1 AND is_template = 0",
                    params![
                        key,
                        display_name,
                        inst.identity_id,
                        inst.memory_id,
                        inst.github_context,
                        inst.instance_name,
                        now_ms,
                        hidden,
                        inst.block_id,
                        inst.session_id,
                        inst.status,
                        inst.started_at,
                        inst.ended_at,
                        working_directory,
                    ],
                )?;
            }
        }
        let canonical = self.instance_get(&key)?.ok_or(StoreError::NotFound)?;
        self.registry_upsert_if_named(&canonical);
        // A freshly created launch's session_id is never a genuine capture
        // in production (continuations start with ""), so only a non-empty
        // one is worth propagating to the registry.
        if !inst.session_id.is_empty() {
            self.registry_propagate_continuation_session_id(&canonical);
        }
        Ok(canonical)
    }

    /// An `updated_at` strictly greater than any already on `db_agents`, so
    /// recency ordering survives fast successive writes within one
    /// millisecond (reagent P2 round 3 on #1013 — the rule the dual-write
    /// used; inherited here).
    fn monotonic_updated_at(conn: &rusqlite::Connection, floor: i64) -> i64 {
        let wall_now: i64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(floor);
        let global_prior: i64 = conn
            .query_row("SELECT COALESCE(MAX(updated_at), 0) FROM db_agents", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0);
        std::cmp::max(wall_now, global_prior.saturating_add(1))
    }

    /// Set the hidden flag on an agent row. Used by the "Forget agent"
    /// affordance — soft-delete only; the row + working directory remain on
    /// disk for audit + recovery.
    ///
    /// Cross-version case: an agent migrated into the registry from another
    /// version's SQLite won't have a row in the current version's SQLite.
    /// The UPDATE returns 0 rows, but the registry still needs to flip —
    /// otherwise "Forget agent" silently no-ops for those. Returns `true`
    /// when either side acted.
    pub fn instance_set_hidden(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE db_agents SET user_hidden = ?1 WHERE id = ?2 AND is_template = 0",
                params![if hidden { 1_i64 } else { 0_i64 }, id],
            )?
        };
        let mut registry_acted = false;
        if let Some(reg) = self.registry() {
            // Only act on the registry side if a record exists there
            // (in either active or retired). Avoids logging spurious
            // "failed to retire" warnings on a no-op for unrelated ids.
            if reg.exists_anywhere(id) {
                let res = if hidden { reg.retire(id) } else { reg.unretire(id) };
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

    /// Named agents for the launch modal's "Continue agent" dropdown and the
    /// picker's "My Agents" surface: non-hidden rows with an `instance_name`,
    /// newest launch first, capped by `limit`.
    ///
    /// `definition_id` restricts the result to that agent itself OR every
    /// agent derived from it (`parent_template_id`) — the launch modal opens
    /// per template, and what a user wants to continue there is any agent
    /// they launched from it. `identity_id` filters on the row's identity
    /// binding.
    ///
    /// `include_continuations` is accepted for API compatibility but no
    /// longer changes the query: chains are pre-collapsed (one row per
    /// agent carrying its latest launch), so the legacy "heads only" and
    /// "latest per chain" modes both yield exactly this list.
    pub fn instance_list_named(
        &self,
        limit: usize,
        definition_id: Option<&str>,
        identity_id: Option<&str>,
        _include_continuations: bool,
    ) -> Result<Vec<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut sql = format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE is_template = 0 AND user_hidden = 0 AND instance_name <> ''"
        );
        let mut param_vals: Vec<String> = Vec::new();
        if let Some(id) = identity_id {
            sql.push_str(&format!(" AND identity_id = ?{}", param_vals.len() + 1));
            param_vals.push(id.to_string());
        }
        if let Some(def) = definition_id {
            let n = param_vals.len() + 1;
            sql.push_str(&format!(" AND (id = ?{n} OR parent_template_id = ?{n})"));
            param_vals.push(def.to_string());
        }
        sql.push_str(&format!(
            " ORDER BY started_at DESC, updated_at DESC, id DESC LIMIT ?{}",
            param_vals.len() + 1
        ));
        param_vals.push(limit.to_string());
        let mut stmt = conn.prepare(&sql)?;
        let iter = stmt.query_map(rusqlite::params_from_iter(param_vals.iter()), map_instance_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Resolve an agent by its persisted `slug` — the App-API self-lookup
    /// (`AGENTMUX_AGENT_ID` is the slug). Matches ONLY `slug`; deliberately
    /// single-purpose after reagentx P1 on PR #2428 (round 2): a lookup
    /// that also matched another column let a coincidental cross-namespace
    /// collision return an unrelated agent's memory/identity/bundle. Hidden
    /// rows are excluded.
    pub fn instance_get_by_slug(&self, slug: &str) -> Result<Option<AgentInstance>, StoreError> {
        if slug.is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE slug = ?1 AND is_template = 0 AND user_hidden = 0
             ORDER BY updated_at DESC, created_at DESC
             LIMIT 1"
        ))?;
        match stmt.query_row(params![slug], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Partial update of an agent's launch state. Only `Some` fields are
    /// written; `None` leaves that column untouched. `Some("")` explicitly
    /// clears (the `updateagentinstance` command contract, and
    /// `poison_resume`'s deliberate stale-session clear).
    ///
    /// Returns the post-update row. `None` is reserved for **not-found**;
    /// an all-`None` update on an existing id is a no-op that returns the
    /// unchanged row, so callers can tell "nothing to change" from "no such
    /// agent". See SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29.md.
    pub fn instance_update_partial(
        &self,
        id: &str,
        upd: &InstanceUpdate,
    ) -> Result<Option<AgentInstance>, StoreError> {
        use rusqlite::types::ToSql;

        let mut sets: Vec<&str> = Vec::new();
        let mut vals: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(v) = &upd.block_id {
            sets.push("last_block_id = ?");
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
            sets.push("github_context = ?");
            vals.push(Box::new(v.clone()));
        }
        if let Some(v) = upd.ended_at {
            sets.push("ended_at = ?");
            vals.push(Box::new(v));
        }
        if sets.is_empty() {
            return self.instance_get(id);
        }

        let rows = {
            let conn = self.conn.lock().unwrap();
            let now_ms = Self::monotonic_updated_at(&conn, 0);
            sets.push("updated_at = ?");
            vals.push(Box::new(now_ms));
            let sql = format!("UPDATE db_agents SET {} WHERE id = ? AND is_template = 0", sets.join(", "));
            vals.push(Box::new(id.to_string()));
            let params: Vec<&dyn ToSql> = vals.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, params.as_slice())?
        };
        if rows == 0 {
            return Ok(None);
        }
        let fresh = self.instance_get(id)?;
        if let Some(f) = &fresh {
            self.registry_upsert_if_named(f);
            // Only propagate when THIS call actually targeted session_id —
            // `Some("")` (a deliberate clear) and `Some(real_sid)` both mean
            // this call wrote it; `None` means the row's existing value is
            // not this call's to broadcast. See
            // registry_propagate_continuation_session_id's doc comment.
            if upd.session_id.is_some() {
                self.registry_propagate_continuation_session_id(f);
            }
        }
        Ok(fresh)
    }

    /// Full-struct update of the mutable launch-state fields (`block_id`,
    /// `session_id`, `status`, `github_context`, `ended_at`). Retained as a
    /// convenience for tests + internal callers; the `updateagentinstance`
    /// handler uses [`Self::instance_update_partial`].
    pub fn instance_update(&self, inst: &AgentInstance) -> Result<bool, StoreError> {
        let upd = InstanceUpdate {
            block_id: Some(inst.block_id.clone()),
            session_id: Some(inst.session_id.clone()),
            status: Some(inst.status.clone()),
            github_context: Some(inst.github_context.clone()),
            ended_at: Some(inst.ended_at),
        };
        Ok(self.instance_update_partial(&inst.id, &upd)?.is_some())
    }

    /// Repoint every agent derived from `old_def_id` at `new_def_id`. Used by
    /// the Phase 1 two-tier-picker migration
    /// (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md): when a seeded template has
    /// been used directly, the migration clones it into a user agent and
    /// repoints its launches so the existing reattach flow keeps working.
    /// Returns the number of rows updated.
    pub fn instance_repoint_definition(&self, old_def_id: &str, new_def_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute(
            // `id != ?1` keeps the promote target out of its own result set:
            // the clone is itself derived from the template, so an unguarded
            // update would re-parent it to itself and count as a repoint.
            "UPDATE db_agents SET parent_template_id = ?1
             WHERE parent_template_id = ?2 AND is_template = 0 AND id != ?1",
            params![new_def_id, old_def_id],
        )?)
    }

    /// Delete an agent row (never a template). The row IS the agent in the
    /// consolidated model, so this removes the agent — previously a launch
    /// row of a user-clone definition could be deleted while the definition
    /// stayed; there is no such split any more.
    pub fn instance_delete(&self, id: &str) -> Result<bool, StoreError> {
        let rows = {
            let conn = self.conn.lock().unwrap();
            let rows =
                conn.execute("DELETE FROM db_agents WHERE id = ?1 AND is_template = 0", params![id])?;
            if rows > 0 {
                // A user agent still has a same-id `db_agent_definitions` row
                // (the definition side is dual-written until PR 3). Leaving it
                // behind lets the startup gap repair — which recreates a
                // `db_agents` row for any definition missing one — resurrect
                // the agent on the next boot, with its launch state reset
                // (codex P2 on #3080). `agent_def_delete` already clears both
                // sides; this is the same deletion arriving from the other
                // direction. A template-launch agent has no definition row of
                // its own, so this is a no-op for one.
                conn.execute(
                    "DELETE FROM db_agent_definitions WHERE id = ?1 AND is_seeded = 0",
                    params![id],
                )?;
            }
            rows
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
        }
        Ok(rows > 0)
    }

    /// Resolve the agent bindings tied to a block.
    ///
    /// Resolves through `block.meta.agentId` (or legacy `agent:id`) first —
    /// the block says which agent it shows — returning that agent's row
    /// with `block_id` set to the asked-for block. Falls back to the agent
    /// whose latest launch is on this block and still active, which is what
    /// a block from before the block meta carried `agentId` needs.
    ///
    /// We deliberately do NOT consult `block.meta.agentInstanceId`: codex
    /// P1 on PR #1114 round 3 surfaced that pane reuse can leave a stale
    /// one behind.
    pub fn instance_get_active_for_block(&self, block_id: &str) -> Result<Option<AgentInstance>, StoreError> {
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
        if !agent_id.is_empty() {
            // Any non-template row, hidden or not: an existing pane of a
            // "forgotten" agent must still resolve its bindings.
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(&format!(
                "SELECT {INSTANCE_COLUMNS} FROM db_agents WHERE id = ?1 AND is_template = 0"
            ))?;
            match stmt.query_row(params![agent_id], map_instance_row) {
                Ok(mut a) => {
                    a.block_id = block_id.to_string();
                    return Ok(Some(a));
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE last_block_id = ?1 AND is_template = 0 AND status IN ('running', 'paused')
             ORDER BY updated_at DESC
             LIMIT 1"
        ))?;
        match stmt.query_row(params![block_id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The agent whose latest launch is on `block_id`, regardless of status
    /// — with its real `session_id`, so callers that write back to that
    /// exact row (keeping `session_id` live as the CLI emits it — see
    /// `persist_session_id` /
    /// SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.1) have
    /// the right id to pass to `instance_update_partial`. Most recently
    /// updated row wins if a block was somehow reused.
    pub fn instance_get_by_block_id(&self, block_id: &str) -> Result<Option<AgentInstance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {INSTANCE_COLUMNS} FROM db_agents
             WHERE last_block_id = ?1 AND is_template = 0
             ORDER BY updated_at DESC
             LIMIT 1"
        ))?;
        match stmt.query_row(params![block_id], map_instance_row) {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// The `db_agents` columns every instance read selects, in the order
/// [`map_instance_row`] expects.
const INSTANCE_COLUMNS: &str = "id, last_block_id, session_id, status, github_context, started_at, ended_at, \
     created_at, identity_id, memory_id, instance_name, working_directory, user_hidden";

/// Project a non-template `db_agents` row (selected with
/// [`INSTANCE_COLUMNS`]) into the `AgentInstance` shape: the row's id is
/// both `id` and `definition_id`, `last_block_id` is `block_id`, chains are
/// pre-collapsed so `parent_instance_id` is empty, and a row that never
/// recorded a launch reports `created_at` as `started_at`.
fn map_instance_row(row: &rusqlite::Row) -> rusqlite::Result<AgentInstance> {
    let id: String = row.get(0)?;
    let started_at: i64 = row.get(5)?;
    let created_at: i64 = row.get(7)?;
    Ok(AgentInstance {
        definition_id: id.clone(),
        id,
        parent_instance_id: String::new(),
        block_id: row.get(1)?,
        session_id: row.get(2)?,
        status: row.get(3)?,
        github_context: row.get(4)?,
        started_at: if started_at == 0 { created_at } else { started_at },
        ended_at: row.get(6)?,
        created_at,
        identity_id: row.get(8)?,
        memory_id: row.get(9)?,
        instance_name: row.get(10)?,
        working_directory: row.get(11)?,
        display_hidden: row.get::<_, i64>(12)? != 0,
    })
}

/// The `db_agents` SELECT that reads a row in the `AgentDefinition` shape —
/// the column order [`map_agent_definition_row`] expects. Shared by
/// `agent_def_list` (all rows) and `agent_row_get` (one row).
const AGENT_DEFINITION_SELECT: &str = "SELECT id, slug, name, icon, provider, description,
            working_directory, shell, provider_flags, auto_start,
            restart_on_crash, idle_timeout_minutes, created_at,
            agent_type, environment, agent_bus_id, is_seeded,
            accounts, parent_template_id, branch_label, updated_at,
            user_hidden, container_image, container_volumes, container_name,
            use_ambient_login, model_vendor_base_url, auto_continue_enabled,
            default_memory_id, conversation_visibility
     FROM db_agents";

/// Row mapper for `db_agents` rows projected back into the
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
        container_image: row.get(22)?,
        container_volumes: row.get(23)?,
        container_name: row.get(24)?,
        use_ambient_login: row.get(25)?,
        model_vendor_base_url: row.get(26)?,
        auto_continue_enabled: row.get(27)?,
        memory_id: row.get(28)?,
        conversation_visibility: row.get(29)?,
    })
}

/// Shared test fixture: an `AgentDefinition` with every field defaulted to
/// an empty/zero value except the ones callers actually vary. Both
/// `tests::bare_agent_def` and `bundle_provisioning_store_separation_tests
/// ::base_agent` used to hardcode this 30-field struct literal
/// independently (reagentx P2 on PR #2602) — kept as one definition here so
/// a future field addition only needs updating once.
#[cfg(test)]
pub(crate) fn test_agent_def(
    id: &str,
    name: &str,
    provider: &str,
    agent_type: &str,
    timestamp: i64,
    model_vendor_base_url: &str,
) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        slug: id.to_string(),
        name: name.to_string(),
        icon: String::new(),
        provider: provider.to_string(),
        description: String::new(),
        working_directory: String::new(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: timestamp,
        agent_type: agent_type.to_string(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: timestamp,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: "[]".to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: model_vendor_base_url.to_string(),
        auto_continue_enabled: 0,
        memory_id: String::new(),
        conversation_visibility: default_conversation_visibility(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        DefContentBlob, DefSkillBlob, DefinitionRecord, DefinitionRecordV1, DefinitionStore,
        DEF_MAX_SUPPORTED_SCHEMA,
    };
    use std::sync::Arc;

    /// Mirrors `def_registry_mirror.rs`'s own test fixture — a user
    /// agent that exists only in the global cross-channel registry,
    /// with one content blob and one skill, so a backfill has
    /// something real to preserve.
    fn global_user_agent(id: &str, name: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: DEF_MAX_SUPPORTED_SCHEMA,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                name: name.to_string(),
                provider: "claude".to_string(),
                is_seeded: 0,
                updated_at: 42,
                content: vec![DefContentBlob {
                    content_type: "agentmd".to_string(),
                    content: "be helpful".to_string(),
                }],
                skills: vec![DefSkillBlob {
                    id: "sk1".to_string(),
                    name: "greet".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    }

    fn instance(id: &str, definition_id: &str) -> AgentInstance {
        AgentInstance {
            id: id.to_string(),
            definition_id: definition_id.to_string(),
            parent_instance_id: String::new(),
            block_id: String::new(),
            session_id: String::new(),
            status: "init".to_string(),
            github_context: String::new(),
            started_at: 1,
            ended_at: 0,
            created_at: 1,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: id.to_string(),
            working_directory: String::new(),
            display_hidden: false,
        }
    }

    #[test]
    fn instance_create_backfills_local_definition_from_registry_only_record() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        def_store
            .upsert(&global_user_agent("remote-1", "Remote"))
            .unwrap();
        store.set_def_registry(def_store.clone());

        // Local db_agent_definitions has no row for "remote-1" — this
        // is exactly the Moras/Oozp/Parko scenario. Without the
        // backfill this INSERT fails with a FOREIGN KEY error.
        let result = store.instance_create(&instance("inst-1", "remote-1"));
        assert!(result.is_ok(), "instance_create failed: {result:?}");

        // The backfilled local row must exist and satisfy the FK.
        let local = store.agent_def_get_local_only("remote-1").unwrap();
        assert!(local.is_some(), "backfill must create a local definition row");

        // Critical regression guard: the registry's real content/skills
        // must NOT have been wiped by the backfill (this is exactly
        // what a naive `agent_def_insert`-based backfill would do —
        // see agent_def_backfill_local_from_registry's doc comment).
        let rec = def_store.get("remote-1").unwrap().unwrap();
        assert_eq!(rec.data.content.len(), 1, "registry content must survive backfill");
        assert_eq!(rec.data.skills.len(), 1, "registry skills must survive backfill");
    }

    #[test]
    fn instance_create_still_errors_when_definition_missing_everywhere() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        store.set_def_registry(def_store);

        // No local row, nothing in the registry either — genuinely
        // orphaned definition_id. Must still fail loudly, not silently
        // swallow a real error.
        let result = store.instance_create(&instance("inst-2", "nowhere"));
        assert!(result.is_err(), "instance_create must still fail for a truly orphaned definition_id");
    }

    // SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md follow-up: App-API
    // self-lookup callers (MemoryList/Read/Write, IdentityAccounts,
    // IdentityValidate, bundle.self.get) pass the AGENTMUX_AGENT_ID routing
    // slug, not the literal display name. Confirmed live: a real agent
    // named "AgentY" (slug "agenty") got "agent agenty not found" from
    // every one of those endpoints before `instance_get_by_slug` existed.
    #[test]
    fn instance_get_by_slug_resolves_a_mixed_case_display_name_via_its_slug() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        def_store.upsert(&global_user_agent("def-agenty", "AgentY")).unwrap();
        store.set_def_registry(def_store);

        let mut inst = instance("inst-agenty", "def-agenty");
        inst.instance_name = "AgentY".to_string();
        store.instance_create(&inst).unwrap();

        // The routing slug ("agenty", what every MCP-tool-backed App-API
        // endpoint actually has) must resolve to the display-cased row.
        let found = store.instance_get_by_slug("agenty").unwrap();
        assert!(found.is_some(), "must resolve via the persisted slug column");
        assert_eq!(found.unwrap().instance_name, "AgentY");
    }

    #[test]
    fn instance_get_by_slug_returns_none_for_a_genuinely_unrelated_slug() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        def_store.upsert(&global_user_agent("def-agenty2", "AgentY")).unwrap();
        store.set_def_registry(def_store);

        let mut inst = instance("inst-agenty2", "def-agenty2");
        inst.instance_name = "AgentY".to_string();
        store.instance_create(&inst).unwrap();

        let found = store.instance_get_by_slug("someone-else").unwrap();
        assert!(found.is_none(), "an unrelated slug must not match by coincidence");
    }

    // reagentx P1 on PR #2428 (round 2): a slug-normalized re-derivation of
    // the CURRENT display name (`derive_slug(instance_name)`) only equals
    // the real routing slug when the display name has never changed and
    // never collided with another agent's at creation. `slug` is stable
    // and persisted once, `name`/`instance_name` are renameable — this
    // test deliberately sets them to unrelated values (as if the agent
    // were renamed, or its slug got a "-2" collision suffix at creation)
    // to prove the fix resolves via the real persisted `slug` column, not
    // a fresh re-derivation that a rename would silently invalidate.
    #[test]
    fn instance_get_by_slug_resolves_via_the_persisted_slug_even_after_a_rename() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        // The registry's own `name` is the CURRENT (post-rename) display
        // name — deliberately unrelated to `slug`, which was fixed at
        // creation and never changes.
        def_store
            .upsert(&DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: DefinitionRecordV1 {
                    id: "def-renamed".to_string(),
                    slug: "agenty".to_string(),
                    name: "Agent Y Renamed".to_string(),
                    provider: "claude".to_string(),
                    updated_at: 42,
                    ..Default::default()
                },
            })
            .unwrap();
        store.set_def_registry(def_store);

        let mut inst = instance("inst-renamed", "def-renamed");
        inst.instance_name = "Agent Y Renamed".to_string();
        store.instance_create(&inst).unwrap();

        // derive_slug("Agent Y Renamed") would be "agent-y-renamed" — NOT
        // "agenty". Only a lookup against the real persisted slug column
        // resolves this; a re-derivation from the current name would miss
        // it, reproducing the exact bug this test guards against.
        let found = store.instance_get_by_slug("agenty").unwrap();
        assert!(found.is_some(), "must resolve via the persisted slug, not a re-derivation of the current name");
        assert_eq!(found.unwrap().instance_name, "Agent Y Renamed");
    }

    // reagentx P1 on PR #2428 (round 3): a single query matching
    // `(instance_name = ?1 OR slug = ?1)` let a coincidental cross-
    // namespace collision (one agent's literal `instance_name` equaling a
    // DIFFERENT agent's `slug`) match both rows simultaneously, silently
    // disambiguated by recency — meaning an App-API self-lookup call could
    // return an unrelated agent's memory/identity/bundle. This test builds
    // exactly that collision (agent A's `instance_name` == agent B's
    // `slug` == "shared-name") and proves each single-purpose function
    // stays within its own namespace: `instance_get_by_name` finds ONLY
    // the literal-name match (A), `instance_get_by_slug` finds ONLY the
    // slug match (B) — never each other's row.
    #[test]
    fn instance_get_by_name_and_by_slug_never_cross_the_others_namespace() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());

        // Agent A: literal display name is exactly "shared-name"; its own
        // slug is unrelated ("agent-a-slug").
        def_store
            .upsert(&DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: DefinitionRecordV1 {
                    id: "def-a".to_string(),
                    slug: "agent-a-slug".to_string(),
                    name: "shared-name".to_string(),
                    provider: "claude".to_string(),
                    updated_at: 1,
                    ..Default::default()
                },
            })
            .unwrap();
        // Agent B: slug is exactly "shared-name" (e.g. a rename left its
        // slug pointing at a string that collides with A's literal name);
        // its own display name is unrelated.
        def_store
            .upsert(&DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: DefinitionRecordV1 {
                    id: "def-b".to_string(),
                    slug: "shared-name".to_string(),
                    name: "Agent B Display".to_string(),
                    provider: "claude".to_string(),
                    updated_at: 2,
                    ..Default::default()
                },
            })
            .unwrap();
        store.set_def_registry(def_store);

        let mut inst_a = instance("inst-a", "def-a");
        inst_a.instance_name = "shared-name".to_string();
        store.instance_create(&inst_a).unwrap();

        let mut inst_b = instance("inst-b", "def-b");
        inst_b.instance_name = "Agent B Display".to_string();
        store.instance_create(&inst_b).unwrap();

        // `instance_get_by_name` was deleted in consolidation Phase 3b (PR 2)
        // — it had no production caller — so the slug side is the whole
        // guarantee now: the slug lookup must never resolve to A's row just
        // because A's literal instance_name equals B's slug.
        let by_slug = store.instance_get_by_slug("shared-name").unwrap()
            .expect("must find agent B by slug");
        assert_eq!(by_slug.id, "def-b", "slug lookup must never resolve to A's row");
    }

    #[test]
    fn instance_create_preserves_registry_updated_at_on_backfill() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        def_store
            .upsert(&global_user_agent("remote-2", "Remote2"))
            .unwrap();
        store.set_def_registry(def_store);

        store
            .instance_create(&instance("inst-3", "remote-2"))
            .unwrap();

        let local = store.agent_def_get_local_only("remote-2").unwrap().unwrap();
        assert_eq!(
            local.updated_at, 42,
            "backfilled row must preserve the registry's real updated_at, not reset to created_at"
        );
    }

    fn bare_agent_def(id: &str, name: &str) -> AgentDefinition {
        super::test_agent_def(id, name, "claude", "standalone", 1, "")
    }

    /// Pins down the module doc's claimed read-side split (this file's
    /// header comment, plus `OBJECT_SCHEMA_VERSION`'s v4 entry and
    /// `dual_write.rs`'s module doc) as an executable check, per
    /// `SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
    /// P4: migration-phase state must be verifiable, not just a comment
    /// someone has to remember to update. Builds two rows that exist in
    /// only ONE of the two tables each (bypassing the normal
    /// `agent_def_insert` path, which always keeps both in sync) and
    /// proves `agent_def_list()` and `agent_def_get()` disagree about
    /// which table is authoritative, on purpose. If this test starts
    /// failing, one of the two functions' read target changed — go
    /// update the three comments this test is guarding, not just this
    /// assertion.
    #[test]
    fn agent_def_list_and_agent_def_get_read_different_tables_by_design() {
        let store = Store::open_in_memory().unwrap();

        // Exists ONLY in the consolidated `db_agents` table (as if some
        // write path populated it without touching the legacy table —
        // agent_def_list()'s actual read target).
        store
            .agents_dual_write_definition_upsert(&bare_agent_def("only-in-db-agents", "OnlyConsolidated"))
            .unwrap();

        // Exists ONLY in the legacy `db_agent_definitions` table — a raw
        // insert bypassing `agent_def_insert`'s automatic dual-write,
        // standing in for agent_def_get()'s actual read target.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO db_agent_definitions (id, slug, name, icon, provider, description,
                 working_directory, shell, provider_flags, auto_start, restart_on_crash,
                 idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id,
                 is_seeded, accounts, parent_id, branch_label, updated_at, user_hidden,
                 container_image, container_volumes, container_name, use_ambient_login,
                 model_vendor_base_url, auto_continue_enabled, memory_id, conversation_visibility)
                 VALUES ('only-in-legacy', 'only-in-legacy', 'OnlyLegacy', '', 'claude', '',
                 '', '', '', 0, 0, 0, 1, 'standalone', '', '', 0, '', '', '', 1, 0, '', '[]', '',
                 0, '', 0, '', 'private')",
                [],
            )
            .unwrap();
        }

        let listed = store.agent_def_list().unwrap();
        assert!(
            listed.iter().any(|d| d.id == "only-in-db-agents"),
            "agent_def_list() must read db_agents"
        );
        assert!(
            !listed.iter().any(|d| d.id == "only-in-legacy"),
            "agent_def_list() must NOT see a db_agent_definitions-only row \
             (documents the current partial-flip state — fails if this ever \
             also starts overlaying the legacy table)"
        );

        assert!(
            store.agent_def_get("only-in-legacy").unwrap().is_some(),
            "agent_def_get() must still read db_agent_definitions directly"
        );
        assert!(
            store.agent_def_get("only-in-db-agents").unwrap().is_none(),
            "agent_def_get() must NOT see a db_agents-only row — it \
             deliberately avoids the consolidated table's template-instance \
             projection rows (see its own doc comment)"
        );
    }
}

// P1/P2 fixes (Codex + ReAgent review on PR #2587):
// - bundle_provision_for_new_agent/agent_def_provision_and_bind_bundle must
//   write the bundle into an explicitly-passed store, never assume the
//   caller's own definition store IS the bundle store — the two are
//   different databases whenever a shared store is configured.
// - resolve_effective_vendor must respect model_vendor_base_url ("custom"),
//   not just the provider's bare default.
#[cfg(test)]
mod bundle_provisioning_store_separation_tests {
    use super::*;

    fn base_agent(id: &str, name: &str, provider: &str, model_vendor_base_url: &str) -> AgentDefinition {
        super::test_agent_def(id, name, provider, "host", 0, model_vendor_base_url)
    }

    #[test]
    fn resolve_effective_vendor_defaults_to_the_providers_first_supported_vendor() {
        assert_eq!(Store::resolve_effective_vendor("claude", ""), "anthropic");
        assert_eq!(Store::resolve_effective_vendor("codex", ""), "openai");
    }

    #[test]
    fn resolve_effective_vendor_returns_custom_when_a_base_url_override_is_set() {
        assert_eq!(
            Store::resolve_effective_vendor("claude", "https://my-proxy.example.com"),
            "custom",
            "an agent with a vendor override must not claim the provider's default vendor"
        );
    }

    #[test]
    fn resolve_effective_vendor_ignores_a_whitespace_only_override() {
        assert_eq!(Store::resolve_effective_vendor("claude", "   "), "anthropic");
    }

    #[test]
    fn resolve_effective_vendor_falls_back_to_the_provider_id_for_an_unknown_provider() {
        assert_eq!(Store::resolve_effective_vendor("not-a-real-provider", ""), "not-a-real-provider");
    }

    #[test]
    fn bundle_provision_for_new_agent_writes_into_whichever_store_its_called_on() {
        let bundle_store = Store::open_in_memory().unwrap();
        let agent = base_agent("a1", "Agent One", "claude", "");
        let bundle_id = bundle_store.bundle_provision_for_new_agent(&agent, 0).unwrap();
        assert!(bundle_store.bundle_memory_get(&bundle_id).unwrap().is_some());
    }

    #[test]
    fn bundle_provision_for_new_agent_respects_a_custom_vendor_override() {
        let bundle_store = Store::open_in_memory().unwrap();
        let agent = base_agent("a1", "Agent One", "claude", "https://my-proxy.example.com");
        let bundle_id = bundle_store.bundle_provision_for_new_agent(&agent, 0).unwrap();
        let bundle = bundle_store.bundle_memory_get(&bundle_id).unwrap().unwrap();
        assert_eq!(bundle.provider, "claude");
        assert_eq!(bundle.model, "custom");
    }

    // The core P1 regression test: definition_store (self) and bundle_store
    // are two GENUINELY SEPARATE Store instances — proves
    // agent_def_provision_and_bind_bundle writes the bundle into
    // bundle_store specifically, never into whichever store the method
    // happens to be called on for the definition side.
    #[test]
    fn agent_def_provision_and_bind_bundle_writes_the_bundle_into_the_explicit_bundle_store_not_self() {
        let definition_store = Store::open_in_memory().unwrap();
        let bundle_store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "claude", "");
        definition_store.agent_def_insert(&mut agent).unwrap();

        definition_store.agent_def_provision_and_bind_bundle(&bundle_store, &mut agent, 0);

        assert!(!agent.memory_id.is_empty(), "caller's struct must reflect the bound bundle id");
        let def = definition_store.agent_def_get(&agent.id).unwrap().unwrap();
        assert_eq!(def.memory_id, agent.memory_id, "binding must land in the definition store (self)");

        // The bundle itself must be absent from definition_store and
        // present only in bundle_store.
        assert!(
            definition_store.bundle_memory_get(&agent.memory_id).unwrap().is_none(),
            "bundle must NOT be written into the definition store"
        );
        assert!(
            bundle_store.bundle_memory_get(&agent.memory_id).unwrap().is_some(),
            "bundle must be reachable via the explicit bundle_store"
        );
    }

    // P1 regression tests (ReAgent review on PR #2587 round 4):
    // db_bundles.name is UNIQUE but only AgentDefinition.slug is
    // guaranteed unique, not the display name two agents can share — a
    // naive "{name} — ABF" used to collide, silently leaving a
    // runtime-provisioned agent unbound and unconditionally aborting
    // m0021's entire backfill loop on the first collision.

    #[test]
    fn resolve_unique_bundle_name_returns_the_base_name_when_no_collision() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.resolve_unique_bundle_name("Agent One — ABF").unwrap(), "Agent One — ABF");
    }

    #[test]
    fn resolve_unique_bundle_name_disambiguates_on_collision() {
        let store = Store::open_in_memory().unwrap();
        let agent_a = base_agent("a1", "Same Name", "claude", "");
        store.bundle_provision_for_new_agent(&agent_a, 0).unwrap();

        let unique = store.resolve_unique_bundle_name("Same Name — ABF").unwrap();
        assert_eq!(unique, "Same Name — ABF (2)");
    }

    #[test]
    fn resolve_unique_bundle_name_walks_past_multiple_collisions() {
        let store = Store::open_in_memory().unwrap();
        store.bundle_provision_for_new_agent(&base_agent("a1", "Dup", "claude", ""), 0).unwrap();
        // Directly seed the "(2)" slot too, so the resolver must walk to "(3)".
        let taken = super::super::memory_bundles::Memory {
            id: "taken-2".to_string(),
            name: "Dup — ABF (2)".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        };
        store.bundle_memory_upsert(&taken).unwrap();

        let unique = store.resolve_unique_bundle_name("Dup — ABF").unwrap();
        assert_eq!(unique, "Dup — ABF (3)");
    }

    #[test]
    fn two_agents_with_the_same_display_name_both_get_bound_not_left_unbound_on_collision() {
        let definition_store = Store::open_in_memory().unwrap();
        let bundle_store = Store::open_in_memory().unwrap();
        let mut agent_a = base_agent("a1", "Twin", "claude", "");
        let mut agent_b = base_agent("a2", "Twin", "claude", "");
        definition_store.agent_def_insert(&mut agent_a).unwrap();
        definition_store.agent_def_insert(&mut agent_b).unwrap();

        definition_store.agent_def_provision_and_bind_bundle(&bundle_store, &mut agent_a, 0);
        definition_store.agent_def_provision_and_bind_bundle(&bundle_store, &mut agent_b, 0);

        assert!(!agent_a.memory_id.is_empty(), "first same-named agent must still get bound");
        assert!(!agent_b.memory_id.is_empty(), "second same-named agent must NOT be silently left unbound");
        assert_ne!(agent_a.memory_id, agent_b.memory_id, "each agent still gets its own distinct bundle");
    }

    // resolve_effective_provider_id — consolidated shared implementation
    // (was previously duplicated between agent_open.rs's spawn path and
    // identity/resolver/inject.rs's layer-3 credential gate; the latter
    // was found reading agent.provider directly during a follow-up
    // scoping pass, the same class of bug agent_open.rs already had
    // fixed for itself). These mirror agent_open.rs's own former
    // `resolve_effective_provider_id_tests` module one-for-one, plus the
    // store-separation case matching this file's other bundle-resolution
    // tests.

    #[test]
    fn resolve_effective_provider_id_prefers_the_bundles_provider_over_a_drifted_agent_column() {
        let store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "codex", "");
        store.agent_def_insert(&mut agent).unwrap();
        let bundle_id = store.bundle_provision_for_new_agent(&base_agent("a1", "Agent One", "claude", ""), 0).unwrap();
        store.agent_def_set_memory_id_if_empty(&agent.id, &bundle_id).unwrap();
        agent.memory_id = bundle_id;

        // Simulates the exact drift this exists to correct: agent.define's
        // if_exists=update path changed agent.provider (still "codex" on
        // this in-memory struct) after creation, but the bundle's own
        // copy ("claude") is backend-enforced immutable.
        assert_eq!(store.resolve_effective_provider_id(&agent), "claude");
    }

    #[test]
    fn resolve_effective_provider_id_falls_back_to_agent_provider_when_unbound() {
        let store = Store::open_in_memory().unwrap();
        let agent = base_agent("a1", "Agent One", "claude", "");
        assert_eq!(store.resolve_effective_provider_id(&agent), "claude");
    }

    #[test]
    fn resolve_effective_provider_id_falls_back_when_bundle_row_is_missing() {
        let store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "claude", "");
        agent.memory_id = "no-such-bundle".to_string();
        assert_eq!(store.resolve_effective_provider_id(&agent), "claude");
    }

    // ReAgent review on PR #2592 round 3: dropped during the consolidation
    // from agent_open.rs's former resolve_effective_provider_id_tests
    // module despite the commit claiming a one-for-one move — restoring
    // it here. The shared "blank" singleton and pre-§7 bundles have an
    // empty `provider` column; must not shadow a real value with "".
    #[test]
    fn resolve_effective_provider_id_falls_back_when_bundle_provider_is_empty() {
        let store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "claude", "");
        // provider left empty ("") deliberately, unlike the other tests'
        // bundle_provision_for_new_agent-built bundles.
        let bundle = super::super::memory_bundles::Memory {
            id: "bundle-empty-provider".to_string(),
            name: "Empty Provider Bundle".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        };
        store.bundle_memory_upsert(&bundle).unwrap();
        agent.memory_id = "bundle-empty-provider".to_string();
        assert_eq!(store.resolve_effective_provider_id(&agent), "claude");
    }

    // The core store-separation regression case: the bundle exists in ONE
    // store but the method is called on a DIFFERENT one — proves the
    // lookup only succeeds via the store it's actually called on, so a
    // caller that mistakenly passes wstore instead of id_store gets the
    // safe fallback rather than a silent, unexplained "wrong" answer.
    #[test]
    fn resolve_effective_provider_id_falls_back_when_called_on_the_wrong_store() {
        let id_store = Store::open_in_memory().unwrap();
        let wstore = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "claude", "");
        let bundle_id = id_store.bundle_provision_for_new_agent(&base_agent("a1", "Agent One", "codex", ""), 0).unwrap();
        agent.memory_id = bundle_id;

        assert_eq!(id_store.resolve_effective_provider_id(&agent), "codex", "must find it via the store it was actually provisioned into");
        assert_eq!(wstore.resolve_effective_provider_id(&agent), "claude", "an unrelated store must fall back to agent.provider, not silently succeed");
    }
}

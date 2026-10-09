// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent subsystem — definitions, instances, and their lifecycle CRUD.
//!
//! Covers `agent_def_*` methods (template + user-clone definitions),
//! `instance_*` methods (per-launch instance rows, named-agent
//! continuation, identity-bound active-for-block resolution), the
//! `AgentDefinition` / `AgentInstance` structs, and the `InstanceStatus` enum.
//!
//! This file holds the types and defaults; the methods are in `definition`
//! (`agent_def_*`) and `instance` (`instance_*`), the shared column lists and
//! row mappers in `row`, slugs and name-keyed rows in `name_keys`, and the
//! delete cascade in `purge`.
//!
//! Every method in both groups reads and writes `db_agents` only, as of the
//! agent-concept consolidation's definition flip
//! (`docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md` Phase 3d) — the
//! instance side flipped first (Phase 3b, PR 2 of 4, #3080), the definition
//! side followed once the six agent-child tables' FK re-pointed to
//! `db_agents` (Phase 3c, PR 3 of 4, #3088) removed the reason
//! `agent_def_get` had to stay scoped to `db_agent_definitions`. Neither
//! legacy table has a live writer left in this file; `dual_write.rs`, which
//! used to mirror definition writes into `db_agents`, is gone — there is no
//! second table left to mirror into one from.
//!
//! `db_agent_definitions` and `db_agent_instances` are still physically
//! present in the schema (dropped in a later PR) — do not read either as a
//! live source anywhere in this file; every read here is `db_agents`.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;
use crate::registry::RecordScope;

mod definition;
mod instance;
mod name_keys;
mod purge;
mod row;

pub use name_keys::AgentNameMatch;
#[cfg(test)]
pub(super) use name_keys::key_tombstones_for_tests;
#[cfg(test)]
pub(super) use purge::purge_agent_dependents_for_tests;
#[allow(unused_imports)]
use self::{name_keys::*, purge::*, row::*};

/// A user-defined AI agent in the user's agent-definition catalog.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
    #[ts(type = "number")]
    pub auto_start: i64,
    #[serde(default)]
    #[ts(type = "number")]
    pub restart_on_crash: i64,
    #[serde(default)]
    #[ts(type = "number")]
    pub idle_timeout_minutes: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
    #[serde(default)]
    #[ts(type = "number")]
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
    #[ts(type = "number")]
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
    #[ts(type = "number")]
    pub user_hidden: i64,
    /// Docker image to use when `agent_type == "container"`.
    /// e.g. `"ghcr.io/agentmuxai/agent-base:latest"`.
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
    /// Inert. It was a per-agent opt-in to the CLI's global login when no
    /// account resolved at spawn; the spawn gate no longer reads it (an
    /// oauth-class agent with no bound account is always refused), no RPC
    /// sets it and no UI shows it. The column and this field stay only so
    /// the shared definition registry keeps the value older builds read
    /// (m0017/m0018 grandfathered linkless agents to `1` there).
    /// SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md R10.
    #[serde(default)]
    #[ts(type = "number")]
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
    /// Default 0 = opt-in required. Schema v17. Toggled from the Warden Supervisor
    /// panel. See
    /// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
    #[serde(default)]
    #[ts(type = "number")]
    pub auto_continue_enabled: i64,
    /// The agent's own dedicated ABF bundle (`db_bundles.id`). Distinct from
    /// `AgentInstance.memory_id` (a specific *launch*'s bundle, which can
    /// still be pointed at a different bundle on purpose — this is just the
    /// default a launch inherits when it doesn't override). Empty string =
    /// not yet provisioned (legacy row predating this field, or a definition
    /// awaiting `m0021`'s backfill). Stored in `db_agents.default_memory_id`
    /// — a separate column from `db_agents.memory_id`, which is the
    /// instance-scoped field this doc paragraph opens by distinguishing it
    /// from. Schema v19; the two-column split dates to
    /// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.1.
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
/// Armory → Bundle → Personal for the common case. See
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
    format!("~/.agentmux/agents/{}", agentmux_common::slug::path_slug(agent_name))
}

/// Derive a filesystem-safe slug from a display name
/// (`agentmux_common::slug::definition_slug`).
pub fn derive_slug(name: &str) -> String {
    agentmux_common::slug::definition_slug(name)
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
    #[ts(type = "number")]
    pub started_at: i64,
    #[serde(default)]
    #[ts(type = "number")]
    pub ended_at: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    /// Legacy Identity-bundle id column — `db_identity_bundles` was
    /// dropped in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md.
    /// The launch modal now writes an account_id here instead; credential
    /// resolution and display names both go through
    /// `db_agent_identity_links`/`db_accounts`. Empty string means no
    /// account was picked at launch.
    #[serde(default)]
    pub identity_id: String,
    /// FK to `db_bundles.id`. Empty string means "use the blank
    /// singleton" (= vanilla CLI, no instructions). Set at
    /// instantiation via the launch modal's Bundle dropdown.
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

/// Shared test fixture: an `AgentDefinition` with every field defaulted to
/// an empty/zero value except the ones callers actually vary. Originally
/// shared to avoid `tests::bare_agent_def` and
/// `bundle_provisioning_store_separation_tests::base_agent` each hardcoding
/// this 30-field struct literal independently (reagentx P2 on PR #2602);
/// `bare_agent_def` is gone (its only caller pinned pre-flip behavior this
/// module no longer has), leaving `base_agent` as the remaining caller —
/// kept as one definition here regardless, so a future field addition only
/// needs updating once.
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
        let local = store.agent_row_get("remote-1").unwrap();
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
    // return an unrelated agent's memory, accounts or bundle. This test builds
    // exactly that collision (agent A's `instance_name` == agent B's
    // `slug` == "shared-name") and proves `instance_get_by_slug` stays
    // within its own namespace: it finds ONLY the slug match (B), never
    // A's row (its literal-name counterpart is gone — see below).
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

    // The §4 safety net. `instance_create` used to bind `def.slug` verbatim,
    // making it the one writer that could mint a duplicate — `agent_def_insert`
    // has always suffix-resolved, and no UNIQUE constraint backs the column.
    //
    // Note this is a contract fix, not a fix for an observed fault: the launch
    // flow names its agent and goes through `agent_def_insert`, so this branch
    // is reached only by handing `instance_create` a seeded template id
    // directly. A data check across 47 databases found 0 duplicate slugs and 18
    // launches, none inheriting a template's slug (§2.5.2).
    #[test]
    fn two_launches_of_one_template_get_distinct_slugs() {
        let store = Store::open_in_memory().unwrap();
        let mut tmpl = test_agent_def("tmpl-1", "Shared Template", "claude", "agent", 1, "");
        tmpl.slug = "shared-template".to_string();
        tmpl.is_seeded = 1;
        store.agent_def_insert_local_only(&mut tmpl, None).unwrap();

        let mut first = instance("launch-a", "tmpl-1");
        first.instance_name = "Launch A".to_string();
        store.instance_create(&first).unwrap();
        let mut second = instance("launch-b", "tmpl-1");
        second.instance_name = "Launch B".to_string();
        store.instance_create(&second).unwrap();

        // Neither launch may leave a resolvable duplicate behind: whatever
        // slugs they got, no slug may name two non-template rows.
        let conn = store.conn.lock().unwrap();
        let dupes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (SELECT slug FROM db_agents
                 WHERE is_template = 0 AND user_hidden = 0 AND slug <> ''
                 GROUP BY slug HAVING COUNT(*) > 1)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 0, "a template launch must not mint a duplicate slug");
    }

    // A slug does not identify an agent once two rows share one, so the honest
    // answer is "unknown" — the same fail-closed rule
    // `agent_registry_lookup::find_active_record_by_slug` adopted for the
    // registry in #3480, here for `db_agents` (#3500).
    //
    // This test predates the §4 safety net above and used to build its
    // duplicate by launching one template twice, on the reasoning that
    // `instance_create` copied `def.slug` verbatim. That is no longer true —
    // and was never true of the real launch flow either (§2.5.2) — so the
    // duplicate is now forced deliberately. The state remains reachable in
    // production: no UNIQUE constraint backs the column.
    #[test]
    fn instance_get_by_slug_fails_closed_when_two_rows_share_a_slug() {
        let store = Store::open_in_memory().unwrap();

        let mut tmpl = test_agent_def("tmpl-1", "Shared Template", "claude", "agent", 1, "");
        tmpl.slug = "shared-template".to_string();
        tmpl.is_seeded = 1;
        store.agent_def_insert_local_only(&mut tmpl, None).unwrap();
        assert_eq!(tmpl.slug, "shared-template", "fixture precondition");

        let mut first = instance("launch-a", "tmpl-1");
        first.instance_name = "Launch A".to_string();
        store.instance_create(&first).unwrap();

        let mut second = instance("launch-b", "tmpl-1");
        second.instance_name = "Launch B".to_string();
        store.instance_create(&second).unwrap();

        // Both write paths suffix-resolve now (§4 safety net), so two launches
        // no longer collide on their own — see
        // `two_launches_of_one_template_get_distinct_slugs`. Force the duplicate
        // this guard exists for.
        //
        // Done in two steps on purpose. Asserting only the final `None` cannot
        // tell "refused an ambiguous slug" from "nothing matched that slug" —
        // both are `None`, and after the safety net the second is what a naive
        // fixture actually produces. Resolving ONE row first pins that the
        // lookup is live and the fixture is real; the second row is then the
        // only thing that changed.
        store.test_force_slug("launch-a", "collide-me").unwrap();
        let single = store.instance_get_by_slug("collide-me").unwrap();
        assert_eq!(
            single.map(|i| i.id).as_deref(),
            Some("launch-a"),
            "one row with the slug must resolve — otherwise the check below is vacuous"
        );

        store.test_force_slug("launch-b", "collide-me").unwrap();

        // Before this guard, `ORDER BY updated_at DESC LIMIT 1` handed back
        // whichever row was touched last — deterministically the same wrong
        // agent every time — and `HistoryService::sessions_for_agent` then
        // resolved one agent's conversation history into the other's request.
        assert!(
            store.instance_get_by_slug("collide-me").unwrap().is_none(),
            "adding a second row with the same slug must flip resolve to refusal"
        );
    }

    #[test]
    fn instance_create_preserves_registry_updated_at_on_backfill() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        let record = global_user_agent("remote-2", "Remote2");
        def_store.upsert(&record).unwrap();
        store.set_def_registry(def_store);

        // Call the backfill directly rather than through a full
        // `instance_create`: "remote-2" is a user agent (is_seeded = 0), so
        // `instance_create` immediately follows the backfill with its own
        // launch-state fold, which legitimately bumps `updated_at` again —
        // by design, every launch touch does (see `agent_def_list`'s doc
        // comment). That second, later bump isn't what this test is
        // pinning; the backfill INSERT preserving the registry's real
        // `updated_at`, rather than resetting it to `created_at`, is.
        store.agent_def_backfill_local_from_registry(&record).unwrap();

        let local = store.agent_row_get("remote-2").unwrap().unwrap();
        assert_eq!(
            local.updated_at, 42,
            "backfilled row must preserve the registry's real updated_at, not reset to created_at"
        );
    }

    /// Pins the definition flip's actual point: `agent_def_list()` and
    /// `agent_def_get()` now read the SAME table, so a row visible to one
    /// is visible to the other. Before the flip these deliberately
    /// disagreed (`agent_def_get` was scoped to `db_agent_definitions`
    /// only) — that inconsistency meant an agent visible in "My Agents"
    /// (`agent_def_list`) could 404 the moment something tried to open its
    /// detail view (`agent_def_get`) if it existed only as a `db_agents`
    /// row (a template launch). A row written directly into `db_agents`,
    /// bypassing `agent_def_insert` entirely, now resolves through both.
    #[test]
    fn agent_def_list_and_agent_def_get_agree_on_the_same_row() {
        let store = Store::open_in_memory().unwrap();

        // A `db_agents` row with no `agent_def_insert` ever having run for
        // it — stands in for a template-launch row, which never had one.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO db_agents (id, slug, name, provider, is_template, created_at, updated_at, is_seeded)
                 VALUES ('launch-only', 'launch-only', 'LaunchOnly', 'claude', 0, 1, 1, 0)",
                [],
            )
            .unwrap();
        }

        let listed = store.agent_def_list().unwrap();
        assert!(
            listed.iter().any(|d| d.id == "launch-only"),
            "agent_def_list() must read db_agents"
        );
        let got = store.agent_def_get("launch-only").unwrap();
        assert!(
            got.is_some(),
            "agent_def_get() must resolve a row agent_def_list() already shows — no more disagreement between the two"
        );
        assert_eq!(got.unwrap().name, "LaunchOnly");
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
        assert!(bundle_store.bundle_get(&bundle_id).unwrap().is_some());
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
            definition_store.bundle_get(&agent.memory_id).unwrap().is_none(),
            "bundle must NOT be written into the definition store"
        );
        assert!(
            bundle_store.bundle_get(&agent.memory_id).unwrap().is_some(),
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
        let taken = super::super::bundles::Bundle {
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
        store.bundle_upsert(&taken).unwrap();

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

    // A bundle row from before the bundle stopped carrying a harness.
    fn bundle_with_provider(store: &Store, id: &str, provider: &str) {
        store
            .bundle_upsert(&super::super::bundles::Bundle {
                id: id.to_string(),
                name: id.to_string(),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: provider.to_string(),
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
            })
            .unwrap();
    }

    #[test]
    fn resolve_effective_provider_id_is_the_agents_own_even_when_its_bundle_says_otherwise() {
        let store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "codex", "");
        bundle_with_provider(&store, "b1", "claude");
        agent.memory_id = "b1".to_string();
        assert_eq!(store.resolve_effective_provider_id(&agent), "codex");
    }

    #[test]
    fn resolve_effective_provider_id_reads_the_bundle_only_for_an_agent_with_no_provider() {
        let store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "", "");
        bundle_with_provider(&store, "b1", "gemini");
        agent.memory_id = "b1".to_string();
        assert_eq!(store.resolve_effective_provider_id(&agent), "gemini");
    }

    #[test]
    fn a_new_agents_bundle_carries_no_harness() {
        let store = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "gemini", "");
        agent.model_vendor_base_url = "https://proxy.example.com".to_string();
        let bundle_id = store.bundle_provision_for_new_agent(&agent, 0).unwrap();
        let bundle = store.bundle_get(&bundle_id).unwrap().unwrap();
        assert_eq!((bundle.provider.as_str(), bundle.model.as_str()), ("", ""));
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

    // The fallback reads the bundle through the store it's called on: a
    // caller that passes mstore instead of id_store gets nothing, not a
    // silently wrong answer.
    #[test]
    fn resolve_effective_provider_id_fallback_reads_only_the_store_it_is_called_on() {
        let id_store = Store::open_in_memory().unwrap();
        let mstore = Store::open_in_memory().unwrap();
        let mut agent = base_agent("a1", "Agent One", "", "");
        bundle_with_provider(&id_store, "b1", "codex");
        agent.memory_id = "b1".to_string();
        assert_eq!(id_store.resolve_effective_provider_id(&agent), "codex");
        assert_eq!(mstore.resolve_effective_provider_id(&agent), "");
    }
}

#[cfg(test)]
mod fallback_id_tests {
    use super::*;

    /// Byte-for-byte with `agent-config-builder.ts`'s
    /// `name.toLowerCase().replace(/[^a-z0-9-_]/g, "-")` (UTF-16 units).
    #[test]
    fn the_frontend_fallback_matches_javascript_utf16_semantics() {
        assert_eq!(frontend_fallback_id("Agent 🚀"), "agent---");
        assert_eq!(frontend_fallback_id("Agent (v2)"), "agent--v2-");
        assert_eq!(frontend_fallback_id("Café Bot"), "caf--bot");
        assert_eq!(agent_open_fallback_id("Café Bot"), "café-bot");
    }
}

/// The only keys a remembered runtime may hold, and the longest value of each.
const LAST_RUNTIME_KEYS: &[(&str, usize)] = &[("permissionMode", 32), ("model", 128), ("effort", 32)];

/// Validate and canonicalise a remembered runtime before it is stored:
/// `''` clears; otherwise a JSON object of non-empty strings under the known
/// keys only (`permissionMode`, `model`, `effort`), each bounded. Anything
/// else is rejected, so the column can only ever hold something the launch
/// path knows how to read.
pub fn normalize_last_runtime(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("last runtime is not JSON: {e}"))?;
    let obj = value.as_object().ok_or("last runtime must be a JSON object")?;
    let mut out = serde_json::Map::new();
    for (key, v) in obj {
        let Some(&(_, max)) = LAST_RUNTIME_KEYS.iter().find(|(k, _)| k == key) else {
            return Err(format!("unknown last-runtime key {key:?}"));
        };
        let s = v.as_str().ok_or_else(|| format!("{key} must be a string"))?;
        if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
            return Err(format!("{key} is empty, too long or has control characters"));
        }
        out.insert(key.clone(), serde_json::Value::String(s.to_owned()));
    }
    if out.is_empty() {
        return Ok(String::new());
    }
    Ok(serde_json::Value::Object(out).to_string())
}

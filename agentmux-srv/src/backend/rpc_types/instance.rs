// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent instance command shapes: instance CRUD, named-agent dropdown rows,
//! and the AgentPicker "Recent sessions" surface.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandListAgentInstancesData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetAgentInstanceData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCreateAgentInstanceData {
    pub definition_id: String,
    #[serde(default)]
    pub block_id: String,
    #[serde(default)]
    pub parent_instance_id: String,
    /// Legacy Identity-bundle id column — `db_identity_bundles` was
    /// dropped in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md.
    /// The launch modal now writes an account_id here instead (see
    /// agent-model.ts), and this value is used only for opaque
    /// same-value filtering (`listrecentsessions`/`listnamedagents`'
    /// `identity_id` param) — display names and credential resolution
    /// both go through `db_agent_identity_links`/`db_accounts` now.
    /// Empty = ambient creds (no env-var injection).
    #[serde(default)]
    pub identity_id: String,
    /// FK to db_bundles. Empty = blank singleton. Set by the launch
    /// modal's Memory dropdown.
    #[serde(default)]
    pub memory_id: String,
    /// User-chosen instance name (becomes `AGENTMUX_AGENT_ID` in the
    /// spawn env). Powers the launch modal's "Continue agent"
    /// dropdown. Empty = un-named, won't appear in the dropdown.
    #[serde(default)]
    pub instance_name: String,
    /// Absolute working directory path resolved by
    /// `allocate_agent_workdir` at spawn time. Stored on the instance
    /// row so the continue flow can reuse it without re-deriving the
    /// slug.
    #[serde(default)]
    pub working_directory: String,
}

/// Request for `listnamedagents`. The launch modal's "Continue
/// agent" dropdown calls this; an absent / zero `limit` defaults to
/// 200 (capped at 1000 to keep the wire payload bounded).
///
/// `definition_id` is server-side filtering: when provided, only
/// instances of that definition are returned. Required for the
/// dropdown to behave correctly when a user has 200+ named agents
/// across many definitions — without server filtering, the current
/// definition's older instances could fall off the global cap.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandListNamedAgentsData {
    #[serde(default)]
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<String>,
}

/// One row of the launch modal's "Continue agent" dropdown. Joins
/// `db_agent_instances` with `db_agent_definitions` (for the definition's
/// display name + provider), `db_agent_identity_links`/`db_accounts`
/// (for the identity display name), and `db_bundles` (for memory bundle
/// names) so the frontend renders without further lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedAgentRow {
    pub instance_id: String,
    pub instance_name: String,
    pub definition_id: String,
    pub definition_name: String,
    pub provider: String,
    pub working_directory: String,
    pub identity_id: String,
    pub identity_name: String,
    pub memory_id: String,
    pub memory_name: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub status: String,
    pub block_id_hint: String,
}

/// Request for `hidenamedagent`. Sets `display_hidden = 1` on the
/// row. Row + working directory remain on disk for audit + recovery
/// (destructive deletion is a separate, confirm-gated flow).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHideNamedAgentData {
    pub id: String,
}

/// Request for `listrecentsessions` — powers the AgentPicker's "Recent
/// sessions" surface (cascade follow-up, 2026-05-23). Optional
/// `identity_id` filter narrows the results to sessions with a matching
/// `db_agent_instances.identity_id` value (opaque same-value filter —
/// display names resolve separately via `db_agent_identity_links`).
/// `limit` defaults to 20 (capped at 100); rows are sorted by the
/// most-recent activity timestamp (filestore `output.state.json` modts
/// when available, otherwise instance `started_at`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandListRecentSessionsData {
    #[serde(default)]
    pub limit: usize,
    /// When set + non-empty, filter to sessions whose `identity_id`
    /// matches. `Some("")` is treated the same as `None` (no filter)
    /// to make the frontend wiring straightforward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<String>,
}

/// One row of the AgentPicker's "Recent sessions" list. Mirrors
/// `NamedAgentRow` but adds preview fields read from the per-block
/// `output.state.json` snapshot in filestore. `node_count == 0` and an
/// empty `preview` mean the snapshot wasn't readable (the block may
/// pre-date the persistence flow or have crashed before its first
/// 30s snapshot) — the row still surfaces so the user can reattach.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentSessionRow {
    pub instance_id: String,
    pub instance_name: String,
    pub definition_id: String,
    pub definition_name: String,
    pub provider: String,
    /// Custom model vendor base URL override, mirrored from
    /// `AgentDefinition.model_vendor_base_url` — empty means the harness
    /// talks to its own default vendor. Lets `MyAgentsList` compute the
    /// dual-icon vendor badge (`resolveEffectiveVendor`) without a second
    /// round trip per row.
    #[serde(default)]
    pub model_vendor_base_url: String,
    pub working_directory: String,
    pub identity_id: String,
    pub identity_name: String,
    pub memory_id: String,
    pub memory_name: String,
    /// The pane / block whose filestore zone holds the conversation.
    /// Empty when the instance row exists but no SQLite row resolved
    /// the block_id (cross-version registry rows). Reattach falls
    /// back to the working-directory continuation path in that case.
    pub block_id_hint: String,
    /// The CLI-emitted session id (`session_id` for Claude/Gemini,
    /// `thread_id` for Codex) captured during the prior run. Empty
    /// when the row predates the capture, the CLI didn't emit a
    /// session id, or the instance was created via a path that
    /// doesn't go through the spawn that captures it. Used by the
    /// picker reattach flow to populate `agent:sessionid` on the new
    /// block's meta so the spawned subprocess gets a real
    /// `--resume <sid>` on the FIRST turn instead of starting a
    /// fresh conversation that re-injects the startup context.
    #[serde(default)]
    pub session_id: String,
    /// Snapshot of the first user message in the conversation (up to
    /// 240 chars, newlines collapsed). Empty when the snapshot doesn't
    /// exist or doesn't contain a user_message node yet.
    pub preview: String,
    /// Total `nodes.length` from the snapshot. 0 when unavailable.
    pub node_count: usize,
    /// Last activity timestamp (filestore modts when the snapshot
    /// exists, otherwise `started_at`). Drives the sort order.
    pub last_active_at: i64,
    /// Whether `output.state.json` was found in filestore for this
    /// block. False when the snapshot doesn't exist yet (no preview)
    /// or the block_id_hint was empty.
    pub has_snapshot: bool,
    /// True when `has_snapshot` is false because the filestore lookup
    /// itself ERRORED (I/O error, lock contention, DB read failure) —
    /// distinct from a genuine `Ok(None)` (the block simply never wrote
    /// a snapshot). Without this, a transient storage error renders
    /// identically to "never had history," with no signal to the user
    /// that this row's real state is unknown rather than confirmed
    /// empty. See
    /// docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md
    /// §5.
    #[serde(default)]
    pub snapshot_check_failed: bool,
    /// When the agent definition was first created (ms since epoch).
    /// Shown as "Created" in the My Agents card.
    pub agent_created_at: i64,
    /// When this instance was last launched (ms since epoch).
    /// Shown as "Last Launch" in the My Agents card.
    pub started_at: i64,
    /// "host" or "container" — drives the runtime badge in the My Agents list.
    #[serde(default)]
    pub agent_type: String,
}

/// Response envelope for `listrecentsessions`. Introduced alongside the
/// per-source degradation hardening in `session.rs` (reagent P1 on
/// PR #2327, re-reviewing
/// docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md's
/// fix): once every one of that handler's six data sources degrades to
/// empty on its OWN failure instead of aborting the whole RPC, the
/// response can no longer distinguish "genuinely zero agents" from "a
/// data source failed and we got nothing" by transport success/failure
/// alone — the exact ambiguity this whole fix exists to close, just
/// pushed one layer deeper. `degraded` lists which source(s) fell back
/// this call (empty = fully healthy); the frontend treats an empty
/// `rows` alongside a non-empty `degraded` as an error state, not a
/// trustworthy zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListRecentSessionsResult {
    pub rows: Vec<RecentSessionRow>,
    #[serde(default)]
    pub degraded: Vec<String>,
}

/// Mutable subset of AgentInstance for PATCH-style updates. Every field is
/// optional — absent fields preserve their current value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUpdateAgentInstanceData {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// JSON-encoded `GitHubContext` or empty string. `None` = leave as-is;
    /// `Some("")` = explicitly clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteAgentInstanceData {
    pub id: String,
}

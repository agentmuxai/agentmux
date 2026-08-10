// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent domain: App API agent lifecycle (agent.open/send/stop/status/…),
//! agent definition CRUD + content, skills, history, import/export, and the
//! two-tier template / fork command shapes.

use serde::{Deserialize, Serialize};

/// Request for agent.open — find or create an agent pane for the given agent_id.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentOpenData {
    pub agent_id: String,
    pub tab_id: Option<String>,
    pub split_direction: Option<String>,
    pub split_reference_block_id: Option<String>,
    pub focus: Option<bool>,
}

/// Request for agent.send — send a message to an agent pane.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentSendData {
    pub block_id: String,
    pub message: String,
}

/// Request for agent.stop — stop a running agent subprocess.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentStopApiData {
    pub block_id: String,
    pub signal: Option<String>,
}

/// Request for agent.status — query status of an agent pane.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentStatusData {
    pub block_id: String,
}

/// Request for agent.output — read buffered output lines from an agent pane.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentOutputData {
    pub block_id: String,
    pub after_line: Option<usize>,
    pub max_lines: Option<usize>,
}

/// Request for agent.stream — subscribe to live output from an agent pane.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentStreamData {
    pub block_id: String,
}

/// Response from agent.open.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentOpenResult {
    pub block_id: String,
    pub tab_id: String,
    pub agent_id: String,
    pub provider: String,
    pub controller_type: String,
    pub status: String,
    pub created: bool,
}

/// Request for agent.define — create or upsert an agent definition.
/// `if_exists` controls behaviour when a slug-matching definition exists:
///   `"skip"` (default) — return existing id unchanged
///   `"update"` — overwrite all provided non-empty fields
///   `"error"` — fail with an error message
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandAgentDefineData {
    pub name: String,
    #[serde(default)]
    pub provider: String,
    /// Alternative to `provider` — inferred from model prefix.
    /// If both are set, `provider` wins.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub environment: String,
    /// System/instruction text written to the agent's CLAUDE.md on spawn.
    pub system_prompt: Option<String>,
    /// Extra env vars injected at agent spawn, stored as KEY=VALUE lines.
    pub env: Option<std::collections::HashMap<String, String>>,
    pub if_exists: Option<String>,
    pub create_instance_stub: Option<bool>,
    /// "host" or "container". Defaults to "host" when absent — the safe
    /// default that works without Docker. Callers must explicitly pass
    /// "container" so a missing field never silently starts the wrong runtime.
    #[serde(default = "default_host_agent_type")]
    pub agent_type: String,
    /// Docker image for container-type agents. Empty string for host agents.
    #[serde(default)]
    pub container_image: String,
    /// JSON array of volume mount specs. Empty array (`"[]"`) for host agents.
    #[serde(default = "default_container_volumes")]
    pub container_volumes: String,
    /// Redirects this agent's harness at a non-default model vendor backend
    /// (e.g. a custom `ANTHROPIC_BASE_URL` for a `claude`-provider agent).
    /// `None` (the default — omitted or absent) = don't touch the stored
    /// value; on a fresh insert this means "no override". `Some("")` is an
    /// explicit clear back to "no override" — required so a caller can ever
    /// undo a previously-set value or recover from a stale override left
    /// behind by a provider change (see `agent_define_core`'s update
    /// branch). `Some(url)` sets it, rejected at define-time unless the
    /// resolved provider declares `ProviderConfig::base_url_env_var`.
    /// Mirrors the existing `Option<T>` = "don't touch" idiom already used
    /// by `CommandUpdateAgentDefinitionData::use_ambient_login`.
    #[serde(default)]
    pub model_vendor_base_url: Option<String>,
}

fn default_host_agent_type() -> String {
    "host".to_string()
}

/// Response from agent.define.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentDefineResult {
    pub definition_id: String,
    pub slug: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_stub_id: Option<String>,
}

/// Response from agent.send.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentSendResult {
    pub block_id: String,
    pub status: String,
    pub session_id: Option<String>,
}

/// Response from agent.stop.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentStopResult {
    pub block_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
}

/// Response from agent.status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentStatusResult {
    pub block_id: String,
    pub agent_id: String,
    pub provider: String,
    pub controller_type: String,
    pub status: String,
    pub session_id: Option<String>,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
}

/// A single entry in the agent.list response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentListEntry {
    pub block_id: String,
    pub tab_id: String,
    pub agent_id: String,
    pub provider: String,
    pub status: String,
    pub session_id: Option<String>,
}

/// Response from agent.list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentListResult {
    pub agents: Vec<AgentListEntry>,
}

/// Response from agent.output.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentOutputResult {
    pub block_id: String,
    pub lines: Vec<String>,
    pub total_lines: usize,
    pub has_more: bool,
}

/// Request for `agent.process-list` — processes tracked under a given block.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentProcessListCommand {
    pub block_id: String,
}

/// One tracked process row. Mirrors `backend::process_tracker::TrackedProcess`
/// — defined here so the RPC layer can expose it without leaking the
/// internal module shape.
#[derive(Debug, Clone, Serialize)]
pub struct AgentProcessInfo {
    pub pid: u32,
    pub command: String,
    pub rss_bytes: u64,
    pub started_at_ms: u64,
}

/// Response from `agent.process-list`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentProcessListResult {
    pub block_id: String,
    /// Platform confidence level — `"high"`, `"best_effort"`, `"none"`.
    /// Frontend shows a badge when anything less than `high`.
    pub confidence: String,
    pub processes: Vec<AgentProcessInfo>,
}

/// Response from `agent.tracked-blocks` — the list of block IDs for
/// which a tracker exists. Swarm pane uses this to render per-agent
/// groups.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentTrackedBlocksResult {
    pub block_ids: Vec<String>,
}

/// Request for `agent.kill-process` — terminate a single PID if it's
/// in a given block's tracker tree.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentKillProcessCommand {
    pub block_id: String,
    pub pid: u32,
}

/// Request for `agent.kill-tree` — nuke every process tracked under a
/// given block.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentKillTreeCommand {
    pub block_id: String,
}

/// Response from `agent.kill-process` / `agent.kill-tree`.
/// `ok: true` means the kill was dispatched; it does NOT guarantee
/// the OS has fully torn down every descendant by the time the RPC
/// returns. The swarm activity panel's next refresh will reflect
/// actual state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentKillResult {
    pub ok: bool,
}

// ---- Agent definition command data types ----

/// Optional filter input for `listagents`. When `is_seeded` is set,
/// only definitions whose `is_seeded` column matches are returned
/// (`Some(1)` → templates only; `Some(0)` → user-owned agents only).
/// Absent / `None` = no filter — backward-compatible with callers
/// that pass `{}` or `null`. Phase 1 of the two-tier picker
/// (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
///
/// `include_hidden` (Phase 2 — Q2 Decision Y): when `false` (default),
/// templates with `user_hidden = 1` are filtered out. The settings
/// "Hidden templates" surface passes `true` so it can render rows for
/// unhiding; the picker proper omits the flag and gets the filtered
/// default. `include_hidden` only affects templates — user-owned rows
/// never set `user_hidden`, so the flag is a no-op for them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandListAgentDefinitionsData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_seeded: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_hidden: bool,
}

/// Request for `agentdefcreatefromtemplate`. Clones a seeded template
/// into a new user-owned definition. Phase 1 of the two-tier picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentDefCreateFromTemplateData {
    /// id of a seeded definition (must have `is_seeded = 1`).
    pub template_id: String,
    /// User-chosen display name for the new agent. Non-empty, ≤200
    /// chars, must not collide with another user-owned agent's name.
    pub name: String,
    /// Identity bundle id to bind (empty string = ambient creds).
    /// Stored on the launch-time `db_agent_instances` row by the
    /// launch flow; the definition itself doesn't hold bindings
    /// pre-Phase 3, so this is reserved for the frontend to thread
    /// through to its subsequent `launchAgentDefinition` call. The
    /// server returns it back in the response for symmetry +
    /// future-proofing.
    #[serde(default)]
    pub identity_id: String,
    /// Memory bundle id to bind (empty string = vanilla CLI).
    /// Same semantics as `identity_id` above.
    #[serde(default)]
    pub memory_id: String,
    /// Runtime to persist on the cloned definition: "host" or
    /// "container". Empty/absent → keep the template's `agent_type`.
    /// Runtime is chosen at instantiation time, not a property of the
    /// template, so the clone records the user's pick rather than
    /// inheriting the (now container-defaulted) template value.
    #[serde(default)]
    pub agent_type: String,
}

/// Response for `agentdefcreatefromtemplate`. The frontend uses
/// `definition_id` to launch the freshly-created agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefCreateFromTemplateResult {
    pub definition_id: String,
    /// Echoed back so the caller's launch step doesn't need to
    /// re-thread these — they flow through to the launch overrides.
    pub identity_id: String,
    pub memory_id: String,
}

/// Request for `agentdefhide` / `agentdefunhide`. Phase 2 of the
/// two-tier picker (Q2 Decision Y). The two RPCs share the same shape
/// — the action is encoded in the command name, not the payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentDefHideData {
    /// id of a seeded definition (must have `is_seeded = 1`).
    pub definition_id: String,
}

/// Response for `agentdefhide` / `agentdefunhide`. `ok = true` when a
/// row was updated; `false` when the id didn't match any row. (A row
/// that exists but isn't a template returns an RPC-level error, not
/// `ok: false` — the caller should never have been able to send that
/// id from the picker UI.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefHideResult {
    pub ok: bool,
}

/// Input for createagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCreateAgentDefinitionData {
    pub name: String,
    #[serde(default = "default_agent_icon")]
    pub icon: String,
    pub provider: String,
    #[serde(default)]
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
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
}

fn default_agent_type() -> String {
    "standalone".to_string()
}

fn default_container_volumes() -> String {
    "[]".to_string()
}

fn default_agent_icon() -> String {
    "✦".to_string()
}

/// Input for updateagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUpdateAgentDefinitionData {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub provider: String,
    #[serde(default)]
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
    #[serde(default = "default_agent_type")]
    pub agent_type: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub agent_bus_id: String,
    /// JSON-encoded per-provider account assignments (see
    /// `AgentDefinition.accounts`). Written by the Agent pane's Identity tab.
    #[serde(default)]
    pub accounts: String,
    /// Docker image for container-type agents. Empty string for host agents.
    #[serde(default)]
    pub container_image: String,
    /// JSON array of volume mount specs. Empty array (`"[]"`) for host agents.
    #[serde(default = "default_container_volumes")]
    pub container_volumes: String,
    /// Explicit per-agent opt-in to the CLI's global (ambient) login when no
    /// oauth-class account resolves at spawn (0/1). `None` (omitted) preserves
    /// the stored value — callers that only edit name/icon/accounts don't
    /// carry it. SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3.
    #[serde(default)]
    pub use_ambient_login: Option<i64>,
}

/// Input for deleteagent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteAgentDefinitionData {
    pub id: String,
}

/// Input for getagentcontent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetAgentContentData {
    pub agent_id: String,
    pub content_type: String,
}

/// Input for setagentcontent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSetAgentContentData {
    pub agent_id: String,
    pub content_type: String,
    pub content: String,
}

/// Input for getallagentcontent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetAllAgentContentData {
    pub agent_id: String,
}

// ---- Agent Skills command data types ----

/// Input for listagentskills
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListAgentSkillsData {
    pub agent_id: String,
}

/// Input for createagentskill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCreateAgentSkillData {
    pub agent_id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

fn default_skill_type() -> String {
    "prompt".to_string()
}

/// Input for updateagentskill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandUpdateAgentSkillData {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

/// Input for deleteagentskill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteAgentSkillData {
    pub id: String,
}

// ---- Agent History command data types ----

/// Input for appendagenthistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAppendAgentHistoryData {
    pub agent_id: String,
    pub entry: String,
}

/// Input for listagenthistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandListAgentHistoryData {
    pub agent_id: String,
    #[serde(default)]
    pub session_date: Option<String>,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_history_limit() -> i64 {
    50
}

/// Input for searchagenthistory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSearchAgentHistoryData {
    pub agent_id: String,
    pub query: String,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
}

// ---- Agent Import command data types ----

/// Input for importagentfromclaw
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandImportAgentFromClawData {
    pub workspace_path: String,
    pub agent_name: String,
}

/// Input for importagents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandImportAgentDefinitionsData {
    pub agents: Vec<AgentDefinitionImport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinitionImport {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub provider: String,
    pub shell: String,
    pub working_directory: String,
    pub agent_bus_id: String,
    pub agent_type: String,
    pub environment: String,
    pub restart_on_crash: bool,
    pub content: std::collections::HashMap<String, String>,
    pub skills: Vec<AgentSkillImport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkillImport {
    pub name: String,
    pub trigger: String,
    pub skill_type: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportAgentDefinitionsResult {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

/// Response for exportagents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportAgentDefinitionsResult {
    pub version: u32,
    pub exported_at: String,
    pub source: String,
    pub agents: Vec<AgentDefinitionExport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinitionExport {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub provider: String,
    pub shell: String,
    pub working_directory: String,
    pub agent_bus_id: String,
    pub agent_type: String,
    pub environment: String,
    pub restart_on_crash: bool,
    pub content: std::collections::HashMap<String, String>,
    pub skills: Vec<AgentSkillExport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkillExport {
    pub name: String,
    pub trigger: String,
    pub skill_type: String,
    pub description: String,
    pub content: String,
}

// ---- Agent definition branching (fork) ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandForkAgentDefinitionData {
    pub source_id: String,
    /// When non-empty this becomes the fork's display name directly.
    /// When empty, the handler auto-generates "Name #N".
    #[serde(default)]
    pub branch_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandForkAgentDefinitionSuggestData {
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkAgentDefinitionSuggestResult {
    pub suggested_label: String,
}

/// SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §4 — renames
/// whichever field a fork tab's title actually resolves from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRenameAgentDefinitionTitleData {
    pub id: String,
    pub title: String,
}

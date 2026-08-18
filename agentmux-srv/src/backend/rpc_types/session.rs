// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session domain: activity summary, session archival, and agent-anchored
//! session zones (read/write_state/append_output/archive/list_archives).
//! See docs/specs/SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md.

use serde::{Deserialize, Serialize};

use crate::agents::TokenCounts;

// ---- Session activity summary types ----

/// Request for session:activity_summary — maintain a stable, session-goal
/// title via Haiku, routed through the Ambient Model Call gateway
/// (`crate::ambient`). See
/// docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md and
/// docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandActivitySummaryData {
    pub block_id: String,
    /// Target word count, derived from pane width. Defaults to 7.
    pub word_target: Option<u32>,
    /// Caller's monotonic turn counter for this block (bumped on every new
    /// turn). Used by the ambient gateway to cancel a stale in-flight call
    /// for the same block and reject a request that arrives out of order.
    pub generation: u64,
    /// The user's newest message, verbatim (the frontend's
    /// `TurnPhase.Submitting.pendingContent`) — what the title-maintaining
    /// Haiku call evaluates against the current title. Falls back to a
    /// FileStore output-tail digest server-side when absent (e.g. an older
    /// frontend build) so the endpoint degrades gracefully instead of going
    /// silent.
    pub user_message: Option<String>,
}

/// Response from session:activity_summary. The backend also writes
/// `term:ambient_summary` to block meta. `tokens` is `None` when the request
/// was rejected as stale-on-arrival or the underlying call failed/was
/// cancelled — callers should only record usage when it's `Some`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ActivitySummaryResult {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenCounts>,
}

// ---- Ghost-text next-prompt suggestion types ----

/// Request for session:next_prompt_suggestion — predict a plausible next
/// user message via Haiku, routed through the Ambient Model Call gateway
/// (same shape as CommandActivitySummaryData). See
/// docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandNextPromptSuggestionData {
    pub block_id: String,
    /// Caller's generation for this block — see CommandActivitySummaryData's
    /// doc comment for the wall-clock-vs-remount rationale (identical here).
    pub generation: u64,
}

/// Response from session:next_prompt_suggestion. The FRONTEND writes
/// `term:next_prompt_suggestion` to block meta after receiving this response
/// (useNextPromptSuggestion.ts) — the handler itself never touches block
/// meta, same as session:activity_summary's handler (see the inline comment
/// at its call site in `register_session_activity_summary`). `tokens` is
/// `None` under the same conditions as ActivitySummaryResult.tokens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct NextPromptSuggestionResult {
    pub suggestion: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenCounts>,
}

// ---- Session archival types ----

/// Request for session:archive — compress and archive a session's FileStore output.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionArchiveData {
    pub block_id: String,
}

/// Response from session:archive.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionArchiveResult {
    pub block_id: String,
    pub archived_bytes: u64,
    pub archived_at: i64,
}

/// Request for session:restore — decompress archive back into FileStore.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionRestoreData {
    pub block_id: String,
}

/// Response from session:restore.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionRestoreResult {
    pub block_id: String,
    pub restored_bytes: u64,
}

/// Request for session:export — read session output and return as base64 JSONL.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionExportData {
    pub block_id: String,
}

/// Response from session:export.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionExportResult {
    /// base64-encoded JSONL content (the raw output file bytes).
    pub content: String,
    pub line_count: u64,
    pub byte_count: u64,
}

// ---- Agent-anchored session zones (Option E, PR 1 of 2) ----

/// Request for `agent:session:read`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentSessionReadData {
    pub definition_id: String,
}

/// Response for `agent:session:read`. `content == None` means no zone /
/// snapshot exists for this definition (NOT an error — fresh agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionReadResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// `modts` of the `output.state.json` file in the agent's
    /// `:current` zone, if it exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modts: Option<i64>,
}

/// Request for `agent:session:write_state`. Writes `output.state.json`
/// into `agent:<definition_id>:current` (creates the zone if missing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentSessionWriteStateData {
    pub definition_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionWriteStateResult {
    pub bytes_written: u64,
}

/// Request for `agent:session:append_output`. Appends a single
/// NDJSON line to `output` in `agent:<definition_id>:current`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentSessionAppendOutputData {
    pub definition_id: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionAppendOutputResult {
    pub bytes_written: u64,
}

/// Request for `agent:session:archive`. Snapshots `agent:<defId>:current`
/// into `agent:<defId>:archive:<now_ms>` then clears the current zone.
/// Returns the archive zoneid (empty if no-op).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAgentSessionArchiveData {
    pub definition_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionArchiveResult {
    /// Empty string when nothing was archived (current zone was empty).
    pub archive_zoneid: String,
    pub archived_at_ms: i64,
}

/// Request for `agent:session:list_archives`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandAgentSessionListArchivesData {
    pub definition_id: String,
    #[serde(default)]
    pub limit: usize,
}

/// One row of the agent's archive list. Mirrors `RecentSessionRow`
/// preview shape so the frontend can reuse the same row component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentArchiveRow {
    pub archive_zoneid: String,
    pub archived_at_ms: i64,
    /// First user_message in the archived `output.state.json` (up to
    /// 240 chars, newlines collapsed). Empty when unreadable.
    pub preview: String,
    /// Total `nodes.length` from the archived snapshot. 0 when
    /// unreadable / missing.
    pub node_count: usize,
}

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
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandActivitySummaryData {
    pub block_id: String,
    /// Target word count, derived from pane width. Defaults to 7.
    #[ts(optional)]
    pub word_target: Option<u32>,
    /// Caller's monotonic turn counter for this block (bumped on every new
    /// turn). Used by the ambient gateway to cancel a stale in-flight call
    /// for the same block and reject a request that arrives out of order.
    #[ts(type = "number")]
    pub generation: u64,
    /// The user's newest message, verbatim (the frontend's
    /// `TurnPhase.Submitting.pendingContent`) — what the title-maintaining
    /// Haiku call evaluates against the current title. Falls back to a
    /// FileStore output-tail digest server-side when absent (e.g. an older
    /// frontend build) so the endpoint degrades gracefully instead of going
    /// silent.
    #[ts(optional)]
    pub user_message: Option<String>,
}

/// Response from session:activity_summary. The backend also writes
/// `term:ambient_summary` to block meta. `tokens` is `None` when the request
/// was rejected as stale-on-arrival or the underlying call failed/was
/// cancelled — callers should only record usage when it's `Some`.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct ActivitySummaryResult {
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tokens: Option<TokenCounts>,
}

// ---- Ghost-text next-prompt suggestion types ----

/// Request for session:next_prompt_suggestion — predict a plausible next
/// user message via Haiku, routed through the Ambient Model Call gateway
/// (same shape as CommandActivitySummaryData). See
/// docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandNextPromptSuggestionData {
    pub block_id: String,
    /// Caller's generation for this block — see CommandActivitySummaryData's
    /// doc comment for the wall-clock-vs-remount rationale (identical here).
    #[ts(type = "number")]
    pub generation: u64,
}

/// Response from session:next_prompt_suggestion. The FRONTEND writes
/// `term:next_prompt_suggestion` to block meta after receiving this response
/// (useNextPromptSuggestion.ts) — the handler itself never touches block
/// meta, same as session:activity_summary's handler (see the inline comment
/// at its call site in `register_session_activity_summary`). `tokens` is
/// `None` under the same conditions as ActivitySummaryResult.tokens.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct NextPromptSuggestionResult {
    pub suggestion: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tokens: Option<TokenCounts>,
}

// ---- Resume preflight types ----

/// Request for session:resume_preflight — asks, before any spawn, whether this
/// pane's next turn will continue its conversation or start a new one.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionResumePreflightData {
    pub block_id: String,
}

/// One row of the pane's progress list while the preflight runs — same shape as
/// the launcher splash's stage rows.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct ResumePreflightStep {
    pub id: String,
    pub label: String,
    /// `false` is normal — it's how the sequence narrows, not an error.
    pub ok: bool,
    pub detail: String,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

/// Response from session:resume_preflight.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct SessionResumePreflightResult {
    pub block_id: String,
    /// `"resume" | "recover" | "fresh" | "unknown"`.
    #[ts(type = "\"resume\" | \"recover\" | \"fresh\" | \"unknown\"")]
    pub verdict: String,
    /// The session that would actually load, when one would.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<String>,
    /// A real session on disk that the next spawn will NOT reach for — set
    /// only alongside `"fresh"`, as evidence that history exists even though
    /// nothing will load it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recoverable_session_id: Option<String>,
    pub steps: Vec<ResumePreflightStep>,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

// ---- Session archival types ----

/// Request for session:archive — compress and archive a session's FileStore output.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionArchiveData {
    pub block_id: String,
}

/// Response from session:archive.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct SessionArchiveResult {
    pub block_id: String,
    #[ts(type = "number")]
    pub archived_bytes: u64,
    #[ts(type = "number")]
    pub archived_at: i64,
}

/// Request for session:restore — decompress archive back into FileStore.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionRestoreData {
    pub block_id: String,
}

/// Response from session:restore.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct SessionRestoreResult {
    pub block_id: String,
    #[ts(type = "number")]
    pub restored_bytes: u64,
}

/// Request for session:export — read session output and return as base64 JSONL.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct CommandSessionExportData {
    pub block_id: String,
}

/// Response from session:export.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct SessionExportResult {
    /// base64-encoded JSONL content (the raw output file bytes).
    pub content: String,
    #[ts(type = "number")]
    pub line_count: u64,
    #[ts(type = "number")]
    pub byte_count: u64,
}

// ---- Agent-anchored session zones (Option E, PR 1 of 2) ----

/// Request for `agent:session:read`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentSessionReadData {
    pub definition_id: String,
}

/// Response for `agent:session:read`. `content == None` means no zone /
/// snapshot exists for this definition (NOT an error — fresh agent).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AgentSessionReadResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content: Option<String>,
    /// `modts` of the `output.state.json` file in the agent's
    /// `:current` zone, if it exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub modts: Option<i64>,
}

/// Request for `agent:session:write_state`. Writes `output.state.json`
/// into `agent:<definition_id>:current` (creates the zone if missing).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentSessionWriteStateData {
    pub definition_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AgentSessionWriteStateResult {
    #[ts(type = "number")]
    pub bytes_written: u64,
}

/// Request for `agent:session:append_output`. Appends a single
/// NDJSON line to `output` in `agent:<definition_id>:current`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentSessionAppendOutputData {
    pub definition_id: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AgentSessionAppendOutputResult {
    #[ts(type = "number")]
    pub bytes_written: u64,
}

/// Request for `agent:session:archive`. Snapshots `agent:<defId>:current`
/// into `agent:<defId>:archive:<now_ms>` then clears the current zone.
/// Returns the archive zoneid (empty if no-op).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentSessionArchiveData {
    pub definition_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AgentSessionArchiveResult {
    /// Empty string when nothing was archived (current zone was empty).
    pub archive_zoneid: String,
    #[ts(type = "number")]
    pub archived_at_ms: i64,
}

/// Request for `agent:session:list_archives`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAgentSessionListArchivesData {
    pub definition_id: String,
    // Generated as a REQUIRED `limit: number`, where the hand-written type said
    // `limit?: number`. `#[serde(default)]` on a non-`Option` field is the same
    // shape ts-rs cannot express as an optional property (it rejects
    // `#[ts(optional)]` on anything that is not `Option<T>`), and making it
    // `Option<usize>` would change what the handler receives. Harmless in
    // practice: `AgentSessionListArchivesCommand` has no callers today, so
    // nothing is forced to start passing a limit it did not pass before.
    #[serde(default)]
    #[ts(type = "number")]
    pub limit: usize,
}

/// One row of the agent's archive list. Mirrors `RecentSessionRow`
/// preview shape so the frontend can reuse the same row component.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AgentArchiveRow {
    pub archive_zoneid: String,
    #[ts(type = "number")]
    pub archived_at_ms: i64,
    /// First user_message in the archived `output.state.json` (up to
    /// 240 chars, newlines collapsed). Empty when unreadable.
    pub preview: String,
    /// Total `nodes.length` from the archived snapshot. 0 when
    /// unreadable / missing.
    #[ts(type = "number")]
    pub node_count: usize,
}

// Request-shape tests for the eleven `session:*` / `agent:session:*` commands.
//
// These exist because nothing else catches a Req/payload mismatch: `tsc` only
// checks the frontend against the GENERATED types, and
// `scripts/check-rpc-bindings.sh` only checks that a generated type exists per
// command and is current. Neither ever deserializes a real payload into the
// Rust struct, so a `Req` that cannot parse what the stub sends compiles,
// typechecks, passes the binding gate, and fails on every call — which is
// exactly how the `Req = ()` rejects `{}` bug on `bookmarks.list` reached
// runtime. Each case below is pinned to the literal JSON its call site sends.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    // useAgentActivitySummary.ts:90
    #[test]
    fn activity_summary_accepts_the_full_payload() {
        let r: CommandActivitySummaryData = serde_json::from_value(json!({
            "block_id": "b1",
            "word_target": 7,
            "generation": 1_789_000_000_000_i64,
            "user_message": "hello",
        }))
        .expect("session:activity_summary must accept the full payload");
        assert_eq!(r.word_target, Some(7));
        // `generation` is u64 and carries a JS `Date.now()`, which is why the
        // generated binding overrides it to `number` rather than `bigint`.
        assert_eq!(r.generation, 1_789_000_000_000);
    }

    // Both Options are generated as optional properties, and serde treats a
    // missing `Option<T>` as None, so omitting them must parse.
    #[test]
    fn activity_summary_accepts_omitted_optionals() {
        let r: CommandActivitySummaryData =
            serde_json::from_value(json!({"block_id": "b1", "generation": 1}))
                .expect("word_target and user_message must both be optional");
        assert!(r.word_target.is_none() && r.user_message.is_none());
    }

    // useNextPromptSuggestion.ts:150
    #[test]
    fn next_prompt_suggestion_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandNextPromptSuggestionData>(
            json!({"block_id": "b1", "generation": 1_789_000_000_000_i64}),
        )
        .expect("session:next_prompt_suggestion must accept block_id and generation");
    }

    // useResumePreflight.ts:63 and AgentControlBar.tsx:56/:68/:80 — all four
    // send exactly one field.
    #[test]
    fn the_four_block_id_only_commands_accept_a_bare_block_id() {
        let payload = json!({"block_id": "b1"});
        serde_json::from_value::<CommandSessionResumePreflightData>(payload.clone())
            .expect("session:resume_preflight");
        serde_json::from_value::<CommandSessionArchiveData>(payload.clone())
            .expect("session:archive");
        serde_json::from_value::<CommandSessionRestoreData>(payload.clone())
            .expect("session:restore");
        serde_json::from_value::<CommandSessionExportData>(payload).expect("session:export");
    }

    // useHistoryPagination.ts:257
    #[test]
    fn agent_session_read_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandAgentSessionReadData>(json!({"definition_id": "d1"}))
            .expect("agent:session:read must accept definition_id");
    }

    // useSnapshotPersistence.ts:115
    #[test]
    fn agent_session_write_state_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandAgentSessionWriteStateData>(
            json!({"definition_id": "d1", "content": "{}"}),
        )
        .expect("agent:session:write_state must accept definition_id and content");
    }

    // No non-test frontend caller today, but the stubs are still registered and
    // reachable, so pin their shapes rather than leaving them unverified.
    #[test]
    fn the_callerless_agent_session_commands_still_parse_their_declared_shapes() {
        serde_json::from_value::<CommandAgentSessionAppendOutputData>(
            json!({"definition_id": "d1", "line": "{}"}),
        )
        .expect("agent:session:append_output");
        serde_json::from_value::<CommandAgentSessionArchiveData>(json!({"definition_id": "d1"}))
            .expect("agent:session:archive");
        serde_json::from_value::<CommandAgentSessionListArchivesData>(
            json!({"definition_id": "d1", "limit": 10}),
        )
        .expect("agent:session:list_archives");
    }

    // `limit` is `#[serde(default)]` on a non-Option, so it is optional on the
    // wire even though the generated TS marks it required — ts-rs cannot
    // express an optional property for a non-Option field. This pins the wire
    // behaviour so the mismatch stays deliberate and visible.
    #[test]
    fn list_archives_limit_is_optional_on_the_wire_despite_the_required_binding() {
        let r: CommandAgentSessionListArchivesData =
            serde_json::from_value(json!({"definition_id": "d1"}))
                .expect("limit must be omittable on the wire");
        assert_eq!(r.limit, 0);
    }

    // `content`/`modts` are `skip_serializing_if = "Option::is_none"`, so the
    // server OMITS them rather than sending null. The generated binding says
    // `content?: string` for exactly this reason, and the frontend test mocks
    // were corrected from a null-valued shape to an empty object to match.
    #[test]
    fn agent_session_read_result_omits_empty_fields_rather_than_nulling_them() {
        let v = serde_json::to_value(AgentSessionReadResult { content: None, modts: None })
            .expect("serializable");
        assert_eq!(v, json!({}), "a no-snapshot read must serialize as an empty object");
    }
}

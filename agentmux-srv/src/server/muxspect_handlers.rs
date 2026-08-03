// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers backing the `muxspect` CLI — Phase 1 of
//! `docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`.
//!
//! Diagnostic-only surface: a thin read composition over `ProcessBroker`
//! (Phase A of the process-tracking consolidation,
//! `agentmux-srv/src/broker/process.rs`) and its sibling registries — never
//! a new independent snapshot of process/turn state (spec §5.1/§3 point 8).
//! Reached the same way `agentmux-mcp` already reaches every other
//! `/api/v1/*` route: plain HTTP, `X-AuthKey` header, `$AGENTMUX_LOCAL_URL`/
//! `$AGENTMUX_AUTH_KEY` inherited from the caller's own environment (spec
//! §5.2) — no new IPC mechanism, no new auth scheme.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde_json::json;

use crate::backend::storage::filestore::FileStore;

use super::AppState;

/// Bounded tail window read from a block's own persisted `output` file
/// when looking for a trailing `error_during_execution` frame — see
/// [`last_error_frame`]. Large enough to comfortably hold the frame itself
/// (a single JSON line, typically well under 1KB) plus whatever partial
/// line/chunking noise might precede it; small enough to stay O(1) and not
/// turn `muxspect` into a log-walker (that's `muxlog`'s job, not this
/// tool's — see the tool's own design spec §1).
const LAST_ERROR_TAIL_BYTES: i64 = 8192;

/// The most recent persisted `error_during_execution` frame at the tail of
/// a block's own `output` file — populated ONLY when the very last
/// non-blank line IS one (a block that errored once and then kept
/// producing normal output afterward must NOT be flagged; only "the last
/// thing that happened to this block was an unrecovered error" counts).
///
/// This reads a signal that already exists and is already durable: per
/// `agent_handlers/input.rs`'s own comment, this exact frame shape is
/// deliberately persisted to the block file (not just live-broadcast) so
/// it survives pane reload — the same source the frontend itself renders
/// as the pane's error bubble. Composing a second reader over that
/// existing store, rather than adding a new independent tracker, follows
/// this module's own design constraint (spec §5.1/§3 pt. 8).
///
/// See `docs/reports/REPORT_MUXSPECT_SPAWN_REFUSAL_DIAGNOSIS_EXTENSION_2026_08_03.md`
/// for the incident this closes the diagnostic gap on: a block whose spawn
/// was refused before any controller/process ever existed showed up as
/// featureless "no controller, no processes" — indistinguishable from a
/// healthy idle block — even though the actual, actionable reason was
/// already sitting in this exact file.
#[derive(serde::Serialize)]
pub struct LastErrorFrame {
    /// The frame's `error.message` verbatim (includes the `[AgentMux] `
    /// prefix every construction site already writes).
    pub message: String,
    /// Best-effort tag for which pre-spawn failure path wrote the frame —
    /// see [`classify_last_error_source`]. NOT a stable/tested taxonomy
    /// like `agents::failure::FailureClass` (which only classifies
    /// POST-spawn exit failures); building that properly for pre-spawn
    /// refusals is real, separate follow-up work (report §3.3), not
    /// attempted here.
    pub source: &'static str,
    /// Epoch ms the `output` file was last written — i.e. when this frame
    /// (being the file's last line) was appended. Deliberately a raw
    /// timestamp, not a precomputed "age," matching `last_computed_ms`'s
    /// existing convention elsewhere in this response so `--json`
    /// consumers get a stable value and the CLI renderer computes age the
    /// same way it already does for every other timestamp.
    pub written_ms: u64,
}

/// Best-effort classification of which pre-spawn failure path wrote a
/// persisted `error_during_execution` frame, inferred from the message
/// text itself (each construction site's message is distinct and stable —
/// see `identity/resolver/errors.rs`'s `SpawnGateError::Display`,
/// `subprocess/container_spawn.rs`, `subprocess/host_spawn.rs`, and
/// `agent_handlers/input.rs`'s container `ensure_running` path, the only
/// four call sites that build this frame today). `message` should have
/// the `[AgentMux] ` prefix already stripped.
fn classify_last_error_source(message: &str) -> &'static str {
    if message.starts_with("no credentials for") || message.starts_with("credential injection could not run") {
        "identity"
    } else if message.starts_with("container exec failed") || message.starts_with("container ensure_running failed")
    {
        "container_spawn"
    } else if message.starts_with("queued message could not be sent") {
        "host_spawn"
    } else {
        "unknown"
    }
}

/// Read the bounded tail of `block_id`'s `output` file and return the
/// parsed [`LastErrorFrame`] if its last non-blank line is one. `None`
/// covers every other case (no such block, empty file, last line is
/// normal output, last line doesn't parse as this exact frame shape) —
/// deliberately not distinguished further, since all of them mean the
/// same thing to a caller: nothing actionable to surface here.
fn last_error_frame(filestore: &FileStore, block_id: &str) -> Option<LastErrorFrame> {
    let file = filestore.stat(block_id, "output").ok().flatten()?;
    if file.size <= 0 {
        return None;
    }
    let start = (file.size - LAST_ERROR_TAIL_BYTES).max(0);
    let (_, bytes) = filestore
        .read_at(block_id, "output", start, file.size - start)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let last_line = text.lines().rev().find(|l| !l.trim().is_empty())?;

    let value: serde_json::Value = serde_json::from_str(last_line).ok()?;
    if value.get("type").and_then(|v| v.as_str()) != Some("result")
        || value.get("is_error").and_then(|v| v.as_bool()) != Some(true)
        || value.get("subtype").and_then(|v| v.as_str()) != Some("error_during_execution")
    {
        return None;
    }
    let message = value.get("error")?.get("message")?.as_str()?.to_string();
    let source = classify_last_error_source(message.strip_prefix("[AgentMux] ").unwrap_or(&message));

    Some(LastErrorFrame {
        message,
        source,
        written_ms: file.modts.max(0) as u64,
    })
}

/// `GET /api/v1/muxspect/list` — every controller-backed block's current
/// `ProcessStatus`, full detail (unlike `agent.tracked-blocks`, which
/// intentionally returns only `block_ids` for its Swarm-pane contract).
/// Each row also carries `is_agent` — `ProcessStatus::is_agent()`'s complete
/// classification rule (subprocess/persistent/acp are ALWAYS agents
/// regardless of `is_agent_pane`, which only applies to shell/cmd) — so
/// consumers don't reimplement that rule themselves (codex P2 on PR #2380:
/// the CLI's own naive `is_agent_pane`-only rendering mislabeled exactly
/// those three controller types).
pub async fn handle_muxspect_list(State(state): State<AppState>) -> impl IntoResponse {
    let blocks: Vec<serde_json::Value> = state
        .process_broker
        .list()
        .into_iter()
        .map(|status| {
            let is_agent = status.is_agent();
            let last_error = last_error_frame(&state.filestore, &status.block_id);
            let mut value = serde_json::to_value(&status).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("is_agent".to_string(), json!(is_agent));
                obj.insert("last_error".to_string(), json!(last_error));
            }
            value
        })
        .collect();
    Json(json!({ "blocks": blocks })).into_response()
}

#[derive(serde::Deserialize)]
pub struct MuxspectDescribeQuery {
    pub block_id: String,
}

/// `GET /api/v1/muxspect/describe?block_id=X` — composes `ProcessBroker`
/// status, the coarse `BlockControllerRuntimeStatus`, and the OS-process
/// tree for one block into a single response. This is the "describe
/// everything about block X" query
/// `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` §5.4 named
/// as missing — getting this picture today takes 2-3 separate, uncomposed
/// RPC round-trips.
pub async fn handle_muxspect_describe(
    State(state): State<AppState>,
    Query(q): Query<MuxspectDescribeQuery>,
) -> impl IntoResponse {
    if q.block_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing block_id" })),
        )
            .into_response();
    }

    // `process_status` (via `ProcessBroker::compute_status`) already reads
    // BOTH the process-tracker registry (for `processes`/
    // `liveness_confidence`) AND the controller's `BlockControllerRuntimeStatus`
    // (carried on `process_status.controller_status`) in one pass. A second,
    // independent read of either would risk a process starting/exiting or a
    // turn starting/finishing between the two calls, returning two
    // contradictory snapshots in one response (codex P2 on PR #2380, twice —
    // once for the process list, once for the controller status). Derive
    // everything from this one snapshot instead of reading twice.
    let process_status = state.process_broker.status(&q.block_id);
    let is_agent = process_status.is_agent();
    // Read unconditionally, not gated on "no live controller" — cheap
    // (bounded tail read), and gating it would mean re-deriving exactly
    // the liveness logic `process_status` already computes just to decide
    // whether to bother. In practice this is only ever non-null for
    // exactly the case it exists for: nothing has appended to this
    // block's output since an unrecovered pre-spawn failure.
    let last_error = last_error_frame(&state.filestore, &q.block_id);

    Json(json!({
        "block_id": q.block_id,
        "process_status": &process_status,
        "is_agent": is_agent,
        "controller_status": &process_status.controller_status,
        "processes": &process_status.processes,
        "tracking_confidence": process_status.liveness_confidence,
        "last_error": last_error,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    fn error_frame_line(message: &str) -> String {
        json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": {"message": message}
        })
        .to_string()
    }

    #[test]
    fn classify_last_error_source_matches_every_known_construction_site() {
        // The exact message text each of the four call sites builds today
        // (identity/resolver/errors.rs, container_spawn.rs, host_spawn.rs,
        // agent_handlers/input.rs) — with the `[AgentMux] ` prefix already
        // stripped, as the real caller does before classifying.
        assert_eq!(
            classify_last_error_source("no credentials for claude: the bound account was deleted or is unresolvable. Bind an account for this provider in the Armory."),
            "identity"
        );
        assert_eq!(
            classify_last_error_source("credential injection could not run (task join failed); the spawn was refused rather than falling back to the global CLI login. Retry, and check `muxlog auth` if it persists."),
            "identity"
        );
        assert_eq!(
            classify_last_error_source("container exec failed: no such container"),
            "container_spawn"
        );
        assert_eq!(
            classify_last_error_source("container ensure_running failed: image not found"),
            "container_spawn"
        );
        assert_eq!(
            classify_last_error_source("queued message could not be sent: lease held by another process"),
            "host_spawn"
        );
        assert_eq!(classify_last_error_source("something nobody wrote yet"), "unknown");
    }

    #[test]
    fn last_error_frame_none_for_missing_block() {
        let filestore = FileStore::open_in_memory().unwrap();
        assert!(last_error_frame(&filestore, "no-such-block").is_none());
    }

    #[test]
    fn last_error_frame_none_for_empty_output() {
        let filestore = FileStore::open_in_memory().unwrap();
        filestore
            .make_file("block-1", "output", FileMeta::new(), FileOpts::default())
            .unwrap();
        assert!(last_error_frame(&filestore, "block-1").is_none());
    }

    #[test]
    fn last_error_frame_found_when_last_line_is_the_frame() {
        let filestore = FileStore::open_in_memory().unwrap();
        filestore
            .make_file("block-1", "output", FileMeta::new(), FileOpts::default())
            .unwrap();
        let mut content = String::new();
        content.push_str("{\"type\":\"assistant\",\"text\":\"normal turn output\"}\n");
        content.push_str(&error_frame_line(
            "[AgentMux] no credentials for claude: bind an account in the Armory.",
        ));
        content.push('\n');
        filestore.write_file("block-1", "output", content.as_bytes()).unwrap();

        let found = last_error_frame(&filestore, "block-1").expect("last line is an error frame");
        assert_eq!(
            found.message,
            "[AgentMux] no credentials for claude: bind an account in the Armory."
        );
        assert_eq!(found.source, "identity");
        assert!(found.written_ms > 0);
    }

    /// Pins the "only the LAST non-blank line counts" rule (see the
    /// function's own doc comment): a block that errored once and then
    /// recovered — kept producing normal output afterward — must NOT be
    /// flagged. Only "the last thing that happened was an unrecovered
    /// error" is what this exists to surface.
    #[test]
    fn last_error_frame_none_when_normal_output_follows_an_earlier_error() {
        let filestore = FileStore::open_in_memory().unwrap();
        filestore
            .make_file("block-1", "output", FileMeta::new(), FileOpts::default())
            .unwrap();
        let mut content = String::new();
        content.push_str(&error_frame_line("[AgentMux] container exec failed: transient"));
        content.push('\n');
        content.push_str("{\"type\":\"assistant\",\"text\":\"turn resumed fine after the retry\"}\n");
        filestore.write_file("block-1", "output", content.as_bytes()).unwrap();

        assert!(last_error_frame(&filestore, "block-1").is_none());
    }

    #[test]
    fn last_error_frame_ignores_a_result_frame_that_is_not_an_error() {
        let filestore = FileStore::open_in_memory().unwrap();
        filestore
            .make_file("block-1", "output", FileMeta::new(), FileOpts::default())
            .unwrap();
        let normal_result = json!({"type": "result", "is_error": false, "subtype": "success"}).to_string();
        filestore
            .write_file("block-1", "output", format!("{normal_result}\n").as_bytes())
            .unwrap();

        assert!(last_error_frame(&filestore, "block-1").is_none());
    }
}

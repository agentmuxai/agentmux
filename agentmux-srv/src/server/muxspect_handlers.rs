// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers backing the `muxspect` CLI — Phase 1 of
//! `docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`, plus
//! the `dock`/`dock clear` extension from
//! `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md`.
//!
//! Diagnostic-only surface: a thin read composition over `ProcessBroker`
//! (Phase A of the process-tracking consolidation,
//! `agentmux-srv/src/broker/process.rs`) and its sibling registries — never
//! a new independent snapshot of process/turn state (spec §5.1/§3 point 8).
//! Reached the same way `agentmux-mcp` already reaches every other
//! `/api/v1/*` route: plain HTTP, `X-AuthKey` header, `$AGENTMUX_LOCAL_URL`/
//! `$AGENTMUX_AUTH_KEY` inherited from the caller's own environment (spec
//! §5.2) — no new IPC mechanism, no new auth scheme. `dock clear` is this
//! module's first MUTATING route — see the 2026-08-06 spec §2 for why that
//! scope call is narrow and deliberate, not a reversal of the read-only
//! design generally.

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
///
/// codex P2, PR #2802: `SpawnGateError::AmbientHomeDirNotAllowed`'s wording
/// starts with "this agent's", not either prefix this function originally
/// recognized — without this branch every ambient-home refusal reported
/// `unknown` instead of `identity`, regressing this diagnostic for exactly
/// the pre-spawn refusal case it exists to classify.
fn classify_last_error_source(message: &str) -> &'static str {
    if message.starts_with("no credentials for")
        || message.starts_with("credential injection could not run")
        || message.starts_with("this agent's")
    {
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

#[derive(serde::Deserialize)]
pub struct MuxspectFindQuery {
    pub block_id: Option<String>,
    pub agent: Option<String>,
}

/// `GET /api/v1/muxspect/find?block_id=X` or `?agent=X` — Ext 4 of
/// `docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`:
/// "which running instance(s), if any, have a controller or subagent
/// dispatch matching block_id/agent X" — the query that debugging session
/// needed and had no tool for, ending in manual filesystem/env-var
/// archaeology across every channel by hand.
///
/// Checks THIS instance first (host tier, no network) via the same
/// `ProcessBroker::list()` `handle_muxspect_list` uses (so "found" here
/// means the same thing `muxspect list` would show, not a different
/// existence check invented for this endpoint) and, for an `agent` query,
/// `reactive_handler.list_agents()` (same source `handle_muxspect_conversations`'s
/// host tier and `verify-sender` use).
///
/// Then checks every OTHER channel via the shared reactive registry
/// (`resolve_shared_reactive_dir` + `list_all_shared` — the exact mechanism
/// `handle_muxspect_conversations`'s cross-channel tier already uses, see
/// that handler's own doc comment for the security rationale) WITHOUT any
/// network call at all for basic existence: `AgentEntry` already carries
/// `block_id`/`agent_id`/`channel`/`local_url`, so a match there already
/// answers "which channel" before any forwarding happens. Only a MATCHED
/// cross-channel entry gets a forwarded `describe` call (same
/// single-forwarded-hop / timeout discipline as the conversations handler)
/// to fill in process/controller detail — unmatched channels cost nothing.
///
/// Always 200 — zero results is a legitimate answer ("not found on this
/// host, in any known channel"), not an error, same posture as every other
/// handler in this file. `results` can have more than one entry only if the
/// same block_id/agent name somehow exists in more than one place at once
/// (a real, worth-surfacing anomaly, not something to silently collapse to
/// the first match).
///
/// **Cross-channel `block_id` scope (reagent P2 on PR #2745):** the
/// cross-channel tier only searches the shared `AgentEntry` registry —
/// agent-registered blocks. A plain shell/terminal block, or any other
/// non-agent controller, in a DIFFERENT channel is not findable by
/// `block_id` here (the host tier above has no such limit — it searches
/// this instance's FULL `ProcessBroker::list()`, any controller type).
/// Reaching a non-agent controller cross-channel would need a remote
/// `/api/v1/muxspect/list`-style forward-and-filter, not just a registry
/// lookup — out of scope for this change; see
/// `SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md`'s Non-goals.
///
/// True when a forwarded `describe` response's `process_status.lifecycle`
/// reads `"unknown"` — `broker::process::Lifecycle::Unknown`'s own doc
/// comment: "no controller found for this block_id at all." A `None`
/// input (the forward itself failed/timed out) is NOT the same as this —
/// deliberately returns `false` for that case, since "we couldn't ask" is
/// not the same claim as "we asked and it's confirmed gone." Pure
/// (extracted from the async handler above) for direct unit testing —
/// reagent P1 on PR #2745 found the un-extracted inline version reported
/// "found": true unconditionally, with no test coverage of the "gone but
/// still registered" case at all.
fn describe_lifecycle_is_unknown(describe: Option<&serde_json::Value>) -> bool {
    describe
        .and_then(|d| d.get("process_status"))
        .and_then(|ps| ps.get("lifecycle"))
        .and_then(|l| l.as_str())
        == Some("unknown")
}

pub async fn handle_muxspect_find(
    State(state): State<AppState>,
    Query(q): Query<MuxspectFindQuery>,
) -> impl IntoResponse {
    if q.block_id.as_deref().unwrap_or("").is_empty() && q.agent.as_deref().unwrap_or("").is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "provide block_id or agent" })),
        )
            .into_response();
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    let own_channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());

    // Host tier — no network, same source handle_muxspect_list/
    // handle_muxspect_conversations already use. list_agents() (mutex lock +
    // full clone of the agent map) is fetched ONCE here, not once per block
    // inside the loop below — reagent P2 on PR #2745 caught the original
    // version paying that cost per block, unnecessary repeated
    // locking/cloning that scaled with block count, matching how
    // handle_muxspect_conversations's own host tier already does it.
    let agents = state.reactive_handler.list_agents();
    for status in state.process_broker.list() {
        let block_matches = q.block_id.as_deref().is_some_and(|b| b == status.block_id);
        let agent_matches = q.agent.as_deref().is_some_and(|name| {
            agents
                .iter()
                .any(|r| r.block_id == status.block_id && r.agent_id.eq_ignore_ascii_case(name))
        });
        if !block_matches && !agent_matches {
            continue;
        }
        results.push(json!({
            "tier": "host",
            "channel": own_channel,
            "block_id": status.block_id,
            "found": true,
            "process_status": &status,
        }));
    }

    // Cross-channel tier — AgentEntry already carries block_id/agent_id, so
    // matching costs zero network calls; only a match gets forwarded.
    //
    // reagent P1 on PR #2745: an earlier version unconditionally reported
    // "found": true for any registry match, even a possibly-stale one whose
    // owning agent/block already exited but hasn't been evicted from the
    // shared registry yet — reported as a successful match with CLI exit
    // code 0. Two checks now guard against that, mirroring the sibling
    // handle_muxspect_verify_sender's staleness handling in this same file:
    //   1. should_evict_on_forward_failure(&entry) (reused verbatim from
    //      backend::reactive::registry — the exact function the registry's
    //      OWN eviction logic uses, checking both real PID liveness and
    //      FORWARD_FAILURE_GRACE_MS age) filters out entries already known
    //      to be stale, before wasting a network call on them.
    //   2. Even a fresh-looking entry can point at a block the remote
    //      ProcessBroker no longer knows about (lifecycle: "unknown" — see
    //      broker::process::Lifecycle's doc comment: "No controller found
    //      for this block_id at all"). A successful forward with that
    //      lifecycle is reported with "found": false, not true.
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let local_url = state.local_web_url.clone();
        let matches: Vec<_> = crate::backend::reactive::registry::list_all_shared(&shared_dir)
            .into_iter()
            .filter(|e| e.channel != own_channel && e.local_url != local_url)
            .filter(|e| {
                q.block_id.as_deref().is_some_and(|b| b == e.block_id)
                    || q.agent.as_deref().is_some_and(|name| e.agent_id.eq_ignore_ascii_case(name))
            })
            .filter(|e| !crate::backend::reactive::registry::should_evict_on_forward_failure(e))
            .collect();

        let mut join_set = tokio::task::JoinSet::new();
        for entry in matches {
            let http_client = state.http_client.clone();
            join_set.spawn(async move {
                let fetch = http_client
                    .get(format!("{}/api/v1/muxspect/describe", entry.local_url))
                    .header("X-AuthKey", &entry.auth_key)
                    .query(&[("block_id", entry.block_id.as_str())])
                    .send();

                let describe = match tokio::time::timeout(
                    std::time::Duration::from_millis(CROSS_CHANNEL_PREVIEW_TIMEOUT_MS),
                    fetch,
                )
                .await
                {
                    Ok(Ok(resp)) if resp.status().is_success() => resp.json::<serde_json::Value>().await.ok(),
                    _ => None, // timeout/connect/parse failure: still report the match, just without detail
                };

                let lifecycle_unknown = describe_lifecycle_is_unknown(describe.as_ref());

                json!({
                    "tier": "cross-channel",
                    "channel": entry.channel,
                    "block_id": entry.block_id,
                    "agent_id": entry.agent_id,
                    "found": !lifecycle_unknown,
                    "describe": describe,
                })
            });
        }
        while let Some(res) = join_set.join_next().await {
            if let Ok(value) = res {
                results.push(value);
            }
        }
    }

    Json(json!({
        "query": { "block_id": q.block_id, "agent": q.agent },
        "results": results,
    }))
    .into_response()
}

/// A `ToolNode`'s status stays `"running"` this long (matches the
/// frontend's own `TOOL_PROMOTION_MS`, `frontend/app/view/agent/activity/
/// tool-adapter.ts`) before it's eligible to be flagged `stuck` here — a
/// tool call that's merely taking a while must not be flagged the instant
/// it starts. No shared constant exists between the two languages; keep
/// this value in sync with the frontend's if either changes.
const DOCK_STUCK_THRESHOLD_MS: i64 = 30_000;

#[derive(serde::Deserialize)]
pub struct MuxspectDockQuery {
    pub block_id: String,
}

#[derive(serde::Serialize)]
pub struct DockNodeView {
    pub node_id: String,
    pub tool_name: String,
    pub status: String,
    pub age_ms: i64,
    /// `true` when `status == "running"`, past `DOCK_STUCK_THRESHOLD_MS`,
    /// AND the block's own `ProcessBroker` lifecycle is not `Running` —
    /// i.e. nothing srv-side is actively backing this block's turn at
    /// all. Coarse: this is block-level, not node-level, liveness (a
    /// block mid-turn on a LATER tool call would still read `Running` and
    /// suppress the flag for an earlier stuck node in the same block) —
    /// see spec §3.1's own framing of this as "the same signal a human
    /// would manually cross-reference today, automated," not a perfect
    /// per-node liveness oracle.
    ///
    /// Does NOT cover a backgrounded call stuck in the actual dock UI
    /// (issue #2518): `status` here is the RAW `ToolNode` status, which is
    /// terminal ("success") for an accepted background launch within ~a
    /// second — this field can never go `true` for that case no matter how
    /// long the real dock row has been showing `running`, because the
    /// server has no visibility into whether a `<task-notification>` ever
    /// arrived (that reclassification is entirely client-side, in
    /// `tool-adapter.ts`). See `run_in_background` below: a `true` there
    /// combined with a long `age_ms` is worth checking by hand even when
    /// `stuck` reads `false`.
    pub stuck: bool,
    /// `params.run_in_background === true` on the pushing client's own
    /// `ToolNode`, if it's a Bash call and the renderer reported it.
    /// `None` for every non-Bash tool and for older clients that predate
    /// this field. See `stuck`'s doc comment above for why this exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_in_background: Option<bool>,
}

/// `GET /api/v1/muxspect/dock?block_id=X` — the cached `ToolNode` status
/// snapshot for one block, cross-referenced against `ProcessBroker` to
/// flag entries that look stuck. See
/// `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md` §3.1.
pub async fn handle_muxspect_dock(
    State(state): State<AppState>,
    Query(q): Query<MuxspectDockQuery>,
) -> impl IntoResponse {
    if q.block_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing block_id" })),
        )
            .into_response();
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    // One process_broker read, reused for every node — same rationale as
    // handle_muxspect_describe: derive from one snapshot, not N reads that
    // could observe N different, mutually-contradictory states.
    let lifecycle = state.process_broker.status(&q.block_id).lifecycle;
    let backed = lifecycle == crate::broker::process::Lifecycle::Running;

    let nodes = dock_node_views(state.dock_snapshots.get(&q.block_id, now_ms), now_ms, backed);

    // `DockSnapshotCache` is intentionally ephemeral (see its own doc
    // comment) and evicts any entry — including a genuinely-still-running
    // `bg: true` one — after MAX_NODE_AGE_MS (1 hour). A `task dev`
    // session in this repo's own retros has run 12+ hours, so this CLI's
    // output would go dark on it long before it actually finishes.
    // `db_background_tasks` (Phase A/B) has no such eviction — merge in
    // anything it still shows `Running` that the cache no longer has,
    // rather than changing the cache's own eviction semantics (which are
    // correct for what it's actually for — see dock_snapshot.rs's doc
    // comment). See docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.4.
    let background_tasks = state.wstore.background_task_list_for_block(&q.block_id).unwrap_or_default();
    let nodes = merge_background_tasks(nodes, background_tasks, now_ms);

    Json(json!({ "block_id": q.block_id, "nodes": nodes })).into_response()
}

/// Append a synthetic `DockNodeView` for every `Running` `db_background_tasks`
/// row not already represented in `nodes` (by id — `db_background_tasks.id`
/// mirrors the dock's own `node_id`, see `background_tasks.rs`'s module doc
/// comment). Pure, unit-testable like `dock_node_views` above. A task
/// still present in `nodes` is left as-is — the live cache's own status is
/// more specific (carries `tool_name`, real `stuck` computation) than
/// anything this synthesizes from the registry alone.
fn merge_background_tasks(
    mut nodes: Vec<DockNodeView>,
    background_tasks: Vec<crate::backend::storage::background_tasks::BackgroundTask>,
    now_ms: i64,
) -> Vec<DockNodeView> {
    use std::collections::HashSet;
    let existing: HashSet<String> = nodes.iter().map(|n| n.node_id.clone()).collect();
    for task in background_tasks {
        if task.status != crate::backend::storage::background_tasks::BackgroundTaskStatus::Running {
            continue;
        }
        if existing.contains(&task.id) {
            continue;
        }
        nodes.push(DockNodeView {
            node_id: task.id,
            tool_name: task.label,
            status: "running".to_string(),
            age_ms: (now_ms - task.started_at_ms).max(0),
            // Not computed the same way as a live cache entry's `stuck`
            // (no ProcessBroker cross-reference here, and a durable
            // registry row surviving this long is expected, not
            // suspicious, for a declared-background task) — false rather
            // than guessing.
            stuck: false,
            run_in_background: Some(true),
        });
    }
    nodes
}

/// Pure computation behind `handle_muxspect_dock` — separated out so the
/// stuck-flagging logic (the actual diagnostic value of this endpoint) is
/// unit-testable without constructing a full `AppState`/axum request, same
/// discipline as `last_error_frame`/`classify_last_error_source` above.
fn dock_node_views(
    nodes: Vec<crate::backend::dock_snapshot::DockNodeSnapshot>,
    now_ms: i64,
    backed: bool,
) -> Vec<DockNodeView> {
    nodes
        .into_iter()
        .map(|n| {
            let age_ms = (now_ms - n.observed_at).max(0);
            let stuck = n.status == "running" && age_ms >= DOCK_STUCK_THRESHOLD_MS && !backed;
            DockNodeView {
                node_id: n.node_id,
                tool_name: n.tool_name,
                status: n.status,
                age_ms,
                stuck,
                run_in_background: n.run_in_background,
            }
        })
        .collect()
}

#[derive(serde::Deserialize, Default)]
pub struct MuxspectBackgroundTasksQuery {
    /// Omit to list every still-running declared-background task across
    /// every block on this instance (the Swarm/fleet-view case); pass to
    /// scope to one block's own history (including terminal tasks, unlike
    /// the global list).
    #[serde(default)]
    pub block_id: Option<String>,
}

#[derive(serde::Serialize)]
pub struct BackgroundTaskView {
    pub id: String,
    pub block_id: String,
    pub label: String,
    pub pid: Option<i64>,
    pub started_at_ms: i64,
    pub status: &'static str,
    pub last_seen_ms: i64,
    pub ended_at_ms: Option<i64>,
}

impl From<crate::backend::storage::background_tasks::BackgroundTask> for BackgroundTaskView {
    fn from(t: crate::backend::storage::background_tasks::BackgroundTask) -> Self {
        BackgroundTaskView {
            id: t.id,
            block_id: t.block_id,
            label: t.label,
            pid: t.pid,
            started_at_ms: t.started_at_ms,
            status: t.status.as_str(),
            last_seen_ms: t.last_seen_ms,
            ended_at_ms: t.ended_at_ms,
        }
    }
}

/// `GET /api/v1/muxspect/background-tasks[?block_id=X]` — the durable
/// declared-background task registry (`db_background_tasks`), the source
/// of truth `handle_muxspect_dock`'s ephemeral, 1-hour-evicted
/// `DockSnapshotCache` deliberately isn't (see
/// docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md). A
/// task still shows up here after the dock cache has evicted it, and
/// survives a pane reload — it's backed by SQLite, not an in-memory cache.
pub async fn handle_muxspect_background_tasks(
    State(state): State<AppState>,
    Query(q): Query<MuxspectBackgroundTasksQuery>,
) -> impl IntoResponse {
    let result = match q.block_id.as_deref() {
        Some(block_id) if !block_id.is_empty() => state.wstore.background_task_list_for_block(block_id),
        _ => state.wstore.background_task_list_running(),
    };
    match result {
        Ok(tasks) => {
            let views: Vec<BackgroundTaskView> = tasks.into_iter().map(Into::into).collect();
            Json(json!({ "tasks": views })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct MuxspectDockClearRequest {
    pub block_id: String,
    pub node_id: String,
}

/// `POST /api/v1/muxspect/dock/clear` — validates `node_id` is a currently
/// tracked node for `block_id` (per the cache — see
/// `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md`
/// §3.2), then publishes `EVENT_DOCK_CLEAR` scoped to `block:<id>`. Only a
/// renderer currently displaying that block and still holding that node
/// ever receives or acts on it (real server-side scope routing, not
/// client-side self-filtering — same mechanism `EVENT_SHELL_NODE_CREATE`
/// already uses). Returns 404 if the node isn't in the cache — either it
/// was never pushed, or it already resolved/was already cleared; either
/// way there's nothing to do.
pub async fn handle_muxspect_dock_clear(
    State(state): State<AppState>,
    Json(req): Json<MuxspectDockClearRequest>,
) -> impl IntoResponse {
    if req.block_id.is_empty() || req.node_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing block_id or node_id" })),
        )
            .into_response();
    }

    if !state.dock_snapshots.has_node(&req.block_id, &req.node_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such node in this block's dock snapshot", "cleared": false })),
        )
            .into_response();
    }

    state.broker.publish(crate::backend::wps::WaveEvent {
        event: crate::backend::wps::EVENT_DOCK_CLEAR.to_string(),
        scopes: vec![format!("block:{}", req.block_id)],
        sender: String::new(),
        persist: 0,
        data: Some(json!({ "node_id": req.node_id })),
    });
    state.dock_snapshots.remove_node(&req.block_id, &req.node_id);

    Json(json!({ "cleared": true, "block_id": req.block_id, "node_id": req.node_id })).into_response()
}

#[derive(serde::Deserialize)]
pub struct MuxspectVerifySenderQuery {
    pub name: String,
}

/// **Not a security/trust primitive.** This reports registry LIVENESS only
/// (does an agent named X currently exist in the discovery data at all) —
/// it performs NO cryptographic check and must not be confused with the
/// actual JEKT sender-authentication mechanism, which is per-message,
/// automatic, and already computes a real `TRUST=`/`SIG=` value on every
/// delivered jekt via HMAC-SHA256 (host tier) / Ed25519 (LAN tier) / a
/// pinned Ed25519 key (reagent WAN) — see this repo's root `CLAUDE.md`,
/// "Is a jekt's sender identity actually verified? — the real answer".
/// Earlier drafts of this route reused that exact vocabulary
/// (`host-verified`/`network-claimed`) for a mere presence check, which
/// collided with — and could be mistaken for — that real, stronger
/// guarantee (reagentx-workflow review on PR #2702, P1). Deliberately no
/// `trust`/`verified` field here at all: `status` + `tier` fully describe
/// what was actually checked (presence, not authenticity), with nothing
/// that could be misread as more than that.
#[derive(serde::Serialize, Debug, PartialEq, Eq)]
pub struct VerifySenderVerdict {
    pub name: String,
    /// `"found"` | `"not_found"` | `"stale"`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_url: Option<String>,
}

/// One already-fetched candidate, tagged with the tier it was found on.
/// Keeps [`classify_sender`] pure and independent of `AgentRegistration`/
/// `AgentEntry`/`LanInstance`'s actual (differently-shaped) types — the
/// handler below does the field mapping once, at the IO boundary, same
/// discipline as `dock_node_views`/`last_error_frame` above.
struct SenderCandidate {
    name: String,
    /// `"host"` | `"cross-channel"` | `"lan"` | `"wan"` — NOT `"spawner"`;
    /// that tier is checked entirely client-side by `muxspect.mjs` before
    /// this route is ever called (it depends on the CALLER's own
    /// environment, which this handler has no way to observe). See
    /// `docs/specs/SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md`.
    tier: &'static str,
    last_seen_ms: Option<i64>,
    /// Whether `last_seen_ms` is a genuinely-refreshed heartbeat this
    /// candidate's tier can be judged stale by. `false` for host tier —
    /// `ReactiveHandler::list_agents()` is synchronously accurate (an
    /// agent is removed from it on unregister, not aged out by timestamp;
    /// see `bootstrap.rs`'s 20s heartbeat task's own doc comment: "always
    /// accurate, with no staleness window of its own") and nothing in this
    /// codebase currently calls `update_last_seen` to refresh
    /// `AgentRegistration.last_seen` after registration — applying a
    /// staleness cutoff to it would eventually flag every healthy,
    /// long-running host-tier agent as stale (codex review on PR #2702,
    /// P1). `false` for WAN too (`last_seen_ms` is always `None` there —
    /// `cloud_subscriber` doesn't track a per-agent heartbeat). `true` for
    /// cross-channel (the same 20s heartbeat task above re-writes the
    /// shared registry's `updated_at` for every live host-tier agent) and
    /// LAN (mDNS peers re-announce and bump `last_seen` on every
    /// `ServiceResolved`).
    stale_eligible: bool,
    channel: Option<String>,
    local_url: Option<String>,
}

/// Pure verdict computation over already-fetched discovery data — see
/// [`handle_muxspect_verify_sender`] for the IO/fetch boundary this
/// composes with. `candidates` must already be in tier-priority order
/// (host, then cross-channel, then lan, then wan — most-trusted first).
///
/// Among all case-insensitive name matches (a name can legitimately appear
/// on more than one tier, or more than once within cross-channel if
/// several channels registered it), prefers the first NON-stale match in
/// tier-priority order; only falls back to a stale match if every match is
/// stale. Picking the literal first match regardless of staleness (an
/// earlier version of this function) could report `stale` even when a
/// later, live entry for the same sender existed (codex review on PR
/// #2702, P2).
fn classify_sender(name: &str, candidates: &[SenderCandidate], now_ms: i64) -> VerifySenderVerdict {
    let needle = name.to_lowercase();
    let is_stale = |c: &&SenderCandidate| {
        c.stale_eligible
            && c.last_seen_ms
                .is_some_and(|ls| now_ms - ls >= crate::backend::reactive::registry::FORWARD_FAILURE_GRACE_MS as i64)
    };
    let matches: Vec<&SenderCandidate> = candidates.iter().filter(|c| c.name.to_lowercase() == needle).collect();
    let chosen = matches.iter().find(|c| !is_stale(c)).or_else(|| matches.first());

    match chosen {
        None => VerifySenderVerdict {
            name: name.to_string(),
            status: "not_found",
            tier: None,
            last_seen_ms: None,
            channel: None,
            local_url: None,
        },
        Some(c) => VerifySenderVerdict {
            name: name.to_string(),
            status: if is_stale(c) { "stale" } else { "found" },
            tier: Some(c.tier),
            last_seen_ms: c.last_seen_ms,
            channel: c.channel.clone(),
            local_url: c.local_url.clone(),
        },
    }
}

/// `GET /api/v1/muxspect/verify-sender?name=X` — answers "is an agent named
/// X currently registered, and via what tier" in one call, so an agent
/// that just received a `[JEKT:FROM=X ...]` marker doesn't have to fall
/// back to manual filesystem/process archaeology to sanity-check the
/// claimed sender exists at all. See [`VerifySenderVerdict`]'s doc comment
/// for the important caveat this is NOT the JEKT protocol's own
/// cryptographic sender verification. See `docs/retro/RETRO_JEKT_CROSS_
/// CHANNEL_TRUST_SELF_DECLARED_2026_08_21.md` for the incident this closes
/// the liveness-lookup gap on, and `docs/specs/SPEC_MUXSPECT_VERIFY_
/// SENDER_2026_08_21.md` for the full design.
///
/// Composes the same four sources `handle_discovery` (`GET
/// /agentmux/discovery`) already aggregates — host-tier
/// `AgentRegistration`, the host-global cross-channel shared registry, LAN
/// mDNS peers, and the WAN cloud subscriber — into a verdict instead of
/// raw discovery data. Always 200 (matching `list`/`dock`/`describe`'s own
/// convention of encoding "nothing found" in the body rather than the HTTP
/// status — a `not_found` verdict is a legitimate query result, not a
/// caller error) — `apiGet`'s fail-on-non-2xx contract in `muxspect.mjs`
/// would otherwise turn a normal "no such sender" answer into a hard CLI
/// failure.
pub async fn handle_muxspect_verify_sender(
    State(state): State<AppState>,
    Query(q): Query<MuxspectVerifySenderQuery>,
) -> impl IntoResponse {
    if q.name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing name" }))).into_response();
    }

    let mut candidates: Vec<SenderCandidate> = Vec::new();

    candidates.extend(state.reactive_handler.list_agents().into_iter().map(|a| SenderCandidate {
        name: a.agent_id,
        tier: "host",
        last_seen_ms: Some(a.last_seen as i64),
        stale_eligible: false,
        channel: None,
        local_url: None,
    }));

    // Mirrors handle_discovery's own cross_channel derivation exactly
    // (same exclusion of this instance's own channel/URL) — kept as a
    // separate read here rather than refactored into a shared helper, to
    // avoid touching handle_discovery's existing, already-tested behavior
    // for an unrelated new route.
    let own_channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
    let local_url = state.local_web_url.clone();
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        candidates.extend(
            crate::backend::reactive::registry::list_all_shared(&shared_dir)
                .into_iter()
                .filter(|e| e.channel != own_channel && e.local_url != local_url)
                .map(|e| SenderCandidate {
                    name: e.agent_id,
                    tier: "cross-channel",
                    last_seen_ms: Some(e.updated_at as i64),
                    stale_eligible: true,
                    channel: Some(e.channel),
                    local_url: Some(e.local_url),
                }),
        );
    }

    for lan_instance in state.lan_discovery.get_instances() {
        let peer_url = format!("{}:{}", lan_instance.address, lan_instance.port);
        // LanInstance.last_seen is UNIX SECONDS (lan_discovery.rs stamps it
        // via `.as_secs()`), not milliseconds like every other timestamp in
        // this handler — multiplying is required or every LAN candidate
        // reads as ~1970 and is immediately (wrongly) flagged stale (codex
        // review on PR #2702, P1).
        candidates.extend(lan_instance.agents.iter().map(|agent_name| SenderCandidate {
            name: agent_name.clone(),
            tier: "lan",
            last_seen_ms: Some((lan_instance.last_seen as i64).saturating_mul(1000)),
            stale_eligible: true,
            channel: None,
            local_url: Some(peer_url.clone()),
        }));
    }

    if let Some(subscriber) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
        candidates.extend(subscriber.subscribed_agents().into_iter().map(|agent_name| SenderCandidate {
            name: agent_name,
            tier: "wan",
            last_seen_ms: None,
            stale_eligible: false,
            channel: None,
            local_url: None,
        }));
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    Json(classify_sender(&q.name, &candidates, now_ms)).into_response()
}

/// Truncation length for a `last_message_preview` — keeps `conversations`'
/// response small even when the tail line is a long tool-call/result frame;
/// this is a liveness-glance preview, not a transcript reader (that's
/// `GetAgentTranscript`/`muxspect conversation <agent>`'s job). Applied to
/// BOTH the host-tier read ([`last_line_preview_and_activity`]) and the
/// cross-channel forwarded read — codex P2 on PR #2715 caught the
/// cross-channel path skipping this entirely, which let one huge remote
/// tool-call/result line inflate an otherwise-bounded response.
const PREVIEW_MAX_CHARS: usize = 200;

/// Bounded tail window read directly off the live FileStore file in
/// [`last_line_preview_and_activity`]'s fast path — deliberately small
/// relative to a whole session (which can be many MB): only needs to
/// comfortably contain the single last non-blank line, not search for a
/// specific frame shape the way [`LAST_ERROR_TAIL_BYTES`]'s window does.
const PREVIEW_TAIL_BYTES: i64 = 4096;

/// Short per-forward timeout for cross-channel preview fetches in
/// [`handle_muxspect_conversations`] — a single dead/unresponsive channel
/// must not stall the whole listing; best-effort, matches this file's
/// diagnostic (never authoritative) posture elsewhere.
const CROSS_CHANNEL_PREVIEW_TIMEOUT_MS: u64 = 1500;

/// Truncate a preview line to [`PREVIEW_MAX_CHARS`], appending an ellipsis
/// when it was actually cut. Shared by the host-tier and cross-channel
/// preview paths so they can't drift out of sync again (codex P2 on
/// PR #2715 — the cross-channel path originally didn't call this at all).
fn truncate_preview(line: &str) -> String {
    if line.chars().count() > PREVIEW_MAX_CHARS {
        line.chars().take(PREVIEW_MAX_CHARS).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

/// Last non-blank line of a block's own transcript, truncated for preview
/// display, plus a real "when was this last written" timestamp — `None`
/// for either if the block has no session output yet (e.g. an agent
/// registered but no turn has produced output) — missing is not an error,
/// same "encode absence in the body, not the status" posture as the rest
/// of this file.
///
/// Fast path: a bounded tail read straight off the live FileStore file
/// (`stat` + a small `read_at`, not a full read) — covers the overwhelming
/// common case (an active or recently-active, non-archived block) without
/// loading/decompressing full session history just to preview one line
/// (codex P2 on PR #2715: the original version called
/// `session_archive::read_session_output`, a full-session-export
/// primitive, for every host agent on every `conversations` call). The
/// file's own `modts` doubles as a REAL last-activity timestamp — unlike
/// `AgentRegistration::last_seen`, which nothing in this codebase updates
/// after registration (codex P2 on PR #2715: using it as
/// `last_activity_ms` made a long-lived, actively-working agent look
/// hours-stale). Falls back to the full-read primitive (which also
/// handles archived/gzip sessions) only when the fast path finds nothing —
/// no live file yet, or genuinely archived — at the cost of no cheap
/// activity timestamp in that fallback case (`None`, not a guessed value).
fn last_line_preview_and_activity(
    wstore: &std::sync::Arc<crate::backend::storage::store::Store>,
    filestore: &std::sync::Arc<FileStore>,
    block_id: &str,
) -> (Option<String>, Option<u64>) {
    if let Ok(Some(file)) = filestore.stat(block_id, "output") {
        if file.size > 0 {
            let start = (file.size - PREVIEW_TAIL_BYTES).max(0);
            if let Ok((_, bytes)) = filestore.read_at(block_id, "output", start, file.size - start) {
                let text = String::from_utf8_lossy(&bytes);
                if let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                    return (Some(truncate_preview(last)), Some(file.modts.max(0) as u64));
                }
            }
        }
    }

    match crate::backend::session_archive::read_session_output(wstore, filestore, block_id) {
        Ok((raw_bytes, _)) => {
            let text = String::from_utf8_lossy(&raw_bytes);
            let preview = text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(truncate_preview);
            (preview, None)
        }
        Err(_) => (None, None),
    }
}

/// `GET /api/v1/muxspect/conversations` — a single-call, all-tier glance at
/// every agent's most recent activity, so an agent (or a human via `muxspect
/// conversations`) doesn't have to compose `DiscoverAgents` +
/// N×`GetAgentTranscript` calls by hand. See
/// `docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
/// Phase A.
///
/// Host and cross-channel entries carry a `last_message_preview` (the tail
/// non-blank transcript line) and `turn_active`, read directly for host
/// (this instance's own `wstore`/`filestore`) and via a single best-effort
/// forwarded HTTP call per channel for cross-channel (same auth/loopback
/// pattern `handle_reactive_inject`'s Tier 2b and this instance's own
/// `handle_muxspect_verify_sender` already use — see those for the security
/// rationale; not repeated here). LAN and WAN entries carry no preview
/// (`remote_fetch_required: true`) — Phase A deliberately does not invent a
/// remote-read protocol for those tiers; see the spec's Phase B/C.
///
/// Always 200 — an empty `agents` list is a legitimate result, not an
/// error, matching every other handler in this file.
pub async fn handle_muxspect_conversations(State(state): State<AppState>) -> impl IntoResponse {
    let mut agents: Vec<serde_json::Value> = Vec::new();

    // Host tier — direct local read, no network.
    for reg in state.reactive_handler.list_agents() {
        let (preview, activity_ms) =
            last_line_preview_and_activity(&state.wstore, &state.filestore, &reg.block_id);
        let turn_active = crate::backend::blockcontroller::get_block_controller_status(&reg.block_id)
            .map(|s| s.turn_active)
            .unwrap_or(false);
        agents.push(json!({
            "name": reg.agent_id,
            "tier": "host",
            "turn_active": turn_active,
            // Real transcript-write time when available (see
            // last_line_preview_and_activity's doc comment) — falls back
            // to registration time only when there's no output yet at
            // all, which is still a meaningful "how long has this agent
            // existed" signal in that one specific case, not a stand-in
            // for a genuinely unknown value.
            "last_activity_ms": activity_ms.unwrap_or(reg.last_seen),
            "last_message_preview": preview,
        }));
    }

    // Cross-channel tier — one best-effort forwarded call per channel, ALL
    // concurrent (codex P2 on PR #2715: sequential fetches meant a single
    // dead/slow channel added its full CROSS_CHANNEL_PREVIEW_TIMEOUT_MS to
    // TOTAL latency instead of every channel paying it once, in parallel).
    // `forwarded=true` caps each of these at the same single-hop guard
    // `handle_reactive_transcript`'s own cross-channel fallback uses (see
    // TranscriptQuery::forwarded's doc comment) — this call IS a forward,
    // from the target instance's point of view, exactly like that one.
    let own_channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
    let local_url = state.local_web_url.clone();
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let cross_channel_entries: Vec<_> =
            crate::backend::reactive::registry::list_all_shared(&shared_dir)
                .into_iter()
                .filter(|e| e.channel != own_channel && e.local_url != local_url)
                .collect();

        let mut join_set = tokio::task::JoinSet::new();
        for entry in cross_channel_entries {
            let http_client = state.http_client.clone();
            join_set.spawn(async move {
                let mut preview: Option<String> = None;
                let mut turn_active: Option<bool> = None;

                let fetch = http_client
                    .get(format!("{}/agentmux/reactive/transcript", entry.local_url))
                    .header("X-AuthKey", &entry.auth_key)
                    .query(&[
                        ("agent", entry.agent_id.as_str()),
                        ("max_lines", "1"),
                        ("forwarded", "true"),
                    ])
                    .send();

                if let Ok(Ok(resp)) = tokio::time::timeout(
                    std::time::Duration::from_millis(CROSS_CHANNEL_PREVIEW_TIMEOUT_MS),
                    fetch,
                )
                .await
                {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            turn_active = body.get("turn_active").and_then(|v| v.as_bool());
                            preview = body
                                .get("lines")
                                .and_then(|v| v.as_array())
                                .and_then(|arr| arr.last())
                                .and_then(|v| v.as_str())
                                .map(truncate_preview);
                        }
                    }
                }
                // Timeout/connection/parse failure: list the agent anyway
                // with no preview — best-effort, same as every other tier
                // here.

                json!({
                    "name": entry.agent_id,
                    "tier": "cross-channel",
                    "channel": entry.channel,
                    "turn_active": turn_active,
                    "last_activity_ms": entry.updated_at,
                    "last_message_preview": preview,
                })
            });
        }
        while let Some(res) = join_set.join_next().await {
            // A panicked/cancelled task just doesn't contribute a row —
            // best-effort, matching this whole tier's posture elsewhere.
            if let Ok(value) = res {
                agents.push(value);
            }
        }
    }

    // LAN tier — liveness only, no remote-read protocol in Phase A.
    for lan_instance in state.lan_discovery.get_instances() {
        for agent_name in &lan_instance.agents {
            agents.push(json!({
                "name": agent_name,
                "tier": "lan",
                "host": format!("{}:{}", lan_instance.address, lan_instance.port),
                "remote_fetch_required": true,
            }));
        }
    }

    // WAN tier — liveness only, no remote-read protocol in Phase A.
    if let Some(subscriber) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
        for agent_name in subscriber.subscribed_agents() {
            agents.push(json!({
                "name": agent_name,
                "tier": "wan",
                "remote_fetch_required": true,
            }));
        }
    }

    Json(json!({ "agents": agents })).into_response()
}

/// Flatten one layout tree into a compact, renderable node list.
///
/// Deliberately a FLAT list carrying a `depth`, not nested JSON: the CLI
/// renders an indented outline, and a flat list keeps both the wire shape and
/// the renderer trivial.
fn flatten_layout_nodes(
    node: &agentmux_common::LayoutNode,
    depth: usize,
    index_path: &str,
    out: &mut Vec<serde_json::Value>,
) {
    let is_branch = !node.children.is_empty();
    out.push(json!({
        "id": node.id,
        "depth": depth,
        "path": index_path,
        "kind": if is_branch { "branch" } else { "leaf" },
        "flex_direction": match node.flex_direction {
            agentmux_common::FlexDirection::Row => "row",
            agentmux_common::FlexDirection::Column => "column",
        },
        "size": node.size,
        // `minimized` round-trips through the untyped `extra` catch-all on
        // this side, not as a typed field — same access the reducer's own
        // `is_node_locked` uses. Reported as "locked" rather than raw
        // `minimized` because legacy pre-display-mode markers
        // (`minimizedSize`/`slipMinimize`/`columnDissolve`) count too, and an
        // unmigrated tree on disk is exactly the case this route should show.
        "locked": crate::backend::layout::is_node_locked(node),
        // A branch is "effectively minimized" when every leaf under it is —
        // the state that drives chip geometry. Surfaced explicitly because it
        // is NOT visible from the `minimized` flag alone on a branch.
        "effectively_minimized": crate::backend::layout::is_effectively_minimized(node),
        "block_id": node.data.as_ref().map(|d| d.block_id.clone()),
        "child_count": node.children.len(),
    }));
    for (i, child) in node.children.iter().enumerate() {
        let child_path = if index_path.is_empty() {
            i.to_string()
        } else {
            format!("{index_path}.{i}")
        };
        flatten_layout_nodes(child, depth + 1, &child_path, out);
    }
}

/// `GET /api/v1/muxspect/layout[?tab_id=X]` — the persisted layout tree for
/// every tab (or one), plus the layout doctor's verdict on each.
///
/// Why this exists: layout state was previously only inspectable by reading
/// `db_layout` out of SQLite by hand and walking the JSON, which is what
/// diagnosing the cross-split minimize bugs (#2848, #2850, #2855) actually
/// required. This applies `muxspect`'s existing "what is this instance doing
/// right now" contract to layout.
///
/// It reports the PERSISTED tree, a genuinely different question from what
/// the frontend currently renders: `validate_layout_invariants` normally runs
/// at reducer write-time, so running it on demand here also catches
/// corruption already sitting on disk, including trees written by older
/// versions that never ran the check.
///
/// Read-only, like every route in this module except `dock clear`.
pub async fn handle_muxspect_layout(
    State(state): State<AppState>,
    Query(q): Query<MuxspectLayoutQuery>,
) -> impl IntoResponse {
    let tabs = match state.wstore.get_all::<crate::backend::obj::Tab>() {
        Ok(t) => t,
        Err(e) => {
            // 200 with an `error` field, not a 4xx/5xx — muxspect.mjs's
            // apiGet() treats a non-2xx as a transport failure and prints a
            // connection error, which would misattribute a store-read problem
            // as "srv is unreachable".
            return Json(json!({ "error": format!("failed to list tabs: {e}") })).into_response();
        }
    };

    let mut layouts = Vec::new();
    for tab in tabs {
        if let Some(want) = q.tab_id.as_deref() {
            if tab.oid != want {
                continue;
            }
        }
        let ls = match state
            .wstore
            .must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate)
        {
            Ok(ls) => ls,
            Err(e) => {
                // A tab pointing at a missing layoutstate is itself a finding
                // worth surfacing rather than skipping silently.
                layouts.push(json!({
                    "tab_id": tab.oid,
                    "tab_name": tab.name,
                    "layoutstate_oid": tab.layoutstate,
                    "error": format!("layoutstate unreadable: {e}"),
                }));
                continue;
            }
        };

        let violations = crate::backend::layout::validate_layout_invariants(&ls.rootnode);
        let mut nodes = Vec::new();
        if let Some(root) = ls.rootnode.as_ref() {
            flatten_layout_nodes(root, 0, "", &mut nodes);
        }
        let leaf_count = nodes.iter().filter(|n| n["kind"] == "leaf").count();
        let minimized_leaves = nodes
            .iter()
            .filter(|n| n["kind"] == "leaf" && n["locked"] == true)
            .count();

        layouts.push(json!({
            "tab_id": tab.oid,
            "tab_name": tab.name,
            "layoutstate_oid": ls.oid,
            "magnified_node_id": ls.magnifiednodeid,
            "node_count": nodes.len(),
            "leaf_count": leaf_count,
            "minimized_leaf_count": minimized_leaves,
            "violations": violations,
            "healthy": violations.is_empty(),
            "nodes": nodes,
        }));
    }

    // An explicit tab_id that matched nothing is a FAILED LOOKUP, not an
    // empty result. Returning `{"layouts": []}` made a stale or mistyped id
    // indistinguishable from a successful run over a tab with no panes --
    // the CLI printed "no layouts found" and exited 0, which is exactly the
    // wrong answer for a targeted diagnostic in a script (codex P2 on
    // PR #2856). Same 200-with-`error` shape as the store-read failure above,
    // for the same reason: a non-2xx would be reported by muxspect.mjs's
    // apiGet() as "srv is unreachable".
    if let Some(want) = q.tab_id.as_deref() {
        if layouts.is_empty() {
            return Json(json!({ "error": format!("no tab with id '{want}' in this instance") }))
                .into_response();
        }
    }

    Json(json!({ "layouts": layouts })).into_response()
}

#[derive(serde::Deserialize)]
pub struct MuxspectLayoutQuery {
    /// Restrict to one tab. Omitted = every tab in this instance.
    pub tab_id: Option<String>,
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

    // reagent P1 on PR #2745 — pins describe_lifecycle_is_unknown, the
    // pure piece extracted from handle_muxspect_find's cross-channel
    // staleness check.
    #[test]
    fn describe_lifecycle_is_unknown_true_for_lifecycle_unknown() {
        let describe = json!({ "process_status": { "lifecycle": "unknown" } });
        assert!(describe_lifecycle_is_unknown(Some(&describe)));
    }

    #[test]
    fn describe_lifecycle_is_unknown_false_for_a_real_lifecycle() {
        for lifecycle in ["running", "idle", "done", "error"] {
            let describe = json!({ "process_status": { "lifecycle": lifecycle } });
            assert!(!describe_lifecycle_is_unknown(Some(&describe)), "lifecycle={lifecycle}");
        }
    }

    #[test]
    fn describe_lifecycle_is_unknown_false_when_the_forward_itself_failed() {
        // None (the describe call timed out/errored) is NOT the same claim
        // as "confirmed gone" — "we couldn't ask" must not read as "unknown".
        assert!(!describe_lifecycle_is_unknown(None));
    }

    #[test]
    fn describe_lifecycle_is_unknown_false_for_malformed_or_missing_fields() {
        assert!(!describe_lifecycle_is_unknown(Some(&json!({}))));
        assert!(!describe_lifecycle_is_unknown(Some(&json!({ "process_status": {} }))));
        assert!(!describe_lifecycle_is_unknown(Some(&json!({ "process_status": { "lifecycle": 123 } }))));
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
        // codex P2, PR #2802: SpawnGateError::AmbientHomeDirNotAllowed.
        assert_eq!(
            classify_last_error_source("this agent's claude identity points directly at your personal claude config directory (C:\\Users\\asafe\\.claude) instead of an isolated AgentMux account — AgentMux no longer allows spawning an agent against your own global CLI login. Re-bind this identity to an isolated account in Armory \u{2192} Accounts (delete the current claude account and log in again to create a fresh, isolated one), then retry."),
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

    fn dock_node(id: &str, status: &str, observed_at: i64) -> crate::backend::dock_snapshot::DockNodeSnapshot {
        crate::backend::dock_snapshot::DockNodeSnapshot {
            node_id: id.to_string(),
            tool_name: "Bash".to_string(),
            status: status.to_string(),
            timestamp: Some(observed_at),
            observed_at,
            run_in_background: None,
        }
    }

    #[test]
    fn dock_node_views_flags_stuck_running_node_past_threshold_when_unbacked() {
        let now_ms = 100_000;
        let views = dock_node_views(vec![dock_node("n1", "running", 50_000)], now_ms, false);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].age_ms, 50_000);
        assert!(views[0].stuck, "running, 50s old, unbacked — must be flagged stuck");
    }

    #[test]
    fn dock_node_views_does_not_flag_running_node_within_threshold() {
        let now_ms = 100_000;
        // Only 10s old — well under DOCK_STUCK_THRESHOLD_MS (30s), even unbacked.
        let views = dock_node_views(vec![dock_node("n1", "running", 90_000)], now_ms, false);
        assert!(!views[0].stuck, "a tool merely taking a while must not be flagged instantly");
    }

    #[test]
    fn dock_node_views_does_not_flag_running_node_when_backed_by_a_live_process() {
        let now_ms = 100_000;
        // Old enough to qualify on age alone, but `backed=true` (block lifecycle
        // is Running) — something IS actively going on for this block.
        let views = dock_node_views(vec![dock_node("n1", "running", 50_000)], now_ms, true);
        assert!(!views[0].stuck, "backed by a live process — not stuck, just slow");
    }

    #[test]
    fn dock_node_views_does_not_flag_non_running_nodes_regardless_of_age() {
        let now_ms = 100_000;
        for status in ["success", "failed", "canceled", "denied"] {
            let views = dock_node_views(vec![dock_node("n1", status, 0)], now_ms, false);
            assert!(!views[0].stuck, "status={status} already resolved, must never be flagged stuck");
        }
    }

    #[test]
    fn dock_node_views_preserves_every_input_node() {
        let now_ms = 100_000;
        let views = dock_node_views(
            vec![dock_node("n1", "running", 99_000), dock_node("n2", "success", 80_000)],
            now_ms,
            false,
        );
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].node_id, "n1");
        assert_eq!(views[1].node_id, "n2");
    }

    #[test]
    fn dock_node_views_surfaces_run_in_background_even_though_status_is_terminal() {
        // Issue #2518: an accepted background launch is terminal
        // ("success") within ~a second server-side, so `stuck` can never
        // catch it — this is the one signal that lets a human tell "this
        // success might actually still be running in the real dock" apart
        // from an ordinary finished call, without the server having to
        // duplicate the client's <task-notification> cross-referencing.
        let now_ms = 100_000;
        let mut bg_node = dock_node("n1", "success", 0);
        bg_node.run_in_background = Some(true);
        let views = dock_node_views(vec![bg_node], now_ms, false);
        assert_eq!(views[0].run_in_background, Some(true));
        assert!(!views[0].stuck, "status is terminal — the raw heuristic correctly stays quiet");
    }

    fn bg_task(
        id: &str,
        status: crate::backend::storage::background_tasks::BackgroundTaskStatus,
        started_at_ms: i64,
    ) -> crate::backend::storage::background_tasks::BackgroundTask {
        crate::backend::storage::background_tasks::BackgroundTask {
            id: id.to_string(),
            block_id: "block-1".to_string(),
            label: "task dev".to_string(),
            pid: Some(4242),
            started_at_ms,
            status,
            last_seen_ms: started_at_ms,
            ended_at_ms: None,
        }
    }

    #[test]
    fn merge_background_tasks_adds_a_running_task_the_cache_evicted() {
        // The exact §3.4 scenario: DockSnapshotCache's 1-hour TTL evicted
        // the entry, but db_background_tasks still knows it's running.
        use crate::backend::storage::background_tasks::BackgroundTaskStatus;
        let now_ms = 100_000_000;
        let merged = merge_background_tasks(vec![], vec![bg_task("bg-1", BackgroundTaskStatus::Running, 1_000)], now_ms);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].node_id, "bg-1");
        assert_eq!(merged[0].status, "running");
        assert_eq!(merged[0].age_ms, now_ms - 1_000);
        assert_eq!(merged[0].run_in_background, Some(true));
        assert!(!merged[0].stuck);
    }

    #[test]
    fn merge_background_tasks_skips_a_task_the_cache_still_has() {
        use crate::backend::storage::background_tasks::BackgroundTaskStatus;
        let now_ms = 100_000;
        let existing = dock_node_views(vec![dock_node("bg-1", "running", 90_000)], now_ms, false);
        let merged = merge_background_tasks(existing, vec![bg_task("bg-1", BackgroundTaskStatus::Running, 1_000)], now_ms);
        assert_eq!(merged.len(), 1, "must not duplicate a node the live cache already has");
        assert_eq!(merged[0].tool_name, "Bash", "the live cache's own entry wins, not a synthesized one");
    }

    #[test]
    fn merge_background_tasks_ignores_non_running_tasks() {
        use crate::backend::storage::background_tasks::BackgroundTaskStatus;
        let now_ms = 100_000;
        for status in [BackgroundTaskStatus::Done, BackgroundTaskStatus::Error, BackgroundTaskStatus::Stopped] {
            let merged = merge_background_tasks(vec![], vec![bg_task("bg-1", status, 1_000)], now_ms);
            assert!(merged.is_empty(), "status={status:?} is terminal — must not be synthesized as a live dock row");
        }
    }

    #[test]
    fn merge_background_tasks_on_no_tasks_is_a_true_no_op() {
        let now_ms = 100_000;
        let existing = dock_node_views(vec![dock_node("n1", "running", 90_000)], now_ms, false);
        let expected_len = existing.len();
        let merged = merge_background_tasks(existing, vec![], now_ms);
        assert_eq!(merged.len(), expected_len);
        assert_eq!(merged[0].node_id, "n1");
    }

    #[tokio::test]
    async fn handle_muxspect_dock_rejects_empty_block_id() {
        let state = crate::server::tests::test_state();
        let resp = handle_muxspect_dock(
            State(state),
            Query(MuxspectDockQuery { block_id: String::new() }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handle_muxspect_dock_clear_rejects_empty_ids() {
        let state = crate::server::tests::test_state();
        let resp = handle_muxspect_dock_clear(
            State(state),
            Json(MuxspectDockClearRequest { block_id: String::new(), node_id: "n1".to_string() }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handle_muxspect_dock_clear_404s_on_unknown_node() {
        let state = crate::server::tests::test_state();
        let resp = handle_muxspect_dock_clear(
            State(state),
            Json(MuxspectDockClearRequest { block_id: "block-1".to_string(), node_id: "no-such-node".to_string() }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_muxspect_dock_clear_succeeds_and_removes_from_cache() {
        let state = crate::server::tests::test_state();
        state.dock_snapshots.push_delta("block-1", dock_node("n1", "running", 0));
        assert!(state.dock_snapshots.has_node("block-1", "n1"));

        let resp = handle_muxspect_dock_clear(
            State(state.clone()),
            Json(MuxspectDockClearRequest { block_id: "block-1".to_string(), node_id: "n1".to_string() }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            !state.dock_snapshots.has_node("block-1", "n1"),
            "a cleared node must not still be reported by a subsequent `dock` read"
        );
    }

    async fn json_body(resp: axum::response::Response) -> serde_json::Value {
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn handle_muxspect_background_tasks_scoped_to_block_includes_terminal_tasks() {
        let state = crate::server::tests::test_state();
        state.wstore.background_task_observe("t1", "block-1", "task dev", 0, 0).unwrap();
        state.wstore.background_task_observe("t2", "block-1", "finished build", 0, 0).unwrap();
        state
            .wstore
            .background_task_complete(
                "t2",
                crate::backend::storage::background_tasks::BackgroundTaskStatus::Done,
                500,
            )
            .unwrap();
        // Different block — must not leak into a block-scoped query.
        state.wstore.background_task_observe("t3", "block-2", "other pane", 0, 0).unwrap();

        let resp = handle_muxspect_background_tasks(
            State(state),
            Query(MuxspectBackgroundTasksQuery { block_id: Some("block-1".to_string()) }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        let ids: Vec<&str> = body["tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["t1", "t2"], "block-scoped query includes terminal tasks for that block");
    }

    #[tokio::test]
    async fn handle_muxspect_background_tasks_without_block_id_lists_running_globally() {
        let state = crate::server::tests::test_state();
        state.wstore.background_task_observe("t1", "block-1", "still running", 0, 0).unwrap();
        state.wstore.background_task_observe("t2", "block-2", "also running", 0, 0).unwrap();
        state.wstore.background_task_observe("t3", "block-1", "finished", 0, 0).unwrap();
        state
            .wstore
            .background_task_complete(
                "t3",
                crate::backend::storage::background_tasks::BackgroundTaskStatus::Done,
                500,
            )
            .unwrap();

        let resp = handle_muxspect_background_tasks(
            State(state),
            Query(MuxspectBackgroundTasksQuery { block_id: None }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        let ids: Vec<&str> = body["tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["t1", "t2"], "global query is running-only, across every block");
    }

    fn candidate(name: &str, tier: &'static str, last_seen_ms: Option<i64>, stale_eligible: bool) -> SenderCandidate {
        SenderCandidate { name: name.to_string(), tier, last_seen_ms, stale_eligible, channel: None, local_url: None }
    }

    #[test]
    fn classify_sender_not_found_on_empty_candidates() {
        let verdict = classify_sender("AgentA", &[], 1_000_000);
        assert_eq!(verdict.status, "not_found");
        assert_eq!(verdict.tier, None);
    }

    #[test]
    fn classify_sender_found_on_host_tier() {
        let candidates = [candidate("AgentA", "host", Some(999_000), false)];
        let verdict = classify_sender("AgentA", &candidates, 1_000_000);
        assert_eq!(verdict.status, "found");
        assert_eq!(verdict.tier, Some("host"));
    }

    #[test]
    fn classify_sender_found_on_cross_channel_tier() {
        let candidates = [candidate("AgentA", "cross-channel", Some(999_000), true)];
        let verdict = classify_sender("AgentA", &candidates, 1_000_000);
        assert_eq!(verdict.status, "found");
        assert_eq!(verdict.tier, Some("cross-channel"));
    }

    #[test]
    fn classify_sender_found_on_lan_or_wan_tier() {
        let lan = classify_sender("Peer", &[candidate("Peer", "lan", Some(999_000), true)], 1_000_000);
        assert_eq!(lan.status, "found");
        let wan = classify_sender("Peer", &[candidate("Peer", "wan", None, false)], 1_000_000);
        assert_eq!(wan.status, "found");
    }

    #[test]
    fn classify_sender_name_match_is_case_insensitive() {
        let candidates = [candidate("agenta", "host", Some(999_000), false)];
        let verdict = classify_sender("AgentA", &candidates, 1_000_000);
        assert_eq!(verdict.status, "found");
    }

    /// Host tier must NEVER be flagged stale, no matter how old
    /// `last_seen_ms` is — `ReactiveHandler::list_agents()` is
    /// synchronously accurate (removed on unregister, not aged out), and
    /// nothing in this codebase refreshes `AgentRegistration.last_seen`
    /// after registration. Applying a staleness cutoff here would flag
    /// every long-running, perfectly healthy host-tier agent as stale
    /// (codex review on PR #2702, P1 — this pins the fix).
    #[test]
    fn classify_sender_host_tier_never_stale_regardless_of_age() {
        let now_ms = 1_000_000;
        let ancient = now_ms - 10 * crate::backend::reactive::registry::FORWARD_FAILURE_GRACE_MS as i64;
        let candidates = [candidate("AgentA", "host", Some(ancient), false)];
        let verdict = classify_sender("AgentA", &candidates, now_ms);
        assert_eq!(verdict.status, "found", "host tier is stale_eligible: false — age must never matter");
    }

    /// Cross-channel and LAN entries DO get a real, periodically-refreshed
    /// timestamp (the 20s heartbeat task for cross-channel, mDNS
    /// re-announce for LAN) — pins that a hit past the real
    /// `FORWARD_FAILURE_GRACE_MS` threshold (reused from `registry.rs`,
    /// not a separately-invented constant) downgrades to `stale` rather
    /// than being silently dropped to `not_found`.
    #[test]
    fn classify_sender_stale_when_last_seen_past_the_real_grace_threshold() {
        let now_ms = 1_000_000;
        let stale_ts = now_ms - crate::backend::reactive::registry::FORWARD_FAILURE_GRACE_MS as i64;
        let candidates = [candidate("AgentA", "cross-channel", Some(stale_ts), true)];
        let verdict = classify_sender("AgentA", &candidates, now_ms);
        assert_eq!(verdict.status, "stale");
        assert_eq!(verdict.tier, Some("cross-channel"), "still reports which tier it was found on");
    }

    #[test]
    fn classify_sender_wan_tier_has_no_last_seen_and_is_never_stale() {
        // WAN candidates carry no last_seen_ms (cloud_subscriber doesn't
        // track a per-agent heartbeat) — must not be misclassified as
        // stale just because the field is absent.
        let candidates = [candidate("AgentA", "wan", None, false)];
        let verdict = classify_sender("AgentA", &candidates, 1_000_000);
        assert_eq!(verdict.status, "found");
    }

    /// Priority (non-stale case): a name present on multiple tiers reports
    /// the FIRST match — callers push candidates in tier-priority order
    /// (host, cross-channel, lan, wan) — so a host hit wins over a later
    /// cross-channel entry for the same name.
    #[test]
    fn classify_sender_reports_first_matching_tier_when_present_on_multiple() {
        let candidates = [
            candidate("AgentA", "host", Some(999_000), false),
            candidate("AgentA", "cross-channel", Some(999_000), true),
        ];
        let verdict = classify_sender("AgentA", &candidates, 1_000_000);
        assert_eq!(verdict.tier, Some("host"));
    }

    /// Priority (staleness case): a STALE earlier-tier match must not win
    /// over a LIVE later-tier match for the same name — picking the
    /// literal first match regardless of staleness could report `stale`
    /// even though a live entry for the same sender existed elsewhere in
    /// the candidate list (codex review on PR #2702, P2 — this pins the
    /// fix). Only when EVERY match is stale does the first (most-trusted)
    /// one win as a fallback.
    #[test]
    fn classify_sender_prefers_a_live_candidate_over_an_earlier_stale_one() {
        let now_ms = 1_000_000;
        let stale_ts = now_ms - crate::backend::reactive::registry::FORWARD_FAILURE_GRACE_MS as i64;
        let candidates = [
            candidate("AgentA", "cross-channel", Some(stale_ts), true), // stale, but tier-priority first
            candidate("AgentA", "lan", Some(999_000), true),            // live, lower tier-priority
        ];
        let verdict = classify_sender("AgentA", &candidates, now_ms);
        assert_eq!(verdict.status, "found");
        assert_eq!(verdict.tier, Some("lan"));
    }

    #[test]
    fn classify_sender_falls_back_to_first_stale_match_when_all_are_stale() {
        let now_ms = 1_000_000;
        let stale_ts = now_ms - crate::backend::reactive::registry::FORWARD_FAILURE_GRACE_MS as i64;
        let candidates = [
            candidate("AgentA", "cross-channel", Some(stale_ts), true),
            candidate("AgentA", "lan", Some(stale_ts), true),
        ];
        let verdict = classify_sender("AgentA", &candidates, now_ms);
        assert_eq!(verdict.status, "stale");
        assert_eq!(verdict.tier, Some("cross-channel"), "falls back to the most-trusted tier among stale matches");
    }

    #[tokio::test]
    async fn handle_muxspect_verify_sender_rejects_empty_name() {
        let state = crate::server::tests::test_state();
        let resp = handle_muxspect_verify_sender(State(state), Query(MuxspectVerifySenderQuery { name: String::new() }))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handle_muxspect_verify_sender_not_found_is_still_200() {
        // A "no such sender" verdict is a legitimate query result, not a
        // caller error — must stay 200 so muxspect.mjs's apiGet() (which
        // treats any non-2xx as fatal) doesn't turn it into a hard failure.
        let state = crate::server::tests::test_state();
        let resp = handle_muxspect_verify_sender(
            State(state),
            Query(MuxspectVerifySenderQuery { name: "NoSuchAgent".to_string() }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        assert_eq!(body["status"], "not_found");
    }

    #[tokio::test]
    async fn handle_muxspect_conversations_empty_state_is_still_200() {
        // No agents registered at all is a legitimate result, not an error —
        // same "encode absence in the body, not the status" posture as the
        // rest of this file's handlers.
        let state = crate::server::tests::test_state();
        let resp = handle_muxspect_conversations(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        assert!(body["agents"].is_array());
    }

    #[tokio::test]
    async fn handle_muxspect_conversations_includes_a_registered_host_agent() {
        let state = crate::server::tests::test_state();
        let unique = uuid::Uuid::new_v4();
        let agent_id = format!("conversations-test-agent-{unique}");
        let block_id = format!("conversations-test-block-{unique}");
        state
            .reactive_handler
            .register_agent(&agent_id, &block_id, None)
            .unwrap();

        let resp = handle_muxspect_conversations(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json_body(resp).await;
        let agents = body["agents"].as_array().expect("agents array");
        let entry = agents
            .iter()
            .find(|a| a["name"] == agent_id)
            .expect("registered agent should appear in the listing");
        assert_eq!(entry["tier"], "host");
        // No filestore content written for this block in this test — a
        // missing preview is the correct, non-error result (see
        // last_line_preview's own doc comment), not something this test
        // should treat as a failure.
        assert_eq!(entry["last_message_preview"], serde_json::Value::Null);
    }

    /// Minimal, non-archived `Block` row so `read_session_output` takes its
    /// FileStore-read path — matches the pattern
    /// `session_archive.rs`'s own tests use, just without the extra
    /// archival-specific meta this module's read path never inspects.
    fn insert_bare_block(
        wstore: &std::sync::Arc<crate::backend::storage::store::Store>,
        block_id: &str,
    ) {
        use crate::backend::obj::Block;
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: Default::default(),
            subblockids: None,
        };
        wstore.insert(&mut block).expect("wstore insert");
    }

    #[test]
    fn last_line_preview_and_activity_returns_last_non_blank_line() {
        let filestore = std::sync::Arc::new(FileStore::open_in_memory().unwrap());
        let wstore = std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let block_id = uuid::Uuid::new_v4().to_string();
        filestore
            .make_file(&block_id, "output", crate::backend::storage::filestore::FileMeta::default(), crate::backend::storage::filestore::FileOpts::default())
            .expect("make_file");
        filestore
            .append_data(&block_id, "output", b"first line\n\nlast line\n")
            .expect("append_data");
        insert_bare_block(&wstore, &block_id);

        let (preview, activity_ms) = last_line_preview_and_activity(&wstore, &filestore, &block_id);
        assert_eq!(preview, Some("last line".to_string()));
        // Fast path (live FileStore file present) — a real write timestamp
        // must come back, not None, per codex P2 on PR #2715.
        assert!(activity_ms.is_some());
    }

    #[test]
    fn last_line_preview_and_activity_truncates_long_lines() {
        let filestore = std::sync::Arc::new(FileStore::open_in_memory().unwrap());
        let wstore = std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let block_id = uuid::Uuid::new_v4().to_string();
        let long_line = "x".repeat(PREVIEW_MAX_CHARS + 50);
        filestore
            .make_file(&block_id, "output", crate::backend::storage::filestore::FileMeta::default(), crate::backend::storage::filestore::FileOpts::default())
            .expect("make_file");
        filestore
            .append_data(&block_id, "output", format!("{long_line}\n").as_bytes())
            .expect("append_data");
        insert_bare_block(&wstore, &block_id);

        let (preview, _) = last_line_preview_and_activity(&wstore, &filestore, &block_id);
        let preview = preview.unwrap();
        assert_eq!(preview.chars().count(), PREVIEW_MAX_CHARS + 1); // +1 for the trailing "…"
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn last_line_preview_and_activity_returns_none_for_missing_block() {
        let filestore = std::sync::Arc::new(FileStore::open_in_memory().unwrap());
        let wstore = std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
        let (preview, activity_ms) = last_line_preview_and_activity(&wstore, &filestore, "no-such-block");
        assert_eq!(preview, None);
        assert_eq!(activity_ms, None);
    }

    /// `flatten_layout_nodes` is what `muxspect layout` renders from, so its
    /// shape is the contract. Covers the cross-split arrangement the layout
    /// fixes (#2848/#2850/#2855) came from: a Row nested inside a Column.
    #[test]
    fn flatten_layout_nodes_reports_depth_path_and_effective_minimize() {
        use agentmux_common::{FlexDirection, LayoutNode, LayoutNodeData};

        fn leaf(id: &str, block: &str, minimized: bool) -> LayoutNode {
            let mut n = LayoutNode {
                id: id.to_string(),
                flex_direction: FlexDirection::Row,
                size: 10.0,
                children: Vec::new(),
                data: Some(LayoutNodeData {
                    block_id: block.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            if minimized {
                // Round-trips through `extra`, not a typed field — same as
                // the frontend writes it.
                n.extra.insert("minimized".to_string(), json!(true));
            }
            n
        }

        // Column[ top, Row[ A(min), B(min) ], bottom ]
        let inner = LayoutNode {
            id: "inner".into(),
            flex_direction: FlexDirection::Row,
            size: 10.0,
            children: vec![leaf("a", "blk-a", true), leaf("b", "blk-b", true)],
            data: None,
            ..Default::default()
        };
        let root = LayoutNode {
            id: "root".into(),
            flex_direction: FlexDirection::Column,
            size: 10.0,
            children: vec![leaf("top", "blk-top", false), inner, leaf("bot", "blk-bot", false)],
            data: None,
            ..Default::default()
        };

        let mut out = Vec::new();
        flatten_layout_nodes(&root, 0, "", &mut out);

        // Flat, pre-order, one entry per node.
        assert_eq!(out.len(), 6);
        assert_eq!(out[0]["id"], "root");
        assert_eq!(out[0]["depth"], 0);
        assert_eq!(out[0]["kind"], "branch");
        assert_eq!(out[0]["flex_direction"], "column");

        // Index path is positional, so a reader can locate a node in the tree.
        assert_eq!(out[1]["path"], "0");
        assert_eq!(out[2]["path"], "1");
        assert_eq!(out[3]["path"], "1.0");
        assert_eq!(out[3]["depth"], 2);

        // The inner Row is a BRANCH with no minimized flag of its own, but is
        // effectively minimized because every leaf under it is — the state
        // that drives chip geometry, and invisible from `locked` alone. This
        // is the distinction the cross-split bugs turned on.
        assert_eq!(out[2]["id"], "inner");
        assert_eq!(out[2]["locked"], false);
        assert_eq!(out[2]["effectively_minimized"], true);
        assert_eq!(out[2]["child_count"], 2);

        // Leaves carry their block id; the minimized ones report locked.
        assert_eq!(out[3]["block_id"], "blk-a");
        assert_eq!(out[3]["locked"], true);
        assert_eq!(out[1]["locked"], false);
        assert_eq!(out[1]["effectively_minimized"], false);
    }

    #[test]
    fn truncate_preview_is_shared_by_host_and_cross_channel_paths() {
        // Pins the fix for codex P2 on PR #2715 (cross-channel previews
        // originally skipped truncation entirely) at the unit-test level
        // — the handler-level concurrent-fetch behavior itself needs a
        // live second instance to exercise end-to-end, out of reach for
        // this module's test harness.
        let long = "y".repeat(PREVIEW_MAX_CHARS + 10);
        let truncated = truncate_preview(&long);
        assert_eq!(truncated.chars().count(), PREVIEW_MAX_CHARS + 1);
        assert!(truncated.ends_with('…'));

        let short = "short line";
        assert_eq!(truncate_preview(short), short);
    }
}

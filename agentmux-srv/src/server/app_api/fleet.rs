// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Fleet control — select, broadcast, and bulk-act on many agents at once.
//! See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.
//!
//! `fleet.broadcast`/`fleet.bulk-stop` are thin server-side loops over the
//! EXISTING single-target primitives (`ReactiveHandler::inject_message`,
//! `agent_io::stop_one_agent_block`) — no new transport is invented, and
//! every action returns `FleetActionResult` (succeeded/failed per target),
//! never a single bool, per the spec's §3 finding that silent partial
//! failure is the single most commonly-cited fleet-ops UX failure mode.
//!
//! `fleet.broadcast` is deliberately WS-RPC-only (the human/Swarm-UI path),
//! not exposed over HTTP to agentmux-mcp: each target is delivered via
//! `inject_message` with `source_agent: None` (self-declared, same trust
//! tier as e.g. the Slack/Discord bridges — a human broadcasting via the
//! UI isn't claiming to BE any agent, so no jekt signature is expected or
//! possible here). An AGENT-initiated broadcast instead loops the
//! EXISTING signed single-target `SendMessage` MCP tool path client-side
//! (see `agentmux-mcp`'s `FleetBroadcast` tool) — only the calling
//! agent's own process holds its `AGENTMUX_JEKT_KEY`, so per-message
//! signing can only happen there, never in a server-side batch RPC.
//! `fleet.bulk-stop` has no such constraint (a controller stop involves no
//! jekt signing at all, same as `agent.stop` today) and IS exposed over
//! HTTP (`POST /api/v1/fleet/bulk-stop`) for `FleetBulkStop`.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::reactive::types::InjectionRequest;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;

use super::AppState;
use super::agent_io::stop_one_agent_block;

/// `ReactiveHandler`'s injection rate limiter resets to `RATE_LIMIT_MAX`
/// (10, `backend/reactive/mod.rs`) once per full second — a token-bucket
/// hard reset, not smooth refill. A tight loop sending more than that many
/// injections within one second exhausts it, and every target past the
/// 10th in that window deterministically fails with "rate limit exceeded"
/// (reagent/Codex P1, PR #2687 review). Chunking to this size and pausing
/// just over a second between chunks keeps every chunk under the limiter's
/// own budget instead of racing it. Not imported directly — `agentmux-mcp`
/// is a separate process/crate with no dependency on `agentmux-srv`
/// internals, so its own client-side broadcast loop (`FleetBroadcast`,
/// `agentmux-mcp/src/main.rs`) mirrors this constant rather than sharing it;
/// keep both in sync if `RATE_LIMIT_MAX` ever changes.
const BROADCAST_CHUNK_SIZE: usize = 10;
const BROADCAST_CHUNK_PAUSE: Duration = Duration::from_millis(1100);

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_fleet_broadcast(engine, state);
    register_fleet_bulk_stop(engine, state);
    register_fleet_group_create(engine, state);
    register_fleet_group_list(engine, state);
    register_fleet_group_update(engine, state);
    register_fleet_group_delete(engine, state);
}

fn register_fleet_broadcast(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(
        COMMAND_FLEET_BROADCAST,
        move |cmd: CommandFleetBroadcastData, _ctx| {
            let state = state.clone();
            async move {
                let result = fleet_broadcast_impl(&state, cmd.targets, cmd.message, None).await;
                Ok(result)
            }
        },
    );
}

/// `source_agent` is `None` for the human/Swarm-UI path (self-declared
/// origin — see this module's doc comment); reserved for a future
/// server-side agent-initiated path should one ever be added, but nothing
/// in this codebase currently calls this with `Some(..)` (see the
/// module doc comment for why an agent-initiated broadcast instead loops
/// the client-side signed path).
///
/// Sends in chunks of `BROADCAST_CHUNK_SIZE`, pausing `BROADCAST_CHUNK_PAUSE`
/// between chunks — see that constant's doc comment for why a tight loop
/// would otherwise starve past `ReactiveHandler`'s own rate limiter.
pub(crate) async fn fleet_broadcast_impl(
    state: &AppState,
    targets: Vec<String>,
    message: String,
    source_agent: Option<String>,
) -> FleetActionResult {
    let mut result = FleetActionResult::default();
    for (chunk_idx, chunk) in targets.chunks(BROADCAST_CHUNK_SIZE).enumerate() {
        if chunk_idx > 0 {
            tokio::time::sleep(BROADCAST_CHUNK_PAUSE).await;
        }
        for block_id in chunk {
            let block_id = block_id.clone();
            let Some(agent) = state.reactive_handler.get_agent_by_block(&block_id) else {
                result.failed.push(FleetActionFailure {
                    id: block_id,
                    error: "no registered agent for this block (not a live agent pane, or not yet registered)".to_string(),
                });
                continue;
            };
            let req = InjectionRequest {
                target_agent: agent.agent_id.clone(),
                message: message.clone(),
                source_agent: source_agent.clone(),
                request_id: Some(uuid::Uuid::new_v4().to_string()),
                ..Default::default()
            };
            let resp = state.reactive_handler.inject_message(req);
            if resp.success {
                result.succeeded.push(block_id);
            } else {
                result.failed.push(FleetActionFailure {
                    id: block_id,
                    error: resp.error.unwrap_or_else(|| "delivery failed".to_string()),
                });
            }
        }
    }
    result
}

fn register_fleet_bulk_stop(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(
        COMMAND_FLEET_BULK_STOP,
        move |cmd: CommandFleetBulkStopData, _ctx| {
            let state = state.clone();
            async move {
                let result = fleet_bulk_stop_impl(&state, cmd.targets, cmd.signal.as_deref(), cmd.staged).await;
                Ok(result)
            }
        },
    );
}

const FLEET_BULK_STOP_AUDIT_ACTION: &str = "fleet.bulk-stop";

/// Stops `targets` (block ids) via the existing single-target
/// `stop_one_agent_block`, one call per target, and — unlike that
/// single-target primitive, which involves no jekt signing and so was
/// never audited — records one `AuditLogEntry` per target via
/// `ReactiveHandler::log_fleet_action_audit`, so Warden's Audit tab sees
/// every fleet-initiated stop (`SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`
/// §6; reagent/Codex P2, PR #2687 review — this was missing entirely).
/// Without `staged`, runs every target as a single batch (still returns
/// full per-target detail, never a bool). With `staged`, targets are
/// stopped `batch_size` at a time; if the failure rate WITHIN a completed
/// batch exceeds `max_fail_percentage`, remaining targets are recorded as
/// failed (untried) and `aborted_early` is set — caps blast radius on a
/// bad selection rather than plowing through every remaining target.
/// `aborted_early` is only ever set when targets were genuinely left
/// unattempted — tripping the threshold on the LAST batch (nothing left to
/// skip) must not report "aborted early" when the full list actually ran
/// (reagent P2, same review).
/// Look up `block_id` in the shared cross-channel registry (this host,
/// other channels — same registry `server/reactive.rs`'s inject cascade
/// tier-2b already reads) and forward a stop request over loopback HTTP to
/// that channel's own srv (`/agentmux/agent/stop`), using its own
/// `auth_key` from the registry entry — identical trust model to the
/// inject cascade's own cross-channel forward (same host, same user).
///
/// Returns `None` when `block_id` isn't in the shared registry at all, or
/// every matching entry is filtered out (not loopback, or a stale
/// self-entry pointing at THIS instance) — the caller falls through to the
/// normal local "not running" error in that case, same message as before
/// this feature existed. `Some((agent_name, outcome))` otherwise.
pub(crate) async fn forward_stop_to_shared_channel(
    state: &AppState,
    block_id: &str,
    signal: Option<&str>,
) -> Option<(String, Result<(), String>)> {
    let shared_dir = crate::registry::resolve_shared_reactive_dir()?;
    let entry = crate::backend::reactive::registry::list_all_shared(&shared_dir)
        .into_iter()
        .find(|e| e.block_id == block_id)?;

    let is_loopback = entry.local_url.starts_with("http://127.0.0.1")
        || entry.local_url.starts_with("http://localhost")
        || entry.local_url.starts_with("http://[::1]");
    if !is_loopback || entry.local_url == state.local_web_url {
        return None;
    }

    let url = format!("{}/agentmux/agent/stop", entry.local_url);
    let mut req = state.http_client.post(&url).json(&serde_json::json!({
        "block_id": block_id,
        "signal": signal,
    }));
    if !entry.auth_key.is_empty() {
        req = req.header("X-AuthKey", &entry.auth_key);
    }
    let outcome = async {
        let resp = req
            .send()
            .await
            .map_err(|e| format!("cross-channel forward failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("cross-channel forward: HTTP {}", resp.status()));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("cross-channel forward: response parse failed: {e}"))?;
        if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
            Ok(())
        } else {
            Err(body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("cross-channel forward failed")
                .to_string())
        }
    }
    .await;

    Some((entry.agent_id, outcome))
}

pub(crate) async fn fleet_bulk_stop_impl(
    state: &AppState,
    targets: Vec<String>,
    signal: Option<&str>,
    staged: Option<StagePlanInput>,
) -> FleetActionResult {
    let mut result = FleetActionResult::default();
    let batch_size = staged.as_ref().map(|s| s.batch_size.max(1)).unwrap_or(targets.len().max(1));
    let max_fail_percentage = staged.as_ref().map(|s| s.max_fail_percentage);

    let mut iter = targets.into_iter().peekable();
    'batches: while iter.peek().is_some() {
        let batch: Vec<String> = (&mut iter).take(batch_size).collect();
        let batch_len = batch.len();
        let mut batch_failures = 0usize;
        for block_id in batch {
            let request_id = uuid::Uuid::new_v4().to_string();
            // Local (this instance's own in-process controller registry)
            // first — unchanged, fast path. Only reach for the shared
            // cross-channel registry when nothing local matches
            // (REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md
            // §3.2 — this was host-tier-only before).
            let (target_agent, outcome) = match state.reactive_handler.get_agent_by_block(&block_id) {
                Some(agent) => (agent.agent_id, stop_one_agent_block(&block_id, signal).map(|_| ())),
                None => match forward_stop_to_shared_channel(state, &block_id, signal).await {
                    Some((agent_name, outcome)) => (agent_name, outcome),
                    None => (block_id.clone(), stop_one_agent_block(&block_id, signal).map(|_| ())),
                },
            };
            match outcome {
                Ok(()) => {
                    state.reactive_handler.log_fleet_action_audit(
                        None, &target_agent, &block_id, FLEET_BULK_STOP_AUDIT_ACTION,
                        true, None, &request_id, None,
                    );
                    result.succeeded.push(block_id);
                }
                Err(e) => {
                    state.reactive_handler.log_fleet_action_audit(
                        None, &target_agent, &block_id, FLEET_BULK_STOP_AUDIT_ACTION,
                        false, Some(&e), &request_id, None,
                    );
                    batch_failures += 1;
                    result.failed.push(FleetActionFailure { id: block_id, error: e });
                }
            }
        }
        if let Some(max_pct) = max_fail_percentage {
            if exceeds_fail_threshold(batch_failures, batch_len, max_pct) {
                // Remaining, untried targets are recorded as failed so the
                // caller's succeeded+failed count always equals the
                // original target count — never a silently-dropped subset.
                let mut skipped_any = false;
                for remaining in iter {
                    skipped_any = true;
                    result.failed.push(FleetActionFailure {
                        id: remaining,
                        error: "skipped: staged rollout aborted after a prior batch's failure rate exceeded max_fail_percentage".to_string(),
                    });
                }
                // Only a genuine early abort if something was actually left
                // unattempted — tripping the threshold on the final batch
                // ran the whole list, so it isn't "early" at all.
                result.aborted_early = skipped_any;
                break 'batches;
            }
        }
    }
    result
}

/// `FleetBulkStop` from an agent (the HTTP path; the Swarm UI's own
/// `fleet.bulk-stop` is the user and stops at once): every target running on
/// this instance waits for its user's 15 s override, all windows in parallel
/// (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). Targets that aren't
/// (another channel's, or not running) go through `fleet_bulk_stop_impl` as
/// before, `staged` included.
pub(crate) async fn fleet_bulk_stop_with_override(
    state: &AppState,
    by: &str,
    targets: Vec<String>,
    signal: Option<&str>,
    staged: Option<StagePlanInput>,
) -> (FleetActionResult, Vec<crate::sagas::pending_shutdown::PendingView>) {
    let (local, other): (Vec<String>, Vec<String>) = targets
        .into_iter()
        .partition(|b| crate::backend::blockcontroller::get_controller(b).is_some());
    let pending = local
        .iter()
        .map(|block_id| {
            let mut v = crate::sagas::pending_shutdown::request(
                state,
                block_id,
                by,
                "FleetBulkStop",
                "",
                crate::sagas::pending_shutdown::Action::Stop { signal: signal.map(str::to_string) },
            );
            // A target can join a request opened for another block (its
            // pane's close, via a sibling tab): audit and report it under the
            // block this caller named (ReAgent P1 on #3798), with that
            // request's id.
            let target = state
                .reactive_handler
                .get_agent_by_block(block_id)
                .map(|a| a.agent_id)
                .unwrap_or_else(|| block_id.clone());
            state.reactive_handler.log_fleet_action_audit(
                Some(by), &target, block_id, FLEET_BULK_STOP_AUDIT_ACTION,
                true, None, &v.request_id, Some(&crate::sagas::pending_shutdown::audit_note(&v, by, "FleetBulkStop")),
            );
            v.block_id = block_id.clone();
            v.target = target;
            v
        })
        .collect();
    let now = if other.is_empty() {
        FleetActionResult::default()
    } else {
        fleet_bulk_stop_impl(state, other, signal, staged).await
    };
    (now, pending)
}

/// `batch_failures / batch_len` (as a percentage) exceeds `max_pct`.
/// Cross-multiplies instead of computing a truncated integer percentage
/// first — `batch_failures * 100 / batch_len` rounds DOWN before
/// comparing, so e.g. 1 failure in a batch of 3 (33.3%) reads as 33 and
/// never exceeds `max_pct = 33`, silently missing the threshold it was
/// meant to catch (Codex P2, PR #2687 review). `batch_failures * 100 >
/// max_pct * batch_len` is the same inequality with no rounding.
fn exceeds_fail_threshold(batch_failures: usize, batch_len: usize, max_pct: u8) -> bool {
    batch_failures * 100 > max_pct as usize * batch_len
}

#[cfg(test)]
mod threshold_tests {
    use super::exceeds_fail_threshold;

    #[test]
    fn one_in_three_at_33_percent_threshold_now_trips() {
        // 1/3 = 33.33...% — the old truncated-integer comparison rounded
        // this down to exactly 33 and never exceeded max_pct=33. The real
        // rate DOES exceed 33%, so this must trip.
        assert!(exceeds_fail_threshold(1, 3, 33));
    }

    #[test]
    fn one_in_three_at_34_percent_threshold_does_not_trip() {
        // 33.33% does not exceed a 34% threshold.
        assert!(!exceeds_fail_threshold(1, 3, 34));
    }

    #[test]
    fn zero_failures_never_trips() {
        assert!(!exceeds_fail_threshold(0, 10, 0));
    }

    #[test]
    fn all_failures_trips_any_threshold_below_100() {
        assert!(exceeds_fail_threshold(5, 5, 99));
        assert!(!exceeds_fail_threshold(5, 5, 100));
    }

    #[test]
    fn two_in_seven_at_28_percent_threshold() {
        // 2/7 = 28.57...% — exceeds a 28% threshold, does not exceed 29%.
        assert!(exceeds_fail_threshold(2, 7, 28));
        assert!(!exceeds_fail_threshold(2, 7, 29));
    }
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

fn register_fleet_group_create(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_FLEET_GROUP_CREATE,
        move |cmd: CommandFleetGroupCreateData, _ctx| {
            let mstore = mstore.clone();
            async move {
                if cmd.name.trim().is_empty() {
                    return Err("fleet.group.create: name is required".to_string());
                }
                let id = uuid::Uuid::new_v4().to_string();
                let created_at = now_ms();
                mstore
                    .agent_group_create(&id, cmd.name.trim(), &cmd.member_ids, created_at)
                    .map_err(|e| format!("fleet.group.create: {e}"))?;
                Ok(FleetGroup {
                    id,
                    name: cmd.name.trim().to_string(),
                    member_ids: cmd.member_ids,
                    created_at,
                })
            }
        },
    );
}

fn register_fleet_group_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_FLEET_GROUP_LIST,
        move |_req: CommandFleetGroupListData, _ctx| {
            let mstore = mstore.clone();
            async move {
                let groups = mstore.agent_group_list().map_err(|e| format!("fleet.group.list: {e}"))?;
                let groups = groups
                    .into_iter()
                    .map(|g| FleetGroup { id: g.id, name: g.name, member_ids: g.member_ids, created_at: g.created_at })
                    .collect();
                Ok(FleetGroupListResult { groups })
            }
        },
    );
}

fn register_fleet_group_update(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_FLEET_GROUP_UPDATE,
        move |cmd: CommandFleetGroupUpdateData, _ctx| {
            let mstore = mstore.clone();
            async move {
                let name = cmd.name.as_deref().map(|n| n.trim());
                if matches!(name, Some("")) {
                    return Err("fleet.group.update: name cannot be blank".to_string());
                }
                let updated = mstore
                    .agent_group_update(&cmd.id, name, cmd.member_ids.as_deref())
                    .map_err(|e| format!("fleet.group.update: {e}"))?;
                if !updated {
                    return Err(format!("fleet.group.update: no group with id {}", cmd.id));
                }
                let group = mstore
                    .agent_group_get(&cmd.id)
                    .map_err(|e| format!("fleet.group.update: {e}"))?
                    .ok_or_else(|| format!("fleet.group.update: group {} vanished after update", cmd.id))?;
                Ok(FleetGroup {
                    id: group.id,
                    name: group.name,
                    member_ids: group.member_ids,
                    created_at: group.created_at,
                })
            }
        },
    );
}

fn register_fleet_group_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_FLEET_GROUP_DELETE,
        move |cmd: CommandFleetGroupDeleteData, _ctx| {
            let mstore = mstore.clone();
            async move {
                let deleted = mstore.agent_group_delete(&cmd.id).map_err(|e| format!("fleet.group.delete: {e}"))?;
                Ok(FleetGroupDeleteResult { ok: deleted })
            }
        },
    );
}

#[cfg(test)]
mod override_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A running agent that records whether it was stopped.
    struct Stoppable {
        block_id: String,
        stopped: Arc<AtomicBool>,
    }

    impl crate::backend::blockcontroller::Controller for Stoppable {
        fn start(&self, _: crate::backend::obj::MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self, _: bool, _: &str) -> Result<(), String> {
            self.stopped.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn get_runtime_status(&self) -> crate::backend::blockcontroller::BlockControllerRuntimeStatus {
            crate::backend::blockcontroller::BlockControllerRuntimeStatus { blockid: self.block_id.clone(), ..Default::default() }
        }
        fn send_input(&self, _: crate::backend::blockcontroller::BlockInputUnion, _: Option<u64>) -> Result<(), String> {
            Ok(())
        }
        fn controller_type(&self) -> &str {
            "stub"
        }
        fn block_id(&self) -> &str {
            &self.block_id
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn running(prefix: &str) -> (String, Arc<AtomicBool>) {
        let block_id = format!("{prefix}-{}", uuid::Uuid::new_v4());
        let stopped = Arc::new(AtomicBool::new(false));
        crate::backend::blockcontroller::register_controller(
            &block_id,
            Arc::new(Stoppable { block_id: block_id.clone(), stopped: stopped.clone() }),
        );
        (block_id, stopped)
    }

    #[tokio::test]
    async fn an_agent_s_bulk_stop_waits_for_each_user_and_stops_when_nobody_keeps_it() {
        let state = crate::server::tests::test_state();
        let (a, a_stopped) = running("bulk-override-a");
        let (b, b_stopped) = running("bulk-override-b");
        let gone = format!("bulk-override-gone-{}", uuid::Uuid::new_v4());

        let (now, pending) =
            fleet_bulk_stop_with_override(&state, "Korp", vec![a.clone(), b.clone(), gone.clone()], None, None).await;
        assert_eq!(now.failed.iter().map(|f| f.id.clone()).collect::<Vec<_>>(), vec![gone], "not running: fails at once");
        assert_eq!(pending.iter().map(|p| p.block_id.clone()).collect::<Vec<_>>(), vec![a.clone(), b.clone()]);
        assert!(pending.iter().all(|p| p.by == "Korp" && p.via == "FleetBulkStop"));
        assert!(!a_stopped.load(Ordering::SeqCst) && !b_stopped.load(Ordering::SeqCst), "nothing stops during the window");

        // The user keeps b; a's window runs out.
        assert_eq!(crate::sagas::pending_shutdown::keep(&state, &b, &pending[1].request_id), "kept_by_user");
        crate::sagas::pending_shutdown::expire_now(&pending[0].request_id);
        assert_eq!(crate::sagas::pending_shutdown::settled(&pending[0].request_id).await.status, "shut_down");
        assert_eq!(crate::sagas::pending_shutdown::settled(&pending[1].request_id).await.status, "kept_by_user");
        assert!(a_stopped.load(Ordering::SeqCst));
        assert!(!b_stopped.load(Ordering::SeqCst), "the user kept it");
        crate::backend::blockcontroller::delete_controller(&a);
        crate::backend::blockcontroller::delete_controller(&b);
    }

    #[tokio::test]
    async fn a_target_that_joins_its_pane_s_pending_close_is_reported_under_its_own_block() {
        let state = crate::server::tests::test_state();
        let (tab, _) = running("bulk-join-tab");
        let sibling = format!("bulk-join-sibling-{}", uuid::Uuid::new_v4());
        let close = crate::sagas::pending_shutdown::request(
            &state,
            &sibling,
            "Korp",
            "ClosePane",
            "",
            crate::sagas::pending_shutdown::Action::ClosePane { block_ids: vec![sibling.clone(), tab.clone()] },
        );

        let (_, pending) = fleet_bulk_stop_with_override(&state, "Posa", vec![tab.clone()], None, None).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].block_id, tab, "the block the caller named, not the sibling");
        assert_eq!(pending[0].request_id, close.request_id, "the pane's one window");
        assert!(pending[0].joined);
        crate::sagas::pending_shutdown::keep(&state, &sibling, &close.request_id);
        crate::backend::blockcontroller::delete_controller(&tab);
    }
}

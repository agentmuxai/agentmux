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

use crate::backend::reactive::types::InjectionRequest;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;

use super::AppState;
use super::agent_io::stop_one_agent_block;

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
    engine.register_handler(
        COMMAND_FLEET_BROADCAST,
        Box::new(move |data, _ctx| {
            let state = state.clone();
            Box::pin(async move {
                let cmd: CommandFleetBroadcastData = serde_json::from_value(data)
                    .map_err(|e| format!("fleet.broadcast: {e}"))?;
                let result = fleet_broadcast_impl(&state, cmd.targets, cmd.message, None);
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );
}

/// `source_agent` is `None` for the human/Swarm-UI path (self-declared
/// origin — see this module's doc comment); reserved for a future
/// server-side agent-initiated path should one ever be added, but nothing
/// in this codebase currently calls this with `Some(..)` (see the
/// module doc comment for why an agent-initiated broadcast instead loops
/// the client-side signed path).
pub(crate) fn fleet_broadcast_impl(
    state: &AppState,
    targets: Vec<String>,
    message: String,
    source_agent: Option<String>,
) -> FleetActionResult {
    let mut result = FleetActionResult::default();
    for block_id in targets {
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
    result
}

fn register_fleet_bulk_stop(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_handler(
        COMMAND_FLEET_BULK_STOP,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let cmd: CommandFleetBulkStopData = serde_json::from_value(data)
                    .map_err(|e| format!("fleet.bulk-stop: {e}"))?;
                let result = fleet_bulk_stop_impl(cmd.targets, cmd.signal.as_deref(), cmd.staged);
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );
}

/// Stops `targets` (block ids) via the existing single-target
/// `stop_one_agent_block`, one call per target. Without `staged`, runs
/// every target as a single batch (still returns full per-target detail,
/// never a bool). With `staged`, targets are stopped `batch_size` at a
/// time; if the failure rate WITHIN a completed batch exceeds
/// `max_fail_percentage`, remaining targets are recorded as failed
/// (untried) and `aborted_early` is set — caps blast radius on a bad
/// selection rather than plowing through every remaining target.
pub(crate) fn fleet_bulk_stop_impl(
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
            match stop_one_agent_block(&block_id, signal) {
                Ok(_) => result.succeeded.push(block_id),
                Err(e) => {
                    batch_failures += 1;
                    result.failed.push(FleetActionFailure { id: block_id, error: e });
                }
            }
        }
        if let Some(max_pct) = max_fail_percentage {
            let batch_fail_pct = (batch_failures * 100) / batch_len.max(1);
            if batch_fail_pct as u8 > max_pct {
                // Remaining, untried targets are recorded as failed so the
                // caller's succeeded+failed count always equals the
                // original target count — never a silently-dropped subset.
                for remaining in iter {
                    result.failed.push(FleetActionFailure {
                        id: remaining,
                        error: "skipped: staged rollout aborted after a prior batch's failure rate exceeded max_fail_percentage".to_string(),
                    });
                }
                result.aborted_early = true;
                break 'batches;
            }
        }
    }
    result
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn register_fleet_group_create(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_FLEET_GROUP_CREATE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandFleetGroupCreateData = serde_json::from_value(data)
                    .map_err(|e| format!("fleet.group.create: {e}"))?;
                if cmd.name.trim().is_empty() {
                    return Err("fleet.group.create: name is required".to_string());
                }
                let id = uuid::Uuid::new_v4().to_string();
                let created_at = now_ms();
                wstore
                    .agent_group_create(&id, cmd.name.trim(), &cmd.member_ids, created_at)
                    .map_err(|e| format!("fleet.group.create: {e}"))?;
                Ok(Some(serde_json::to_value(&FleetGroup {
                    id,
                    name: cmd.name.trim().to_string(),
                    member_ids: cmd.member_ids,
                    created_at,
                }).unwrap()))
            })
        }),
    );
}

fn register_fleet_group_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_FLEET_GROUP_LIST,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let groups = wstore.agent_group_list().map_err(|e| format!("fleet.group.list: {e}"))?;
                let groups = groups
                    .into_iter()
                    .map(|g| FleetGroup { id: g.id, name: g.name, member_ids: g.member_ids, created_at: g.created_at })
                    .collect();
                Ok(Some(serde_json::to_value(&FleetGroupListResult { groups }).unwrap()))
            })
        }),
    );
}

fn register_fleet_group_update(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_FLEET_GROUP_UPDATE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandFleetGroupUpdateData = serde_json::from_value(data)
                    .map_err(|e| format!("fleet.group.update: {e}"))?;
                let name = cmd.name.as_deref().map(|n| n.trim());
                if matches!(name, Some("")) {
                    return Err("fleet.group.update: name cannot be blank".to_string());
                }
                let updated = wstore
                    .agent_group_update(&cmd.id, name, cmd.member_ids.as_deref())
                    .map_err(|e| format!("fleet.group.update: {e}"))?;
                if !updated {
                    return Err(format!("fleet.group.update: no group with id {}", cmd.id));
                }
                let group = wstore
                    .agent_group_get(&cmd.id)
                    .map_err(|e| format!("fleet.group.update: {e}"))?
                    .ok_or_else(|| format!("fleet.group.update: group {} vanished after update", cmd.id))?;
                Ok(Some(serde_json::to_value(&FleetGroup {
                    id: group.id,
                    name: group.name,
                    member_ids: group.member_ids,
                    created_at: group.created_at,
                }).unwrap()))
            })
        }),
    );
}

fn register_fleet_group_delete(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_FLEET_GROUP_DELETE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandFleetGroupDeleteData = serde_json::from_value(data)
                    .map_err(|e| format!("fleet.group.delete: {e}"))?;
                let deleted = wstore.agent_group_delete(&cmd.id).map_err(|e| format!("fleet.group.delete: {e}"))?;
                Ok(Some(serde_json::json!({ "ok": deleted })))
            })
        }),
    );
}

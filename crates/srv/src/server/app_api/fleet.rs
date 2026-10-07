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
//! not exposed over HTTP to agentmux-mcp. It is NOT a jekt: a broadcast is the
//! human typing once to several panes, so each target gets a message routed the
//! way an inter-agent message is (`bootstrap::route_agent_message`: persistent, ACP
//! and App Server on their own channel, a subprocess or unspawned agent via
//! `run_agent_turn`) with a one-line `[BROADCAST:FROM=user VIA=swarm ...]` header
//! from `reactive::broadcast_turn_message`, instead of a `self-declared` jekt from
//! `unknown`. See `docs/specs/SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md`.
//! The turn is `BROADCAST_TURN_ORIGIN` (`User`): a broadcast is the human's own
//! message, with the same powers as typing it into each pane, including
//! authorizing an agent's self-quit (owner decision 2026-10-06, spec §6 q2).
//! That adds no power to anything holding the pane key, which could already send
//! any pane a plain `User` turn through `agent.input` (spec §2 finding 7, §4.2).
//! srv, not the text, still decides: the label comes from this module only.
//! A target running on another channel on this machine is reached over loopback,
//! the way a stop is (`forward_broadcast_to_channel`): the sender passes only the
//! target block, the body, the `MSGID` and the `RECIPIENTS` count, and the
//! receiving instance builds the header and starts the turn itself
//! (`deliver_forwarded_broadcast`). Agents on other machines are not reachable
//! this way: nothing on the LAN proves who is asking
//! (docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §6).
//! The forward uses the same loopback auth as the stop forward, and reaches only
//! what that channel's own `fleet.broadcast` already does.
//! An AGENT-initiated broadcast instead loops the EXISTING signed single-target
//! `SendMessage` MCP tool path client-side (see `agentmux-mcp`'s `FleetBroadcast`
//! tool) — only the calling agent's own process holds its `AGENTMUX_JEKT_KEY`, so
//! per-message signing can only happen there, never in a server-side batch RPC.
//! `fleet.bulk-stop` has no such constraint (a controller stop involves no
//! jekt signing at all, same as `agent.stop` today) and IS exposed over
//! HTTP (`POST /api/v1/fleet/bulk-stop`) for `FleetBulkStop`.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::blockcontroller::health::TurnOrigin;
use crate::backend::reactive::broadcast_turn_message;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;

use super::AppState;
use super::agent_io::stop_one_agent_block;
use crate::server::agent_handlers::{run_agent_turn, AgentTurnDeps, TurnRegistration};

/// Every broadcast turn is attributed here, and only here: `User` — see the
/// module doc. A test pins both this value and that `run_broadcast` hands it to
/// the delivery function. It was `Automated` until 2026-10-06.
pub(crate) const BROADCAST_TURN_ORIGIN: TurnOrigin = TurnOrigin::User;

const FLEET_BROADCAST_AUDIT_ACTION: &str = "fleet.broadcast";

/// Turns started at once. Delivery is not rate-limited the way `inject_message`
/// was (which is why the old 10-per-second chunking is gone), but a pane's turn
/// can spawn a CLI, so a broadcast to a large selection is bounded rather than
/// starting every turn in the same instant.
const BROADCAST_CONCURRENCY: usize = 8;

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
                let result = fleet_broadcast_impl(&state, cmd.targets, cmd.message).await;
                Ok(result)
            }
        },
    );
}

/// What happened to one target of a broadcast.
pub(crate) struct BroadcastOutcome {
    pub block_id: String,
    /// The agent the block resolved to; `None` when nothing is registered for it.
    pub agent_id: Option<String>,
    pub result: Result<(), String>,
}

const NO_REGISTERED_AGENT: &str =
    "no registered agent for this block (not a live agent pane, or not yet registered)";

/// The broadcast logic, with srv's two touch points passed in so it can be tested
/// without an `AppState`: `resolve` maps a block id to its agent id, and `deliver`
/// starts one turn (block id, full text, origin).
///
/// Targets are resolved first so `RECIPIENTS` in every header counts the agents
/// actually addressed; the same `msg_id` ties the copies together in the audit
/// log. Turns start up to `concurrency` at a time, and results come back in the
/// order of `targets`.
pub(crate) async fn run_broadcast<R, D, Fut>(
    targets: Vec<String>,
    body: &str,
    msg_id: &str,
    concurrency: usize,
    resolve: R,
    deliver: D,
) -> Vec<BroadcastOutcome>
where
    R: Fn(&str) -> Option<String>,
    D: Fn(String, String, TurnOrigin) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    use futures_util::StreamExt;

    let resolved: Vec<(String, Option<String>)> = targets
        .into_iter()
        .map(|block_id| {
            let agent = resolve(&block_id);
            (block_id, agent)
        })
        .collect();
    let recipients = resolved.iter().filter(|(_, agent)| agent.is_some()).count();

    futures_util::stream::iter(resolved)
        .map(|(block_id, agent_id)| {
            let deliver = &deliver;
            async move {
                let result = match &agent_id {
                    None => Err(NO_REGISTERED_AGENT.to_string()),
                    Some(agent) => {
                        let text = broadcast_turn_message(agent, recipients, msg_id, body);
                        deliver(block_id.clone(), text, BROADCAST_TURN_ORIGIN).await
                    }
                };
                BroadcastOutcome { block_id, agent_id, result }
            }
        })
        .buffered(concurrency.max(1))
        .collect()
        .await
}

/// What to do with one target, given how its controller routes a message.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BroadcastAction {
    /// Taken on a structured channel (now or queued).
    Done,
    /// Start a turn.
    StartTurn,
    Fail(String),
}

/// Maps `route_agent_message`'s answer to an action. A PTY-based target is
/// refused rather than typed into: a broadcast is prose, and keystrokes into a
/// shell prompt would run it as a command.
pub(crate) fn broadcast_action(route: Result<crate::bootstrap::AgentRoute, String>) -> BroadcastAction {
    use crate::backend::reactive::SenderDelivery;
    use crate::bootstrap::AgentRoute;
    match route {
        Ok(AgentRoute::Sent(SenderDelivery::Delivered | SenderDelivery::Deferred)) => BroadcastAction::Done,
        Ok(AgentRoute::Sent(SenderDelivery::Pty)) => BroadcastAction::Fail(
            "this pane is a terminal, not an agent session; a broadcast is not typed into a shell".to_string(),
        ),
        Ok(AgentRoute::StartTurn { .. }) => BroadcastAction::StartTurn,
        Err(e) => BroadcastAction::Fail(e),
    }
}

/// Delivers one broadcast text to one block the way an inter-agent message reaches
/// it (`route_agent_message_from`): structured controllers (persistent, ACP, App
/// Server) take it on their own channel, which also steers a turn already running;
/// a subprocess or not-yet-spawned persistent agent gets a turn started with
/// `origin`. Either way the turn is labelled with `origin` where the controller
/// tracks provenance at all (persistent and subprocess, the same agents a typed
/// message can authorize a self-quit on).
async fn deliver_broadcast_turn(
    deps: &AgentTurnDeps,
    block_id: String,
    text: String,
    origin: TurnOrigin,
) -> Result<(), String> {
    let input = crate::backend::blockcontroller::health::TurnInput { origin, text: text.clone() };
    match broadcast_action(crate::bootstrap::route_agent_message_from(&block_id, &text, Some(input))) {
        BroadcastAction::Done => Ok(()),
        BroadcastAction::Fail(e) => Err(e),
        // The block was resolved through the reactive handler's own agent map, so
        // it is already registered: skip re-registering, as the reactive-delivery
        // caller does.
        BroadcastAction::StartTurn => {
            run_agent_turn(deps, block_id, text, None, TurnRegistration::Skip, origin, Vec::new()).await
        }
    }
}

/// How one broadcast target is reached.
enum BroadcastRoute {
    /// An agent pane of this instance.
    Local(String),
    /// An agent on another channel on this machine, via its own srv.
    Channel(crate::backend::reactive::registry::AgentEntry),
}

impl BroadcastRoute {
    fn agent_id(&self) -> &str {
        match self {
            BroadcastRoute::Local(agent) => agent,
            BroadcastRoute::Channel(entry) => &entry.agent_id,
        }
    }
}

/// What a forwarded broadcast carries. No header text: the receiving instance
/// writes the header, so a caller on loopback cannot supply its own.
pub(crate) struct BroadcastForward<'a> {
    pub block_id: &'a str,
    pub body: &'a str,
    pub msg_id: &'a str,
    /// Agents addressed on every machine and channel, for the header.
    pub recipients: usize,
}

/// Asks the instance at `local_url` (another channel on this machine, its
/// `auth_key` from the shared registry, same user) to deliver one broadcast.
/// A channel from before the route answers 404 and is named as such, for that
/// target only.
pub(crate) async fn forward_broadcast_to_channel(
    client: &reqwest::Client,
    local_url: &str,
    auth_key: &str,
    req: &BroadcastForward<'_>,
) -> Result<(), String> {
    let mut http = client
        .post(format!("{local_url}/agentmux/agent/broadcast"))
        .json(&serde_json::json!({
            "block_id": req.block_id,
            "body": req.body,
            "msg_id": req.msg_id,
            "recipients": req.recipients,
        }));
    if !auth_key.is_empty() {
        http = http.header("X-AuthKey", auth_key);
    }
    let resp = http.send().await.map_err(|e| format!("cross-channel forward failed: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("that channel is on an older build and can't receive a broadcast from here".to_string());
    }
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
        Err(body.get("error").and_then(|v| v.as_str()).unwrap_or("cross-channel forward failed").to_string())
    }
}

/// The receiving end of `forward_broadcast_to_channel`: builds the header here,
/// from the target's own agent id, and delivers it as any broadcast turn
/// (`BROADCAST_TURN_ORIGIN`). Audited on this side too, naming where it came
/// from.
pub(crate) async fn deliver_forwarded_broadcast(
    state: &AppState,
    block_id: &str,
    body: &str,
    msg_id: &str,
    recipients: usize,
) -> Result<(), String> {
    let result = match state.reactive_handler.get_agent_by_block(block_id) {
        None => Err(NO_REGISTERED_AGENT.to_string()),
        Some(agent) => {
            let text = broadcast_turn_message(&agent.agent_id, recipients, msg_id, body);
            let deps = AgentTurnDeps::from_state(state);
            deliver_broadcast_turn(&deps, block_id.to_string(), text, BROADCAST_TURN_ORIGIN).await
        }
    };
    let target = state
        .reactive_handler
        .get_agent_by_block(block_id)
        .map(|a| a.agent_id)
        .unwrap_or_else(|| block_id.to_string());
    state.reactive_handler.log_fleet_action_audit(
        None,
        &target,
        block_id,
        FLEET_BROADCAST_AUDIT_ACTION,
        result.is_ok(),
        result.as_ref().err().map(String::as_str),
        msg_id,
        Some("forwarded from another channel on this machine"),
    );
    result
}

pub(crate) async fn fleet_broadcast_impl(
    state: &AppState,
    targets: Vec<String>,
    message: String,
) -> FleetActionResult {
    let deps = AgentTurnDeps::from_state(state);
    let msg_id = uuid::Uuid::new_v4().to_string();
    // This instance's own panes first, then the machine's other channels. A
    // block known to neither has no route and fails, as before.
    let routes: std::collections::HashMap<String, BroadcastRoute> = targets
        .iter()
        .filter_map(|block_id| {
            let route = match state.reactive_handler.get_agent_by_block(block_id) {
                Some(agent) => BroadcastRoute::Local(agent.agent_id),
                None => BroadcastRoute::Channel(shared_channel_for(state, block_id)?),
            };
            Some((block_id.clone(), route))
        })
        .collect();
    let routes = Arc::new(routes);
    // The same count `run_broadcast` puts in the local headers: every target
    // that resolved, here or on another channel.
    let recipients = targets.iter().filter(|b| routes.contains_key(*b)).count();
    let outcomes = run_broadcast(
        targets,
        &message,
        &msg_id,
        BROADCAST_CONCURRENCY,
        |block_id| routes.get(block_id).map(|r| r.agent_id().to_string()),
        |block_id, text, origin| {
            let deps = deps.clone();
            let routes = routes.clone();
            let client = state.http_client.clone();
            let (body, msg_id) = (message.clone(), msg_id.clone());
            async move {
                match routes.get(&block_id) {
                    Some(BroadcastRoute::Channel(entry)) => {
                        let forward = BroadcastForward { block_id: &block_id, body: &body, msg_id: &msg_id, recipients };
                        forward_broadcast_to_channel(&client, &entry.local_url, &entry.auth_key, &forward).await
                    }
                    _ => deliver_broadcast_turn(&deps, block_id, text, origin).await,
                }
            }
        },
    )
    .await;

    let mut result = FleetActionResult::default();
    for outcome in outcomes {
        let target_agent = outcome.agent_id.as_deref().unwrap_or(&outcome.block_id);
        match outcome.result {
            Ok(()) => {
                state.reactive_handler.log_fleet_action_audit(
                    None, target_agent, &outcome.block_id, FLEET_BROADCAST_AUDIT_ACTION,
                    true, None, &msg_id, None,
                );
                result.succeeded.push(outcome.block_id);
            }
            Err(error) => {
                state.reactive_handler.log_fleet_action_audit(
                    None, target_agent, &outcome.block_id, FLEET_BROADCAST_AUDIT_ACTION,
                    false, Some(&error), &msg_id, None,
                );
                result.failed.push(FleetActionFailure { id: outcome.block_id, error });
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
    let entry = shared_channel_for(state, block_id)?;

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
                Some(agent) => (agent.agent_id, stop_one_agent_block(state, &block_id, signal, None).await.map(|_| ())),
                None => match forward_stop_to_shared_channel(state, &block_id, signal).await {
                    Some((agent_name, outcome)) => (agent_name, outcome),
                    None => (block_id.clone(), stop_one_agent_block(state, &block_id, signal, None).await.map(|_| ())),
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

/// The shared cross-channel registry entry for `block_id` on ANOTHER
/// instance on this machine: loopback only, never this instance itself.
fn shared_channel_for(state: &AppState, block_id: &str) -> Option<crate::backend::reactive::registry::AgentEntry> {
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
    Some(entry)
}

/// What asking another instance's user came to.
enum Forwarded {
    /// That instance opened (or joined) its own override window.
    Pending(crate::sagas::pending_shutdown::PendingView),
    /// That instance predates the window: stopped at once, as before.
    Immediate(Result<(), String>),
    /// It answered, but couldn't take the request (e.g. not running there).
    Refused(String),
}

/// A cross-channel `FleetBulkStop` target: ask ITS instance's user
/// (`/agentmux/agent/stop-pending`), whose banner, chime and Keep running
/// are the ones that user sees. The request's status is served from there;
/// `pending_shutdown::remember_remote` lets this instance proxy it. An
/// instance without that route (404) gets the plain forward it always did.
async fn forward_stop_pending(
    state: &AppState,
    entry: &crate::backend::reactive::registry::AgentEntry,
    block_id: &str,
    by: &str,
    signal: Option<&str>,
) -> Forwarded {
    let url = format!("{}/agentmux/agent/stop-pending", entry.local_url);
    let mut req = state.http_client.post(&url).json(&serde_json::json!({
        "block_id": block_id,
        "signal": signal,
        "by": by,
    }));
    if !entry.auth_key.is_empty() {
        req = req.header("X-AuthKey", &entry.auth_key);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Forwarded::Refused(format!("cross-channel forward failed: {e}")),
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        let outcome = forward_stop_to_shared_channel(state, block_id, signal)
            .await
            .map(|(_, o)| o)
            .unwrap_or_else(|| Err("cross-channel forward: target vanished".to_string()));
        return Forwarded::Immediate(outcome);
    }
    if !resp.status().is_success() {
        return Forwarded::Refused(format!("cross-channel forward: HTTP {}", resp.status()));
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return Forwarded::Refused(format!("cross-channel forward: response parse failed: {e}")),
    };
    if body["success"] != true {
        return Forwarded::Refused(body["error"].as_str().unwrap_or("cross-channel forward failed").to_string());
    }
    let p = &body["pending"];
    let Some(request_id) = p["request_id"].as_str() else {
        return Forwarded::Refused("cross-channel forward: no request id".to_string());
    };
    crate::sagas::pending_shutdown::remember_remote(request_id, &entry.local_url, &entry.auth_key);
    Forwarded::Pending(crate::sagas::pending_shutdown::PendingView {
        request_id: request_id.to_string(),
        block_id: block_id.to_string(),
        by: p["by"].as_str().unwrap_or(by).to_string(),
        target: entry.agent_id.clone(),
        via: p["via"].as_str().unwrap_or("FleetBulkStop").to_string(),
        reason: String::new(),
        deadline_ms: p["deadline_ms"].as_i64().unwrap_or_default(),
        status: "pending",
        error: None,
        joined: p["joined"].as_bool().unwrap_or(false),
    })
}

/// `FleetBulkStop` from an agent (the HTTP path; the Swarm UI's own
/// `fleet.bulk-stop` is the user and stops at once): every target running on
/// this instance waits for its user's 15 s override, all windows in parallel
/// (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). A target running on
/// another instance on this machine is asked there, in that instance's own
/// window (`forward_stop_pending`). Targets running nowhere go through
/// `fleet_bulk_stop_impl` as before, `staged` included.
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
    let mut pending: Vec<crate::sagas::pending_shutdown::PendingView> = local
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
    let mut now = FleetActionResult::default();
    let mut remote_pending = Vec::new();
    let mut nowhere = Vec::new();
    // Local windows are open (above); now ask the other instances, all at
    // once, so one slow peer delays neither the rest nor any countdown
    // (ReAgent P1 on #3824).
    let mut forwards = Vec::new();
    for block_id in other {
        match shared_channel_for(state, &block_id) {
            Some(entry) => forwards.push((block_id, entry)),
            None => nowhere.push(block_id),
        }
    }
    let answers = futures_util::future::join_all(
        forwards.iter().map(|(block_id, entry)| forward_stop_pending(state, entry, block_id, by, signal)),
    )
    .await;
    for ((block_id, entry), answer) in forwards.into_iter().zip(answers) {
        let request_id = uuid::Uuid::new_v4().to_string();
        match answer {
            Forwarded::Pending(v) => {
                state.reactive_handler.log_fleet_action_audit(
                    Some(by), &entry.agent_id, &block_id, FLEET_BULK_STOP_AUDIT_ACTION,
                    true, None, &v.request_id,
                    Some(&format!("pending user override on channel {}", entry.channel)),
                );
                remote_pending.push(v);
            }
            Forwarded::Immediate(outcome) => {
                let note = format!("channel {} predates the user override: stopped at once", entry.channel);
                state.reactive_handler.log_fleet_action_audit(
                    Some(by), &entry.agent_id, &block_id, FLEET_BULK_STOP_AUDIT_ACTION,
                    outcome.is_ok(), outcome.as_ref().err().map(String::as_str), &request_id, Some(&note),
                );
                match outcome {
                    Ok(()) => now.succeeded.push(block_id),
                    Err(error) => now.failed.push(FleetActionFailure { id: block_id, error }),
                }
            }
            Forwarded::Refused(error) => {
                state.reactive_handler.log_fleet_action_audit(
                    Some(by), &entry.agent_id, &block_id, FLEET_BULK_STOP_AUDIT_ACTION,
                    false, Some(&error), &request_id, None,
                );
                now.failed.push(FleetActionFailure { id: block_id, error });
            }
        }
    }
    pending.extend(remote_pending);
    if !nowhere.is_empty() {
        let rest = fleet_bulk_stop_impl(state, nowhere, signal, staged).await;
        now.succeeded.extend(rest.succeeded);
        now.failed.extend(rest.failed);
        now.aborted_early = rest.aborted_early;
    }
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

use agentmux_common::time::now_ms;

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

#[cfg(test)]
mod broadcast_core_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("blk-{i}")).collect()
    }

    fn agent_for(block: &str) -> Option<String> {
        block.strip_prefix("blk-").map(|n| format!("agent-{n}"))
    }

    /// Owner decision 2026-10-06 (spec §6 q2): a broadcast is the user's own
    /// message, with the user's powers.
    #[test]
    fn broadcast_turns_are_attributed_to_the_user() {
        assert_eq!(BROADCAST_TURN_ORIGIN, TurnOrigin::User);
    }

    #[tokio::test]
    async fn every_target_is_delivered_with_the_broadcast_origin() {
        let seen: Arc<Mutex<Vec<TurnOrigin>>> = Arc::default();
        let seen2 = seen.clone();
        let out = run_broadcast(ids(3), "hi", "m1", 4, agent_for, move |_b, _t, origin| {
            seen2.lock().unwrap().push(origin);
            async { Ok(()) }
        })
        .await;
        assert_eq!(out.len(), 3);
        assert!(seen.lock().unwrap().iter().all(|o| *o == TurnOrigin::User));
    }

    /// The self-quit gate passes an untainted broadcast turn whose text has the
    /// quoted instruction, exactly as it does a typed one; the quote check and
    /// the taint rule still apply.
    #[tokio::test]
    async fn the_self_quit_gate_accepts_a_broadcast_turn_like_a_typed_one() {
        use crate::backend::blockcontroller::health::TurnProvenance;
        use crate::sagas::self_quit::{gate, GateRefusal};
        let text = broadcast_turn_message("agent-1", 3, "m1", "finish the PR then quit");
        let p = |tainted| TurnProvenance { origin: BROADCAST_TURN_ORIGIN, tainted, user_text: Some(text.clone()) };
        assert_eq!(gate(Some(&p(false)), "then quit"), Ok(()));
        assert_eq!(gate(Some(&p(false)), "delete everything"), Err(GateRefusal::InstructionMismatch));
        assert_eq!(gate(Some(&p(true)), "then quit"), Err(GateRefusal::Tainted));
    }

    #[tokio::test]
    async fn each_target_gets_its_own_header_with_a_shared_id_and_the_resolved_count() {
        let texts: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let texts2 = texts.clone();
        // blk-1 resolves to nothing: three targets, two recipients.
        let resolve = |b: &str| if b == "blk-1" { None } else { agent_for(b) };
        let out = run_broadcast(ids(3), "great, merge on approval", "bcast-42", 4, resolve, move |b, t, _| {
            texts2.lock().unwrap().push((b, t));
            async { Ok(()) }
        })
        .await;
        assert_eq!(out.len(), 3);
        let texts = texts.lock().unwrap();
        assert_eq!(texts.len(), 2, "the unresolved target is not delivered to");
        for (block, text) in texts.iter() {
            let agent = agent_for(block).unwrap();
            let header = text.lines().next().unwrap();
            assert!(header.starts_with(&format!("[BROADCAST:FROM=user VIA=swarm TO={agent} RECIPIENTS=2 MSGID=bcast-42 TS=")), "{header}");
            assert_eq!(text.lines().nth(1), Some("great, merge on approval"));
        }
    }

    #[tokio::test]
    async fn outcomes_are_per_target_in_order_and_an_unresolved_target_says_why() {
        let out = run_broadcast(
            vec!["blk-0".into(), "nope".into(), "blk-2".into()],
            "x",
            "m",
            4,
            agent_for,
            |b, _t, _| async move { if b == "blk-2" { Err("no controller".to_string()) } else { Ok(()) } },
        )
        .await;
        let blocks: Vec<&str> = out.iter().map(|o| o.block_id.as_str()).collect();
        assert_eq!(blocks, vec!["blk-0", "nope", "blk-2"]);
        assert!(out[0].result.is_ok());
        assert!(out[1].result.as_ref().unwrap_err().contains("no registered agent"));
        assert!(out[1].agent_id.is_none());
        assert_eq!(out[2].result.as_ref().unwrap_err(), "no controller");
        assert_eq!(out[2].agent_id.as_deref(), Some("agent-2"));
    }

    #[tokio::test]
    async fn a_large_broadcast_does_not_pause_between_chunks() {
        let started = std::time::Instant::now();
        let out = run_broadcast(ids(25), "x", "m", BROADCAST_CONCURRENCY, agent_for, |_b, _t, _| async { Ok(()) }).await;
        assert_eq!(out.iter().filter(|o| o.result.is_ok()).count(), 25);
        assert!(started.elapsed() < std::time::Duration::from_millis(900), "the old limiter pause is gone: {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn at_most_the_concurrency_limit_turns_start_at_once() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (inf, pk) = (in_flight.clone(), peak.clone());
        let out = run_broadcast(ids(20), "x", "m", 3, agent_for, move |_b, _t, _| {
            let (inf, pk) = (inf.clone(), pk.clone());
            async move {
                let now = inf.fetch_add(1, Ordering::SeqCst) + 1;
                pk.fetch_max(now, Ordering::SeqCst);
                for _ in 0..5 {
                    tokio::task::yield_now().await;
                }
                inf.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .await;
        assert_eq!(out.len(), 20);
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak <= 3, "peak {peak} exceeded the limit");
        assert!(peak > 1, "targets should overlap, peak {peak}");
    }

    #[tokio::test]
    async fn no_targets_is_a_no_op() {
        let out = run_broadcast(vec![], "x", "m", 4, agent_for, |_b, _t, _| async { Ok(()) }).await;
        assert!(out.is_empty());
    }
}

#[cfg(test)]
mod broadcast_action_tests {
    use super::*;
    use crate::backend::reactive::SenderDelivery;
    use crate::bootstrap::AgentRoute;

    #[test]
    fn structured_channels_take_the_broadcast_without_a_new_turn() {
        // ACP, App Server and a running persistent agent all land here: the
        // regression muxreview found on #4176 was that ACP had no branch at all.
        assert_eq!(broadcast_action(Ok(AgentRoute::Sent(SenderDelivery::Delivered))), BroadcastAction::Done);
        assert_eq!(broadcast_action(Ok(AgentRoute::Sent(SenderDelivery::Deferred))), BroadcastAction::Done);
    }

    #[test]
    fn a_subprocess_or_unspawned_persistent_agent_gets_a_turn() {
        assert_eq!(broadcast_action(Ok(AgentRoute::StartTurn { subprocess: true })), BroadcastAction::StartTurn);
        assert_eq!(broadcast_action(Ok(AgentRoute::StartTurn { subprocess: false })), BroadcastAction::StartTurn);
    }

    #[test]
    fn a_terminal_pane_is_refused_not_typed_into() {
        let BroadcastAction::Fail(e) = broadcast_action(Ok(AgentRoute::Sent(SenderDelivery::Pty))) else {
            panic!("a PTY target must fail");
        };
        assert!(e.contains("not typed into a shell"), "{e}");
    }

    #[test]
    fn a_structured_controller_failure_is_reported_per_target() {
        assert_eq!(
            broadcast_action(Err("persistent process not running".into())),
            BroadcastAction::Fail("persistent process not running".into())
        );
    }
}

#[cfg(test)]
mod forward_broadcast_tests {
    use super::*;
    use axum::routing::post;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(Option<String>, serde_json::Value)>>>;

    /// A fake other channel: records each request's `X-AuthKey` and body, and
    /// answers with `status` and `reply`.
    async fn peer(status: axum::http::StatusCode, reply: serde_json::Value) -> (String, Seen) {
        let seen: Seen = Default::default();
        let s = seen.clone();
        let app = axum::Router::new().route(
            "/agentmux/agent/broadcast",
            post(move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<serde_json::Value>| {
                let (s, reply) = (s.clone(), reply.clone());
                async move {
                    let key = headers.get("X-AuthKey").and_then(|v| v.to_str().ok()).map(str::to_string);
                    s.lock().unwrap().push((key, body));
                    (status, axum::Json(reply))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (url, seen)
    }

    fn forward<'a>() -> BroadcastForward<'a> {
        BroadcastForward { block_id: "blk-loap", body: "merge on approval", msg_id: "m-1", recipients: 3 }
    }

    #[tokio::test]
    async fn the_caller_sends_the_body_and_counts_and_never_a_header() {
        let (url, seen) = peer(axum::http::StatusCode::OK, serde_json::json!({ "success": true })).await;
        let out = forward_broadcast_to_channel(&reqwest::Client::new(), &url, "chan-key", &forward()).await;
        assert_eq!(out, Ok(()));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0.as_deref(), Some("chan-key"), "that channel's own key");
        let mut keys: Vec<&str> = seen[0].1.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["block_id", "body", "msg_id", "recipients"], "no header field for a caller to forge");
        assert_eq!(seen[0].1["body"], "merge on approval");
        assert_eq!(seen[0].1["recipients"], 3);
    }

    #[tokio::test]
    async fn a_channel_that_cannot_take_it_says_why_for_that_target_only() {
        let (url, _) = peer(axum::http::StatusCode::OK, serde_json::json!({ "success": false, "error": "no registered agent" })).await;
        assert_eq!(
            forward_broadcast_to_channel(&reqwest::Client::new(), &url, "", &forward()).await,
            Err("no registered agent".to_string())
        );
    }

    #[tokio::test]
    async fn a_channel_from_before_the_route_is_named_as_an_older_build() {
        let (url, _) = peer(axum::http::StatusCode::NOT_FOUND, serde_json::json!({})).await;
        let Err(e) = forward_broadcast_to_channel(&reqwest::Client::new(), &url, "", &forward()).await else {
            panic!("a 404 must fail the target");
        };
        assert!(e.contains("older build"), "{e}");
    }

    #[tokio::test]
    async fn a_missing_key_sends_no_key_header() {
        let (url, seen) = peer(axum::http::StatusCode::OK, serde_json::json!({ "success": true })).await;
        forward_broadcast_to_channel(&reqwest::Client::new(), &url, "", &forward()).await.unwrap();
        assert_eq!(seen.lock().unwrap()[0].0, None);
    }

    #[tokio::test]
    async fn an_unreachable_channel_fails_the_target() {
        let Err(e) = forward_broadcast_to_channel(&reqwest::Client::new(), "http://127.0.0.1:9", "", &forward()).await else {
            panic!("nothing listens there");
        };
        assert!(e.starts_with("cross-channel forward failed"), "{e}");
    }

    #[tokio::test]
    async fn the_receiver_refuses_a_block_it_has_no_agent_for() {
        let state = crate::server::tests::test_state();
        assert_eq!(
            deliver_forwarded_broadcast(&state, "no-such-block", "hi", "m-1", 1).await,
            Err(NO_REGISTERED_AGENT.to_string())
        );
    }
}

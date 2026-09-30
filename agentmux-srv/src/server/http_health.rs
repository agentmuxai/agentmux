// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Health, diagnostics, LAN instances, discovery, WPS publish.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

pub(super) async fn health_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": state.version,
    }))
}

/// Saga durability PR 2 — operator visibility into the durable saga
/// log. Returns the most-recent 50 saga lifecycle rows + an in-flight
/// count derived from `unresolved_sagas`.
///
/// **Why a JSON HTTP endpoint and not a launcher `--diag sagas`
/// pipe-IPC client.** The `--diag srv` pipe transport (see
/// `agentmux-launcher/src/diag.rs`) routes through `Tool` registration
/// + a 2 s observation window with `GetSrvSnapshot` + `GetEvents`.
/// Adding `GetSagaLogSnapshot` to the IPC `Command` enum + an
/// `Event::SagaLogSnapshot` variant with a Vec of `SagaSnapshot`
/// triples the touched-files surface for one operator command.
/// JSON HTTP is the precedent for raw operator queries (cf
/// `/api/lan-instances`) and matches the spec §9 PR 2 phrasing
/// "tightened scope". Promoting to first-class `--diag sagas` via
/// pipe IPC is a follow-up if anyone asks.
///
/// Operator workflow today:
/// ```text
/// curl -s -H "X-AuthKey: $KEY" http://127.0.0.1:$PORT/agentmux/diag/sagas | jq .
/// ```
/// Response shape:
/// ```json
/// {
///   "recent": [ { "saga_id": ..., "name": ..., "state": ..., ... }, ... ],
///   "in_flight_count": 1,
///   "recently_failed_count": 0,
///   "total_returned": 50
/// }
/// ```
pub(super) async fn handle_diag_sagas(State(state): State<AppState>) -> Json<serde_json::Value> {
    const LIMIT: u32 = 50;
    let recent = match state.saga_log.snapshot_recent(LIMIT) {
        Ok(rows) => rows,
        Err(e) => {
            return Json(json!({
                "error": format!("snapshot_recent failed: {}", e),
            }));
        }
    };
    let in_flight = match state.saga_log.unresolved_sagas() {
        Ok(rows) => rows.len(),
        Err(e) => {
            tracing::warn!("[diag/sagas] unresolved_sagas failed: {}", e);
            0
        }
    };
    let recently_failed = recent
        .iter()
        .filter(|s| s.state == "failed" || s.state == "failed_compensation")
        .count();
    Json(json!({
        "recent": recent,
        "in_flight_count": in_flight,
        "recently_failed_count": recently_failed,
        "total_returned": recent.len(),
    }))
}

pub(super) async fn handle_lan_instances(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!(state.lan_discovery.get_instances()))
}

/// `GET /agentmux/discovery` — a unified, agent-facing view of what exists and
/// what is reachable across the muxbus delivery tiers, so an agent can resolve a
/// target before sending. Aggregates:
///   - `host.addressable`: the authoritative Tier-1/2 reachable set
///     (`reactive_handler.list_agents()` — every entry has a live block_id).
///   - `host.agents`: this host's agent directory (SQLite `instance_list`),
///     each flagged `addressable` iff its name is in the reachable set.
///   - `lan`: Tier-3 mDNS peers (each `LanInstance` carries its own `agents`).
///   - `wan.local_agents_subscribed`: **this host's own** agents that are
///     subscribed to the Tier-4 cloud relay (empty when no token). It is NOT a
///     list of remote agents reachable over WAN — nothing here can answer that
///     question, because the relay exposes no directory. Named
///     `wan.subscribed_agents` until 2026-09-06, which read as "agents
///     reachable via cloud" under a heading called `wan` in a response whose
///     whole purpose is reachability, and produced a wrong diagnosis. See
///     REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md §6.
/// Addressing is case-insensitive (registration lowercases the key). Authed like
/// the other reactive routes; agents reach it via AGENTMUX_LOCAL_URL + X-AuthKey.
/// See SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.
pub(super) async fn handle_discovery(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Tier-1/2 reachable set — the authoritative "addressable" answer. Keep the
    // live block_id per name (lowercased) so an addressable row can surface its
    // real delivery block; the SQLite directory below does not carry one.
    let reachable = state.reactive_handler.list_agents();
    let reachable_block: std::collections::HashMap<String, String> = reachable
        .iter()
        .map(|a| (a.agent_id.to_lowercase(), a.block_id.clone()))
        .collect();

    // Host directory (live SQLite instances). `instance_list` already excludes
    // hidden (user_hidden = 0) and template rows in SQL, and the consolidated
    // path leaves block_id/status empty (agents.rs) — so addressability AND the
    // live block_id come from the reachable set above. `block_id` is null for a
    // known-but-unreachable agent; the always-empty `status` is omitted.
    let instances = state.mstore.instance_list(None, None).unwrap_or_default();
    let agents: Vec<serde_json::Value> = instances
        .into_iter()
        .map(|i| {
            let live_block = if i.instance_name.is_empty() {
                None
            } else {
                reachable_block.get(&i.instance_name.to_lowercase()).cloned()
            };
            json!({
                "name": i.instance_name,
                "id": i.id,
                "definition_id": i.definition_id,
                "working_directory": i.working_directory,
                "addressable": live_block.is_some(),
                "block_id": live_block,
            })
        })
        .collect();

    let wan_agents = crate::muxbus::cloud_subscriber::get_global_subscriber()
        .map(|s| s.subscribed_agents())
        .unwrap_or_default();

    let lan = state.lan_discovery.get_instances();
    let version = state.version.clone();
    let local_url = state.local_web_url.clone();

    // Tier 2b — other channels' agents on this same host (issue #1916), via
    // the host-global shared registry. Excludes this instance's own channel
    // (already covered by `agents`/`addressable` above) and this instance's
    // own URL (a stale self-entry from a prior crash, if any).
    let own_channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
    let cross_channel: Vec<serde_json::Value> = crate::registry::resolve_shared_reactive_dir()
        .map(|shared_dir| {
            crate::backend::reactive::registry::list_all_shared(&shared_dir)
                .into_iter()
                .filter(|e| e.channel != own_channel && e.local_url != local_url)
                .map(|e| {
                    json!({
                        "name": e.agent_id,
                        "channel": e.channel,
                        "local_url": e.local_url,
                        "block_id": e.block_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Json(json!({
        "host": {
            "version": version,
            "hostname": state.hostname.clone(),
            "local_url": local_url,
            "addressable": reachable,
            "agents": agents,
            "cross_channel": cross_channel,
        },
        "lan": lan,
        "wan": { "local_agents_subscribed": wan_agents },
    }))
}

pub(super) async fn stub_501() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "not implemented"})),
    )
}

/// Auth-gated MPS publish endpoint
/// (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §3.2). `agentmux-bashwrap`
/// POSTs here while running a Bash command; we forward to the
/// in-process MPS broker so subscribed frontends receive the event.
pub(super) async fn handle_wps_publish(
    State(state): State<AppState>,
    Json(req): Json<WpsPublishRequest>,
) -> impl IntoResponse {
    // Note: this endpoint used to special-case `compaction_started` to
    // forward into the target block's `HealthMonitor` (silence-detector
    // compaction awareness) — removed along with the unresponsive
    // detector itself, see
    // docs/specs/SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md.
    // `agentmux-bashwrap`'s `precompact` POST still lands here and is now
    // a harmless no-op broadcast like any other MPS event — left as-is
    // rather than removing the route, since deleting it isn't warranted
    // just to avoid one no-op publish.
    let event = crate::backend::mps::MuxEvent {
        event: req.event,
        scopes: req.scopes,
        sender: String::new(),
        persist: req.persist,
        data: Some(req.data),
    };
    state.broker.publish(event);
    (StatusCode::OK, Json(json!({"ok": true})))
}

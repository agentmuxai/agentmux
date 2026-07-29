// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::reactive::InjectionRequest;
use crate::backend::reactive::registry as agent_registry;
use crate::backend::subagent_watcher;
use crate::backend::base;

use super::AppState;

/// Echo a successfully-sent jekt into the SENDER's own pane
/// (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
///
/// Appends a `{"type":"user",...}` NDJSON line carrying the same
/// `[JEKT:...]` marker block the receiver got (re-wrapped with identical
/// fields) to the sender's `output` blockfile — live WPS append (renders
/// immediately in an open agent view), persisted history
/// (`parseHistoryLines` rebuilds on reopen), and global transcript mirror.
/// The frontend's `tryParseJekt` sees FROM == this pane's agent and renders
/// it as an *outgoing* JektBubble (stream-parser.ts direction detection —
/// this is the producer that comment says doesn't exist yet).
///
/// No-op when the sender isn't a registered agent on this instance (cron,
/// external callers) or is messaging itself (the incoming marker already
/// lands in the same pane).
pub(super) fn echo_jekt_to_sender(
    state: &AppState,
    source_agent: Option<&str>,
    target_agent: &str,
    message: &str,
    msgid: &str,
    effective_tier: Option<&str>,
    delivery_tier: &str,
    priority: &str,
) {
    let Some(src) = source_agent.filter(|s| !s.is_empty()) else {
        return;
    };
    if src.eq_ignore_ascii_case(target_agent) {
        return;
    }
    let Some(sender_reg) = state.reactive_handler.get_agent(src) else {
        return;
    };

    let sanitized = crate::backend::reactive::sanitize::sanitize_message(message);
    let wrapped = crate::backend::reactive::sanitize::wrap_jekt_message(
        &sanitized,
        Some(&sender_reg.agent_id),
        target_agent,
        effective_tier.unwrap_or("coord"),
        delivery_tier,
        msgid,
        priority,
    );
    let line = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": wrapped }
    });
    let data = format!("{line}\n");
    let global_zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(
        &Some(state.wstore.clone()),
        &sender_reg.block_id,
    );
    crate::backend::blockcontroller::shell::handle_append_block_file(
        &state.broker,
        &sender_reg.block_id,
        crate::backend::agent_session::OUTPUT_FILE,
        data.as_bytes(),
        Some(&state.filestore),
        global_zone.as_deref(),
    );
}

pub(super) async fn handle_reactive_inject(
    State(state): State<AppState>,
    Json(req): Json<InjectionRequest>,
) -> Json<serde_json::Value> {
    tracing::info!(
        target_agent = %req.target_agent,
        source_agent = ?req.source_agent,
        msg_len = req.message.len(),
        "reactive inject request received"
    );

    // 1. Try local ReactiveHandler first (fast path — same instance).
    let resp = state.reactive_handler.inject_message(req.clone());
    if resp.success {
        echo_jekt_to_sender(
            &state,
            req.source_agent.as_deref(),
            &req.target_agent,
            &req.message,
            &resp.request_id,
            resp.effective_tier.as_deref(),
            req.delivery_tier.as_deref().unwrap_or("host"),
            req.priority.as_deref().unwrap_or("normal"),
        );
        return Json(serde_json::to_value(&resp).unwrap_or_default());
    }

    // 2. On "agent not found", check cross-instance file registry and forward.
    let is_not_found = resp
        .error
        .as_deref()
        .map(|e| e.starts_with("agent not found"))
        .unwrap_or(false);

    if is_not_found {
        // Tier 2: same-host, different sidecar (file registry → HTTP loopback)
        let data_dir = base::get_wave_data_dir();
        if let Some(entry) = agent_registry::lookup(&data_dir, &req.target_agent) {
            // Guard against self-forwarding loops.
            if entry.local_url != state.local_web_url {
                let forward_url = format!("{}/agentmux/reactive/inject", entry.local_url);
                tracing::debug!(
                    target = %req.target_agent,
                    url = %forward_url,
                    "cross-instance inject forward"
                );
                let mut fwd = state.http_client.post(&forward_url).json(&req);
                if !entry.auth_key.is_empty() {
                    fwd = fwd.header("X-AuthKey", &entry.auth_key);
                }
                match fwd.send().await {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
                                echo_jekt_to_sender(
                                    &state,
                                    req.source_agent.as_deref(),
                                    &req.target_agent,
                                    &req.message,
                                    body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                                    body.get("effective_tier").and_then(|v| v.as_str()),
                                    "host",
                                    req.priority.as_deref().unwrap_or("normal"),
                                );
                                return Json(body);
                            }
                            // success:false — this entry is stale (e.g. agent
                            // unregistered without a clean shutdown); evict
                            // and fall through to Tier 2b/3 instead of
                            // returning the failure (reagent P1 on #2350 —
                            // this previously returned unconditionally
                            // whenever the body parsed, regardless of
                            // success, so a stale same-channel entry never
                            // fell through to any later tier).
                            tracing::warn!(
                                target = %req.target_agent,
                                "cross-instance forward: success=false — evicting and falling through"
                            );
                            agent_registry::remove(&data_dir, &req.target_agent);
                        }
                    }
                    Ok(r) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            status = %r.status(),
                            url = %forward_url,
                            "cross-instance forward: non-success status"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            error = %e,
                            url = %forward_url,
                            "cross-instance forward failed — removing stale registry entry"
                        );
                        agent_registry::remove(&data_dir, &req.target_agent);
                    }
                }
            }
        }

        // Tier 2b: same host, DIFFERENT channel (host-global shared registry).
        // Runs when Tier 2a had no same-channel entry or its forward already
        // failed above — closes the gap issue #1916 tracked (Tier 2 previously
        // only ever reached agents in the caller's own channel). Candidates are
        // tried freshest-first (§4.3 of the cross-channel delivery spec);
        // a failed forward evicts just that channel's entry and falls through
        // to the next candidate, same evict-on-fail shape Tier 3 already uses.
        if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
            let candidates = agent_registry::lookup_all_shared(&shared_dir, &req.target_agent);
            for entry in candidates {
                // Self-forward guard, matching Tier 2a. Also loopback-only
                // (§5 of the spec): a poisoned registry entry can't redirect
                // a forward off-box, since resolve_shared_reactive_dir() is a
                // same-user local file, but defense in depth costs nothing here.
                let is_loopback = entry.local_url.starts_with("http://127.0.0.1")
                    || entry.local_url.starts_with("http://localhost")
                    || entry.local_url.starts_with("http://[::1]");
                if !is_loopback || entry.local_url == state.local_web_url {
                    continue;
                }

                let forward_url = format!("{}/agentmux/reactive/inject", entry.local_url);
                tracing::debug!(
                    target = %req.target_agent,
                    channel = %entry.channel,
                    url = %forward_url,
                    "cross-channel inject forward"
                );
                let mut fwd = state.http_client.post(&forward_url).json(&req);
                if !entry.auth_key.is_empty() {
                    fwd = fwd.header("X-AuthKey", &entry.auth_key);
                }
                match fwd.send().await {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
                                echo_jekt_to_sender(
                                    &state,
                                    req.source_agent.as_deref(),
                                    &req.target_agent,
                                    &req.message,
                                    body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                                    body.get("effective_tier").and_then(|v| v.as_str()),
                                    "host",
                                    req.priority.as_deref().unwrap_or("normal"),
                                );
                                return Json(body);
                            }
                            // success:false — this channel's entry is stale
                            // (e.g. agent unregistered without a clean
                            // shutdown); evict and try the next candidate.
                            tracing::warn!(
                                target = %req.target_agent,
                                channel = %entry.channel,
                                "cross-channel forward: success=false — evicting and trying next candidate"
                            );
                            agent_registry::remove_shared(&shared_dir, &req.target_agent, &entry.channel);
                        }
                    }
                    Ok(r) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            channel = %entry.channel,
                            status = %r.status(),
                            url = %forward_url,
                            "cross-channel forward: non-success status"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target = %req.target_agent,
                            channel = %entry.channel,
                            error = %e,
                            url = %forward_url,
                            "cross-channel forward failed — evicting stale entry"
                        );
                        agent_registry::remove_shared(&shared_dir, &req.target_agent, &entry.channel);
                    }
                }
            }
        }

        // Tier 3: LAN peer (mDNS lookup → HTTP). Runs when tier 2 had no registry
        // entry or its forward failed. Queries each discovered LAN peer for the
        // agent; result is cached for 60s to avoid per-inject mDNS fan-out.
        if let Some((peer_url, peer_auth_key)) = state
            .lan_discovery
            .find_agent(&req.target_agent, &state.http_client)
            .await
        {
            let forward_url = format!("{}/agentmux/reactive/inject", peer_url);
            tracing::debug!(
                target = %req.target_agent,
                url = %forward_url,
                "LAN peer inject forward"
            );
            let mut fwd = state.http_client.post(&forward_url).json(&req);
            if !peer_auth_key.is_empty() {
                fwd = fwd.header("X-AuthKey", &peer_auth_key);
            }
            match fwd.send().await {
                Ok(r) if r.status().is_success() => {
                    if let Ok(body) = r.json::<serde_json::Value>().await {
                        // /reactive/inject always returns HTTP 200; check body.success
                        // to detect "agent not found on that peer" (e.g. after migration).
                        if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
                            tracing::warn!(
                                target = %req.target_agent,
                                url = %forward_url,
                                "LAN peer inject: success=false — evicting stale cache entry"
                            );
                            state.lan_discovery.evict_agent(&req.target_agent);
                        } else {
                            echo_jekt_to_sender(
                                &state,
                                req.source_agent.as_deref(),
                                &req.target_agent,
                                &req.message,
                                body.get("request_id").and_then(|v| v.as_str()).unwrap_or(""),
                                body.get("effective_tier").and_then(|v| v.as_str()),
                                "lan",
                                req.priority.as_deref().unwrap_or("normal"),
                            );
                            return Json(body);
                        }
                    }
                }
                Ok(r) => {
                    tracing::warn!(
                        target = %req.target_agent,
                        status = %r.status(),
                        url = %forward_url,
                        "LAN peer forward: non-success HTTP status"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target = %req.target_agent,
                        error = %e,
                        url = %forward_url,
                        "LAN peer forward failed — evicting cache entry"
                    );
                    state.lan_discovery.evict_agent(&req.target_agent);
                }
            }
        }
    }

    // 4. Return original error (muxbus-client will fall back to cloud relay).
    Json(serde_json::to_value(&resp).unwrap_or_default())
}

pub(super) async fn handle_reactive_agents(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let agents = state.reactive_handler.list_agents();
    Json(serde_json::to_value(&agents).unwrap_or(json!([])))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentQuery {
    id: Option<String>,
}

pub(super) async fn handle_reactive_agent(
    State(state): State<AppState>,
    Query(params): Query<AgentQuery>,
) -> Response {
    let id = match &params.id {
        Some(id) if !id.is_empty() => id.as_str(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing id param"})),
            )
                .into_response()
        }
    };
    match state.reactive_handler.get_agent(id) {
        Some(agent) => Json(serde_json::to_value(&agent).unwrap_or_default()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "agent not found"})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
}
fn default_audit_limit() -> usize {
    100
}

pub(super) async fn handle_reactive_audit(
    State(state): State<AppState>,
    Query(params): Query<AuditQuery>,
) -> Json<serde_json::Value> {
    let log = state.reactive_handler.get_audit_log(params.limit);
    Json(serde_json::to_value(&log).unwrap_or(json!([])))
}

#[derive(serde::Deserialize)]
pub(super) struct RegisterRequest {
    agent_id: String,
    block_id: String,
    tab_id: Option<String>,
}

pub(super) async fn handle_reactive_register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Response {
    tracing::info!(
        agent_id = %req.agent_id,
        block_id = %req.block_id,
        "reactive register request"
    );
    match state
        .reactive_handler
        .register_agent(&req.agent_id, &req.block_id, req.tab_id.as_deref())
    {
        Ok(()) => {
            // Also write to cross-instance file registry so other AgentMux
            // instances can forward inject requests to this one.
            let data_dir = base::get_wave_data_dir();
            agent_registry::write(&data_dir, &req.agent_id, &state.local_web_url, &req.block_id);

            // And to the host-global shared registry (Tier 2b) so instances
            // running in OTHER channels on this host can reach this agent
            // too — closes issue #1916 (Tier 2 previously only ever reached
            // the caller's own channel).
            if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
                let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
                agent_registry::write_shared(
                    &shared_dir,
                    &req.agent_id,
                    &state.local_web_url,
                    &req.block_id,
                    &channel,
                );
            }

            // Auto-watch this agent's Claude Code config dir for subagent JSONL files.
            // Pass block_id so subagent events are stamped with the owning pane,
            // letting the frontend route ⚡ panels to that pane only. See
            // `resolve_claude_config_dir`'s doc comment for why this must read
            // the block's own `cmd:env`, not just guess a path convention.
            let block = state.wstore.get::<crate::backend::obj::Block>(&req.block_id).ok().flatten();
            let empty_meta = crate::backend::obj::MetaMapType::new();
            let config_dir = subagent_watcher::resolve_claude_config_dir(
                block.as_ref().map(|b| &b.meta).unwrap_or(&empty_meta),
                &req.agent_id,
            );
            if let Some(config_dir) = config_dir {
                state.subagent_watcher.watch_agent(&req.agent_id, &req.block_id, config_dir.clone());

                // If this block already has a persisted session id, it's
                // resuming a prior conversation (not starting fresh) —
                // backfill just THAT session's own subagents, so a
                // reopened pane shows what it already had without
                // flooding in every OTHER session this agent identity has
                // ever run. A brand-new session has nothing to backfill;
                // watch_agent's live watcher picks up subagents as the
                // Task tool spawns them.
                let session_id = block.as_ref().map(|b| {
                    crate::backend::obj::meta_get_string(
                        &b.meta,
                        crate::backend::blockcontroller::core::META_SESSION_ID,
                        "",
                    )
                }).unwrap_or_default();
                if !session_id.is_empty() {
                    state.subagent_watcher.scan_session_subagents(
                        &req.agent_id,
                        &req.block_id,
                        &config_dir,
                        &session_id,
                    );
                }
            }

            // Notify cloud subscriber so it can subscribe for cloud-push delivery
            if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                sub.add_agent(&req.agent_id);
            }

            // Notify the Swarm view so it calls AgentTrackedBlocksCommand and
            // shows this pane. We use a dedicated event name so useProcessCount
            // (which subscribes to agent:process-added / agent:process-exited)
            // doesn't treat this as a phantom OS process and show a spurious ⚙ N
            // badge or trigger the kill-tree modal on pane close.
            state.broker.publish(crate::backend::wps::WaveEvent {
                event: "agent:reactive-registered".to_string(),
                scopes: vec![format!("block:{}", req.block_id)],
                sender: String::new(),
                persist: 0,
                data: Some(json!({ "block_id": req.block_id })),
            });

            Json(json!({"success": true})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct UnregisterRequest {
    agent_id: String,
}

pub(super) async fn handle_reactive_unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Json<serde_json::Value> {
    // Capture block_id before unregistering so we can emit the Swarm refresh event.
    let block_id = state.reactive_handler.get_agent(&req.agent_id)
        .map(|r| r.block_id.clone());

    state.reactive_handler.unregister_agent(&req.agent_id);
    // Also remove from cross-instance file registry.
    let data_dir = base::get_wave_data_dir();
    agent_registry::remove(&data_dir, &req.agent_id);
    // And from the host-global shared registry (Tier 2b).
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
        agent_registry::remove_shared(&shared_dir, &req.agent_id, &channel);
    }
    // Drop the subagent filesystem watcher (handle + channel + task) — the
    // symmetric teardown for the watch_agent() call in the register handler.
    // Passes block_id (captured above) so a shared-agent-id watcher with
    // another still-open dependent block survives this one's teardown.
    state.subagent_watcher.unwatch_agent(&req.agent_id, block_id.as_deref());
    // Notify cloud subscriber so it stops subscribing for this agent
    if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
        sub.remove_agent(&req.agent_id);
    }

    // Symmetric refresh: tell the Swarm view this pane is gone.
    if let Some(bid) = block_id {
        state.broker.publish(crate::backend::wps::WaveEvent {
            event: "agent:reactive-unregistered".to_string(),
            scopes: vec![format!("block:{}", bid)],
            sender: String::new(),
            persist: 0,
            data: Some(json!({ "block_id": bid })),
        });
    }

    Json(json!({"success": true}))
}

pub(super) async fn handle_reactive_poller_stats(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let stats = state.poller.stats();
    Json(serde_json::to_value(&stats).unwrap_or(json!({})))
}

#[derive(serde::Deserialize)]
pub(super) struct PollerConfigRequest {
    url: Option<String>,
    token: Option<String>,
}

pub(super) async fn handle_reactive_poller_config(
    State(state): State<AppState>,
    Json(req): Json<PollerConfigRequest>,
) -> Json<serde_json::Value> {
    state.poller.reconfigure(req.url, req.token);
    Json(json!({"success": true}))
}

pub(super) async fn handle_reactive_poller_status(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let status = state.poller.status();
    Json(serde_json::to_value(&status).unwrap_or(json!({})))
}

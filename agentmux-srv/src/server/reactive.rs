// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::reactive::{InjectionRequest, SupervisorAction};
use crate::backend::reactive::registry as agent_registry;
use crate::backend::subagent_watcher;
use crate::backend::base;

use super::AppState;

/// Max cross-instance HTTP forwards a single inject request may go through
/// (Tier 2a/2b/3 each increment before forwarding). A legitimate delivery
/// is always exactly one hop (caller's instance → the owning instance);
/// this only exists to bound a pathological cycle — two channels each
/// holding a stale-but-PID-alive shared-registry entry pointing at the
/// other for the same agent name would otherwise forward back and forth
/// indefinitely, hanging the original request (reagent P1 on PR #2350).
const MAX_FORWARD_HOPS: u8 = 3;

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
    sig_verified: Option<bool>,
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
        sig_verified,
        // Sender-echo is inherently host-tier (the sender only ever sees
        // this in their own pane) — reagent signing is WAN-only.
        None,
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

/// Max age (seconds) a signed jekt's `ts_secs` may be from "now" and still
/// verify (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2, anti-replay
/// — reagentx P1 on PR #2565: `ts_secs` was bound into the signed material
/// specifically for this purpose per `jekt_sign.rs`'s own doc comments, but
/// nothing actually checked it, so a captured valid signature verified
/// forever). Generous enough for normal host-tier delivery latency and
/// modest clock skew between the signing agent process and this srv
/// instance (same machine, but not guaranteed same clock read down to the
/// second); tight enough to bound replay to a narrow window instead of
/// indefinite reuse.
const JEKT_SIG_MAX_AGE_SECS: i64 = 300;

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Host-tier jekt sender verification (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
/// §2.2). Mutates `req.sig_verified` in place based on whether the claimed
/// `source_agent`'s stored signing key (if any) verifies `req.jekt_sig`,
/// within the anti-replay freshness window.
///
/// **Every entry point that can build a host-tier `InjectionRequest` from
/// client-supplied fields MUST call this before handing it to
/// `Handler::inject_message`** — reagentx P0 on PR #2565 found two call
/// sites (`messagebus.rs::handle_inject`, `websocket.rs`'s `bus:inject`
/// message handling) that built `InjectionRequest` with a fully
/// client-controlled `source_agent` and called `inject_message` directly,
/// bypassing this entirely — those messages rendered `TRUST=self-declared`
/// (unescalated) exactly as if this feature didn't exist. Both now call
/// this too.
pub(super) fn verify_jekt_signature(state: &AppState, req: &mut InjectionRequest) {
    if req.delivery_tier.as_deref().unwrap_or("host") != "host" {
        return;
    }
    let Some(claimed) = req.source_agent.clone().filter(|s| !s.is_empty()) else {
        return;
    };
    let Ok(Some(key)) = state.wstore.agent_jekt_key_load(&claimed) else {
        return;
    };
    let msgid = req.request_id.clone().unwrap_or_default();
    let ts = req.ts_secs.unwrap_or(0);
    let within_freshness_window =
        ts > 0 && (now_unix_secs() - ts).abs() <= JEKT_SIG_MAX_AGE_SECS;
    let verified = within_freshness_window
        && req.jekt_sig.as_deref().map_or(false, |sig| {
            agentmux_common::jekt_sign::verify_jekt(
                &key, &msgid, &claimed, &req.target_agent, ts, &req.message, sig,
            )
        });
    req.sig_verified = Some(verified);
}

/// Anti-replay window for `req.reagent_ts_secs`, same purpose as
/// `JEKT_SIG_MAX_AGE_SECS` above but WAN-scoped: wider than host-tier's
/// 300s because this covers real network delivery latency, not a
/// same-machine call — matches `cloud_subscriber::REAGENT_SIG_MAX_AGE_SECS`
/// (the WS delivery path's own constant of the same value) and the
/// github-consumer Lambda's own REVIEW_NOTIFICATION_TTL_SECONDS delivery
/// window in the agentmux-cloud repo.
const REAGENT_SIG_MAX_AGE_SECS: i64 = 600;

/// WAN-tier reagent-signature verification for the HTTP
/// `/agentmux/reactive/inject` path — mirrors
/// `cloud_subscriber::sync_agent_reactive`'s in-process verification of the
/// same four fields for the desktop app's WS delivery path, but for callers
/// that deliver over HTTP instead (`@agentmuxai/muxbus-client`'s
/// `pollAndDeliverInjections`, and any future standalone poller). Before
/// this, `InjectionRequest` declared `reagent_sig`/`reagent_key_id` as
/// deserializable input fields but nothing on the HTTP path ever read them
/// — a reagent-signed notification delivered through this path arrived
/// unsigned in effect, `reagent_verified` always `None`, and could never
/// render `SIG=verified` (reagentx P1 on PR #41).
///
/// Only meaningful for `delivery_tier == "wan"` — same scoping as
/// `reagent_verified`'s doc comment ("meaningless off the WAN tier"). A
/// partial set of the four fields (e.g. a sig but no key_id) is treated the
/// same as "not signed" (`reagent_verified` stays `None`), not "signed but
/// broken" — matches `cloud_subscriber.rs`'s identical policy: a legitimate
/// sender always sends all four together, and this field never affects
/// `TIER`/`TRUST` escalation either way, so a stripped signature can't buy
/// an attacker anything a fully-absent one couldn't already.
///
/// Takes `now` explicitly (rather than calling `now_unix_secs()` itself),
/// same reasoning as `cloud_subscriber::reagent_sig_is_fresh`: the pinned
/// Ed25519 key's matching private half isn't in this repo (it lives only in
/// agentmux-cloud's Secrets Manager), so tests can't mint a fresh signature
/// on demand the way the host-tier HMAC tests below do — only a fixed
/// offline-signed fixture at a fixed `ts_secs`. Injecting `now` lets a test
/// hold that fixture inside the freshness window without mocking the clock.
pub(super) fn verify_reagent_signature(req: &mut InjectionRequest, now: i64) {
    if req.delivery_tier.as_deref() != Some("wan") {
        return;
    }
    let (Some(sig), Some(key_id), Some(msg_id), Some(ts_secs)) =
        (req.reagent_sig.as_deref(), req.reagent_key_id.as_deref(), req.reagent_msg_id.as_deref(), req.reagent_ts_secs)
    else {
        return;
    };
    let within_freshness_window = ts_secs > 0 && (now - ts_secs).abs() <= REAGENT_SIG_MAX_AGE_SECS;
    let verified = within_freshness_window
        && agentmux_common::jekt_sign::verify_reagent_jekt(
            key_id,
            msg_id,
            req.source_agent.as_deref().unwrap_or(""),
            &req.target_agent,
            ts_secs,
            &req.message,
            sig,
        );
    req.reagent_verified = Some(verified);
}

pub(super) async fn handle_reactive_inject(
    State(state): State<AppState>,
    Json(mut req): Json<InjectionRequest>,
) -> Json<serde_json::Value> {
    tracing::info!(
        target_agent = %req.target_agent,
        source_agent = ?req.source_agent,
        msg_len = req.message.len(),
        "reactive inject request received"
    );

    verify_jekt_signature(&state, &mut req);
    verify_reagent_signature(&mut req, now_unix_secs());

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
            req.sig_verified,
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

    if is_not_found && req.forward_hops >= MAX_FORWARD_HOPS {
        tracing::warn!(
            target = %req.target_agent,
            hops = req.forward_hops,
            "reactive inject: forward-hop limit reached, not forwarding further"
        );
        return Json(serde_json::to_value(&resp).unwrap_or_default());
    }

    // Every forward below sends this hop-incremented request, not the
    // original `req` — a peer that also fails to find the agent locally
    // and forwards onward needs to see the accumulated hop count too.
    let mut forwarded_req = req.clone();
    forwarded_req.forward_hops = req.forward_hops.saturating_add(1);

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
                let mut fwd = state.http_client.post(&forward_url).json(&forwarded_req);
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
                                    req.sig_verified,
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
                let mut fwd = state.http_client.post(&forward_url).json(&forwarded_req);
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
                                    req.sig_verified,
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
            let mut fwd = state.http_client.post(&forward_url).json(&forwarded_req);
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
                                None,
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
            agent_registry::write_shared_from_env(&req.agent_id, &state.local_web_url, &req.block_id);

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
    agent_registry::remove_shared_from_env(&req.agent_id);
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

/// Server-side ceiling on `max_lines` — protects a Supervisor's transcript
/// pull (and this route in general) from an unbounded read of a huge
/// session file. Callers wanting more must paginate some other way; this
/// route is a "recent tail" primitive, not a full-history export.
const TRANSCRIPT_MAX_LINES_CAP: usize = 500;

#[derive(serde::Deserialize)]
pub(super) struct TranscriptQuery {
    agent: String,
    #[serde(default = "default_transcript_max_lines")]
    max_lines: usize,
}
fn default_transcript_max_lines() -> usize {
    100
}

/// `GET /agentmux/reactive/transcript?agent=<name>&max_lines=<n>` — read the
/// tail of a registered agent's session output, for a Warden Supervisor
/// watcher agent to inspect on its own poll interval (v1 is pull/poll, not
/// push — see
/// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md).
pub(super) async fn handle_reactive_transcript(
    State(state): State<AppState>,
    Query(params): Query<TranscriptQuery>,
) -> Response {
    if params.agent.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing agent param"})),
        )
            .into_response();
    }
    let Some(reg) = state.reactive_handler.get_agent(&params.agent) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "agent not found"})),
        )
            .into_response();
    };
    let block_id = reg.block_id.clone();

    let (raw_bytes, _total_line_count) = match crate::backend::session_archive::read_session_output(
        &state.wstore,
        &state.filestore,
        &block_id,
    ) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("read_session_output: {e}")})),
            )
                .into_response();
        }
    };

    let text = String::from_utf8_lossy(&raw_bytes);
    let all_lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let requested = params.max_lines.min(TRANSCRIPT_MAX_LINES_CAP).max(1);
    let truncated = all_lines.len() > requested;
    let lines: Vec<String> = all_lines
        .iter()
        .rev()
        .take(requested)
        .rev()
        .map(|l| l.to_string())
        .collect();

    let turn_active = crate::backend::blockcontroller::get_block_controller_status(&block_id)
        .map(|s| s.turn_active)
        .unwrap_or(false);

    Json(json!({
        "agent": reg.agent_id,
        "block_id": block_id,
        "turn_active": turn_active,
        "lines": lines,
        "truncated": truncated,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct SupervisorDecisionRequest {
    target_agent: String,
    /// "nudge" | "decline".
    action: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    /// The calling Supervisor agent's own identity — same shape as
    /// `InjectRequest::source_agent` (`SendMessage`/`Loop`). `None` for
    /// callers that omit it (e.g. cron-driven).
    #[serde(default)]
    source_agent: Option<String>,
}

/// `POST /agentmux/reactive/supervisor-decision` — a Warden Supervisor
/// watcher agent's decision about a target agent it just polled (see
/// `GetAgentTranscript`). `action: "nudge"` delivers a fixed continuation
/// message (not caller-supplied text — see `SupervisorAction::Nudge`'s
/// doc) to `target_agent` through the same path `SendMessage`/`Loop` use
/// and audits it as a Supervisor-originated entry; `action: "decline"`
/// sends nothing and just audits the decision. A nudge that would exceed
/// the consecutive-nudge ceiling is refused with HTTP 429 — the calling
/// agent should treat that as a signal to stop and escalate to a human
/// instead of retrying.
pub(super) async fn handle_reactive_supervisor_decision(
    State(state): State<AppState>,
    Json(req): Json<SupervisorDecisionRequest>,
) -> Response {
    if req.target_agent.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing target_agent"})),
        )
            .into_response();
    }

    let action = match req.action.as_str() {
        "nudge" => SupervisorAction::Nudge,
        "decline" => SupervisorAction::Decline,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("unknown action: {other} (expected \"nudge\" or \"decline\")")})),
            )
                .into_response();
        }
    };

    // Entitlement gate (reagentx P1 on PR #2557): a Nudge must not deliver
    // unless the target has actually opted in via `auto_continue_enabled`.
    // `Handler` (backend::reactive) has no `Store` access by design — this
    // check belongs at the HTTP boundary where `state.wstore` is available,
    // not inside `record_supervisor_decision`. Decline never delivers
    // anything, so it isn't gated.
    //
    // Match on `d.slug`, NOT `d.name` (reagentx P0, round 3 — every
    // delivery path keys registration off `AGENTMUX_AGENT_ID`, which
    // `agent_open.rs` sets to the agent's stable `slug`, not its
    // renameable display `name`. Matching on `name` here let a renamed
    // agent's own opt-in go unrecognized, and — worse — let one agent's
    // slug collide with an unrelated agent's current display name,
    // authorizing a nudge off the wrong definition's flag. Same
    // name/slug cross-namespace hazard `agents.rs`'s
    // `instance_get_by_name_and_by_slug_never_cross_the_others_namespace`
    // regression-tests for the read path.)
    if matches!(action, SupervisorAction::Nudge) {
        let opted_in = state
            .wstore
            .agent_def_list()
            .ok()
            .and_then(|defs| {
                defs.into_iter()
                    .find(|d| d.slug.eq_ignore_ascii_case(&req.target_agent))
            })
            .map(|d| d.auto_continue_enabled != 0)
            .unwrap_or(false);
        if !opted_in {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": format!(
                        "target agent '{}' has not opted in to auto_continue_enabled",
                        req.target_agent
                    )
                })),
            )
                .into_response();
        }
    }

    let reason = req.reason.unwrap_or_default();
    let request_id = req
        .request_id
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    match state.reactive_handler.record_supervisor_decision(
        &req.target_agent,
        action,
        &reason,
        &request_id,
        req.source_agent.as_deref(),
    ) {
        Ok(resp) => Json(serde_json::to_value(&resp).unwrap_or_default()).into_response(),
        Err(e) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": e})),
        )
            .into_response(),
    }
}

/// `verify_jekt_signature` unit tests (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
/// §2.2, reagentx review on PR #2565). Deliberately test the extracted
/// function directly rather than the full `handle_inject`/websocket
/// handlers it's now called from: `server::tests::test_state()`'s
/// `reactive_handler` is a *global* singleton shared across every test in
/// the binary (`backend_reactive::get_global_handler()`), so exercising it
/// end-to-end here risks cross-test interference on shared agent
/// registrations. `verify_jekt_signature` itself only touches `state.wstore`
/// (key lookup), not the handler, so it's safe to test in isolation with no
/// such risk — and it's the one piece of logic actually being fixed here;
/// the two call sites (messagebus.rs, websocket.rs) are a one-line "call
/// this before inject_message" wiring, visible directly in their diffs.
#[cfg(test)]
mod verify_jekt_signature_tests {
    use super::*;
    use crate::server::tests::test_state;

    fn base_req(source_agent: &str, target_agent: &str, message: &str) -> InjectionRequest {
        InjectionRequest {
            target_agent: target_agent.to_string(),
            message: message.to_string(),
            source_agent: Some(source_agent.to_string()),
            delivery_tier: Some("host".to_string()),
            ..Default::default()
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[tokio::test]
    async fn a_correctly_signed_message_verifies_true() {
        let state = test_state();
        let key = state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(now());
        req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
            &key,
            req.request_id.as_deref().unwrap(),
            "agentx",
            "agenty",
            req.ts_secs.unwrap(),
            "hello",
        ));

        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, Some(true));
    }

    /// The core P0 fix this whole file's `verify_jekt_signature` extraction
    /// exists for: a claimed sender with a real key on file but NO
    /// signature attached (exactly what `messagebus.rs::handle_inject` and
    /// websocket.rs's `bus:inject` used to send, pre-fix) must render as a
    /// real, escalating "unverified" — not silently pass through unchecked.
    #[tokio::test]
    async fn a_claimed_sender_with_a_key_but_no_signature_is_unverified() {
        let state = test_state();
        state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(now());
        // req.jekt_sig deliberately left None — the exact bypass shape.

        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified,
            Some(false),
            "a signable identity with no signature must be a real 'unverified,' not skipped"
        );
    }

    #[tokio::test]
    async fn no_key_on_file_leaves_sig_verified_unset() {
        let state = test_state();
        // No agent_jekt_key_ensure call — "slack", or any non-agent caller.
        let mut req = base_req("slack", "agenty", "hello");
        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified, None,
            "no key on file means nothing to check — must not be escalated"
        );
    }

    #[tokio::test]
    async fn network_tier_is_never_checked_regardless_of_signature() {
        let state = test_state();
        state.wstore.agent_jekt_key_ensure("agentx").unwrap();
        let mut req = base_req("agentx", "agenty", "hello");
        req.delivery_tier = Some("wan".to_string());
        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, None, "wan/lan never run this check");
    }

    /// Anti-replay (reagentx P1 on PR #2565): a signature that was valid
    /// once must stop verifying once its `ts_secs` falls outside the
    /// freshness window — otherwise a captured signed jekt replays forever.
    #[tokio::test]
    async fn a_stale_timestamp_fails_verification_even_with_a_correct_signature() {
        let state = test_state();
        let key = state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let stale_ts = now() - JEKT_SIG_MAX_AGE_SECS - 60; // well outside the window
        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(stale_ts);
        req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
            &key, "msg-1", "agentx", "agenty", stale_ts, "hello",
        ));

        verify_jekt_signature(&state, &mut req);
        assert_eq!(
            req.sig_verified,
            Some(false),
            "a mathematically correct signature must still fail outside the freshness window"
        );
    }

    #[tokio::test]
    async fn a_timestamp_just_inside_the_window_still_verifies() {
        let state = test_state();
        let key = state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let recent_ts = now() - (JEKT_SIG_MAX_AGE_SECS - 10);
        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(recent_ts);
        req.jekt_sig = Some(agentmux_common::jekt_sign::sign_jekt(
            &key, "msg-1", "agentx", "agenty", recent_ts, "hello",
        ));

        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, Some(true));
    }

    #[tokio::test]
    async fn a_wrong_signature_is_unverified() {
        let state = test_state();
        state.wstore.agent_jekt_key_ensure("agentx").unwrap();

        let mut req = base_req("agentx", "agenty", "hello");
        req.request_id = Some("msg-1".to_string());
        req.ts_secs = Some(now());
        req.jekt_sig = Some("forged-not-a-real-signature".to_string());

        verify_jekt_signature(&state, &mut req);
        assert_eq!(req.sig_verified, Some(false));
    }
}

/// `verify_reagent_signature` unit tests (reagentx P1 on PR #41 —
/// `InjectionRequest` declared `reagent_sig`/`reagent_key_id` as
/// deserializable input fields but nothing on the HTTP
/// `/agentmux/reactive/inject` path ever verified them, so a reagent-signed
/// notification delivered through `@agentmuxai/muxbus-client`'s
/// `pollAndDeliverInjections` arrived unsigned in effect).
#[cfg(test)]
mod verify_reagent_signature_tests {
    use super::*;

    // Reuses the exact fixture from agentmux-common/src/jekt_sign.rs's own
    // `a_correctly_signed_reagent_message_verifies` test: a signature
    // produced offline against the "reagent-v1-dev" pinned public key's
    // matching private half, over signed_material("msg-1",
    // "github-consumer", "agentx", 1000, "hello"). The private key isn't in
    // this repo (agentmux-cloud's Secrets Manager only) so a fresh signature
    // can't be minted at test time — `now` is passed explicitly instead of
    // wall-clock so this fixed ts_secs=1000 can be held inside the
    // freshness window on demand.
    const FIXTURE_SIG_B64: &str =
        "QehidZjJa2jYLPIPYSsVxUlm86W5Fdbr9PV3P4HJyZwJ68/HZR9EaAL0MpcVtTuZJW2+MMGebc0RH9HITNJGCw==";
    const FIXTURE_TS_SECS: i64 = 1_000;

    fn wan_req() -> InjectionRequest {
        InjectionRequest {
            target_agent: "agentx".to_string(),
            message: "hello".to_string(),
            source_agent: Some("github-consumer".to_string()),
            delivery_tier: Some("wan".to_string()),
            reagent_sig: Some(FIXTURE_SIG_B64.to_string()),
            reagent_key_id: Some("reagent-v1-dev".to_string()),
            reagent_msg_id: Some("msg-1".to_string()),
            reagent_ts_secs: Some(FIXTURE_TS_SECS),
            ..Default::default()
        }
    }

    #[test]
    fn a_correctly_signed_and_fresh_reagent_message_verifies() {
        let mut req = wan_req();
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, Some(true));
    }

    #[test]
    fn a_correct_signature_outside_the_freshness_window_fails() {
        let mut req = wan_req();
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS + REAGENT_SIG_MAX_AGE_SECS + 60);
        assert_eq!(
            req.reagent_verified,
            Some(false),
            "a mathematically correct signature must still fail outside the freshness window"
        );
    }

    #[test]
    fn host_tier_is_never_checked_regardless_of_signature() {
        let mut req = wan_req();
        req.delivery_tier = Some("host".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None, "reagent signing only applies to the WAN tier");
    }

    #[test]
    fn lan_tier_is_never_checked_regardless_of_signature() {
        let mut req = wan_req();
        req.delivery_tier = Some("lan".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None, "reagent signing only applies to the WAN tier");
    }

    #[test]
    fn a_wrong_signature_is_unverified() {
        let mut req = wan_req();
        req.reagent_sig = Some("forged-not-a-real-signature".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, Some(false));
    }

    #[test]
    fn an_unknown_key_id_is_unverified() {
        let mut req = wan_req();
        req.reagent_key_id = Some("reagent-v2-does-not-exist".to_string());
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, Some(false));
    }

    // A partial set of the four fields is "not signed," not "signed but
    // broken" — matches cloud_subscriber.rs's identical policy (see
    // reagent_key_id's doc comment in types.rs).
    #[test]
    fn a_partial_signature_set_is_treated_as_unsigned_not_invalid() {
        let mut req = wan_req();
        req.reagent_key_id = None;
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None);
    }

    #[test]
    fn no_reagent_fields_at_all_is_left_unset() {
        let mut req = wan_req();
        req.reagent_sig = None;
        req.reagent_key_id = None;
        req.reagent_msg_id = None;
        req.reagent_ts_secs = None;
        verify_reagent_signature(&mut req, FIXTURE_TS_SECS);
        assert_eq!(req.reagent_verified, None);
    }
}

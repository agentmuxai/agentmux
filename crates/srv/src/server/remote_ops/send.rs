// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sending half (spec §4.1, §4.8): an agent asks this machine's srv to run an operation on
//! another machine. srv builds the envelope, signs it with the agent's own LAN key (the agent's
//! process never holds the signing step, as for a jekt from an agent that signed nothing), sends
//! it to the peer and returns the peer's answer. It writes `remote.request` to this machine's
//! audit trail, by `agent@machine`.
//!
//! `host` is a machine's name as `Agents list` shows it, or `name/channel` when a machine runs
//! several instances. It must be named on every call; a name that fits more than one instance
//! is an error, never a guess.

use std::time::Duration;

use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use agentmux_common::remote_op::{lan_audience, RemoteOp, RemoteOpResult, Requester, SignedRemoteOp, REMOTE_OP_VERSION};

use super::enabled;
use crate::backend::lan_discovery::LanInstance;
use crate::server::actor::{self, ActorSite};
use crate::server::{app_api_response, caller, AppState};

/// How long a peer gets to answer. An operation may wait on the peer's own countdown only by
/// answering `pending_veto` first, so this is a request's round trip, not a human's time.
const PEER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(serde::Deserialize)]
pub(crate) struct RemoteOpRequest {
    /// The calling agent (must be one only the token's own agent answers to).
    pub agent_id: String,
    pub host: String,
    pub op: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub reason: String,
}

/// The one LAN instance `host` names, or why not.
pub(super) fn resolve_peer(peers: &[LanInstance], host: &str) -> Result<LanInstance, String> {
    let wanted = host.trim();
    if wanted.is_empty() {
        return Err("name the machine (host): remote operations never default to a machine".into());
    }
    let (name, channel) = match wanted.split_once('/') {
        Some((n, c)) => (n.trim(), Some(c.trim())),
        None => (wanted, None),
    };
    let found: Vec<&LanInstance> = peers
        .iter()
        .filter(|p| p.hostname.trim().eq_ignore_ascii_case(name) && channel.is_none_or(|c| p.channel.eq_ignore_ascii_case(c)))
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let known: Vec<String> = peers.iter().map(|p| p.hostname.trim().to_string()).collect();
            Err(format!("no machine {wanted:?} among the LAN peers; they are: {}", known.join(", ")))
        }
        many => {
            let names: Vec<String> = many.iter().map(|p| format!("{}/{}", p.hostname.trim(), p.channel)).collect();
            Err(format!("{wanted:?} fits more than one instance: name one of {}", names.join(", ")))
        }
    }
}

fn fresh_nonce() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{}-{n}-{}", agentmux_common::time::now_ms_u64(), std::process::id(), &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// Run `op` on `host` as the agent `slug` and return the machine's answer. A failure to reach or
/// sign is a `failed` result, so every caller gets one shape back. The srv-side door for
/// operations that other features make (the fleet Global Memory push); the first caller follows.
#[allow(dead_code)]
pub(crate) async fn call(state: &AppState, slug: &str, host: &str, op: &str, params: Value, reason: &str) -> RemoteOpResult {
    let uid = match crate::backend::agent_resolve::resolve_agent_id(&state.mstore, slug) {
        Ok(uid) => uid,
        Err(e) => return RemoteOpResult::failed(&format!("no agent {slug:?} here: {e}")),
    };
    call_as(state, slug, &uid, host, op, params, reason).await
}

async fn call_as(state: &AppState, slug: &str, uid: &str, host: &str, op: &str, params: Value, reason: &str) -> RemoteOpResult {
    if !enabled(state) {
        return RemoteOpResult::refused("remote_ops_off");
    }
    let peer = match resolve_peer(&state.lan_discovery.get_instances(), host) {
        Ok(p) => p,
        Err(e) => return RemoteOpResult::failed(&e),
    };
    let Some(key) = state.mstore.agent_lan_key_ensure(slug).ok().and_then(|k| agentmux_common::jekt_sign::decode_key(&k.private_key)) else {
        return RemoteOpResult::failed("this agent has no LAN key to sign with");
    };
    let instance_id = state
        .mstore
        .wan_identity()
        .or_else(crate::backend::storage::wan_identity::global)
        .and_then(|wan| wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()).ok())
        .map(|i| i.instance_id)
        .unwrap_or_default();
    let envelope = RemoteOp {
        v: REMOTE_OP_VERSION,
        op: op.to_string(),
        params,
        requester: Requester { agent_uid: uid.to_string(), slug: slug.to_string(), instance_id },
        audience: lan_audience(&peer.auth_key),
        ts: agentmux_common::time::now_secs(),
        nonce: fresh_nonce(),
        reason: agentmux_common::remote_op::clip_reason(reason),
    };
    let nonce = envelope.nonce.clone();
    let Some(signed) = envelope.sign(&key) else {
        return RemoteOpResult::failed("the request could not be signed (too long, or a malformed field)");
    };
    let result = post(state, &peer, &signed).await;
    let ok = matches!(result.status, agentmux_common::remote_op::OpStatus::Done | agentmux_common::remote_op::OpStatus::Queued | agentmux_common::remote_op::OpStatus::PendingVeto);
    let note = result.refused_reason.clone().or_else(|| result.result.get("error").and_then(Value::as_str).map(str::to_string));
    state.reactive_handler.log_fleet_action_audit(
        Some(&format!("{slug}@{}", state.hostname)),
        peer.hostname.trim(),
        "",
        &format!("remote.request {op}"),
        ok,
        note.as_deref(),
        &nonce,
        (!reason.is_empty()).then_some(reason),
    );
    result
}

async fn post(state: &AppState, peer: &LanInstance, signed: &SignedRemoteOp) -> RemoteOpResult {
    let url = format!("http://{}:{}/agentmux/remote/op", peer.address, peer.port);
    let response = state
        .http_client
        .post(url)
        .header(agentmux_common::AUTH_KEY_HEADER, &peer.auth_key)
        .timeout(PEER_TIMEOUT)
        .json(signed)
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => resp.json::<RemoteOpResult>().await.unwrap_or_else(|e| RemoteOpResult::failed(&format!("that machine's answer was not understood: {e}"))),
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => RemoteOpResult::refused("peer_cannot_do_remote_ops"),
        Ok(resp) => RemoteOpResult::failed(&format!("that machine answered {}", resp.status())),
        Err(e) => RemoteOpResult::failed(&format!("couldn't reach that machine: {e}")),
    }
}

/// `POST /api/v1/agent/remote/op`: backs the `host` argument of the MuxAPI tools. The agent is
/// the one its token proves; the body's word is only checked against it.
pub(crate) async fn handle_agent_remote_op(
    State(state): State<AppState>,
    who: Option<Extension<caller::Caller>>,
    Json(req): Json<RemoteOpRequest>,
) -> Response {
    let (slug, uid) = match actor::require_token_actor(&state, who.as_deref(), ActorSite::RemoteOp, &req.agent_id).await {
        Ok(found) => found,
        Err(e) => return app_api_response::<()>(Err(e)).into_response(),
    };
    let result = call_as(&state, &slug, &uid, &req.host, &req.op, req.params, &req.reason).await;
    Json(json!(result)).into_response()
}

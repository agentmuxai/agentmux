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

use crate::server::remote_ops::op::{lan_audience, RemoteOp, RemoteOpResult, Requester, SignedRemoteOp, REMOTE_OP_VERSION};

use super::enabled;
use crate::backend::lan::discovery::{one_per_instance, LanInstance};
use crate::backend::lan::peer_words::{describe_peer, install_words};
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

/// A refusal because the agent isn't on that machine, with the machine said in words: the Store copy
/// of a host that runs no agents answers like a host that has none of this one.
pub(super) fn explain_not_found(result: &mut RemoteOpResult, peer: &LanInstance) {
    if result.refused_reason.as_deref() != Some("not_found") {
        return;
    }
    let said = format!("That was {}.", describe_peer(peer));
    match result.result.get("message").and_then(Value::as_str).map(str::to_string) {
        Some(message) => result.result["message"] = Value::String(format!("{message} {said}")),
        None => result.result = json!({ "message": said }),
    }
}

/// The one LAN instance `host` names, or why not. `machine`, `machine/channel` or
/// `machine/channel/install` (`windows-store`, or "Microsoft Store"): two installs of one channel on a
/// host differ only by how they are installed.
pub(super) fn resolve_peer(peers: &[LanInstance], host: &str) -> Result<LanInstance, String> {
    let wanted = host.trim();
    if wanted.is_empty() {
        return Err("name the machine (host): remote operations never default to a machine".into());
    }
    let mut parts = wanted.splitn(3, '/').map(str::trim);
    let name = parts.next().unwrap_or(wanted);
    let channel = parts.next();
    let install = parts.next();
    // One machine found at two addresses is one instance, not two to choose between.
    let peers = one_per_instance(peers.to_vec());
    let found: Vec<&LanInstance> = peers
        .iter()
        .filter(|p| {
            p.hostname.trim().eq_ignore_ascii_case(name)
                && channel.is_none_or(|c| p.channel.eq_ignore_ascii_case(c))
                && install.is_none_or(|i| p.install.eq_ignore_ascii_case(i) || install_words(&p.install).eq_ignore_ascii_case(i))
        })
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let known: Vec<String> = peers.iter().map(describe_peer).collect();
            Err(format!("no machine {wanted:?} among the LAN peers; they are: {}", known.join(", ")))
        }
        many => {
            // Said so that a name can be copied: the install part tells two of one channel apart.
            let names: Vec<String> = many
                .iter()
                .map(|p| {
                    let base = format!("{}/{}", p.hostname.trim(), p.channel);
                    if p.install.is_empty() { base } else { format!("{base}/{}", p.install) }
                })
                .collect();
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
/// operations that other features make: the fleet Global Memory sync (`fleet_sync`) is the first.
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
        reason: crate::server::remote_ops::op::clip_reason(reason),
    };
    let nonce = envelope.nonce.clone();
    let Some(signed) = envelope.sign_with(&key, super::max_params_of(op)) else {
        return RemoteOpResult::failed("the request could not be signed (too long, or a malformed field)");
    };
    let mut result = post(state, &peer, &signed).await;
    explain_not_found(&mut result, &peer);
    let ok = matches!(result.status, crate::server::remote_ops::op::OpStatus::Done | crate::server::remote_ops::op::OpStatus::Queued | crate::server::remote_ops::op::OpStatus::PendingVeto);
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
    let url = format!("{}/agentmux/remote/op", crate::backend::lan::discovery::peer_base_url(&peer.address, peer.port));
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

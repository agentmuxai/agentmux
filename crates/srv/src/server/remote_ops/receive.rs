// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The receiving half: `POST /agentmux/remote/op` on the peer listener
//! (spec §4.1-4.2, §6). Every refusal is an answer, `status: refused` with a
//! `refused_reason`, never a prompt and never a downgrade; the order below is the
//! order of the checks.
//!
//! 1. the call came over the LAN key (a peer), not a local credential;
//! 2. this machine takes remote operations at all (`network:remote_ops`);
//! 3. the envelope is well formed, and meant for this machine (`audience`);
//! 4. it is fresh (a minute either way) and not older than this srv: the nonce store is in
//!    memory, so a request signed before a restart could be replayed after it;
//! 5. its signature verifies under the key this machine pins for that agent
//!    (`reactive::lan_signer`, the same judgement a LAN jekt gets), and the install
//!    the agent's certificate names, when one is on hand, is the install it claims;
//! 6. the operation exists, and a state-changing one has a caller whose key is vouched for by a
//!    fresh certificate from an install this machine trusts;
//! 7. the nonce is new, or the first answer is repeated;
//! 8. the operation runs under a [`RemoteCaller`].
//!
//! The broadcast `lan_key` only gets a request here; it never authorizes one.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use axum::extract::{ConnectInfo, Extension, State};
use axum::Json;

use agentmux_common::remote_op::{OpStatus, RemoteOpResult, SignedRemoteOp, REMOTE_OP_MAX_SKEW_SECS};

use super::{lookup, RemoteCaller, RemoteResult, Verification};
use crate::server::reactive::lan_signer::{self, SignerKey};
use crate::server::{AppState, ReactiveAuthVia};

/// How long an operation gets to run before the requester is told it failed.
const RUN_TIMEOUT: Duration = Duration::from_secs(25);

/// Longer than the clock window on both sides of it, so a request can't outlive its nonce.
const NONCE_TTL_SECS: i64 = 2 * REMOTE_OP_MAX_SKEW_SECS + 60;
/// Answers kept at most. Past this, with every entry still live, a new request is refused
/// rather than evict a nonce a captured copy could then pass as.
const MAX_NONCES: usize = 5_000;

enum Seen {
    /// The first copy is still running.
    Running,
    Done(RemoteOpResult),
}

struct Entry {
    until: i64,
    seen: Seen,
}

static NONCES: LazyLock<Mutex<HashMap<(String, String), Entry>>> = LazyLock::new(Default::default);

/// When this srv started, in Unix seconds: forced when the routes are built.
static STARTED_AT: LazyLock<i64> = LazyLock::new(agentmux_common::time::now_secs);

pub(crate) fn mark_started() {
    LazyLock::force(&STARTED_AT);
}

/// What a nonce said when it came in.
enum Reservation {
    First,
    Repeat(RemoteOpResult),
}

fn reserve(key: &(String, String), now: i64) -> Result<Reservation, &'static str> {
    let mut nonces = NONCES.lock().unwrap_or_else(|e| e.into_inner());
    nonces.retain(|_, e| e.until > now);
    match nonces.get(key) {
        Some(Entry { seen: Seen::Done(result), .. }) => return Ok(Reservation::Repeat(result.clone())),
        Some(Entry { seen: Seen::Running, .. }) => return Err("in_progress"),
        None => {}
    }
    if nonces.len() >= MAX_NONCES {
        return Err("busy");
    }
    nonces.insert(key.clone(), Entry { until: now + NONCE_TTL_SECS, seen: Seen::Running });
    Ok(Reservation::First)
}

fn settle(key: &(String, String), result: &RemoteOpResult) {
    let mut nonces = NONCES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = nonces.get_mut(key) {
        entry.seen = Seen::Done(result.clone());
    }
}

/// The name of the machine at `from`, among the LAN `peers`; a short label when none is known.
pub(super) fn machine_in(peers: &[crate::backend::lan_discovery::LanInstance], from: Option<SocketAddr>) -> String {
    let Some(ip) = from.map(|a| a.ip().to_string()) else { return "unknown machine".into() };
    peers
        .iter()
        .find(|p| p.address == ip && !p.hostname.trim().is_empty())
        .map(|p| p.hostname.trim().to_string())
        .unwrap_or_else(|| "unknown machine".into())
}

fn machine_name(state: &AppState, from: Option<SocketAddr>) -> String {
    machine_in(&state.lan_discovery.get_instances(), from)
}

/// Peers whose unproven refusal was written to the audit trail lately, so a flood of bad requests
/// from one address makes one row a minute, not a row each.
static UNPROVEN_AUDITED: LazyLock<Mutex<HashMap<(String, String), i64>>> = LazyLock::new(Default::default);
const UNPROVEN_AUDIT_EVERY_SECS: i64 = 60;
const UNPROVEN_AUDIT_MAX_PEERS: usize = 256;

/// A request refused before its signature verified: nobody is proven, so the row names the peer's
/// address, not the agent the request claims to be, and keeps none of the request's own text.
fn audit_unproven(state: &AppState, from: Option<SocketAddr>, reason: &str, now: i64) {
    let peer = from.map(|a| a.ip().to_string()).unwrap_or_else(|| "unknown peer".into());
    {
        let mut seen = UNPROVEN_AUDITED.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, at| now - *at < UNPROVEN_AUDIT_EVERY_SECS);
        let key = (peer.clone(), reason.to_string());
        if seen.contains_key(&key) || seen.len() >= UNPROVEN_AUDIT_MAX_PEERS {
            return;
        }
        seen.insert(key, now);
    }
    state.reactive_handler.log_fleet_action_audit(
        Some(&format!("unproven peer {peer}")),
        "remote-op",
        "",
        "remote.refused (unproven)",
        false,
        Some(reason),
        "",
        None,
    );
}

/// The audit row on this machine: `remote.<op>`, by `agent@machine`, with how it was proven and how it ended.
fn audit(state: &AppState, who: &str, op: &str, proof: &str, result: &RemoteOpResult, nonce: &str, reason: &str) {
    let ok = matches!(result.status, OpStatus::Done | OpStatus::Queued | OpStatus::PendingVeto);
    let note = result.refused_reason.clone().or_else(|| result.result.get("error").and_then(|e| e.as_str()).map(str::to_string));
    state.reactive_handler.log_fleet_action_audit(
        Some(who),
        "remote-op",
        "",
        &format!("remote.{op} ({proof})"),
        ok,
        note.as_deref(),
        nonce,
        (!reason.is_empty()).then_some(reason),
    );
}

pub(crate) async fn handle_remote_op(
    State(state): State<AppState>,
    via: Option<Extension<ReactiveAuthVia>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    Json(signed): Json<SignedRemoteOp>,
) -> Json<RemoteOpResult> {
    let peer_ip = peer.map(|c| c.0);
    let via = via.map(|Extension(v)| v);
    Json(receive(&state, via, peer_ip, signed, agentmux_common::time::now_secs(), *STARTED_AT).await)
}

/// [`handle_remote_op`] without the HTTP: `now` is the clock and `started_at` when this srv began.
pub(super) async fn receive(
    state: &AppState,
    via: Option<ReactiveAuthVia>,
    from: Option<SocketAddr>,
    signed: SignedRemoteOp,
    now: i64,
    started_at: i64,
) -> RemoteOpResult {
    let op = &signed.op;
    let who = format!("{}@{}", op.requester.slug, machine_name(state, from));
    let refuse = |reason: &str, proof: &str| {
        let result = RemoteOpResult::refused(reason);
        if proof == "unproven" {
            audit_unproven(state, from, reason, now);
        } else {
            audit(state, &who, &op.op, proof, &result, &op.nonce, &op.reason);
        }
        result
    };
    if via != Some(ReactiveAuthVia::LanKey) {
        return refuse("not_a_peer", "unproven");
    }
    if !super::enabled(state) {
        return refuse("remote_ops_off", "unproven");
    }
    let max_params = super::max_params_of(&op.op);
    if let Err(why) = op.check_shape_with(max_params) {
        return refuse(why, "unproven");
    }
    if op.audience != agentmux_common::remote_op::lan_audience(&state.lan_key) {
        return refuse("wrong_audience", "unproven");
    }
    if (now - op.ts).abs() > REMOTE_OP_MAX_SKEW_SECS {
        return refuse("stale_request", "unproven");
    }
    if op.ts < started_at {
        return refuse("signed_before_restart", "unproven");
    }
    let key = match lan_signer::resolve(state, &op.requester.slug).await {
        SignerKey::Pinned(key) => key,
        SignerKey::Unknown => return refuse("unknown_sender", "unproven"),
        SignerKey::Refused => return refuse("unverified_sender", "unproven"),
    };
    if !signed.verify_with(&key, max_params) {
        return refuse("bad_signature", "unproven");
    }
    // The install is named by the certificate its agent's host issued, when this machine holds
    // one; the request's own claim must agree with it, and is not believed alone.
    // A certificate counts only if it certifies this agent's name and exactly this key, on the LAN
    // channel, and is fresh (`lan_continuity::check_cert`); the cert carries the slug, not the uid.
    let cert = state
        .lan_discovery
        .cached_lan_cert(&op.requester.slug)
        .filter(|cert| crate::server::reactive::lan_continuity::check_cert(cert, &op.requester.slug, &key, 0, now).is_ok());
    let instance_id = match cert {
        Some(cert) if !op.requester.instance_id.is_empty() && cert.instance_id != op.requester.instance_id => {
            return refuse("install_mismatch", "lan-verified");
        }
        Some(cert) => cert.instance_id,
        None => String::new(),
    };
    let mut caller = RemoteCaller {
        agent_uid: op.requester.agent_uid.clone(),
        slug: op.requester.slug.clone(),
        machine: machine_name(state, from),
        instance_id,
        install_trusted: false,
        verification: Verification::Lan,
        reason: op.reason.clone(),
    };
    let proof = caller.verification.as_str();
    let Some(operation) = lookup(&op.op) else {
        return refuse("unknown_op", proof);
    };
    if operation.changes_state {
        if caller.instance_id.is_empty() {
            return refuse("needs_certificate", proof);
        }
        // A certificate any install can issue for itself proves nothing about who hosts the agent:
        // the install must be one this machine already trusts, as for moving a pinned key.
        if let Err(why) = crate::server::reactive::lan_continuity::instance_trust(state, &caller.instance_id).await {
            tracing::info!(agent = %caller.slug, install = %caller.instance_id, %why, "remote operation refused: the install that vouches for the agent is not trusted here");
            return refuse("install_not_trusted", proof);
        }
        caller.install_trusted = true;
    }
    let nonce_key = (op.requester.slug.to_ascii_lowercase(), op.nonce.clone());
    match reserve(&nonce_key, now) {
        Ok(Reservation::First) => {}
        Ok(Reservation::Repeat(first)) => return first,
        Err(why) => return refuse(why, proof),
    }
    let ran: RemoteResult = match tokio::time::timeout(RUN_TIMEOUT, (operation.run)(state, &caller, op.params.clone())).await {
        Ok(ran) => ran,
        Err(_) => Err(super::RemoteRefusal::failed("the operation did not finish in time")),
    };
    let mut result = match ran {
        Ok(value) => RemoteOpResult::done(value),
        Err(refusal) if refusal.code == "failed" => RemoteOpResult::failed(&refusal.message),
        Err(refusal) => {
            let mut r = RemoteOpResult::refused(refusal.code);
            r.result = serde_json::json!({ "message": refusal.message });
            r
        }
    };
    result.audit_id = op.nonce.clone();
    audit(state, &caller.actor(), &op.op, proof, &result, &op.nonce, &op.reason);
    settle(&nonce_key, &result);
    result
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The quiet note between machines, the srv half (`docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md`
//! section 19; the closed note itself is `backend::quiet_remote`).
//!
//! **Sending.** The watch queues a note for a lead whose seat names another machine (`Name@machine`). Every few seconds
//! this sends what is queued as a jekt signed by this machine's own sender key, the one a Swarm broadcast uses, to the
//! one LAN peer that lists the lead. A lead whose machine is not exactly one listed peer, or whose key this machine has
//! not pinned, is not sent to; that is logged once. A peer that does not answer loses the note: it is about a moment.
//!
//! **Receiving.** A jekt from another machine's sender key whose text starts like a quiet note is never delivered as a
//! jekt. It is accepted only if the signature verified, the install that vouches for the sender key is one this machine
//! trusts, the sender is a LAN peer that lists the named worker, and the lead is a running agent here. What the lead then
//! reads is written here from the closed fields (`quiet_remote::render`), held for its next turn like every other quiet
//! note, and limited to four an hour per lead and sending machine.

use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};

use super::AppState;
use crate::backend::lan::discovery::LanInstance;
use crate::backend::lan::swarm_broadcast as lan;
use crate::backend::quiet_remote::{self as note, Admit, Note};
use crate::backend::swarm_remote::PeerRoute;

const SEND_EVERY: Duration = Duration::from_secs(5);

/// Starts the sender: what the watch queued is sent every few seconds.
pub(crate) fn install(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SEND_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            for out in note::drain() {
                send_one(&state, &out).await;
            }
        }
    });
}

/// The one LAN peer that is `machine` and lists `lead`, or why none: a machine that is not a peer, or lists no such
/// agent, or is two peers of that name (the seat then has to say which).
pub(crate) fn peer_for<'a>(peers: &'a [LanInstance], lead: &str, machine: &str) -> Result<&'a LanInstance, &'static str> {
    let listed: Vec<&LanInstance> = peers
        .iter()
        .filter(|p| p.hostname.trim().eq_ignore_ascii_case(machine.trim()) && p.agents.iter().any(|a| a.eq_ignore_ascii_case(lead)))
        .collect();
    match listed.as_slice() {
        [one] => Ok(one),
        [] => Err("that machine is not a LAN peer here that lists the lead"),
        _ => Err("that machine is more than one LAN peer that lists the lead"),
    }
}

async fn send_one(state: &AppState, out: &note::Outgoing) {
    let Some((lead, machine)) = note::lead_parts(&out.lead) else { return };
    let drop_once = |why: &str| {
        if note::first_drop(&format!("{}:{why}", out.lead)) {
            tracing::info!(lead = %out.lead, why, "quiet note not sent to a lead on another machine");
        }
    };
    let peers = state.lan_discovery.get_instances();
    let peer = match peer_for(&peers, lead, machine) {
        Ok(p) => p,
        Err(why) => return drop_once(why),
    };
    if !matches!(state.mstore.lan_peer_pubkey_pin_since(lead), Ok(Some(_))) {
        return drop_once("this machine has not pinned the lead's key");
    }
    let sender = lan::this_sender_id(&state.hostname);
    let Ok(pair) = state.mstore.agent_lan_key_ensure(&sender) else { return drop_once("this machine's sender key is unreadable") };
    let msg_id = uuid::Uuid::new_v4().to_string();
    let Some(req) = lan::signed_request(&sender, &pair.private_key, lead, &out.note.wire(), &msg_id, agentmux_common::time::now_secs()) else {
        return drop_once("this machine's sender key is unusable");
    };
    let route = PeerRoute { url: crate::backend::lan::discovery::peer_base_url(&peer.address, peer.port), auth_key: peer.auth_key.clone() };
    if let Err(e) = lan::send(&state.http_client, &route, &req).await {
        tracing::debug!(lead = %out.lead, error = %e, "quiet note not taken by the lead's machine");
    }
}

/// What the receiving side decided about a text that claims to be a quiet note.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Not from a machine's sender key, or not a quiet note: delivered as any jekt is.
    Ordinary,
    Refuse(&'static str),
    Accept { note: Note, machine: String },
}

/// Whether `sender` is a machine's own sender name (`swarm-<host>-<hash>`).
fn is_machine_sender(sender: &str) -> bool {
    sender.starts_with("swarm-") && crate::backend::reactive::sanitize::validate_agent_id(sender)
}

/// The decision, from what was proven: `verified` is the signature check's outcome, `install_ok` whether the install that
/// vouches for the sender key is one this machine trusts. Pure, so each refusal has a test.
pub(crate) fn judge(text: &str, sender: &str, verified: Option<bool>, install_ok: bool, peers: &[LanInstance]) -> Verdict {
    if !note::is_quiet_note(text) || !is_machine_sender(sender) {
        return Verdict::Ordinary;
    }
    if verified != Some(true) {
        return Verdict::Refuse("the sender is not proven");
    }
    if !install_ok {
        return Verdict::Refuse("the install that vouches for the sender is not approved here");
    }
    let Some(n) = note::parse(text) else { return Verdict::Refuse("not a quiet note") };
    let hosts: Vec<&LanInstance> = peers.iter().filter(|p| lan::instance_sender_id(p.hostname.trim(), &p.channel) == sender.to_lowercase()).collect();
    let [peer] = hosts.as_slice() else { return Verdict::Refuse("the sender is not one LAN peer here") };
    if !peer.agents.iter().any(|a| a.eq_ignore_ascii_case(&n.agent)) {
        return Verdict::Refuse("the named agent is not on the sending machine");
    }
    Verdict::Accept { note: n, machine: peer.hostname.trim().to_string() }
}

static LIMITER: LazyLock<Mutex<note::Limiter>> = LazyLock::new(Default::default);

/// Whether the install behind `sender`'s pinned key is one this machine trusts: its certificate verifies for exactly
/// that key, and the install is approved (not new, not revoked).
pub(crate) async fn install_approved(state: &AppState, sender: &str) -> bool {
    let Ok(Some((pinned, _))) = state.mstore.lan_peer_pubkey_pin_since(sender) else { return false };
    let Ok(key) = BASE64.decode(pinned) else { return false };
    let Some(cert) = state.lan_discovery.cached_lan_cert(sender) else { return false };
    if super::reactive::lan_continuity::check_cert(&cert, sender, &key, 0, agentmux_common::time::now_secs()).is_err() {
        return false;
    }
    super::lan_watch::during("install trust check", super::reactive::lan_continuity::instance_trust(state, &cert.instance_id)).await.is_ok()
}

/// Settles the claim on the note's message id: a note that was taken is counted, so a copy of it is not proven again;
/// one that was refused releases it, as any refused jekt does. Returns `answer` unchanged.
fn finish(replay: &mut super::reactive::replay::Reservation, answer: Value) -> Value {
    if answer["success"] == true {
        replay.delivered();
    }
    answer
}

/// Puts the note, or the line that stands for the ones past the limit, where the lead reads it. A note joins a running turn
/// or is held for the next, and never starts one. The folded line is only ever held, under one key: written into a running
/// turn it could not be replaced by the next, and each note past the limit would be a message of its own.
fn place(block_id: &str, sender: &str, machine: &str, ts: i64, n: &Note, admit: Admit) {
    match admit {
        Admit::Show => {
            crate::backend::blockcontroller::system_note_into_turn_or_hold(block_id, &format!("lan-quiet:{sender}:{}", n.agent.to_lowercase()), &note::render(sender, machine, ts, n));
        }
        Admit::Fold { more } => crate::backend::reactive::held_notes::hold(block_id, &format!("lan-quiet-more:{sender}"), &note::render_folded(sender, machine, ts, more)),
    }
}

fn answer(req_id: &str, ok: bool, error: &str) -> Value {
    if ok {
        json!({ "success": true, "request_id": req_id, "handled": "quiet-note" })
    } else {
        json!({ "success": false, "refused": true, "request_id": req_id, "error": error })
    }
}

/// A jekt that claims to be a quiet note: answered here (`Some`), never delivered as a jekt. `None` for every other jekt.
pub(crate) async fn intercept(state: &AppState, req: &crate::backend::reactive::types::InjectionRequest, replay: &mut super::reactive::replay::Reservation) -> Option<Value> {
    intercept_among(state, req, replay, state.lan_discovery.get_instances()).await
}

/// [`intercept`] with the LAN peers named, so the whole path can be run with the peers a test lists.
async fn intercept_among(state: &AppState, req: &crate::backend::reactive::types::InjectionRequest, replay: &mut super::reactive::replay::Reservation, peers: Vec<LanInstance>) -> Option<Value> {
    let sender = req.source_agent.as_deref().unwrap_or_default();
    if req.delivery_tier.as_deref() != Some("lan") || !note::is_quiet_note(&req.message) || !is_machine_sender(sender) {
        return None;
    }
    let req_id = req.request_id.clone().unwrap_or_default();
    let install_ok = req.lan_verified == Some(true) && install_approved(state, sender).await;
    let (n, machine) = match judge(&req.message, sender, req.lan_verified, install_ok, &peers) {
        Verdict::Ordinary => return None,
        Verdict::Refuse(why) => {
            tracing::info!(sender, why, "quiet note from another machine not accepted");
            return Some(answer(&req_id, false, why));
        }
        Verdict::Accept { note, machine } => (note, machine),
    };
    let Some(lead) = state.reactive_handler.get_delivery_target(&req.target_agent) else {
        return Some(answer(&req_id, false, "the lead is not a running agent here"));
    };
    let now_ms = agentmux_common::time::now_ms_u64();
    let ts = (now_ms / 1000) as i64;
    let admit = LIMITER.lock().unwrap_or_else(|e| e.into_inner()).admit(&lead.agent_id, &machine, now_ms);
    place(&lead.block_id, sender, &machine, ts, &n, admit);
    Some(finish(replay, answer(&req_id, true, "")))
}

#[cfg(test)]
#[path = "quiet_remote_tests.rs"]
mod tests;

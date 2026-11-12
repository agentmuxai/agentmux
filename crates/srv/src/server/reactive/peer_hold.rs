// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A message to an agent of a LAN peer that is down is kept on the sender's instance for 24 hours and
//! delivered when the peer answers again, as a message to a stopped local agent is. Before this it went
//! to the cloud relay, which drops it after 30 minutes if no install of the account picks it up; a
//! machine that was off for an hour lost it (task #2228).
//!
//! The agent counts as a known peer's when a LAN peer listed it (`backend::lan::seen_agents`, kept
//! across restarts) and that peer no longer answers its poll. A peer that does answer is left to the
//! old path: it can start a stopped agent when the message reaches it, which a held copy here cannot.
//! The relay keeps the message when it says another install of the account runs the agent, or cannot
//! say (`absent_route_for`), as for a local agent that is absent.
//!
//! The row lives in `db_jekt_held` under the target key `peer:<route name>`, which no agent UID can be,
//! so the replay for local agents never sees it.

use super::*;
use crate::backend::lan::discovery::LanInstance;
use crate::backend::lan::seen_agents::{self, SeenAgent};
use crate::backend::reactive::types::InjectionResponse;
use crate::backend::storage::jekt_held::{HeldJekt, HoldOutcome};

/// The target key of a message held for a peer's agent.
const PEER_KEY_PREFIX: &str = "peer:";
/// A peer whose last answered poll is older than this is not answering (polls run every 30 s).
const POLL_FRESH_SECS: u64 = 90;
/// Agents one replay pass looks at (their machine may be down, which costs only a cached lookup),
/// agents it sends to, and messages it sends to one.
const LOOKUPS_PER_PASS: usize = 32;
const AGENTS_PER_PASS: usize = 8;
const ROWS_PER_AGENT: usize = 16;
/// Refusals by a peer that has the agent, before a held message is dropped (as for a local agent).
const MAX_REFUSALS: i64 = 20;

fn peer_key(route_name: &str) -> String {
    format!("{PEER_KEY_PREFIX}{route_name}")
}

/// The host that listed the agent, when no live LAN peer of that name answers its poll now.
fn down_host(seen: Option<SeenAgent>, peers: &[LanInstance], now_secs: u64) -> Option<String> {
    let seen = seen?;
    let answering = peers.iter().any(|p| {
        p.hostname.eq_ignore_ascii_case(&seen.hostname) && now_secs.saturating_sub(p.last_polled_ok) <= POLL_FRESH_SECS
    });
    (!answering).then_some(seen.hostname)
}

/// Whether the relay should take the message for `name` instead: another install of the account runs
/// it, or the relay cannot say (an older one). A relay that is not reachable, or a sender not signed in
/// to it, has nothing to offer, so the message is kept here.
async fn relay_has_it(state: &AppState, source: Option<&str>, name: &str) -> bool {
    use crate::muxbus::wan_lease;
    if wan_lease::held_elsewhere(name).is_some() {
        return true;
    }
    let Some(source) = source.filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(credential) = crate::muxbus::relay::relay_token(source, &state.id_store, &state.http_client).await else {
        return false;
    };
    let answer = wan_lease::holder(&crate::muxbus::relay::rest_base_url(), name, source, &credential.token, &state.http_client).await;
    absent_route_for(&answer) == AbsentRoute::Relay
}

/// Hold `req` for an agent of a LAN peer that is down. `None` leaves it to the relay and the other
/// holds: the hold does not apply to this sender, the agent is not a known peer's, its machine is
/// answering, it is defined here, or the relay has it.
pub(super) async fn hold_for_down_peer(
    state: &AppState,
    auth_via: crate::server::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &InjectionResponse,
) -> Option<serde_json::Value> {
    if !hold_applies(auth_via, req, resp) {
        return None;
    }
    let name = crate::backend::reactive::sanitize::route_name(&req.target_agent)?.to_lowercase();
    let host = down_host(seen_agents::lookup(&name), &state.lan_discovery.get_instances(), now_unix_secs().max(0) as u64)?;
    hold_for_peer(state, req, resp, &name, host).await
}

/// Hold `req` for an agent of a LAN peer that answers its polls but gave no answer to this message (its
/// inbound check was stuck, as narko's was on 2026-11-12 while its cloud login was refreshing). The replay
/// sends it again; the peer that receives it can still start a stopped agent, which this copy cannot.
pub(super) async fn hold_for_silent_peer(
    state: &AppState,
    auth_via: crate::server::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &InjectionResponse,
) -> Option<serde_json::Value> {
    if !hold_applies(auth_via, req, resp) {
        return None;
    }
    let name = crate::backend::reactive::sanitize::route_name(&req.target_agent)?.to_lowercase();
    let host = seen_agents::lookup(&name).map_or_else(|| "its machine".to_string(), |seen| seen.hostname);
    hold_for_peer(state, req, resp, &name, host).await
}

/// The hold itself, for [`hold_for_down_peer`] and [`hold_for_silent_peer`].
async fn hold_for_peer(state: &AppState, req: &InjectionRequest, resp: &InjectionResponse, name: &str, host: String) -> Option<serde_json::Value> {
    let mstore = state.mstore.clone();
    let target = req.target_agent.clone();
    if tokio::task::spawn_blocking(move || defined_here(&mstore, &target)).await.ok()?.is_some() {
        return None;
    }
    if relay_has_it(state, req.source_agent.as_deref(), name).await {
        return None;
    }
    let mut held = crate::server::jekt_hold::held_row(req, resp.request_id.clone(), agentmux_common::time::now_ms());
    held.target_uid = peer_key(name);
    let mstore = state.mstore.clone();
    let outcome = tokio::task::spawn_blocking(move || mstore.jekt_held_insert(&held)).await.ok()?;
    let target = &req.target_agent;
    match outcome {
        Ok(HoldOutcome::Held | HoldOutcome::AlreadyHeld) => {
            tracing::info!(target = %target, host = %host, "peer jekt: the agent's machine is not answering, holding the message");
            Some(serde_json::json!({
                "success": false,
                "held": true,
                "held_reason": HoldReason::PeerOffline.as_str(),
                "retry_when": "peer_online",
                "held_host": host,
                "request_id": resp.request_id,
                "error": format!(
                    "agent {target} is on {host}, which is not answering — held on this AgentMux instance for up to 24 h, delivered when it is seen again"
                ),
            }))
        }
        Ok(HoldOutcome::Full) => Some(serde_json::json!({
            "success": false,
            "request_id": resp.request_id,
            "error": format!("agent {target} is on {host}, which is not answering (hold full: too many messages already held for {target})"),
        })),
        Err(e) => {
            tracing::warn!(error = %e, target = %target, "peer jekt: hold failed");
            None
        }
    }
}

/// What one held message came to on a replay pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    /// Taken (or kept by the peer, or queued at the relay): the row goes.
    Gone,
    /// Not now: the row stays for the next pass.
    Later,
    /// The peer has the agent and refused it: counted, and dropped after [`MAX_REFUSALS`].
    Refused,
}

/// The request a held row is sent as: addressed by the name the sender used, late messages marked.
fn request_for(row: &HeldJekt) -> InjectionRequest {
    let mut req = crate::server::jekt_held::request_from_held(row);
    req.target_agent = row.target_agent.clone();
    req.marker_target = None;
    req
}

/// `message` with the note that it was sent earlier, when it has waited long enough to need one.
fn late_message(row: &HeldJekt, now_ms: i64) -> String {
    let late = crate::muxbus::delivery_status::notes::delayed_prefix(Some(row.sent_at_ms), now_ms);
    format!("{late}{}", row.message)
}

async fn send_over_lan(state: &AppState, row: &HeldJekt, url: &str, auth_key: &str) -> Sent {
    let mut req = request_for(row);
    req.forward_hops = 1;
    let mut on_the_wire = req.clone();
    on_the_wire.message = late_message(row, agentmux_common::time::now_ms());
    let signed = lan_v2::sign_and_log(&state.mstore, &row.audit_source_uid, on_the_wire, url);
    let peer = ForwardPeer {
        url,
        auth_key,
        kind: "lan-peer-held",
        channel: None,
        trust: EchoTrust { delivery_tier: "lan", sig_verified: None, reagent_verified: None, lan_verified: None, channel_verified: None },
    };
    match forward_inject_to_peer(state, &req, &signed, peer).await {
        ForwardOutcome::Delivered(_) => Sent::Gone,
        // The peer kept it itself (it holds for an agent that cannot work): ours is a second copy.
        ForwardOutcome::Refused(body) if body.get("held").and_then(|v| v.as_bool()) == Some(true) => Sent::Gone,
        ForwardOutcome::Refused(_) => Sent::Refused,
        ForwardOutcome::Stale | ForwardOutcome::NoAnswer => {
            state.lan_discovery.evict_agent(&row.target_agent);
            Sent::Later
        }
        ForwardOutcome::Inconclusive => Sent::Later,
    }
}

/// Whether a row's 24 h hold, or the sender's own deadline, has run out: it is dropped, not sent.
fn spent(row: &HeldJekt, now_ms: i64) -> bool {
    row.expires_at_ms <= now_ms || row.sender_expires_at_ms.is_some_and(|deadline| deadline <= now_ms)
}

/// One held message, sent over the LAN (`lan`) or the relay, unless it ran out while the pass was
/// asking where its agent is or sending the ones before it. The peer that receives it cannot tell.
async fn send_row(state: &AppState, row: &HeldJekt, lan: &Option<(String, String)>) -> Sent {
    if spent(row, agentmux_common::time::now_ms()) {
        return Sent::Gone;
    }
    match lan {
        Some((url, auth_key)) => send_over_lan(state, row, url, auth_key).await,
        None => send_over_relay(state, row).await,
    }
}

async fn send_over_relay(state: &AppState, row: &HeldJekt) -> Sent {
    let mut req = request_for(row);
    req.message = late_message(row, agentmux_common::time::now_ms());
    match try_cloud_relay(state, &req, &row.audit_source_uid).await {
        Some(_) => Sent::Gone,
        None => Sent::Later,
    }
}

async fn settle(state: &AppState, row: &HeldJekt, sent: Sent) {
    let mstore = state.mstore.clone();
    let id = row.request_id.clone();
    let _ = tokio::task::spawn_blocking(move || match sent {
        Sent::Gone => {
            let _ = mstore.jekt_held_delete(&id);
        }
        Sent::Refused => {
            if mstore.jekt_held_record_failure(&id, "refused by the peer").unwrap_or(0) >= MAX_REFUSALS {
                let _ = mstore.jekt_held_delete(&id);
            }
        }
        Sent::Later => {}
    })
    .await;
}

/// `keys` starting at the `by`-th, so each pass looks first at a different agent.
fn rotated(mut keys: Vec<String>, by: usize) -> Vec<String> {
    if !keys.is_empty() {
        let n = keys.len();
        keys.rotate_left(by % n);
    }
    keys
}

/// One pass over the messages held for peers' agents: each agent now on the LAN gets its messages,
/// oldest first; one the relay has (another install runs it) goes there. Run beside the replay for
/// local agents, whose pass also drops what expired.
pub(crate) async fn replay_pass(state: &AppState) {
    let mstore = state.mstore.clone();
    let keys: Vec<String> = tokio::task::spawn_blocking(move || mstore.jekt_held_targets().unwrap_or_default())
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.starts_with(PEER_KEY_PREFIX))
        .collect();
    // The list is oldest message first, so the agents of a machine that stays down would hold the front
    // of it for good: start at a different one each pass, and count against the cap only an agent that
    // can be sent to (the lookups of the rest are cached).
    static ROTATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let keys = rotated(keys, ROTATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let mut sent_to = 0;
    for key in keys.into_iter().take(LOOKUPS_PER_PASS) {
        if sent_to >= AGENTS_PER_PASS {
            break;
        }
        let mstore = state.mstore.clone();
        let rows_key = key.clone();
        let rows = tokio::task::spawn_blocking(move || mstore.jekt_held_for_target(&rows_key, ROWS_PER_AGENT))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let Some(first) = rows.first() else { continue };
        let name = key.trim_start_matches(PEER_KEY_PREFIX);
        // Asked for as the sender wrote it, as the live path does.
        let lan = state.lan_discovery.find_agent(&first.target_agent, &state.http_client).await;
        if lan.is_none() && !relay_has_it(state, Some(&first.source_agent), name).await {
            continue;
        }
        sent_to += 1;
        for row in &rows {
            let sent = send_row(state, row, &lan).await;
            settle(state, row, sent).await;
            if sent == Sent::Later {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;

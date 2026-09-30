// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Taking an agent over from another AgentMux instance on this host
//! (`docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` Phase 2, §4.6).
//!
//! Two halves, both behind full `auth_key` auth:
//!
//! - `POST /api/v1/agent/takeover {block_id}` — on the instance the user is
//!   in. Its pane was refused because another instance runs the agent; the
//!   user confirmed "Take over". Finds that instance (lease holder, or the
//!   compat probe for one that takes no lease), asks it to let go, and waits
//!   until the agent is free. The pane then runs its turn as usual.
//! - `POST /agentmux/agent/release {uid}` — on the holder. Gracefully stops
//!   the CLI process of every pane of this srv that holds `uid`'s lease,
//!   shows "taken over" in those panes, and keeps this srv from re-claiming
//!   the agent on its own for a short hold.
//!
//! Never jekt-drivable (spec I5, D7): a jekt body cannot reach either route.
//! The requester side is reached from the pane's own Take over action after
//! a confirmation; the holder side authenticates with the holder's own full
//! auth key, which the requester reads from the host-global shared registry
//! — the same trust cross-channel jekt forwarding already relies on.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::backend::agent_admission;
use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block};

/// How long the holder lets a turn in flight finish before killing it.
const RELEASE_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// How long the requester waits for the agent to become free.
const TAKEOVER_WAIT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Debug, Deserialize)]
pub(super) struct TakeoverRequest {
    block_id: String,
}

pub(super) async fn handle_agent_takeover(
    State(state): State<AppState>,
    Json(req): Json<TakeoverRequest>,
) -> Response {
    let block = {
        let mstore = state.mstore.clone();
        let block_id = req.block_id.clone();
        tokio::task::spawn_blocking(move || mstore.get::<Block>(&block_id)).await
    };
    let Ok(Ok(Some(block))) = block else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "block not found" }))).into_response();
    };
    let uid = obj::meta_get_string(&block.meta, "agentId", "");
    if uid.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "this pane has no agent" }))).into_response();
    }
    let name = obj::meta_get_string(&block.meta, "agentName", "");

    // The user is deliberately taking the agent (back) here.
    agent_admission::clear_handover_hold(&uid);
    let store = agent_admission::lease_store_for(state.mstore.shared_agent_registry());
    let holder = match agent_admission::locate_holder(store.clone(), &uid, &state.boot_id).await {
        Some(holder) => holder,
        // No local lease or running instance: the relay may still name a
        // holder (its WAN lease outlives a closed pane on an older build).
        None => match wan_takeover_target(&name) {
            // Nobody else runs it (any more): nothing to take over.
            WanTarget::NoHolder => {
                return (StatusCode::OK, Json(json!({ "ok": true, "released": false }))).into_response();
            }
            WanTarget::OtherComputer(where_) => {
                return (StatusCode::CONFLICT, Json(json!({ "error": other_computer_error(&name, &where_) })))
                    .into_response();
            }
            WanTarget::ThisComputer(channel) => match agent_admission::endpoint_for_channel(&channel).await {
                Some(holder) => holder,
                None => {
                    let agent = if name.is_empty() { "this agent" } else { name.as_str() };
                    let error = format!(
                        "AgentMux cloud says the instance on this computer in channel {channel} holds {agent}, \
                         but that instance isn't running. Its hold ends within a minute; try again then."
                    );
                    return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response();
                }
            },
        },
    };
    tracing::info!(
        uid = %uid,
        block_id = %req.block_id,
        holder_channel = %holder.channel,
        "agent_admission.takeover: requesting release from the holder"
    );
    if let Err(e) = agent_admission::request_release(&holder, &uid, &name).await {
        return (StatusCode::CONFLICT, Json(json!({ "error": e }))).into_response();
    }
    if let Err(e) = agent_admission::wait_until_free(store, &uid, &state.boot_id, TAKEOVER_WAIT).await {
        return (StatusCode::GATEWAY_TIMEOUT, Json(json!({ "error": e }))).into_response();
    }
    // The holder let go: forget the cached relay refusal, or the pane's
    // retry is refused from it for up to 90 s (Codex P1 on #3899).
    crate::muxbus::wan_lease::forget_elsewhere(&name).await;
    // ...and take the relay lease now. A holder older than #3899 answers the
    // release but keeps renewing the lease for as long as it runs, so the new
    // pane would be fenced seconds after starting (narko 2026-09-27; see
    // docs/investigations/INVESTIGATION_TAKE_OVER_OLD_HOLDER_RELAY_LEASE_2026_09_27.md).
    // Unknown (no MuxBus session, an unreachable relay) fails open as before.
    match crate::muxbus::cloud_subscriber::take_over_lease_now(&state.id_store, &state.http_client, &name).await {
        crate::muxbus::wan_lease::Outcome::HeldElsewhere(where_) => {
            let error = if crate::muxbus::wan_lease::holder_on_this_computer(&where_) {
                kept_relay_hold_error(&name, &where_)
            } else {
                other_computer_error(&name, &where_)
            };
            tracing::warn!(uid = %uid, holder = %where_, "agent_admission.takeover: the relay still names another holder");
            return (StatusCode::CONFLICT, Json(json!({ "error": error }))).into_response();
        }
        crate::muxbus::wan_lease::Outcome::Held => {
            tracing::info!(uid = %uid, "agent_admission.takeover: relay lease is ours");
        }
        crate::muxbus::wan_lease::Outcome::Unknown => {}
    }
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "released": true, "from_channel": holder.channel })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct HoldingQuery {
    uid: String,
}

/// `GET /agentmux/agent/holding?uid=…` — a LAN peer asking whether this host
/// runs `uid` (spec §4.4, Phase 4). LAN-key readable, like the agent-name
/// list: it discloses only whether an agent UID is live here, since when,
/// and in which channel/version.
pub(super) async fn handle_agent_holding(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<HoldingQuery>,
) -> Response {
    let uid = q.uid.trim().to_string();
    if uid.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "uid required" }))).into_response();
    }
    let store = agent_admission::lease_store_for(state.mstore.shared_agent_registry());
    let holding = tokio::task::spawn_blocking(move || agent_admission::local_holding(store.as_ref(), &uid))
        .await
        .unwrap_or_default();
    (StatusCode::OK, Json(json!(holding))).into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseRequest {
    uid: String,
    /// The agent's name, from requesters that send it: lets this srv drop a
    /// relay subscription (and WAN lease) that outlived the agent's panes.
    #[serde(default)]
    agent: String,
    #[serde(default)]
    requested_by_channel: String,
    #[serde(default)]
    requested_by_version: String,
}

pub(super) async fn handle_agent_release(State(state): State<AppState>, Json(req): Json<ReleaseRequest>) -> Response {
    let uid = req.uid.trim().to_string();
    if uid.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "uid required" }))).into_response();
    }
    let blocks = {
        let mstore = state.mstore.clone();
        let uid = uid.clone();
        tokio::task::spawn_blocking(move || {
            mstore
                .get_all::<Block>()
                .unwrap_or_default()
                .into_iter()
                .filter(|b| obj::meta_get_string(&b.meta, "agentId", "") == uid)
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default()
    };
    let to = describe_requester(&req.requested_by_channel, &req.requested_by_version);
    let deadline = std::time::Instant::now() + RELEASE_GRACE;
    let mut released = 0usize;
    for block in blocks {
        let Some(ctrl) = blockcontroller::get_controller(&block.oid) else { continue };
        let Some(p) = ctrl
            .as_any()
            .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
        else {
            continue;
        };
        if !p.release_to_other_instance(deadline) {
            continue;
        }
        released += 1;
        let name = obj::meta_get_string(&block.meta, "agentName", "");
        let who = if name.is_empty() { "This agent" } else { name.as_str() };
        let reason = format!(
            "{who} was taken over by another AgentMux instance on this host{}. \
             Use Take over to bring it back here.",
            if to.is_empty() { String::new() } else { format!(" ({to})") }
        );
        surface_failure(&state, &block.oid, &reason);
    }
    // The relay may still see this srv as the holder — even with no pane
    // running it: a subscription that outlived its panes (charlie,
    // 2026-09-26) renews the WAN lease every tick. Let go of it now, and
    // before answering, so the requester's retry can claim the lease.
    //
    // Also after stopping live panes (Codex P1 on #3903): their exit hands
    // the WAN release to a background task, so the requester's retry could
    // still meet this srv's lease and be fenced. The handover hold keeps
    // this srv from re-claiming the agent, so letting go here is safe.
    let agent = req.agent.trim();
    let was_live = released > 0;
    // The hold goes up before the release is awaited (Codex P1 on #3908):
    // otherwise a local turn in that window re-registers the agent, re-adds
    // its subscription, and this srv keeps the lease it claims to hand over.
    let hand_over = was_live || !agent.is_empty();
    if hand_over {
        agent_admission::hold_after_handover(&uid, &to);
    }
    if !agent.is_empty() {
        use crate::muxbus::cloud_subscriber::RelayRelease;
        match crate::muxbus::cloud_subscriber::release_agent_now(&state.id_store, agent).await {
            RelayRelease::Released => {
                tracing::info!(agent = %agent, was_live, "agent_admission.takeover: released the relay subscription and lease");
                if !was_live {
                    released += 1;
                }
            }
            // Re-added despite the hold (a turn already past admission): the
            // relay lease stays here, so the requester's retry would be fenced.
            RelayRelease::Kept => {
                tracing::warn!(agent = %agent, "agent_admission.takeover: the agent was re-added while releasing — relay lease kept");
            }
            // No relay subscription (or no relay at all): nothing to release.
            RelayRelease::NotSubscribed => {}
        }
    }
    if released == 0 && hand_over {
        // Nothing was handed over after all: don't block this srv's own use.
        agent_admission::clear_handover_hold(&uid);
    }
    tracing::info!(uid = %uid, released, to = %to, "agent_admission.takeover: release handled");
    (StatusCode::OK, Json(json!({ "ok": true, "released": released }))).into_response()
}

fn describe_requester(channel: &str, version: &str) -> String {
    match (channel.is_empty(), version.is_empty()) {
        (false, false) => format!("channel {channel}, v{version}"),
        (false, true) => format!("channel {channel}"),
        (true, false) => format!("v{version}"),
        (true, true) => String::new(),
    }
}

/// Show `message` in the pane's recovery row — the same persisted meta and
/// live event a spawn refusal uses.
fn surface_failure(state: &AppState, block_id: &str, message: &str) {
    let failure = crate::agents::failure::classify(None, None, message, None);
    blockcontroller::core::persist_last_failure(
        block_id,
        Some(&failure),
        &Some(state.mstore.clone()),
        &Some(state.event_bus.clone()),
    );
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: crate::backend::mps::EVENT_AGENT_FAILURE.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 1,
        data: serde_json::to_value(&failure).ok(),
    });
}

/// Who holds `agent` per the relay, when no local lease or running
/// instance does. Charlie 2026-09-26: a same-computer instance (another
/// channel) held only the relay lease, and Take over looked nowhere else,
/// answered `released: false`, and the pane was refused again.
#[derive(Debug, PartialEq, Eq)]
enum WanTarget {
    NoHolder,
    /// Another instance on this computer, by channel.
    ThisComputer(String),
    /// Another computer, as described by the relay's holder record.
    OtherComputer(String),
}

fn wan_takeover_target(agent: &str) -> WanTarget {
    if let Some(channel) = crate::muxbus::wan_lease::held_on_this_computer_by(agent) {
        return WanTarget::ThisComputer(channel);
    }
    match crate::muxbus::wan_lease::held_on_another_computer(agent) {
        Some(where_) => WanTarget::OtherComputer(where_),
        None => WanTarget::NoHolder,
    }
}

/// Take over reaches instances on this computer only.
/// The instance on this computer let go of the pane but the relay still
/// names it, and the relay wouldn't move the lease (a relay without
/// `/agents/lease/take`, or a lease claimed for another account).
fn kept_relay_hold_error(agent: &str, where_: &str) -> String {
    let agent = if agent.is_empty() { "this agent" } else { agent };
    format!(
        "AgentMux cloud still says {agent} is held by the AgentMux instance on {where_}. \
         That instance is probably an older version that can't hand it over. \
         Quit it completely (from the tray, not just its window); its hold ends \
         within a minute, then try again."
    )
}

fn other_computer_error(agent: &str, where_: &str) -> String {
    let agent = if agent.is_empty() { "this agent" } else { agent };
    format!(
        "{agent} is running on another computer ({where_}), according to AgentMux cloud. \
         Take over only works between AgentMux instances on this computer. \
         Close {agent} there, then try again."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requester_is_described_by_what_it_sent() {
        assert_eq!(describe_requester("dev-x", "0.58.0"), "channel dev-x, v0.58.0");
        assert_eq!(describe_requester("", "0.58.0"), "v0.58.0");
        assert_eq!(describe_requester("", ""), "");
    }

    /// The holder pane's message is classified as the takeover case, so its
    /// row offers Take over (to bring the agent back), not Retry.
    #[test]
    fn the_holder_side_message_classifies_as_taken_over() {
        let reason = "Agent3 was taken over by another AgentMux instance on this host (channel dev-x, v0.58.0). \
                      Use Take over to bring it back here.";
        let f = crate::agents::failure::classify(None, None, reason, None);
        assert_eq!(f.code, crate::agents::failure::FailureClass::LiveElsewhere);
        assert_eq!(f.title, "Taken over by another AgentMux instance");
    }

    // Charlie 2026-09-26: the relay said 0.57.6 on the same computer held
    // Opaz, but Take over only looked at the local lease file and running
    // instances, found nobody, answered `released: false` and did nothing.
    #[test]
    fn take_over_follows_a_relay_holder_on_this_computer() {
        let here = crate::backend::reactive::registry::local_host_label();
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        crate::muxbus::wan_lease::note_not_holder(
            &agent,
            &serde_json::json!({ "held_by": { "host": here, "channel": "local-main-x", "version": "0.57.6" } }),
        );
        assert_eq!(wan_takeover_target(&agent), WanTarget::ThisComputer("local-main-x".into()));
    }

    #[test]
    fn take_over_says_why_it_cannot_reach_another_computer() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        crate::muxbus::wan_lease::note_not_holder(&agent, &serde_json::json!({ "held_by": { "host": "desk", "channel": "stable" } }));
        let WanTarget::OtherComputer(where_) = wan_takeover_target(&agent) else { panic!("expected another computer") };
        let m = other_computer_error("Opaz", &where_);
        assert!(m.contains("another computer (computer desk, channel stable)"), "{m}");
        assert!(m.contains("Close Opaz there"), "{m}");
        assert_eq!(wan_takeover_target("never-seen"), WanTarget::NoHolder);
    }
}

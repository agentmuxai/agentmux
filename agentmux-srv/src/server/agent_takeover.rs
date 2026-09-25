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
    let Some(holder) = agent_admission::locate_holder(store.clone(), &uid, &state.boot_id).await else {
        // Nobody else runs it (any more): nothing to take over.
        return (StatusCode::OK, Json(json!({ "ok": true, "released": false }))).into_response();
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
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "released": true, "from_channel": holder.channel })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseRequest {
    uid: String,
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
    if released > 0 {
        agent_admission::hold_after_handover(&uid, &to);
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
}

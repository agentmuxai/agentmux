// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /agentmux/viewer/agents/:name/input`: a paired device sends a
//! message to an agent, as the user (agentmux-mobile's
//! SPEC_SEND_FROM_THE_PHONE_2026_10_13 §4, §5). The one viewer route that
//! changes anything, so it asks for more than the token:
//!
//! - the owner's grant for this device (Settings › Paired devices, Can send);
//! - a signature by the key the device made at pairing, over method, path,
//!   time and body (`viewer::signing`), fresh within a minute;
//! - a `message_id`, so a retry is delivered once (`viewer::sends`), and at
//!   most 30 sends a minute.
//!
//! The agent gets a plain user turn, delivered the way a Swarm broadcast is:
//! a running agent reads it at its next step, mid-turn included; an idle or
//! stopped one starts a turn. The stored record says which device sent it
//! (`viewer::origin`), for the desktop pane's note and the phone's own
//! "Sending…" line.

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::Path;
use serde::Deserialize;

use super::http_viewer::{agent_hidden, ViewerCaller};
use super::*;
use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
use crate::backend::reactive::{device_turn_message, SenderDelivery, MAX_MESSAGE_LENGTH};
use crate::backend::viewer::origin::{self, DeviceOrigin};
use crate::backend::viewer::sends::{Admit, Sent};
use crate::backend::viewer::signing::{self, DeviceKey};
use crate::bootstrap::AgentRoute;
use crate::server::agent_handlers::{run_agent_turn, AgentTurnDeps, TurnRegistration};

/// The largest request body: a message at `MAX_MESSAGE_LENGTH` bytes, JSON
/// escaped, with room to spare.
pub(crate) const MAX_INPUT_BODY: usize = 64 * 1024;

#[derive(Deserialize)]
struct InputBody {
    message: String,
    message_id: String,
}

fn refuse(status: StatusCode, reason: Option<&str>, error: &str) -> Response {
    let mut body = json!({ "error": error });
    if let Some(reason) = reason {
        body["reason"] = json!(reason);
    }
    (status, Json(body)).into_response()
}

/// Why delivery failed, for the phone: `starting` and `needs_login` pass;
/// `not_running` means nothing will take it until the agent is opened.
fn delivery_refusal(error: &str) -> Response {
    let lower = error.to_lowercase();
    let reason = if lower.contains("identity spawn gate") || lower.contains("log in") || lower.contains("sign in") {
        "needs_login"
    } else if lower.contains("starting up") || lower.contains("stopping") || lower.contains("queue is full") {
        "starting"
    } else if lower.contains("no controller") {
        "not_running"
    } else {
        "failed"
    };
    refuse(StatusCode::CONFLICT, Some(reason), error)
}

/// Deliver `text` to `block_id` as the user's turn, the way a Swarm
/// broadcast is (`app_api::fleet::deliver_broadcast_turn`).
async fn deliver(state: &AppState, block_id: &str, text: &str) -> Result<Sent, Response> {
    let input = TurnInput { origin: TurnOrigin::User, text: text.to_string(), joins_turn: None };
    match crate::bootstrap::route_agent_message_from(block_id, text, Some(input)) {
        Ok(AgentRoute::Sent(SenderDelivery::Delivered)) => Ok(Sent::Delivered),
        Ok(AgentRoute::Sent(SenderDelivery::Deferred)) => Ok(Sent::Queued),
        Ok(AgentRoute::Sent(SenderDelivery::Pty)) => Err(refuse(
            StatusCode::CONFLICT,
            Some("not_supported"),
            "this pane runs in a terminal; messages can't be sent to it",
        )),
        // Not running, or a kind that takes a turn per message: start one.
        // The block came from the agent registry, so it is registered.
        Ok(AgentRoute::StartTurn { .. }) => {
            let deps = AgentTurnDeps::from_state(state);
            run_agent_turn(&deps, block_id.to_string(), text.to_string(), None, TurnRegistration::Skip, TurnOrigin::User, Vec::new())
                .await
                .map(|()| Sent::Delivered)
                .map_err(|e| delivery_refusal(&e))
        }
        Err(e) => Err(delivery_refusal(&e)),
    }
}

/// `POST /agentmux/viewer/agents/:name/input`: `{message, message_id}`,
/// signed; `202 {message_id, state}`. See the module comment and the spec's
/// §5.2.1 for every refusal.
pub(crate) async fn handle_viewer_input(
    State(state): State<AppState>,
    Extension(caller): Extension<ViewerCaller>,
    Path(name): Path<String>,
    uri: axum::extract::OriginalUri,
    headers: axum::http::HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Ok(body) = body else {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, None, "the request is too large");
    };
    let device = match state.mstore.viewer_device_get(&caller.device_id) {
        Ok(Some(device)) => device,
        // Revoked between the token check and here.
        Ok(None) => return super::auth::unauthorized(),
        Err(e) => {
            tracing::warn!(error = %e, "viewer input: could not read the device");
            return refuse(StatusCode::INTERNAL_SERVER_ERROR, None, "could not read the device");
        }
    };
    if !device.can_send {
        return refuse(
            StatusCode::FORBIDDEN,
            Some("send_off"),
            "sending is off for this device: turn it on in AgentMux › Settings › Paired devices",
        );
    }
    let Ok(key) = DeviceKey::parse(&device.device_key) else {
        return refuse(StatusCode::FORBIDDEN, Some("pair_again"), "this device has no key: pair it again to send");
    };

    // The signature, before anything in the body is trusted.
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let (Some(timestamp), Some(signature)) = (header(signing::TIMESTAMP_HEADER), header(signing::SIGNATURE_HEADER)) else {
        return refuse(StatusCode::FORBIDDEN, Some("bad_signature"), "the request is not signed");
    };
    let Ok(timestamp_ms) = timestamp.parse::<i64>() else {
        return refuse(StatusCode::FORBIDDEN, Some("bad_signature"), "the signing time is not a number");
    };
    let signed = signing::signed_text("POST", uri.0.path(), timestamp_ms, &body);
    if !key.verifies(signed.as_bytes(), signature) {
        tracing::warn!(device = %caller.device_id, "viewer input: signature did not verify");
        return refuse(StatusCode::FORBIDDEN, Some("bad_signature"), "the signature does not verify");
    }
    if !signing::fresh(timestamp_ms, agentmux_common::time::now_ms()) {
        return refuse(
            StatusCode::FORBIDDEN,
            Some("stale_timestamp"),
            "the request was signed more than a minute from this computer's clock",
        );
    }

    let Ok(input) = serde_json::from_slice::<InputBody>(&body) else {
        return refuse(StatusCode::BAD_REQUEST, None, "expected {message, message_id}");
    };
    let message = input.message.trim_end();
    if message.trim().is_empty() {
        return refuse(StatusCode::BAD_REQUEST, None, "the message is empty");
    }
    if message.len() > MAX_MESSAGE_LENGTH {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, None, &format!("a message is at most {MAX_MESSAGE_LENGTH} bytes"));
    }
    if uuid::Uuid::parse_str(&input.message_id).is_err() {
        return refuse(StatusCode::BAD_REQUEST, None, "message_id must be a UUID");
    }

    let Some(agent) = state.reactive_handler.get_agent(&name) else {
        return refuse(StatusCode::NOT_FOUND, None, "no such agent");
    };
    // Hidden agents are 404, as in the list and the feed: not even named.
    if agent_hidden(&state, &agent.block_id) {
        return refuse(StatusCode::NOT_FOUND, None, "no such agent");
    }

    let admitted = state.viewer.sends.lock().admit(&device.device_id, &input.message_id, std::time::Instant::now());
    match admitted {
        Admit::New => {}
        Admit::Duplicate(sent) => {
            return (
                StatusCode::ACCEPTED,
                Json(json!({ "message_id": input.message_id, "state": sent.as_str(), "duplicate": true })),
            )
                .into_response()
        }
        Admit::InFlight => {
            return refuse(StatusCode::CONFLICT, Some("in_flight"), "this message is being sent; try again in a moment")
        }
        Admit::RateLimited(wait) => {
            let secs = wait.as_secs().max(1).to_string();
            let mut resp = refuse(StatusCode::TOO_MANY_REQUESTS, None, "too many messages; wait a moment");
            if let Ok(v) = axum::http::HeaderValue::from_str(&secs) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            return resp;
        }
    }

    // Delivered and recorded in a task of its own: a phone that drops the
    // connection mid-request must not leave the id in flight (a retry would
    // get `in_flight` for 10 minutes) or a delivery half done.
    let text = device_turn_message(message);
    let block_id = agent.block_id.clone();
    let message_id = input.message_id.clone();
    let task = tokio::spawn({
        let state = state.clone();
        let device_id = device.device_id.clone();
        let origin = DeviceOrigin { device: device.device_name.clone(), message_id: message_id.clone() };
        let message_id = message_id.clone();
        async move {
            origin::expect(&block_id, &text, origin);
            let outcome = deliver(&state, &block_id, &text).await;
            state.viewer.sends.lock().finish(&device_id, &message_id, outcome.as_ref().ok().copied());
            if outcome.is_err() {
                origin::forget(&block_id, &message_id);
            }
            outcome
        }
    });
    match task.await {
        Ok(Ok(sent)) => {
            tracing::info!(device = %device.device_id, agent = %name, state = sent.as_str(), "viewer input: sent");
            (StatusCode::ACCEPTED, Json(json!({ "message_id": message_id, "state": sent.as_str() }))).into_response()
        }
        Ok(Err(resp)) => resp,
        Err(e) => {
            // The task panicked: nothing recorded it as sent, so a retry tries again.
            tracing::warn!(error = %e, "viewer input: delivery task failed");
            state.viewer.sends.lock().finish(&device.device_id, &message_id, None);
            origin::forget(&agent.block_id, &message_id);
            refuse(StatusCode::INTERNAL_SERVER_ERROR, Some("failed"), "delivery failed")
        }
    }
}

#[cfg(test)]
#[path = "tests/viewer/viewer_input_tests.rs"]
mod tests;

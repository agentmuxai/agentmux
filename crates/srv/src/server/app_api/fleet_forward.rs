// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The receiving end of a Swarm broadcast forwarded between channels of one install, and the
//! confirmation that decides how it is delivered.
//!
//! A forward is the person's own broadcast (the user-voice header, `MessageOrigin::broadcast()`)
//! only when another instance on this machine confirms it sent that forward from the operator's
//! window. Any other forward is delivered as an ordinary agent message from `swarm-forward`,
//! labelled self-declared. The sending instance keeps what it is about to forward in memory
//! (`vouch_for_forward`, called only by `fleet_broadcast_impl`, which only the operator's window
//! reaches) and answers a confirmation once, for exactly that forward; a confirmation carries the
//! body's SHA-256, never the words.

use agentmux_common::AUTH_KEY_HEADER;

use crate::backend::message_origin::MessageOrigin;
use crate::backend::reactive::broadcast_turn_message;

use super::fleet::{deliver_broadcast_turn, BroadcastForward, FLEET_BROADCAST_AUDIT_ACTION, NO_REGISTERED_AGENT};
use super::AppState;
use crate::server::agent_handlers::AgentTurnDeps;

/// The broadcasts this instance has sent on to another channel and not yet had confirmed.
static VOUCHED_FORWARDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), VouchedForward>>> = std::sync::OnceLock::new();
const VOUCH_FOR: std::time::Duration = std::time::Duration::from_secs(120);

struct VouchedForward {
    body_sha256: String,
    recipients: usize,
    until: std::time::Instant,
}

/// The SHA-256 of a forwarded body, in hex: what a confirmation carries instead of the words themselves.
pub(crate) fn body_digest(body: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn vouched() -> std::sync::MutexGuard<'static, std::collections::HashMap<(String, String), VouchedForward>> {
    VOUCHED_FORWARDS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Records a forward this instance is about to send (called only for a real operator broadcast).
pub(super) fn vouch_for_forward(req: &BroadcastForward<'_>) {
    let now = std::time::Instant::now();
    let mut table = vouched();
    table.retain(|_, v| v.until > now);
    table.insert(
        (req.msg_id.to_string(), req.block_id.to_string()),
        VouchedForward { body_sha256: body_digest(req.body), recipients: req.recipients, until: now + VOUCH_FOR },
    );
}

/// Whether this instance sent exactly this forward (the body given by its SHA-256): true once, and the entry is spent.
pub(crate) fn confirm_forward(msg_id: &str, block_id: &str, body_sha256: &str, recipients: usize) -> bool {
    let now = std::time::Instant::now();
    let mut table = vouched();
    table.retain(|_, v| v.until > now);
    match table.get(&(msg_id.to_string(), block_id.to_string())) {
        Some(v) if v.body_sha256 == body_sha256 && v.recipients == recipients => {
            table.remove(&(msg_id.to_string(), block_id.to_string()));
            true
        }
        _ => false,
    }
}

/// Asks the other instances on this machine (the shared registry's loopback entries, never this one)
/// whether one of them sent this forward. The channel that did answers yes, once.
async fn forward_confirmed_by_a_sender(state: &AppState, block_id: &str, body: &str, msg_id: &str, recipients: usize) -> Option<String> {
    let dir = crate::registry::resolve_shared_reactive_dir()?;
    let mut asked = std::collections::HashSet::new();
    for entry in crate::backend::reactive::registry::list_all_shared(&dir) {
        let url = entry.local_url.clone();
        let loopback = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost") || url.starts_with("http://[::1]");
        if !loopback || url == state.local_web_url || !asked.insert(url.clone()) {
            continue;
        }
        let mut http = state
            .http_client
            .post(format!("{url}/agentmux/agent/broadcast/confirm"))
            .timeout(std::time::Duration::from_secs(3))
            // The digest, not the words: an instance asked here is not necessarily one of ours.
            .json(&serde_json::json!({ "block_id": block_id, "body_sha256": body_digest(body), "msg_id": msg_id, "recipients": recipients }));
        if !entry.auth_key.is_empty() {
            http = http.header(AUTH_KEY_HEADER, entry.auth_key.as_str());
        }
        let Ok(resp) = http.send().await else { continue };
        let Ok(answer) = resp.json::<serde_json::Value>().await else { continue };
        if answer.get("confirmed").and_then(|v| v.as_bool()) == Some(true) {
            return Some(url);
        }
    }
    None
}

/// The receiving end of `forward_broadcast_to_channel`. A forward that another instance of this
/// machine confirms it sent (from the operator's window) is delivered as the person's broadcast: the
/// header is built here, from the target's own agent id, as any broadcast turn
/// (`MessageOrigin::broadcast()`). Any other forward is delivered as an ordinary agent message from
/// `swarm-forward`, labelled self-declared, never as the person. Audited on this side too, naming
/// which of the two it was.
pub(crate) async fn deliver_forwarded_broadcast(
    state: &AppState,
    block_id: &str,
    body: &str,
    msg_id: &str,
    recipients: usize,
) -> Result<(), String> {
    let confirmed = state.reactive_handler.get_agent_by_block(block_id).is_some()
        && forward_confirmed_by_a_sender(state, block_id, body, msg_id, recipients).await.is_some();
    deliver_forward(state, block_id, body, msg_id, recipients, confirmed).await.1
}

/// How a forwarded broadcast was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardedAs {
    /// The person's own broadcast: the sending instance confirmed it came from the operator's window.
    Person,
    /// An ordinary agent message from `swarm-forward`.
    AgentMessage,
}

async fn deliver_forward(
    state: &AppState,
    block_id: &str,
    body: &str,
    msg_id: &str,
    recipients: usize,
    confirmed: bool,
) -> (ForwardedAs, Result<(), String>) {
    let mut how = "forwarded from another channel on this machine";
    let mut kind = ForwardedAs::Person;
    let result = match state.reactive_handler.get_agent_by_block(block_id) {
        None => Err(NO_REGISTERED_AGENT.to_string()),
        Some(agent) if confirmed => {
            let text = broadcast_turn_message(&agent.agent_id, recipients, msg_id, body);
            let deps = AgentTurnDeps::from_state(state);
            deliver_broadcast_turn(&deps, block_id.to_string(), text, MessageOrigin::broadcast()).await
        }
        Some(agent) => {
            how = "forward no other instance confirmed sending: delivered as an agent message, not as the person";
            kind = ForwardedAs::AgentMessage;
            deliver_unconfirmed_forward(state, &agent.agent_id, body).await
        }
    };
    let target = state
        .reactive_handler
        .get_agent_by_block(block_id)
        .map(|a| a.agent_id)
        .unwrap_or_else(|| block_id.to_string());
    state.reactive_handler.log_fleet_action_audit(
        None,
        &target,
        block_id,
        FLEET_BROADCAST_AUDIT_ACTION,
        result.is_ok(),
        result.as_ref().err().map(String::as_str),
        msg_id,
        Some(how),
    );
    (kind, result)
}

/// A forward nobody confirmed: an ordinary message to the agent from the sender name `swarm-forward`,
/// through the same delivery as any unattributed one (labelled self-declared, never host-verified).
async fn deliver_unconfirmed_forward(state: &AppState, agent_id: &str, body: &str) -> Result<(), String> {
    let req = crate::backend::reactive::InjectionRequest {
        target_agent: agent_id.to_string(),
        message: format!("A Swarm broadcast was forwarded to you but not proven to come from the person's window, so it is delivered as an ordinary message:\n{body}"),
        source_agent: Some("swarm-forward".to_string()),
        reply: Some("none".to_string()),
        ..Default::default()
    };
    let answer = crate::server::reactive::deliver(state, crate::server::ReactiveAuthVia::FullAuthKey, "", req).await;
    if answer.get("success").and_then(|v| v.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(answer.get("error").and_then(|v| v.as_str()).unwrap_or("not delivered").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_is_confirmed_once_and_only_as_it_was_sent() {
        let f = BroadcastForward { block_id: "blk-confirm", body: "ship it", msg_id: "m-confirm", recipients: 2 };
        let digest = body_digest("ship it");
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("ship"), "the words are not in what is sent to be confirmed");
        assert!(!confirm_forward("m-confirm", "blk-confirm", &digest, 2), "nothing was sent yet");
        vouch_for_forward(&f);
        assert!(!confirm_forward("m-confirm", "blk-confirm", &body_digest("something else"), 2), "another body is not that forward");
        assert!(!confirm_forward("m-confirm", "blk-confirm", &digest, 9), "another count is not that forward");
        assert!(!confirm_forward("m-confirm", "blk-other", &digest, 2), "another block is not that forward");
        assert!(confirm_forward("m-confirm", "blk-confirm", &digest, 2));
        assert!(!confirm_forward("m-confirm", "blk-confirm", &digest, 2), "a confirmation is spent");
    }

    #[tokio::test]
    async fn only_a_confirmed_forward_arrives_as_the_person() {
        let state = crate::server::tests::test_state();
        state.reactive_handler.register_agent("fwd-target", "blk-fwd-target", None).unwrap();
        let (kind, _) = deliver_forward(&state, "blk-fwd-target", "hi", "m-2", 1, false).await;
        assert_eq!(kind, ForwardedAs::AgentMessage, "no instance vouched for it");
        let (kind, _) = deliver_forward(&state, "blk-fwd-target", "hi", "m-2", 1, true).await;
        assert_eq!(kind, ForwardedAs::Person);
    }
}

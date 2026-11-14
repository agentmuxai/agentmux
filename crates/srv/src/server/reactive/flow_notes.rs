// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tells the Atlas map what [`deliver`](super::deliver) did with a message: `backend::message_flow`. Ids, hosts and the
//! outcome only; nothing here reads the message's text or files.

use super::*;
use crate::backend::message_flow::{self, MessageFlowOutcome, MessageFlowVia, Seen};

/// How the message came to this srv: by the tier it carries.
pub(super) fn arrival_via(req: &InjectionRequest) -> MessageFlowVia {
    match req.delivery_tier.as_deref() {
        Some("channel") => MessageFlowVia::Channel,
        Some("lan") => MessageFlowVia::Lan,
        Some("wan") => MessageFlowVia::Wan,
        _ => MessageFlowVia::Host,
    }
}

/// What an answer to a sender says became of the message.
pub(super) fn answer_outcome(answer: &serde_json::Value) -> MessageFlowOutcome {
    if answer.get("held").and_then(|h| h.as_bool()) == Some(true) {
        MessageFlowOutcome::Held
    } else if answer.get("success").and_then(|s| s.as_bool()) == Some(true) {
        MessageFlowOutcome::Delivered
    } else {
        MessageFlowOutcome::Refused
    }
}

/// `req` was taken by its receiver here.
pub(super) fn note_delivered(state: &AppState, req: &InjectionRequest) {
    note(state, req, arrival_via(req), MessageFlowOutcome::Delivered, None);
}

/// `req` was given to the cloud relay, which knows it as `injection_id`; gives `answer` back to return.
pub(super) fn queued(state: &AppState, req: &InjectionRequest, injection_id: &str, answer: serde_json::Value) -> serde_json::Value {
    note_as(state, req, injection_id, MessageFlowVia::Wan, MessageFlowOutcome::Queued, None);
    answer
}

/// Reports what `answer` made of `req` where it arrived, and gives the answer back to return.
pub(super) fn noted(state: &AppState, req: &InjectionRequest, answer: serde_json::Value) -> serde_json::Value {
    note(state, req, arrival_via(req), answer_outcome(&answer), None);
    answer
}

/// The host of the LAN peer that runs `agent`, when the peer list has it.
pub(super) fn lan_host_of(state: &AppState, agent: &str) -> Option<String> {
    state
        .lan_discovery
        .get_instances()
        .into_iter()
        .find(|peer| peer.agents.iter().any(|a| a.eq_ignore_ascii_case(agent)))
        .map(|peer| peer.hostname)
        .filter(|host| !host.is_empty())
}

/// Reports `req` as handled here: `to_host` is where its receiver runs, when known (`None` is this computer for a
/// local result). A message with no sender (a cron job, a bridge) has no line to draw and says nothing.
pub(super) fn note(state: &AppState, req: &InjectionRequest, via: MessageFlowVia, outcome: MessageFlowOutcome, to_host: Option<&str>) {
    note_as(state, req, req.request_id.as_deref().unwrap_or(""), via, outcome, to_host);
}

/// [`note`] under `msg_id`, for a message the relay gave a new id (the receiving srv knows it by that one).
pub(super) fn note_as(state: &AppState, req: &InjectionRequest, msg_id: &str, via: MessageFlowVia, outcome: MessageFlowOutcome, to_host: Option<&str>) {
    let Some(from) = req.source_agent.as_deref() else { return };
    let here = crate::backend::reactive::registry::local_host_label();
    let from_host = match arrival_via(req) {
        MessageFlowVia::Host | MessageFlowVia::Channel => Some(here.clone()),
        MessageFlowVia::Lan => lan_host_of(state, from),
        MessageFlowVia::Wan => None,
    };
    message_flow::note(&Seen {
        msg_id,
        from,
        from_host: from_host.as_deref(),
        to: &req.target_agent,
        to_host: Some(to_host.unwrap_or(&here)),
        via,
        outcome,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(tier: Option<&str>) -> InjectionRequest {
        InjectionRequest { delivery_tier: tier.map(str::to_string), ..Default::default() }
    }

    #[test]
    fn a_message_arrives_by_the_tier_it_carries_and_a_local_caller_has_none() {
        let via = |t| arrival_via(&req(t));
        assert_eq!(via(None), MessageFlowVia::Host);
        assert_eq!(via(Some("host")), MessageFlowVia::Host);
        assert_eq!(via(Some("channel")), MessageFlowVia::Channel);
        assert_eq!(via(Some("lan")), MessageFlowVia::Lan);
        assert_eq!(via(Some("wan")), MessageFlowVia::Wan);
    }

    #[test]
    fn an_answer_is_held_before_it_is_delivered_and_anything_else_is_refused() {
        assert_eq!(answer_outcome(&json!({ "success": true, "request_id": "m" })), MessageFlowOutcome::Delivered);
        assert_eq!(answer_outcome(&json!({ "success": true, "held": true })), MessageFlowOutcome::Held);
        assert_eq!(answer_outcome(&json!({ "success": false, "held": true })), MessageFlowOutcome::Held);
        assert_eq!(answer_outcome(&json!({ "success": false, "error": "no" })), MessageFlowOutcome::Refused);
        assert_eq!(answer_outcome(&json!({})), MessageFlowOutcome::Refused);
    }
}

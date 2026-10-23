// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Entry points that hand a jekt to the reactive handler themselves rather
//! than through `reactive::deliver()`: `/api/bus/inject`, the WebSocket
//! `bus:inject` message, widget `agents.send` and the chat bridges. Each
//! readies its request here, so a `transcript_request` body reads
//! `TIER=sensitive`, with the recipient's own `ESCALATE`, on every entry
//! point alike (SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md).

use std::sync::Arc;

use super::AppState;
use crate::backend::reactive::types::{InjectionRequest, InjectionResponse};
use crate::backend::storage::store::Store;

/// The request `/api/bus/inject` and `bus:inject` deliver: host tier, from
/// the named sender, readied by [`prepare_host_entry`].
pub(super) fn bus_inject_request(
    state: &AppState,
    from: &str,
    target: &str,
    message: &str,
    priority: Option<String>,
    audit_source_uid: String,
) -> InjectionRequest {
    let mut req = InjectionRequest {
        target_agent: target.to_string(),
        message: message.to_string(),
        source_agent: Some(from.to_string()),
        priority,
        delivery_tier: Some("host".to_string()),
        audit_source_uid,
        ..Default::default()
    };
    prepare_host_entry(state, &mut req);
    req
}

/// Readies a host-tier request for the handler, as `deliver()` does: checks
/// the claimed sender's signature (a sender with a key on file and no valid
/// signature reads `TRUST=unverified`) and resolves the transcript-request
/// fields against the recipient's `conversation_visibility`.
pub(super) fn prepare_host_entry(state: &AppState, req: &mut InjectionRequest) {
    super::reactive::verify_jekt_signature(state, req);
    super::reactive::resolve_transcript_request_tier_fields(&state.mstore, req);
}

/// Delivers a message a chat bridge (Slack, Discord, Telegram, WhatsApp)
/// received, with its transcript-request fields resolved against this
/// channel's store first.
pub(crate) fn inject_from_bridge(mut req: InjectionRequest) -> InjectionResponse {
    resolve_with_store(crate::backend::identity_spawn::row_store(), &mut req);
    crate::backend::reactive::handler::get_global_handler().inject_message(req)
}

/// `resolve_transcript_request_tier_fields` for a caller that reaches the
/// store through the boot-time handle. Before boot attaches one, a
/// transcript request reads as it does under `ask`: `TIER=sensitive` and
/// `ESCALATE=required`.
pub(crate) fn resolve_with_store(store: Option<&Arc<Store>>, req: &mut InjectionRequest) {
    match store {
        Some(store) => super::reactive::resolve_transcript_request_tier_fields(store, req),
        None => {
            if agentmux_common::transcript_request::parse_transcript_request(&req.message).is_some() {
                req.is_transcript_request = true;
                req.transcript_request_escalate_forced = true;
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/jekt_entry_tests.rs"]
mod tests;

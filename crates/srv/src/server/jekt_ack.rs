// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The sender-side check that refuses a reply saying nothing (`backend::ack_policy`).

use super::AppState;
use crate::backend::reactive::types::InjectionRequest;
use crate::backend::storage::store::Store;

/// The agent a name, display binding, former slug or UID stands for, as one string, so a reply is
/// matched to the message it answers whichever way each side addressed the other.
fn canon(store: &Store, name: &str) -> String {
    crate::backend::agent_resolve::resolve_agent_id(store, name).unwrap_or_else(|_| name.to_string())
}

/// Remember a message arriving here (from another host, by the cloud relay, or with files), for the
/// next reply's check. It is never refused: only the sender's own srv does that.
pub(crate) fn remember_arrival(store: &Store, req: &InjectionRequest) {
    let Some(source) = req.source_agent.as_deref().filter(|s| !s.is_empty() && *s != "cron") else { return };
    crate::backend::ack_policy::record(
        &canon(store, source),
        &canon(store, &req.target_agent),
        req.request_id.as_deref(),
        req.reply.as_deref() == Some("none"),
    );
}

/// Refuse an agent's reply to a message that asked for none, or a bare acknowledgement to an agent
/// that wrote to it a moment ago (`backend::ack_policy`); otherwise remember the message, for the
/// next reply's check. Only the sender's own srv refuses (a full-key, first-hop, host-tier
/// request); a message arriving from another host is only remembered.
pub(super) fn refuse_empty_reply(state: &AppState, auth_via: super::auth::ReactiveAuthVia, req: &InjectionRequest) -> Option<serde_json::Value> {
    use crate::backend::ack_policy;
    let source = req.source_agent.as_deref().filter(|s| !s.is_empty() && *s != "cron")?;
    let from_here = auth_via == super::auth::ReactiveAuthVia::FullAuthKey
        && req.forward_hops == 0
        && req.delivery_tier.as_deref() == Some("host");
    if from_here {
        let extra = state.config_watcher.get_full_config().settings.extra.clone();
        if let Err(why) = ack_policy::check(
            &canon(&state.mstore, source),
            &canon(&state.mstore, &req.target_agent),
            &req.message,
            req.in_reply_to.as_deref(),
            ack_policy::refuses_bare_acks(&extra),
        ) {
            tracing::info!(source, target = %req.target_agent, "reactive inject: refusing a reply that says nothing");
            return Some(serde_json::json!({
                "success": false,
                "refused": true,
                "error": format!("Not sent: {why}."),
                "request_id": req.request_id.clone().unwrap_or_default(),
            }));
        }
    }
    remember_arrival(&state.mstore, req);
    None
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The sender-side check that refuses a reply saying nothing (`backend::ack_policy`).

use super::AppState;
use crate::backend::reactive::types::InjectionRequest;

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
            source,
            &req.target_agent,
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
    ack_policy::record(source, &req.target_agent, req.request_id.as_deref(), req.reply.as_deref() == Some("none"));
    None
}

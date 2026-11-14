// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The sender-side check that refuses a reply saying nothing (`backend::ack_policy`).

use super::AppState;
use crate::backend::ack_policy::{canon, remember_arrival};
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
            &canon(&state.mstore, source),
            &canon(&state.mstore, &req.target_agent),
            &req.message,
            req.in_reply_to.as_deref(),
            // A caption on an attachment is not a bare acknowledgement: the files are the news.
            ack_policy::refuses_bare_acks(&extra) && req.files.as_ref().is_none_or(|f| f.is_empty()),
        ) {
            tracing::info!(source, target = %req.target_agent, "reactive inject: refusing a reply that says nothing");
            state.reactive_handler.log_ack_refused(source, &req.target_agent, req.request_id.as_deref().unwrap_or_default(), &why);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    fn arrival(source: Option<&str>, request_id: &str, reply: Option<&str>) -> InjectionRequest {
        InjectionRequest {
            target_agent: "jack-target".into(),
            source_agent: source.map(str::to_string),
            request_id: Some(request_id.into()),
            reply: reply.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn an_arrival_that_asked_for_no_reply_is_remembered_for_the_next_check() {
        let store = Store::open_in_memory().unwrap();
        remember_arrival(&store, &arrival(Some("jack-sender"), "jack-msg-none", Some("none")));
        remember_arrival(&store, &arrival(Some("jack-sender"), "jack-msg-reply", None));
        assert!(crate::backend::ack_policy::asked_for_no_reply("jack-msg-none"));
        assert!(!crate::backend::ack_policy::asked_for_no_reply("jack-msg-reply"));
    }

    #[test]
    fn an_arrival_with_no_source_or_from_cron_is_not_remembered() {
        let store = Store::open_in_memory().unwrap();
        for source in [None, Some(""), Some("cron")] {
            remember_arrival(&store, &arrival(source, "jack-msg-unsourced", Some("none")));
        }
        assert!(!crate::backend::ack_policy::asked_for_no_reply("jack-msg-unsourced"));
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A delivery that finishes whether or not the request that asked for it is still waiting (task #2485).
//!
//! `deliver` can take longer than a sender waits: each tier for an agent this instance does not run is a
//! network call with its own timeout (a LAN peer, the cloud relay's token, lease and publish), and the last
//! step of the chain is the one that holds the message for an agent that is stopped. The MCP client gave up
//! after 10 s, hyper dropped the handler's future with the connection, and the message was neither delivered
//! nor held: the sender saw only "error sending request", three times, for a stopped agent.

use super::*;

/// [`deliver`], run as a task of its own: the answer is awaited here, but a caller that stops waiting does
/// not cancel the delivery halfway through its tiers. A delivery that panics answers with a failure.
pub(super) async fn deliver_to_completion(
    state: &AppState,
    auth_via: crate::server::ReactiveAuthVia,
    caller_uid: &str,
    req: InjectionRequest,
) -> serde_json::Value {
    let (state, caller_uid, target) = (state.clone(), caller_uid.to_string(), req.target_agent.clone());
    match tokio::spawn(async move { deliver(&state, auth_via, &caller_uid, req).await }).await {
        Ok(answer) => answer,
        Err(e) => {
            tracing::error!(target = %target, error = %e, "reactive inject: the delivery task failed");
            serde_json::json!({ "success": false, "error": format!("delivery to {target} failed inside srv: {e}") })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;
    use crate::server::tests::test_state;

    /// A message to a known agent that is stopped (no pane, no controller) is held for it, even when the
    /// sender stops waiting right after sending (task #2485): the wait is dropped after its first poll.
    #[tokio::test]
    async fn a_message_the_sender_stopped_waiting_for_is_still_held_for_a_stopped_agent() {
        let state = test_state();
        let slug = format!("stopped-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let uid = format!("uid-{slug}");
        let mut def = test_agent_def(&uid, "Stopped Agent", "claude", "agent", 1, "");
        def.slug = slug.clone();
        state.mstore.agent_def_insert(&mut def).unwrap();
        let request_id = format!("req-{slug}");

        let mut held = Vec::new();
        // The global handler's rate limiter is shared by every test (10/s): an attempt it refused is sent again.
        for _ in 0..25 {
            let req: InjectionRequest = serde_json::from_value(
                serde_json::json!({ "target_agent": slug, "message": "hi", "source_agent": "sender", "request_id": request_id }),
            )
            .unwrap();
            let _ = tokio::time::timeout(std::time::Duration::ZERO, deliver_to_completion(&state, crate::server::ReactiveAuthVia::FullAuthKey, "", req)).await;
            for _ in 0..20 {
                held = state.mstore.jekt_held_for_target(&uid, 10).unwrap();
                if !held.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            if !held.is_empty() {
                break;
            }
        }
        assert_eq!(held.len(), 1, "the message is held, not lost with the dropped wait");
        assert_eq!(held[0].request_id, request_id);
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who typed into an agent's composer (`agentinput`), decided from the
//! connection, never from the request
//! (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §3.4).
//!
//! `TurnOrigin::User` is the human, the one thing nothing else may pass
//! itself off as, so the `agentinput` route takes it from the caller:
//!
//! - a connection registered as an agent is refused: agents send with the
//!   `Message` tool, which is delivered as a jekt;
//! - the AgentMux window, proven by the CEF host (`RpcContext::operator_only`),
//!   is the human (`System` for a hidden memory reinjection, or the pane's own start-up
//!   document, which the window sends on its own behalf);
//! - any other connection is refused when srv runs with a CEF host (every
//!   desktop build); a headless srv (the cloud web host) has no CEF host and logs
//!   the call. The mode comes from how srv was started, never from anything
//!   the connection says.

use crate::backend::blockcontroller::health::TurnOrigin;
use crate::backend::rpc_types::RpcContext;

const WHY: &str = "only the AgentMux window types into an agent's composer; an agent sends a message with the Message tool";

/// The origin of an `agentinput` turn from the caller, or why it is refused. `lowered` (a hidden
/// reinjection, the pane's start-up document) can only turn the window's `User` into `System`.
pub(super) fn composer_origin(ctx: &RpcContext, lowered: bool) -> Result<TurnOrigin, String> {
    composer_origin_in(ctx, lowered, crate::headless::active())
}

/// The heading the pane's start-up document, and the opening update prompt, begin with
/// (`frontend/app/view/agent/startup/buildStartupPayload.ts`).
const STARTUP_DOCUMENT_HEADING: &str = "# Session Context";

/// Whether `message`, as the window's composer sent it, is the pane's own start-up document: AgentMux's
/// words, not the human's. A temporary text mark on the one route that has no field for it yet (the
/// pane sends these through its ordinary send path); it can only lower the window's `User` to
/// `System`, and only here: a jekt or any other route never reaches it. The pane marking its own
/// documents replaces it (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md P2).
pub(super) fn is_startup_document(message: &str) -> bool {
    if message.starts_with(STARTUP_DOCUMENT_HEADING) {
        return true;
    }
    // The stream-json line that carries it, as a string or as text blocks.
    serde_json::from_str::<serde_json::Value>(message).ok().is_some_and(|v| {
        let content = v.get("message").and_then(|m| m.get("content"));
        let first = match content {
            Some(serde_json::Value::String(s)) => Some(s.as_str()),
            Some(serde_json::Value::Array(blocks)) => blocks
                .iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str()),
            _ => None,
        };
        first.is_some_and(|s| s.starts_with(STARTUP_DOCUMENT_HEADING))
    })
}

fn composer_origin_in(ctx: &RpcContext, lowered: bool, headless: bool) -> Result<TurnOrigin, String> {
    // A registered agent is refused in every mode, whatever else it claims.
    if !ctx.agent_id.is_empty() {
        return Err(format!("FORBIDDEN: {WHY}"));
    }
    if let Err(refusal) = ctx.operator_only(WHY) {
        if !headless {
            return Err(refusal);
        }
        // A headless srv has no CEF host: the cloud web host's own turns.
        tracing::warn!(
            client_type = %ctx.client_type,
            "agentinput from a connection no host proved, in a headless srv"
        );
    }
    Ok(if lowered { TurnOrigin::System } else { TurnOrigin::User })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(proven: bool) -> RpcContext {
        RpcContext { agent_id: "AgentX".into(), operator_proven: proven, ..Default::default() }
    }

    #[test]
    fn the_proven_window_is_the_user_or_the_system_for_a_hidden_reinjection() {
        let window = RpcContext::operator_for_tests();
        assert_eq!(composer_origin_in(&window, false, false), Ok(TurnOrigin::User));
        assert_eq!(composer_origin_in(&window, true, false), Ok(TurnOrigin::System));
    }

    #[test]
    fn the_start_up_document_is_agentmux_s_words_and_the_human_s_mention_of_it_is_not() {
        let doc = "# Session Context\nIdentity: Claude\n## Description\nStartup boilerplate";
        assert!(is_startup_document(doc));
        let line = serde_json::json!({"type": "user", "message": {"role": "user", "content": doc}});
        assert!(is_startup_document(&line.to_string()));
        let blocks = serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": doc}]}});
        assert!(is_startup_document(&blocks.to_string()));
        // Only the heading at the start: the human's own message that mentions it is still theirs.
        assert!(!is_startup_document("what is a # Session Context?"));
        assert!(!is_startup_document("[JEKT:FROM=cron]\n# Session Context"));
    }

    #[test]
    fn an_agent_connection_is_never_the_user_in_any_mode() {
        for headless in [false, true] {
            for proven in [false, true] {
                for hidden in [false, true] {
                    let err = composer_origin_in(&agent(proven), hidden, headless).unwrap_err();
                    assert!(err.starts_with("FORBIDDEN: only the AgentMux window"), "{err}");
                }
            }
        }
    }

    #[test]
    fn an_unproven_connection_is_refused_when_srv_has_a_host() {
        let err = composer_origin_in(&RpcContext::default(), false, false).unwrap_err();
        assert!(err.contains("isn't the AgentMux window"), "{err}");
    }

    #[test]
    fn a_headless_srv_keeps_the_old_behaviour_for_an_unproven_connection() {
        let ctx = RpcContext::default();
        assert_eq!(composer_origin_in(&ctx, false, true), Ok(TurnOrigin::User));
        assert_eq!(composer_origin_in(&ctx, true, true), Ok(TurnOrigin::System));
    }

    #[test]
    fn nothing_the_request_carries_can_raise_the_origin() {
        // `hidden` only ever lowers User to System; it never turns a refusal into an origin.
        assert!(composer_origin_in(&RpcContext::default(), true, false).is_err());
        assert!(composer_origin_in(&agent(false), true, true).is_err());
    }
}

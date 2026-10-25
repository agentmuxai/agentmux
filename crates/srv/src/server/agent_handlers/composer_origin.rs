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
//!   is the human (`System` for a hidden memory reinjection the window sends on
//!   its own behalf);
//! - any other connection is refused when srv runs with a CEF host (every
//!   desktop build); a headless srv (the cloud web host) has no CEF host and logs
//!   the call. The mode comes from how srv was started, never from anything
//!   the connection says.

use crate::backend::blockcontroller::health::TurnOrigin;
use crate::backend::rpc_types::RpcContext;

const WHY: &str = "only the AgentMux window types into an agent's composer; an agent sends a message with the Message tool";

/// The origin of an `agentinput` turn from the caller, or why it is refused.
pub(super) fn composer_origin(ctx: &RpcContext, hidden: bool) -> Result<TurnOrigin, String> {
    composer_origin_in(ctx, hidden, crate::headless::active())
}

fn composer_origin_in(ctx: &RpcContext, hidden: bool, headless: bool) -> Result<TurnOrigin, String> {
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
    Ok(if hidden { TurnOrigin::System } else { TurnOrigin::User })
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

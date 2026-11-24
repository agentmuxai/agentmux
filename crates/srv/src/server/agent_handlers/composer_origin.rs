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
//!   is the human (`System` for a hidden memory reinjection, or a message the pane sends on
//!   AgentMux's behalf, such as its start-up document: the request says so with `system`);
//! - any other connection is refused, in every mode. A headless srv (the cloud web
//!   host) has no CEF host: there the person is proven by the browser they signed
//!   in with (a confirmed session marks the connection, `operator_only` passes).
//!   A connection that holds only the instance key is not the person.

use crate::backend::blockcontroller::health::TurnOrigin;
use crate::backend::rpc_types::RpcContext;

const WHY: &str = "only the AgentMux window types into an agent's composer; an agent sends a message with the Message tool";

/// The origin of an `agentinput` turn from the caller, or why it is refused. `lowered` (a hidden
/// reinjection, a message the pane sends on AgentMux's behalf) can only turn the window's `User` into `System`.
pub(super) fn composer_origin(ctx: &RpcContext, lowered: bool) -> Result<TurnOrigin, String> {
    composer_origin_in(ctx, lowered)
}

fn composer_origin_in(ctx: &RpcContext, lowered: bool) -> Result<TurnOrigin, String> {
    // A registered agent is refused in every mode, whatever else it claims.
    if !ctx.agent_id.is_empty() {
        return Err(format!("FORBIDDEN: {WHY}"));
    }
    ctx.operator_only(WHY)?;
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
        assert_eq!(composer_origin_in(&window, false), Ok(TurnOrigin::User));
        assert_eq!(composer_origin_in(&window, true), Ok(TurnOrigin::System));
    }

    #[test]
    fn an_agent_connection_is_never_the_user() {
        for proven in [false, true] {
            for hidden in [false, true] {
                let err = composer_origin_in(&agent(proven), hidden).unwrap_err();
                assert!(err.starts_with("FORBIDDEN: only the AgentMux window"), "{err}");
            }
        }
    }

    #[test]
    fn a_connection_with_no_proof_is_refused_in_a_headless_srv_as_on_the_desktop() {
        // The signed-in browser of a headless srv is proven by its session (`operator_for_tests` stands for it above);
        // a connection that holds only the instance key is not.
        for hidden in [false, true] {
            let err = composer_origin_in(&RpcContext::default(), hidden).unwrap_err();
            assert!(err.contains("isn't the AgentMux window"), "{err}");
        }
    }

    #[test]
    fn nothing_the_request_carries_can_raise_the_origin() {
        // `hidden` only ever lowers User to System; it never turns a refusal into an origin.
        assert!(composer_origin_in(&RpcContext::default(), true).is_err());
        assert!(composer_origin_in(&agent(false), true).is_err());
    }
}

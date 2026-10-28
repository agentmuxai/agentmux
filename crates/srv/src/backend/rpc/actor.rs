// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who made a websocket RPC call, for the record of what was done.
//!
//! The answer comes from the connection, never from the request: an agent is
//! the one the connection registered as (`bus:register`, token-bound), the
//! operator is a connection the CEF host proved is the AgentMux window
//! (`server::operator_proof`), and a connection that is neither is
//! [`Actor::Unproven`], not the operator. Nothing in a request's data names the
//! actor.

use crate::backend::rpc_types::RpcContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// A connection registered as this agent.
    Agent(String),
    /// A connection the host proved is the AgentMux window.
    Operator,
    /// Neither: a socket that holds the instance key and has proved nothing more.
    Unproven,
}

impl Actor {
    /// How the record names the actor. An agent is `agent:<id>`, so no agent id can
    /// read as `operator` or `unproven`.
    pub fn label(&self) -> String {
        match self {
            Actor::Agent(id) => format!("agent:{id}"),
            Actor::Operator => "operator".to_string(),
            Actor::Unproven => "unproven".to_string(),
        }
    }
}

/// The actor of a call made on a connection with `ctx`. A connection registered as an
/// agent is that agent whatever else it carries; the operator only with the host's proof.
pub fn actor_of(ctx: &RpcContext) -> Actor {
    if !ctx.agent_id.is_empty() {
        Actor::Agent(ctx.agent_id.clone())
    } else if ctx.operator_proven {
        Actor::Operator
    } else {
        Actor::Unproven
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agents_connection_is_that_agent() {
        let ctx = RpcContext { agent_id: "opaz".into(), ..Default::default() };
        assert_eq!(actor_of(&ctx), Actor::Agent("opaz".into()));
        assert_eq!(actor_of(&ctx).label(), "agent:opaz");
    }

    #[test]
    fn the_operator_needs_the_hosts_proof() {
        assert_eq!(actor_of(&RpcContext::operator_for_tests()), Actor::Operator);
        // Holding the instance key, or being any other connection, is not the operator.
        assert_eq!(actor_of(&RpcContext::default()), Actor::Unproven);
        assert_eq!(actor_of(&RpcContext { blockid: "b".into(), tabid: "t".into(), conn: "c".into(), ..Default::default() }), Actor::Unproven);
    }

    #[test]
    fn an_agent_is_never_the_operator_even_with_the_proof_flag_set() {
        let ctx = RpcContext { agent_id: "opaz".into(), operator_proven: true, ..Default::default() };
        assert_eq!(actor_of(&ctx), Actor::Agent("opaz".into()));
    }

    #[test]
    fn no_agent_id_can_read_as_the_operator_or_unproven() {
        assert_eq!(Actor::Agent("operator".into()).label(), "agent:operator");
        assert_eq!(Actor::Agent("unproven".into()).label(), "agent:unproven");
        assert_ne!(Actor::Agent("operator".into()).label(), Actor::Operator.label());
    }
}

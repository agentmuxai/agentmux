// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may add, change, bind, unbind or probe an MCP server whose command would start on this computer.
//!
//! A catalog (global) server is usually a host command, and a probe starts a stored server's command on this
//! computer whoever asked. A container agent starts only what runs in its container, so these commands refuse
//! one. A container agent's own private row (`mcp.upsert`, `mcp.delete`, `mcp.bind`, `mcp.unbind`: always its
//! own, by `check_s1`) starts in its container and is not refused. Every call that asks is recorded with the
//! actor it came from; a host agent, the window and a connection that proved nothing are not refused.

use crate::backend::rpc::actor::{actor_of, Actor};
use crate::backend::rpc_types::RpcContext;
use crate::backend::storage::store::Store;

/// What a container agent is told when one of these commands is refused it.
pub(crate) const REFUSAL: &str = "CONTAINER_AGENT: this would start a program on this computer, which a container agent can't ask for. Ask a host agent, or have the person add it in Connectors.";

/// `Err` with [`REFUSAL`] when `ctx` is a connection of a container agent; otherwise records the actor and goes on.
pub(crate) fn refuse_container_agent(store: &Store, ctx: &RpcContext, command: &str) -> Result<(), String> {
    let actor = actor_of(ctx);
    if let Actor::Agent(id) = &actor {
        let in_container = crate::backend::agent_resolve::resolve_agent_id(store, id)
            .ok()
            .and_then(|id| store.agent_def_get(&id).ok().flatten())
            .is_some_and(|def| def.runtime().is_container());
        if in_container {
            tracing::warn!(command, actor = %actor.label(), "MCP server command refused: it would start on this computer");
            return Err(REFUSAL.to_string());
        }
    }
    tracing::info!(command, actor = %actor.label(), "MCP server command asked for");
    Ok(())
}

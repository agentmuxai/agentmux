// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may add, change, bind, unbind or probe an MCP server whose command would start on this computer.
//!
//! A catalog (global) server is usually a host command, and a probe starts a stored server's command on this
//! computer whoever asked. A container agent starts only what runs in its container, so these commands refuse
//! one. A container agent's own private row (`mcp.upsert`, `mcp.delete`, `mcp.bind`, `mcp.unbind`: always its
//! own, by `check_s1`) starts in its container and is not refused. Every call that asks is recorded with the
//! actor it came from; a host agent, the window and a connection that proved nothing are not refused. A
//! connection registered as an agent whose runtime can't be told (the id doesn't resolve, is ambiguous, has no
//! definition, or the store can't be read) is refused like a container agent's.

use crate::backend::rpc::actor::{actor_of, Actor};
use crate::backend::rpc_types::RpcContext;
use crate::backend::storage::store::Store;

/// What a container agent is told when one of these commands is refused it.
pub(crate) const REFUSAL: &str = "CONTAINER_AGENT: this would start a program on this computer, which a container agent can't ask for. Ask a host agent, or have the person add it in Connectors.";

/// What an agent whose runtime can't be told is refused with.
pub(crate) const UNKNOWN_REFUSAL: &str = "CONTAINER_AGENT: couldn't tell where this agent runs, so this would not be a program started for it on this computer. Ask a host agent, or have the person add it in Connectors.";

/// Whether the agent connected as `id` runs in a container: `None` when that can't be told.
fn runs_in_a_container(store: &Store, id: &str) -> Option<bool> {
    let def_id = crate::backend::agent_resolve::resolve_agent_id(store, id).ok()?;
    let def = store.agent_def_get(&def_id).ok()??;
    Some(def.runtime().is_container())
}

/// Whether a catalog server starts only in containers: it reaches container agents and no host agent by
/// reach (`McpServerReach`), and every agent it is bound to runs in a container. A server bound to no one, to
/// an agent whose runtime can't be told, or that reaches every agent, is not.
pub(crate) fn catalog_server_runs_only_in_containers(mstore: &Store, identity_store: &Store, mcp_id: &str) -> bool {
    let reach = identity_store.mcp_server_reach_all().ok().and_then(|all| all.get(mcp_id).copied()).unwrap_or_default();
    if reach.all_agents || !reach.in_containers {
        return false;
    }
    let Ok(bound) = mstore.mcp_server_bound_agent_ids(mcp_id) else {
        return false;
    };
    !bound.is_empty() && bound.iter().all(|id| runs_in_a_container(mstore, id) == Some(true))
}

/// `Err` with [`REFUSAL`] when `ctx` is a connection of a container agent, and with [`UNKNOWN_REFUSAL`] when it is
/// an agent's whose runtime can't be told; otherwise records the actor and goes on.
pub(crate) fn refuse_container_agent(store: &Store, ctx: &RpcContext, command: &str) -> Result<(), String> {
    let actor = actor_of(ctx);
    if let Actor::Agent(id) = &actor {
        let refusal = match runs_in_a_container(store, id) {
            Some(false) => None,
            Some(true) => Some(REFUSAL),
            None => Some(UNKNOWN_REFUSAL),
        };
        if let Some(why) = refusal {
            tracing::warn!(command, actor = %actor.label(), "MCP server command refused: it would start on this computer");
            return Err(why.to_string());
        }
    }
    tracing::info!(command, actor = %actor.label(), "MCP server command asked for");
    Ok(())
}

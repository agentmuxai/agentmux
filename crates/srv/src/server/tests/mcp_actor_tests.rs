// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An MCP command that would start a program on this computer is refused to a container agent's connection
//! (`app_api::mcp_actor`), through the handlers a real connection registers. Every `mcp.*` command is decided
//! below, so a new one fails until someone says whether it starts a program on the host.

use std::sync::Arc;

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;

/// Add, change, bind, unbind or probe a server whose command would start on this computer: refused to a
/// container agent.
const HOST_STARTS: &[&str] = &[
    "mcp.probe",
    "mcp.catalog.upsert",
    "mcp.catalog.delete",
    "mcp.catalog.probe",
    "mcp.catalog.set_reach",
    "mcp.catalog.bind",
    "mcp.catalog.unbind",
];

/// Start nothing on this computer: a read, or a change to the calling agent's own rows (`check_s1`).
const NOT_HOST_STARTS: &[&str] = &[
    "mcp.list",
    "mcp.get",
    "mcp.upsert",
    "mcp.delete",
    "mcp.bind",
    "mcp.unbind",
    "mcp.catalog.list",
    "mcp.catalog.list_for_agent",
    "mcp.observed.list",
];

const REFUSED: &str = "CONTAINER_AGENT";

fn connection() -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>, crate::server::AppState) {
    let (engine, rx) = WshRpcEngine::new();
    let state = test_state();
    register_handlers(&engine, state.clone(), format!("mcp-actor-{}", uuid::Uuid::new_v4()));
    (engine, rx, state)
}

/// An agent of the given runtime, with the slug it connects as.
fn an_agent(state: &crate::server::AppState, slug: &str, agent_type: &str) {
    let mut def = crate::backend::storage::agents::test_agent_def(&format!("uid-{slug}"), slug, "claude", agent_type, 1, "");
    def.slug = slug.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
}

/// The error `command` answers with, empty when it answers without one. Every field any of them takes is given,
/// so a request is never refused for its shape before the handler runs.
async fn error_of(engine: &Arc<WshRpcEngine>, rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>, command: &str, agent: &str) -> String {
    let reqid = format!("mcp-actor-{}", uuid::Uuid::new_v4());
    let data = json!({
        "agent_id": agent, "id": "no-such-server", "mcp_id": "no-such-server", "name": "probe-test", "transport": "stdio",
        "config": "{}", "all_agents": false, "in_containers": false,
    });
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: reqid.clone(), data: Some(data), ..Default::default() });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let m = rx.recv().await.expect("the connection stays open");
            if m.resid == reqid {
                return m.error;
            }
        }
    })
    .await
    .expect("an answer within 10s")
}

#[tokio::test]
async fn every_mcp_command_is_decided_and_every_decided_command_is_registered() {
    let (engine, _rx, _state) = connection();
    let registered: std::collections::BTreeSet<String> = engine.command_names().into_iter().filter(|c| c.starts_with("mcp.")).collect();
    let decided: std::collections::BTreeSet<String> = HOST_STARTS.iter().chain(NOT_HOST_STARTS).map(|c| c.to_string()).collect();
    let undecided: Vec<_> = registered.difference(&decided).collect();
    let stale: Vec<_> = decided.difference(&registered).collect();
    assert!(
        undecided.is_empty(),
        "mcp commands in neither list of server/tests/mcp_actor_tests.rs: say whether each starts a program on this computer, and if it does, ask `mcp_actor::refuse_container_agent` first: {undecided:?}"
    );
    assert!(stale.is_empty(), "listed commands that no connection registers any more: {stale:?}");
}

#[tokio::test]
async fn a_container_agent_is_refused_every_command_that_would_start_a_program_on_this_computer() {
    let (engine, mut rx, state) = connection();
    an_agent(&state, "boxed", "container");
    engine.set_rpc_context(RpcContext { agent_id: "boxed".into(), ..Default::default() });
    for command in HOST_STARTS {
        let error = error_of(&engine, &mut rx, command, "boxed").await;
        assert!(error.contains(REFUSED), "{command}: {error:?}");
    }
    for command in NOT_HOST_STARTS {
        let error = error_of(&engine, &mut rx, command, "boxed").await;
        assert!(!error.contains(REFUSED), "{command} is its own row or a read: {error:?}");
    }
}

#[tokio::test]
async fn a_host_agent_the_window_and_an_unproven_connection_are_not_refused() {
    for who in ["host-agent", "window", "unproven"] {
        let (engine, mut rx, state) = connection();
        an_agent(&state, "hosted", "agent");
        match who {
            "host-agent" => engine.set_rpc_context(RpcContext { agent_id: "hosted".into(), ..Default::default() }),
            "window" => assert!(engine.mark_operator_proven()),
            _ => {}
        }
        for command in HOST_STARTS.iter().chain(NOT_HOST_STARTS) {
            let error = error_of(&engine, &mut rx, command, "hosted").await;
            assert!(!error.contains(REFUSED), "{who}: {command}: {error:?}");
        }
    }
}

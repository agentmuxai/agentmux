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

/// An agent connection whose runtime can't be told gets the same answer as a container agent's: an id no
/// definition has, and an id whose lookup fails because the store can't be read.
#[tokio::test]
async fn an_agent_whose_runtime_cannot_be_told_is_refused_like_a_container_agent() {
    let (engine, mut rx, state) = connection();
    an_agent(&state, "known-host", "agent");
    for step in ["unknown id", "store error"] {
        if step == "store error" {
            state.mstore.conn().lock().unwrap().execute_batch("DROP TABLE db_agents;").unwrap();
        }
        let id = if step == "unknown id" { "nobody-by-this-name" } else { "known-host" };
        engine.set_rpc_context(RpcContext { agent_id: id.into(), ..Default::default() });
        for command in HOST_STARTS {
            let error = error_of(&engine, &mut rx, command, id).await;
            assert!(error.contains(REFUSED), "{step}: {command}: {error:?}");
            assert!(error.contains("couldn't tell where this agent runs"), "{step}: {command}: {error:?}");
        }
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

/// A catalog server whose command no program answers to: a probe that starts it on this computer says so ("failed to
/// start"), one that does not says it runs in the container.
const NO_SUCH_PROGRAM: &str = "agentmux-test-no-such-program";

fn a_catalog_server(state: &crate::server::AppState, id: &str) {
    let server = crate::backend::storage::mcp_servers::McpServer {
        id: id.to_string(),
        name: id.to_string(),
        transport: "stdio".to_string(),
        config: format!("{{\"command\":\"{NO_SUCH_PROGRAM}\"}}"),
        is_global: true,
        created_at: 1,
        updated_at: 1,
    };
    state.identity_store.mcp_server_upsert_unique_global(&server, "test").unwrap();
}

/// What `command` answers for the window: the probe's error text, or the error of the call.
async fn probe_error_text(engine: &Arc<WshRpcEngine>, rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>, command: &str, agent: &str, server: &str) -> String {
    let reqid = format!("mcp-probe-{}", uuid::Uuid::new_v4());
    let data = json!({ "agent_id": agent, "id": server });
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: reqid.clone(), data: Some(data), ..Default::default() });
    let answer = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let m = rx.recv().await.expect("the connection stays open");
            if m.resid == reqid {
                return m;
            }
        }
    })
    .await
    .expect("an answer within 20s");
    format!("{} {}", answer.error, answer.data.map(|d| d.to_string()).unwrap_or_default())
}

#[tokio::test]
async fn a_catalog_probe_starts_nothing_on_this_computer_for_a_server_bound_only_to_container_agents() {
    use crate::backend::storage::mcp_servers::McpServerReach;
    let in_container = crate::backend::mcp_probe::RUNS_IN_CONTAINER;
    let (engine, mut rx, state) = connection();
    assert!(engine.mark_operator_proven());
    an_agent(&state, "boxed", "container");
    an_agent(&state, "hosted", "agent");
    for id in ["only-boxed", "boxed-and-hosted", "not-in-containers", "bound-to-nobody", "every-agent"] {
        a_catalog_server(&state, id);
    }
    let reach = |id: &str, all_agents: bool, in_containers: bool| state.identity_store.mcp_server_set_reach(id, McpServerReach { all_agents, in_containers }, 1).unwrap();
    let bind = |agent: &str, id: &str| state.mstore.mcp_server_bind(&state.identity_store, &format!("uid-{agent}"), id, "test", 1).unwrap();
    reach("only-boxed", false, true);
    bind("boxed", "only-boxed");
    reach("boxed-and-hosted", false, true);
    bind("boxed", "boxed-and-hosted");
    bind("hosted", "boxed-and-hosted");
    reach("not-in-containers", false, false);
    bind("boxed", "not-in-containers");
    reach("bound-to-nobody", false, true);
    reach("every-agent", true, true);
    bind("boxed", "every-agent");

    let only = probe_error_text(&engine, &mut rx, "mcp.catalog.probe", "", "only-boxed").await;
    assert!(only.contains(in_container) && !only.contains("failed to start"), "{only}");
    for (id, why) in [
        ("boxed-and-hosted", "a host agent has it too"),
        ("not-in-containers", "it does not reach containers"),
        ("bound-to-nobody", "no agent has it"),
        ("every-agent", "it reaches every agent"),
    ] {
        let started = probe_error_text(&engine, &mut rx, "mcp.catalog.probe", "", id).await;
        assert!(started.contains("failed to start") && !started.contains(in_container), "{why}: {started}");
    }
}

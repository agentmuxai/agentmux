// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An MCP server records who added it and who last changed it, and a binding records who bound it and when, for
//! every command that writes one, through the handlers a real connection registers. No command asks for the
//! actor: it is the connection's (`rpc::actor::actor_of`).

use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;
use crate::server::AppState;

enum Who {
    Agent(&'static str),
    Window,
    Unproven,
}

struct Connection {
    engine: Arc<WshRpcEngine>,
    rx: tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
}

fn connect(state: &AppState, who: Who) -> Connection {
    let (engine, rx) = WshRpcEngine::new();
    register_handlers(&engine, state.clone(), format!("mcp-origin-{}", uuid::Uuid::new_v4()));
    match who {
        Who::Agent(id) => engine.set_rpc_context(RpcContext { agent_id: id.into(), ..Default::default() }),
        Who::Window => assert!(engine.mark_operator_proven()),
        Who::Unproven => {}
    }
    Connection { engine, rx }
}

impl Connection {
    /// The data `command` answers with; a test fails on an error.
    async fn call(&mut self, command: &str, data: Value) -> Value {
        let reqid = format!("mcp-origin-{}", uuid::Uuid::new_v4());
        self.engine.handle_message(RpcMessage { command: command.to_string(), reqid: reqid.clone(), data: Some(data), ..Default::default() });
        let answer = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let m = self.rx.recv().await.expect("the connection stays open");
                if m.resid == reqid {
                    return m;
                }
            }
        })
        .await
        .expect("an answer within 10s");
        assert!(answer.error.is_empty(), "{command}: {}", answer.error);
        answer.data.unwrap_or(Value::Null)
    }
}

fn an_agent(state: &AppState, id: &str) {
    let mut def = crate::backend::storage::agents::test_agent_def(id, id, "claude", "agent", 1, "");
    def.slug = id.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
}

fn state_with_agents() -> AppState {
    let state = test_state();
    for id in ["hosted", "other"] {
        an_agent(&state, id);
    }
    state
}

/// The catalog as the Connectors list reads it: a server's `added_by`, `changed_by` and its bindings as
/// `(agent, bound_by)`.
fn catalog_row(state: &AppState, id: &str) -> (String, String, Vec<(String, String)>) {
    let item = state
        .mstore
        .mcp_server_list_global(&state.identity_store)
        .unwrap()
        .into_iter()
        .find(|i| i.server.id == id)
        .unwrap_or_else(|| panic!("{id} is in the catalog"));
    let bindings = item.bindings.iter().map(|b| (b.agent_id.clone(), b.bound_by.clone())).collect();
    (item.origin.added_by, item.origin.changed_by, bindings)
}

fn catalog_upsert(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "transport": "stdio", "config": "{}" })
}

#[tokio::test]
async fn a_catalog_server_records_who_added_it_and_who_last_changed_it_for_each_kind_of_caller() {
    for (who, label) in [(Who::Window, "operator"), (Who::Agent("hosted"), "agent:hosted"), (Who::Unproven, "unproven")] {
        let state = state_with_agents();
        let mut conn = connect(&state, who);
        let made = conn.call("mcp.catalog.upsert", catalog_upsert("", "tool")).await;
        let id = made["id"].as_str().unwrap().to_string();
        assert_eq!(catalog_row(&state, &id), (label.to_string(), label.to_string(), vec![]), "{label}");
    }
}

#[tokio::test]
async fn a_change_by_another_caller_changes_who_last_changed_it_and_keeps_who_added_it() {
    let state = state_with_agents();
    let made = connect(&state, Who::Window).call("mcp.catalog.upsert", catalog_upsert("", "tool")).await;
    let id = made["id"].as_str().unwrap().to_string();
    connect(&state, Who::Agent("hosted")).call("mcp.catalog.upsert", catalog_upsert(&id, "tool")).await;
    assert_eq!(catalog_row(&state, &id), ("operator".to_string(), "agent:hosted".to_string(), vec![]));
}

#[tokio::test]
async fn an_agents_own_server_records_the_agent_as_who_added_and_bound_it() {
    let state = state_with_agents();
    let made = connect(&state, Who::Agent("hosted"))
        .call("mcp.upsert", json!({ "agent_id": "hosted", "id": "", "name": "own", "transport": "stdio", "config": "{}" }))
        .await;
    let id = made["id"].as_str().unwrap().to_string();
    let listed = state.mstore.mcp_server_list(&state.identity_store, "hosted").unwrap();
    let item = listed.iter().find(|i| i.server.id == id).unwrap();
    assert_eq!((item.origin.added_by.as_str(), item.origin.changed_by.as_str()), ("agent:hosted", "agent:hosted"));
    assert_eq!((item.bound_to_agent, item.bound_by.as_str()), (true, "agent:hosted"));
    assert!(item.bound_at > 0);
}

#[tokio::test]
async fn each_bind_command_records_who_bound_it_and_when() {
    let state = state_with_agents();
    let made = connect(&state, Who::Window).call("mcp.catalog.upsert", catalog_upsert("", "tool")).await;
    let id = made["id"].as_str().unwrap().to_string();

    connect(&state, Who::Window).call("mcp.catalog.bind", json!({ "agent_id": "hosted", "mcp_id": id })).await;
    connect(&state, Who::Agent("other")).call("mcp.bind", json!({ "agent_id": "other", "mcp_id": id })).await;
    let (_, _, bindings) = catalog_row(&state, &id);
    assert_eq!(bindings, vec![("hosted".to_string(), "operator".to_string()), ("other".to_string(), "agent:other".to_string())]);

    let listed = state.mstore.mcp_server_list_global_for_agent(&state.identity_store, "hosted").unwrap();
    let item = listed.iter().find(|i| i.server.id == id).unwrap();
    assert_eq!((item.bound_by.as_str(), item.bound_at > 0), ("operator", true));
}

#[tokio::test]
async fn binding_again_keeps_the_first_binder_and_an_unbind_forgets_it() {
    let state = state_with_agents();
    let made = connect(&state, Who::Window).call("mcp.catalog.upsert", catalog_upsert("", "tool")).await;
    let id = made["id"].as_str().unwrap().to_string();
    connect(&state, Who::Window).call("mcp.catalog.bind", json!({ "agent_id": "hosted", "mcp_id": id })).await;
    connect(&state, Who::Agent("hosted")).call("mcp.bind", json!({ "agent_id": "hosted", "mcp_id": id })).await;
    assert_eq!(catalog_row(&state, &id).2, vec![("hosted".to_string(), "operator".to_string())]);

    connect(&state, Who::Window).call("mcp.catalog.unbind", json!({ "agent_id": "hosted", "mcp_id": id })).await;
    connect(&state, Who::Agent("hosted")).call("mcp.bind", json!({ "agent_id": "hosted", "mcp_id": id })).await;
    assert_eq!(catalog_row(&state, &id).2, vec![("hosted".to_string(), "agent:hosted".to_string())]);
}

#[tokio::test]
async fn a_server_or_bind_made_before_this_was_kept_reads_unknown_and_deleting_a_server_forgets_it() {
    let state = state_with_agents();
    let old = crate::backend::storage::mcp_servers::McpServer {
        id: "old".into(),
        name: "old".into(),
        transport: "stdio".into(),
        config: "{}".into(),
        is_global: true,
        created_at: 1,
        updated_at: 1,
    };
    state.identity_store.mcp_server_insert_raw(&old).unwrap();
    state.mstore.conn().lock().unwrap().execute("INSERT INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES ('hosted', 'old')", []).unwrap();
    assert_eq!(catalog_row(&state, "old"), ("unknown".to_string(), "unknown".to_string(), vec![("hosted".to_string(), "unknown".to_string())]));

    // Changed now: who changed it is known, who added it is not.
    connect(&state, Who::Window).call("mcp.catalog.upsert", catalog_upsert("old", "old")).await;
    assert_eq!(catalog_row(&state, "old").0, "unknown");
    assert_eq!(catalog_row(&state, "old").1, "operator");

    connect(&state, Who::Window).call("mcp.catalog.delete", json!({ "id": "old" })).await;
    assert_eq!(state.identity_store.mcp_origin_of("old").unwrap(), crate::backend::storage::McpOrigin::default());
}

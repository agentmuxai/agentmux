// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use super::*;
use crate::backend::rpc_types::{RpcMessage, COMMAND_LIST_AGENTS, COMMAND_UPDATE_AGENT};
use crate::backend::storage::store::AgentContent;
use crate::backend::storage::AgentDefinition;
use crate::registry::DefinitionStore;
use crate::server::tests::test_state;

struct Harness {
    state: AppState,
    engine: Arc<WshRpcEngine>,
    rx: tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    _defs: tempfile::TempDir,
}

/// srv with a shared definition store (where seats live), `agent.setSeat`
/// and the definition RPCs; `as_agent` registers the connection as an agent,
/// and `None` is the operator's window, proven by the host.
fn harness(as_agent: Option<&str>) -> Harness {
    harness_with(match as_agent {
        Some(agent) => RpcContext { agent_id: agent.to_string(), ..Default::default() },
        None => RpcContext::operator_for_tests(),
    })
}

fn harness_with(ctx: RpcContext) -> Harness {
    let state = test_state();
    let defs = tempfile::tempdir().unwrap();
    state.mstore.set_def_registry(Arc::new(DefinitionStore::open(defs.path().join("definitions")).unwrap()));
    let (engine, rx) = WshRpcEngine::new();
    register(&engine, &state);
    super::super::core::register(&engine, &state);
    engine.set_rpc_context(ctx);
    Harness { state, engine, rx, _defs: defs }
}

impl Harness {
    async fn call(&mut self, command: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
        self.engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: "req-1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(10), self.rx.recv()).await.unwrap().unwrap();
        if resp.error.is_empty() {
            Ok(resp.data.unwrap_or_default())
        } else {
            Err(resp.error)
        }
    }

    fn agent(&self, id: &str, name: &str) -> AgentDefinition {
        let mut def = crate::backend::storage::agents::test_agent_def(id, name, "claude", "host", 1, "");
        self.state.mstore.agent_def_insert(&mut def).unwrap();
        def
    }

    fn listed(&self, id: &str) -> AgentDefinition {
        self.state.mstore.agent_def_list().unwrap().into_iter().find(|d| d.id == id).unwrap()
    }
}

fn seat_json(role: &str) -> serde_json::Value {
    serde_json::json!({ "role": role, "qualifier": "product", "area": "agent pane, identity" })
}

#[tokio::test]
async fn an_agent_connection_is_refused() {
    let mut h = harness(Some("Loap"));
    h.agent("a-1", "Loap");
    let err = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "a-1", "seat": seat_json("leader") })).await.unwrap_err();
    assert!(err.contains("FORBIDDEN"), "{err}");
    assert!(h.state.mstore.agent_seat_get("a-1").is_none(), "nothing written");
}

#[tokio::test]
async fn a_connection_the_host_never_proved_is_refused() {
    let mut h = harness_with(RpcContext::default());
    h.agent("a-1", "Loap");
    let err = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "a-1", "seat": seat_json("leader") })).await.unwrap_err();
    assert!(err.contains("FORBIDDEN") && err.contains("isn't the AgentMux window"), "{err}");
    assert!(h.state.mstore.agent_seat_get("a-1").is_none(), "nothing written");
}

#[tokio::test]
async fn the_operator_sets_and_clears_a_seat() {
    let mut h = harness(None);
    h.agent("a-1", "AgentX");
    let before = agentmux_common::time::now_ms();
    let got = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "a-1", "seat": seat_json("leader") })).await.unwrap();
    assert_eq!(got["seat"]["role"], "leader");
    let stored = h.state.mstore.agent_seat_get("a-1").unwrap();
    assert_eq!(stored.qualifier.as_deref(), Some("product"));
    assert_eq!(stored.area.as_deref(), Some("agent pane, identity"));
    assert!(stored.set_at_ms >= before);
    assert_eq!(h.listed("a-1").seat, Some(stored));

    let got = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "a-1", "seat": null })).await.unwrap();
    assert!(got["seat"].is_null());
    assert!(h.state.mstore.agent_seat_get("a-1").is_none());
    assert!(h.listed("a-1").seat.is_none());
}

#[tokio::test]
async fn an_invalid_seat_is_refused_and_nothing_written() {
    let mut h = harness(None);
    h.agent("a-1", "AgentX");
    h.agent("b-1", "Camper");
    // b reports to a.
    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "b-1", "seat": { "role": "worker", "lead": "a-1" } })).await.unwrap();
    for (seat, why) in [
        (serde_json::json!({ "role": "boss" }), "unknown role"),
        (serde_json::json!({ "role": "worker", "lead": "a-1" }), "its own lead"),
        (serde_json::json!({ "role": "worker", "lead": "ghost" }), "not an agent"),
        (serde_json::json!({ "role": "worker", "lead": "b-1" }), "cycle"),
        (serde_json::json!({ "role": "worker", "area": "x".repeat(121) }), "area"),
        (serde_json::json!({ "role": "worker", "qualifier": "x".repeat(41) }), "qualifier"),
    ] {
        let err = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "a-1", "seat": seat })).await.unwrap_err();
        assert!(err.contains(why), "{why}: {err}");
    }
    assert!(h.state.mstore.agent_seat_get("a-1").is_none());
    let err = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "nobody", "seat": seat_json("leader") })).await.unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

/// A rename through `updateagent` and an `agent.define` update, neither of
/// which carries a seat, keep the stored one.
#[tokio::test]
async fn updateagent_and_agent_define_keep_the_seat() {
    let mut h = harness(None);
    let id = define(&h, "seat-keeper").await;
    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": id, "seat": seat_json("specialist") })).await.unwrap();
    let stored = h.state.mstore.agent_seat_get(&id).unwrap();

    let def = h.listed(&id);
    let renamed = h
        .call(
            COMMAND_UPDATE_AGENT,
            serde_json::json!({ "id": id, "name": "Seat Keeper", "icon": def.icon, "provider": def.provider, "description": "" }),
        )
        .await
        .unwrap();
    assert_eq!(renamed["seat"]["role"], "specialist", "the response shows the seat");
    assert_eq!(h.listed(&id).name, "Seat Keeper");
    assert_eq!(h.state.mstore.agent_seat_get(&id), Some(stored.clone()));

    crate::server::app_api::agent_define_core(
        h.state.mstore.clone(),
        h.state.id_store.clone(),
        h.state.broker.clone(),
        serde_json::from_value(serde_json::json!({ "name": "seat-keeper", "if_exists": "update", "description": "touched" })).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(h.listed(&id).description, "touched");
    assert_eq!(h.state.mstore.agent_seat_get(&id), Some(stored));
}

fn make_worker_of(h: &Harness, worker: &str, leader: &str) {
    h.state
        .mstore
        .agent_content_set(&AgentContent {
            agent_id: worker.into(),
            content_type: crate::server::app_api::agent_spawn::WORKER_OF_CONTENT.into(),
            content: serde_json::json!({ "agent": "Leader", "id": leader }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

/// `listagents` shows a spawned worker's effective seat, and a stored seat
/// wins over it.
#[tokio::test]
async fn listagents_carries_the_effective_seat() {
    let mut h = harness(None);
    h.agent("lead-1", "Leader");
    h.agent("work-1", "Worker");
    make_worker_of(&h, "work-1", "lead-1");

    let list = h.call(COMMAND_LIST_AGENTS, serde_json::json!({})).await.unwrap();
    let row = |list: &serde_json::Value, id: &str| list.as_array().unwrap().iter().find(|d| d["id"] == id).cloned().unwrap();
    let worker = row(&list, "work-1");
    assert_eq!(worker["seat"]["role"], "worker");
    assert_eq!(worker["seat"]["qualifier"], "spawned");
    assert_eq!(worker["seat"]["lead"], "lead-1");
    assert_eq!(worker["seat"]["set_at_ms"], 0);
    assert!(row(&list, "lead-1").get("seat").is_none(), "no seat, no key");

    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "work-1", "seat": { "role": "analyst" } })).await.unwrap();
    let list = h.call(COMMAND_LIST_AGENTS, serde_json::json!({})).await.unwrap();
    assert_eq!(row(&list, "work-1")["seat"]["role"], "analyst");
    assert!(row(&list, "work-1")["seat"].get("lead").is_none());
}

/// Clearing a spawned worker's seat gives it its leader back as lead, so the
/// clear is refused when that leader now reports to the worker.
#[tokio::test]
async fn clearing_a_workers_seat_cant_make_a_cycle() {
    let mut h = harness(None);
    h.agent("lead-1", "Leader");
    h.agent("work-1", "Worker");
    make_worker_of(&h, "work-1", "lead-1");
    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "work-1", "seat": { "role": "specialist" } })).await.unwrap();
    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "lead-1", "seat": { "role": "worker", "lead": "work-1" } })).await.unwrap();

    let err = h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "work-1", "seat": null })).await.unwrap_err();
    assert!(err.contains("cycle"), "{err}");
    assert_eq!(h.state.mstore.agent_seat_get("work-1").unwrap().role, "specialist", "nothing written");

    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "lead-1", "seat": null })).await.unwrap();
    h.call(COMMAND_AGENT_SET_SEAT, serde_json::json!({ "agent_id": "work-1", "seat": null })).await.unwrap();
    assert!(h.state.mstore.agent_seat_get("work-1").is_none());
}

/// MuxAPI's `Agents list` (`/agentmux/discovery`) gives each host agent its
/// seat with the lead named, and `null` for one with none.
#[tokio::test]
async fn agents_list_carries_seats() {
    let mut h = harness(None);
    let lead = define(&h, "seat-lead").await;
    let work = define(&h, "seat-work").await;
    h.call(
        COMMAND_AGENT_SET_SEAT,
        serde_json::json!({ "agent_id": work, "seat": { "role": "worker", "lead": lead, "area": "panes" } }),
    )
    .await
    .unwrap();

    let axum::Json(body) = crate::server::http_health::handle_discovery(axum::extract::State(h.state.clone())).await;
    let agents = body["host"]["agents"].as_array().unwrap();
    let entry = |id: &str| agents.iter().find(|a| a["definition_id"] == id).cloned().unwrap_or_else(|| panic!("{id} in {agents:?}"));
    assert_eq!(
        entry(&work)["seat"],
        serde_json::json!({ "role": "worker", "qualifier": null, "lead": { "id": lead, "name": "seat-lead" }, "area": "panes" })
    );
    assert!(entry(&lead)["seat"].is_null());
}

/// An agent made the way `Agents` `open` finds it: a definition with its
/// My Agents row.
async fn define(h: &Harness, name: &str) -> String {
    crate::server::app_api::agent_define_core(
        h.state.mstore.clone(),
        h.state.id_store.clone(),
        h.state.broker.clone(),
        serde_json::from_value(serde_json::json!({ "name": name, "provider": "claude" })).unwrap(),
    )
    .await
    .unwrap()
    .definition_id
}

fn set(h: &Harness, id: &str, role: &str) {
    let change = SeatChange { agent_id: id.to_string(), seat: Some(SeatInput { role: role.to_string(), qualifier: Some("q".into()), lead: None, area: None, lane: None }) };
    let _seats = h.state.mstore.agent_seat_lock().unwrap();
    let defs = h.state.mstore.agent_def_list().unwrap();
    let planned = plan_seats(&h.state.mstore, &defs, std::slice::from_ref(&change), "").unwrap();
    write_seats(&h.state.mstore, &planned, &SeatActor::Operator).unwrap();
}

#[tokio::test]
async fn a_second_super_agent_takes_the_seat_and_the_first_goes_back() {
    let h = harness(None);
    h.agent("a-1", "AgentX");
    h.agent("a-2", "AgentY");
    set(&h, "a-1", "leader");
    set(&h, "a-1", "super");
    let first = h.state.mstore.agent_seat_get("a-1").unwrap();
    assert_eq!((first.role.as_str(), first.prior_role.as_deref()), ("super", Some("leader")));
    let defs = h.state.mstore.agent_def_list().unwrap();
    assert_eq!(crate::backend::agent_seat::super_holder(&h.state.mstore, &defs).as_deref(), Some("a-1"));

    set(&h, "a-2", "super");
    let back = h.state.mstore.agent_seat_get("a-1").unwrap();
    assert_eq!((back.role.as_str(), back.prior_role.as_deref(), back.qualifier.as_deref()), ("leader", None, Some("q")), "back to what it was");
    assert_eq!(h.state.mstore.agent_seat_get("a-2").unwrap().role, "super");
    let defs = h.state.mstore.agent_def_list().unwrap();
    assert_eq!(crate::backend::agent_seat::super_holder(&h.state.mstore, &defs).as_deref(), Some("a-2"));
}

#[tokio::test]
async fn a_super_agent_that_had_no_seat_goes_back_to_worker() {
    let h = harness(None);
    h.agent("a-1", "AgentX");
    h.agent("a-2", "AgentY");
    set(&h, "a-1", "super");
    assert_eq!(h.state.mstore.agent_seat_get("a-1").unwrap().prior_role, None);
    set(&h, "a-2", "super");
    assert_eq!(h.state.mstore.agent_seat_get("a-1").unwrap().role, "worker");
}

#[tokio::test]
async fn setting_the_seat_again_keeps_the_role_to_go_back_to() {
    let h = harness(None);
    h.agent("a-1", "AgentX");
    set(&h, "a-1", "specialist");
    set(&h, "a-1", "super");
    set(&h, "a-1", "super");
    assert_eq!(h.state.mstore.agent_seat_get("a-1").unwrap().prior_role.as_deref(), Some("specialist"));
}

#[tokio::test]
async fn two_super_agents_in_one_call_are_refused_and_nothing_is_written() {
    let h = harness(None);
    h.agent("a-1", "AgentX");
    h.agent("a-2", "AgentY");
    let input = || Some(SeatInput { role: "super".to_string(), qualifier: None, lead: None, area: None, lane: None });
    let changes = [SeatChange { agent_id: "a-1".into(), seat: input() }, SeatChange { agent_id: "a-2".into(), seat: input() }];
    let _seats = h.state.mstore.agent_seat_lock().unwrap();
    let defs = h.state.mstore.agent_def_list().unwrap();
    let err = plan_seats(&h.state.mstore, &defs, &changes, "").unwrap_err();
    assert!(err.contains("only one agent can be the super agent"), "{err}");
    assert!(h.state.mstore.agent_seat_get("a-1").is_none() && h.state.mstore.agent_seat_get("a-2").is_none());
}

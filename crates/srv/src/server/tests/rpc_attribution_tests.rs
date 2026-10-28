// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who each state-changing websocket RPC call is recorded as
//! (`backend::rpc::action_record`), through a real connection's handlers.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::rpc::action_commands::{ALWAYS, LIVE_INPUT, READ_ONLY, UNLESS_OPERATOR};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;

fn connection() -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) {
    let (engine, rx) = WshRpcEngine::new();
    register_handlers(&engine, test_state(), format!("attr-{}", uuid::Uuid::new_v4()));
    (engine, rx)
}

/// Calls `command` on `engine` and waits for the answer.
async fn call(engine: &Arc<WshRpcEngine>, rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>, command: &str, data: Value) -> String {
    let reqid = format!("attr-{}", uuid::Uuid::new_v4());
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: reqid.clone(), data: Some(data), ..Default::default() });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let m = rx.recv().await.expect("the connection stays open");
            if m.resid == reqid {
                return;
            }
        }
    })
    .await
    .expect("an answer within 10s");
    reqid
}

/// The recorded lines for one call.
fn recorded(reqid: &str) -> Vec<crate::backend::reactive::AuditLogEntry> {
    crate::backend::reactive::get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.event_kind == "action" && e.request_id == reqid)
        .collect()
}

#[tokio::test]
async fn every_registered_command_is_decided_and_every_decided_command_is_registered() {
    let (engine, _rx) = connection();
    let registered: std::collections::BTreeSet<String> = engine.command_names().into_iter().collect();
    let decided: std::collections::BTreeSet<String> =
        [ALWAYS, UNLESS_OPERATOR, LIVE_INPUT, READ_ONLY].iter().flat_map(|l| l.iter().map(|s| s.to_string())).collect();
    let undecided: Vec<_> = registered.difference(&decided).collect();
    let stale: Vec<_> = decided.difference(&registered).collect();
    assert!(undecided.is_empty(), "commands in no list of backend/rpc/action_commands.rs (decide whether they change anything): {undecided:?}");
    assert!(stale.is_empty(), "listed commands that no connection registers any more: {stale:?}");
}

#[tokio::test]
async fn an_agents_call_is_recorded_as_that_agent_with_its_target() {
    let (engine, mut rx) = connection();
    engine.set_rpc_context(RpcContext { agent_id: "agent-attr".into(), ..Default::default() });
    let reqid = call(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "no-such-pane", "agentid": "victim"})).await;
    let lines = recorded(&reqid);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let l = &lines[0];
    assert_eq!(l.source_agent.as_deref(), Some("agent:agent-attr"));
    assert_eq!((l.outcome.as_deref(), l.target_agent.as_str(), l.block_id.as_str()), (Some("agent.kill-tree"), "victim", "no-such-pane"));
}

#[tokio::test]
async fn a_socket_that_proved_nothing_is_never_recorded_as_the_operator() {
    let (engine, mut rx) = connection();
    // The default context: holds the instance key, is not an agent, has no host proof.
    let reqid = call(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "no-such-pane"})).await;
    assert_eq!(recorded(&reqid)[0].source_agent.as_deref(), Some("unproven"));
}

#[tokio::test]
async fn the_proven_window_is_the_operator() {
    let (engine, mut rx) = connection();
    assert!(engine.mark_operator_proven());
    let reqid = call(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "no-such-pane"})).await;
    assert_eq!(recorded(&reqid)[0].source_agent.as_deref(), Some("operator"));
}

#[tokio::test]
async fn registering_as_an_agent_after_the_proof_is_the_agent_not_the_operator() {
    let (engine, mut rx) = connection();
    assert!(engine.mark_operator_proven());
    engine.set_rpc_context(RpcContext { agent_id: "agent-attr".into(), ..Default::default() });
    assert!(!engine.mark_operator_proven(), "an agent connection cannot be proven");
    let reqid = call(&engine, &mut rx, "agent.kill-tree", json!({"blockid": "no-such-pane"})).await;
    assert_eq!(recorded(&reqid)[0].source_agent.as_deref(), Some("agent:agent-attr"));
}

#[tokio::test]
async fn a_refused_call_is_recorded_with_why() {
    let (engine, mut rx) = connection();
    let reqid = call(&engine, &mut rx, "widgets.setenabled", json!({"id": "nobody.nothing", "enabled": true})).await;
    let l = &recorded(&reqid)[0];
    assert_eq!(l.source_agent.as_deref(), Some("unproven"));
    assert_eq!(l.outcome.as_deref(), Some("widgets.setenabled"));
    assert_eq!(l.reason.as_deref(), Some("id=nobody.nothing"));
}

#[tokio::test]
async fn reads_and_the_operators_own_window_plumbing_are_not_recorded() {
    let (engine, mut rx) = connection();
    let read = call(&engine, &mut rx, "listagents", json!({})).await;
    assert!(recorded(&read).is_empty());
    // The window's own view state: the proven operator moves it unrecorded, an unproven socket does not.
    let before = call(&engine, &mut rx, "setmeta", json!({"oref": "block:none", "meta": {}})).await;
    assert_eq!(recorded(&before).len(), 1, "an unproven socket changing view state is recorded");
    assert!(engine.mark_operator_proven());
    let after = call(&engine, &mut rx, "setmeta", json!({"oref": "block:none", "meta": {}})).await;
    assert!(recorded(&after).is_empty());
}

#[tokio::test]
async fn a_requests_contents_never_reach_the_record() {
    let (engine, mut rx) = connection();
    let reqid = call(
        &engine,
        &mut rx,
        "upsertidentityaccount",
        json!({"id": "acct-attr", "clientSecret": "SECRET-VALUE-1", "token": "TOKEN-VALUE-2", "content": "BODY-VALUE-3"}),
    )
    .await;
    let lines = recorded(&reqid);
    assert_eq!(lines.len(), 1);
    let all = serde_json::to_string(&lines).unwrap();
    for secret in ["SECRET-VALUE-1", "TOKEN-VALUE-2", "BODY-VALUE-3"] {
        assert!(!all.contains(secret), "{secret} reached the record: {all}");
    }
    assert!(all.contains("acct-attr"));
}

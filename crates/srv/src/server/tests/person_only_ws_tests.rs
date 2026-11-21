// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What only the person does, through the handlers a real connection registers: a settings write that waives or
//! widens consent, "Keep running" on a stopping agent, and what is kept as typed by the person. An agent's connection
//! and one that proved nothing are refused each; the proven window reaches the handler.

use std::sync::Arc;

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::server::tests::test_state;
use crate::server::websocket::register_handlers;

/// Each call, with arguments that reach the handler's own work.
fn calls() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("setconfig", json!({"agent:maystartagents": true})),
        ("setconfig", json!({"term:fontsize": 12, "supervisor:enabled": true})),
        ("agentshutdownkeep", json!({"blockid": "no-such-block", "request_id": "r-1"})),
        ("agent.input-history-append", json!({"agent_id": "no-such-agent", "text": "hello", "kind": "message"})),
        ("agent.input-draft", json!({"agent_id": "no-such-agent", "text": "half a thought"})),
    ]
}

async fn error_of(ctx: RpcContext, command: &str, data: serde_json::Value) -> String {
    let (engine, mut rx) = WshRpcEngine::new();
    register_handlers(&engine, test_state(), format!("person-only-{}", uuid::Uuid::new_v4()));
    engine.set_rpc_context(ctx);
    let reqid = format!("person-only-{}", uuid::Uuid::new_v4());
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
    .expect("the call is answered")
}

#[tokio::test]
async fn a_connection_registered_as_an_agent_is_refused_even_with_the_proof() {
    for (command, data) in calls() {
        let agent = RpcContext { agent_id: "korp".into(), operator_proven: true, ..Default::default() };
        let error = error_of(agent, command, data.clone()).await;
        assert!(error.starts_with("FORBIDDEN: only the person"), "{command} {data}: {error:?}");
        assert!(error.contains("ask the person"), "{command} {data}: what to do instead: {error:?}");
    }
}

#[tokio::test]
async fn a_connection_that_proved_nothing_is_refused() {
    for (command, data) in calls() {
        let error = error_of(RpcContext::default(), command, data.clone()).await;
        assert!(error.contains("isn't the AgentMux window"), "{command} {data}: {error:?}");
    }
}

#[tokio::test]
async fn the_window_reaches_keep_running_and_what_is_typed() {
    for (command, data) in calls().into_iter().filter(|(command, _)| *command != "setconfig") {
        let error = error_of(RpcContext::operator_for_tests(), command, data.clone()).await;
        assert!(!error.contains("FORBIDDEN"), "{command} {data}: {error:?}");
    }
}

#[tokio::test]
async fn an_agent_reads_its_draft_as_before() {
    let agent = RpcContext { agent_id: "korp".into(), ..Default::default() };
    let error = error_of(agent, "agent.input-draft", json!({"agent_id": "no-such-agent"})).await;
    assert!(!error.contains("FORBIDDEN"), "{error:?}");
}

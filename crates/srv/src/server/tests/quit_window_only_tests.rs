// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/shutdown` and Shut down are the person's own choices: the window makes them over its websocket, an agent's or
//! an unproven connection is refused, and the generic service route asks the person (as `Agents stop` does) in
//! the name of its caller instead of ending the agent as the person's slash or menu action.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::blockcontroller::{self, Controller};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::backend::service::WebCallType;
use crate::sagas::pending_shutdown as ps;
use crate::server::tests::test_state;
use crate::server::AppState;

struct Running(String, Arc<AtomicBool>);

impl Controller for Running {
    fn start(&self, _: crate::backend::obj::MetaMapType, _: Option<Value>, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn stop(&self, _: bool, _: &str) -> Result<(), String> {
        self.1.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn get_runtime_status(&self) -> blockcontroller::BlockControllerRuntimeStatus {
        blockcontroller::BlockControllerRuntimeStatus { blockid: self.0.clone(), ..Default::default() }
    }
    fn send_input(&self, _: blockcontroller::BlockInputUnion, _: Option<u64>) -> Result<(), String> {
        Ok(())
    }
    fn controller_type(&self) -> &str {
        "stub"
    }
    fn block_id(&self) -> &str {
        &self.0
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

async fn ws_error(state: &AppState, ctx: RpcContext, command: &str, data: Value) -> String {
    let (engine, mut rx) = WshRpcEngine::new();
    crate::server::websocket::register_handlers(&engine, state.clone(), "quit-window-only".to_string());
    engine.set_rpc_context(ctx);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    resp.error
}

#[tokio::test]
async fn an_agent_connection_even_one_carrying_the_proof_and_an_unproven_one_cannot_end_an_agent_as_the_person() {
    let state = test_state();
    let agent = RpcContext { agent_id: "korp".into(), operator_proven: true, ..Default::default() };
    for (command, data) in [("agentquit", json!({"blockid": "b-1"})), ("agentshutdown", json!({"blockid": "b-1", "stop_background": true}))] {
        let error = ws_error(&state, agent.clone(), command, data.clone()).await;
        assert!(error.starts_with("FORBIDDEN: only the person shuts an agent down"), "{command}: {error}");
        assert!(error.contains("Agents stop"), "{command}: what to do instead: {error}");
        let error = ws_error(&state, RpcContext::default(), command, data).await;
        assert!(error.contains("isn't the AgentMux window"), "{command}: {error}");
    }
}

#[tokio::test]
async fn the_window_reaches_the_shutdown_and_is_told_the_saga_s_own_verdict() {
    let state = test_state();
    for (command, data) in [("agentquit", json!({"blockid": "b-none"})), ("agentshutdown", json!({"blockid": "b-none"}))] {
        let error = ws_error(&state, RpcContext::operator_for_tests(), command, data).await;
        assert!(error.contains("block not found: b-none"), "{command}: {error}");
    }
}

fn running_agent() -> (String, Arc<AtomicBool>) {
    let block_id = format!("blk-quit-{}", uuid::Uuid::new_v4());
    let stopped = Arc::new(AtomicBool::new(false));
    blockcontroller::register_controller(&block_id, Arc::new(Running(block_id.clone(), stopped.clone())));
    (block_id, stopped)
}

fn service_call(method: &str, args: Vec<Value>) -> WebCallType {
    WebCallType { service: "object".into(), method: method.into(), uicontext: None, args }
}

#[tokio::test]
async fn the_service_route_asks_the_person_in_the_callers_name_and_ends_nothing_at_once() {
    let state = test_state();
    for (method, args) in [("QuitAgent", vec![]), ("ShutdownAgent", vec![json!(true)])] {
        let (block_id, stopped) = running_agent();
        let mut call_args = vec![json!(block_id)];
        call_args.extend(args);
        let reply = crate::server::service::run_service_call_as(&state, &service_call(method, call_args), Some("uid-quit-korp")).await;
        assert!(reply.success, "{method}: {reply:?}");
        let request_id = reply.data.as_ref().unwrap()["pending"]["request_id"].as_str().unwrap().to_string();
        let pending = ps::status(&request_id).unwrap();
        assert_eq!((pending.status, pending.by.as_str(), pending.via.as_str()), ("pending", "uid-quit-korp", "Service"), "{method}");
        assert!(!stopped.load(Ordering::SeqCst), "{method}: not ended while the person can still keep it");
        assert_eq!(ps::keep(&state, &block_id, &request_id), "kept_by_user");
        assert!(!stopped.load(Ordering::SeqCst), "{method}: kept by the person");
        blockcontroller::delete_controller(&block_id);
    }
}

#[tokio::test]
async fn a_service_request_with_no_agent_credential_is_named_as_unverified_never_as_the_person() {
    let state = test_state();
    let (block_id, _) = running_agent();
    let reply = crate::server::service::run_service_call(&state, &service_call("QuitAgent", vec![json!(block_id)])).await;
    assert!(reply.success, "{reply:?}");
    let request_id = reply.data.as_ref().unwrap()["pending"]["request_id"].as_str().unwrap().to_string();
    let pending = ps::status(&request_id).unwrap();
    assert_eq!(pending.by, "an unverified caller");
    assert_ne!(pending.via, "UserSlash");
    ps::keep(&state, &block_id, &request_id);
    blockcontroller::delete_controller(&block_id);
}

#[tokio::test]
async fn the_service_route_refuses_a_block_with_no_running_agent() {
    let state = test_state();
    for method in ["QuitAgent", "ShutdownAgent"] {
        let reply = crate::server::service::run_service_call_as(&state, &service_call(method, vec![json!("blk-nowhere")]), None).await;
        assert!(!reply.success && reply.error.as_deref().unwrap_or_default().contains("no running agent"), "{method}: {reply:?}");
    }
}

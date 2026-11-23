// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/shutdown` and Shut down are the person's own choices: the window makes them over its websocket, an agent's or
//! an unproven connection is refused, and the generic service route asks the person (as `Agents stop` does) in
//! the name of its caller instead of ending the agent as the person's slash or menu action, under the same
//! who-may-stop-whom as `Stop agents` (and none without an agent's token) and a limit on the countdowns one agent opens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::blockcontroller::{self, Controller};
use crate::backend::obj::{Block, MetaMapType};
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

/// A running agent's pane: its controller, and the block that says whose pane it is (`view` "term" makes it no agent's).
fn running_pane(state: &AppState, view: &str, agent_id: &str) -> (String, Arc<AtomicBool>) {
    let (block_id, stopped) = running_agent();
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), json!(view));
    if !agent_id.is_empty() {
        meta.insert("agentId".to_string(), json!(agent_id));
    }
    let mut block = Block { oid: block_id.clone(), parentoref: String::new(), version: 0, runtimeopts: None, stickers: None, meta, subblockids: None };
    state.mstore.insert(&mut block).unwrap();
    (block_id, stopped)
}

/// An agent srv knows, so that a request carrying its token is a known agent's.
fn known_agent(state: &AppState, uid: &str) {
    let mut def: crate::backend::storage::store::AgentDefinition = serde_json::from_value(json!({
        "id": uid, "name": uid, "slug": uid, "icon": "wand-magic-sparkles", "provider": "claude", "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
}

async fn quit_as(state: &AppState, block_id: &str, caller: Option<&str>) -> crate::backend::service::WebReturnType {
    crate::server::service::run_service_call_as(state, &service_call("QuitAgent", vec![json!(block_id)]), caller).await
}

fn request_id(reply: &crate::backend::service::WebReturnType) -> String {
    reply.data.as_ref().unwrap()["pending"]["request_id"].as_str().unwrap().to_string()
}

fn service_call(method: &str, args: Vec<Value>) -> WebCallType {
    WebCallType { service: "object".into(), method: method.into(), uicontext: None, args }
}

#[tokio::test]
async fn the_service_route_asks_the_person_in_the_callers_name_and_ends_nothing_at_once() {
    let state = test_state();
    known_agent(&state, "uid-quit-korp");
    for (method, args) in [("QuitAgent", vec![]), ("ShutdownAgent", vec![json!(true)])] {
        let (block_id, stopped) = running_pane(&state, "agent", "target-of-korp");
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
async fn a_service_request_with_no_agent_credential_is_refused_and_opens_no_countdown() {
    let state = test_state();
    for method in ["QuitAgent", "ShutdownAgent"] {
        let (block_id, stopped) = running_pane(&state, "agent", "target-of-nobody");
        let reply = crate::server::service::run_service_call(&state, &service_call(method, vec![json!(block_id)])).await;
        let error = reply.error.as_deref().unwrap_or_default();
        assert!(!reply.success && error.contains("FORBIDDEN") && error.contains("AGENTMUX_AGENT_TOKEN") && error.contains("from the window"), "{method}: {reply:?}");
        assert!(ps::active_for(&block_id).is_none(), "{method}: nothing is put in front of the person");
        assert!(!stopped.load(Ordering::SeqCst));
        blockcontroller::delete_controller(&block_id);
    }
}

#[tokio::test]
async fn an_agent_may_open_ten_countdowns_a_minute_one_per_agent_and_another_agent_has_its_own_ten() {
    let state = test_state();
    let (busy, other) = ("uid-quit-busy", "uid-quit-other");
    known_agent(&state, busy);
    known_agent(&state, other);
    let mut opened = Vec::new();
    for i in 0..10 {
        let (block_id, _) = running_pane(&state, "agent", &format!("target-{i}"));
        let reply = quit_as(&state, &block_id, Some(busy)).await;
        assert!(reply.success, "{i}: {reply:?}");
        opened.push((block_id, request_id(&reply)));
    }
    let (extra, _) = running_pane(&state, "agent", "target-extra");
    let refused = quit_as(&state, &extra, Some(busy)).await;
    assert!(!refused.success && refused.error.as_deref().unwrap_or_default().contains("RATE_LIMITED"), "{refused:?}");
    assert!(ps::active_for(&extra).is_none());
    let first_deadline = ps::status(&opened[0].1).unwrap().deadline_ms;
    let joined = quit_as(&state, &opened[0].0, Some(busy)).await;
    assert!(joined.success && request_id(&joined) == opened[0].1, "a second request for the same agent joins its countdown: {joined:?}");
    let other_asks = quit_as(&state, &opened[0].0, Some(other)).await;
    assert!(other_asks.success && request_id(&other_asks) == opened[0].1, "another agent joins it too, and the clock does not restart");
    assert_eq!(ps::status(&opened[0].1).unwrap().deadline_ms, first_deadline, "the clock never restarts");
    let fresh = quit_as(&state, &extra, Some(other)).await;
    assert!(fresh.success, "{fresh:?}");
    ps::keep(&state, &extra, &request_id(&fresh));
    for (block_id, request_id) in opened {
        ps::keep(&state, &block_id, &request_id);
        blockcontroller::delete_controller(&block_id);
    }
    blockcontroller::delete_controller(&extra);
}

#[tokio::test]
async fn the_service_route_refuses_what_stop_agents_refuses_and_opens_no_countdown() {
    let state = test_state();
    known_agent(&state, "uid-quit-asker");
    let (term, _) = running_pane(&state, "term", "");
    let reply = quit_as(&state, &term, Some("uid-quit-asker")).await;
    assert!(!reply.success && reply.error.as_deref().unwrap_or_default().contains("not an agent pane"), "{reply:?}");
    assert!(ps::active_for(&term).is_none());
    let (own, _) = running_pane(&state, "agent", "uid-quit-asker");
    let reply = quit_as(&state, &own, Some("uid-quit-asker")).await;
    assert!(!reply.success && reply.error.as_deref().unwrap_or_default().contains("Stop with action \"self\""), "{reply:?}");
    assert!(ps::active_for(&own).is_none());
    let (target, _) = running_pane(&state, "agent", "target-of-ghost");
    let reply = quit_as(&state, &target, Some("uid-never-defined")).await;
    assert!(!reply.success && reply.error.as_deref().unwrap_or_default().contains("the token's agent is unknown"), "{reply:?}");
    assert!(ps::active_for(&target).is_none());
    for block_id in [term, own, target] {
        blockcontroller::delete_controller(&block_id);
    }
}

#[tokio::test]
async fn the_service_route_refuses_a_block_with_no_running_agent() {
    let state = test_state();
    known_agent(&state, "uid-quit-nowhere");
    for method in ["QuitAgent", "ShutdownAgent"] {
        let reply = crate::server::service::run_service_call_as(&state, &service_call(method, vec![json!("blk-nowhere")]), Some("uid-quit-nowhere")).await;
        assert!(!reply.success && reply.error.as_deref().unwrap_or_default().contains("no running agent"), "{method}: {reply:?}");
    }
}

async fn preview_as(state: &AppState, block_id: &str, caller: Option<&str>) -> crate::backend::service::WebReturnType {
    crate::server::service::run_service_call_as(state, &service_call("ShutdownPreview", vec![json!(block_id)]), caller).await
}

#[tokio::test]
async fn an_agent_reads_the_shutdown_preview_of_its_own_pane_only_and_the_window_reads_any() {
    let state = test_state();
    known_agent(&state, "uid-prev-reader");
    let (own, _) = running_pane(&state, "agent", "uid-prev-reader");
    let (other, _) = running_pane(&state, "agent", "uid-prev-other");
    let reply = preview_as(&state, &own, Some("uid-prev-reader")).await;
    assert!(reply.success && reply.data.as_ref().is_some_and(|d| d.get("claims").is_some()), "its own: {reply:?}");
    let reply = preview_as(&state, &other, Some("uid-prev-reader")).await;
    let error = reply.error.as_deref().unwrap_or_default();
    assert!(!reply.success && reply.data.is_none() && error.contains("only its own pane") && error.contains("Agents list"), "another's: {reply:?}");
    let reply = preview_as(&state, &other, Some("uid-prev-never-defined")).await;
    assert!(!reply.success && reply.error.as_deref().unwrap_or_default().contains("the token's agent is unknown"), "{reply:?}");
    for block_id in [&own, &other] {
        let reply = preview_as(&state, block_id, None).await;
        assert!(reply.success && reply.data.as_ref().is_some_and(|d| d.get("working").is_some()), "the window: {reply:?}");
        blockcontroller::delete_controller(block_id);
    }
}

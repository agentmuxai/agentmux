// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A stop or release another AgentMux asks for names its asker itself and
//! cannot be checked, so this instance's user is shown it first and can keep
//! the agent running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::blockcontroller::{self, Controller};
use crate::sagas::pending_shutdown as ps;
use crate::server::routes::build_router;
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

async fn post(state: &AppState, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = build_router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The instant stop forward no longer stops at once: it opens this instance's window, names the asker as a claim,
/// and the stop happens only if the user lets the window run out.
#[tokio::test]
async fn a_forwarded_stop_waits_for_the_user_and_names_its_asker_as_a_claim() {
    let state = test_state();
    let block_id = format!("blk-peer-stop-{}", uuid::Uuid::new_v4());
    let stopped = Arc::new(AtomicBool::new(false));
    blockcontroller::register_controller(&block_id, Arc::new(Running(block_id.clone(), stopped.clone())));

    let (status, v) = post(&state, "/agentmux/agent/stop", json!({ "block_id": block_id, "by": "Korp" })).await;
    assert_eq!((status, &v["success"]), (StatusCode::OK, &json!(true)), "{v}");
    let request_id = v["pending"]["request_id"].as_str().unwrap().to_string();
    let here = ps::status(&request_id).unwrap();
    assert_eq!((here.status, here.by.as_str()), ("pending", "Korp (as claimed by another AgentMux)"));
    assert!(!stopped.load(Ordering::SeqCst), "not stopped while the user can still keep it");

    assert_eq!(ps::keep(&state, &block_id, &request_id), "kept_by_user");
    assert!(!stopped.load(Ordering::SeqCst));
    blockcontroller::delete_controller(&block_id);

    // Not running here: refused, no window.
    let (_, v) = post(&state, "/agentmux/agent/stop", json!({ "block_id": "peer-nowhere" })).await;
    assert_eq!(v["success"], false);
    assert!(v["error"].as_str().unwrap().starts_with("NOT_RUNNING"), "{v}");
}

/// A window for a release that the user lets run out ends as shut down; one the user keeps ends as kept, which the
/// release handler answers with a refusal.
#[tokio::test]
async fn a_release_the_user_keeps_is_reported_as_kept() {
    let state = test_state();
    let kept = ps::request(&state, "blk-peer-rel-a", "AgentMux channel x (as claimed by another AgentMux)", "Takeover", "", ps::Action::Release { to: "channel x".into() });
    assert_eq!((kept.effect, kept.via.as_str()), ("stop", "Takeover"));
    assert_eq!(ps::keep(&state, "blk-peer-rel-a", &kept.request_id), "kept_by_user");
    assert_eq!(ps::status(&kept.request_id).unwrap().status, "kept_by_user");

    // Nothing is running under this block, so letting it run out finds nothing to release.
    let ran = ps::request(&state, "blk-peer-rel-b", "x", "Takeover", "", ps::Action::Release { to: String::new() });
    ps::expire_now(&ran.request_id);
    assert_eq!(ps::settled(&ran.request_id).await.status, "superseded");
}

/// A pending stop that a release joins becomes the release: it does all a stop does and hands the lease over.
#[tokio::test]
async fn a_release_joining_a_pending_stop_takes_it_over() {
    let state = test_state();
    let stop = ps::request(&state, "blk-peer-rel-c", "an agent", "FleetBulkStop", "", ps::Action::Stop { signal: None, stop_background: None });
    let release = ps::request(&state, "blk-peer-rel-c", "AgentMux", "Takeover", "", ps::Action::Release { to: "channel y".into() });
    assert!(release.joined && release.request_id == stop.request_id);
    assert_eq!(ps::keep(&state, "blk-peer-rel-c", &stop.request_id), "kept_by_user");
}

/// A bulk-stop that forwarded to another instance reports the stop when that instance's user lets the window run
/// out, and a refusal when they keep the agent running.
#[tokio::test]
async fn a_forward_waits_for_the_other_window_and_reports_its_outcome() {
    use axum::routing::get;
    let state = test_state();
    for (status, expect_ok) in [("shut_down", true), ("kept_by_user", false), ("failed", false)] {
        let app = axum::Router::new().route(
            "/api/v1/agent/shutdown/:id",
            get(move || async move { axum::Json(json!({ "status": status, "error": "boom" })) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let r = crate::server::app_api::fleet::wait_for_remote_window(&state, &url, "k", "req-1").await;
        assert_eq!(r.is_ok(), expect_ok, "{status}: {r:?}");
    }
}

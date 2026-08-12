// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use crate::backend::reactive as backend_reactive;
use crate::backend::wconfig;
use crate::backend::wcore;

pub(crate) fn test_state() -> AppState {
    let wstore = Arc::new(Store::open_in_memory().unwrap());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let event_bus = Arc::new(EventBus::new());
    let broker = Arc::new(Broker::new());
    let reactive_handler = backend_reactive::get_global_handler();
    let poller = Arc::new(Poller::new(
        backend_reactive::PollerConfig {
            muxbus_url: None,
            muxbus_token: None,
            poll_interval_secs: 30,
        },
        reactive_handler,
    ));

    // Bootstrap initial data
    wcore::ensure_initial_data(&wstore).unwrap();

    let config_watcher = Arc::new(wconfig::ConfigWatcher::new());

    let process_tracker = Arc::new(
        crate::backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
    );
    let process_broker = Arc::new(crate::broker::ProcessBroker::new(Some(broker.clone())));
    let fs_watch_pool = crate::backend::fs_watch::FsWatchPool::new();

    AppState {
        auth_key: "test-secret-key".to_string(),
        boot_id: std::sync::Arc::from("test-boot"),
        version: "0.28.20".to_string(),
        app_path: String::new(),
        wstore: wstore.clone(),
        shared_store: None,
        id_store: wstore.clone(),
        filestore,
        global_transcript_store: None,
        event_bus: event_bus.clone(),
        broker: broker.clone(),
        reactive_handler,
        poller,
        config_watcher,
        messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
        http_client: reqwest::Client::new(),
        local_web_url: String::new(),
        subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone(), wstore.clone())),
        history_service: Arc::new(crate::backend::history::HistoryService::new()),
        lan_discovery: Arc::new(crate::backend::lan_discovery::LanDiscoveryController::new(
            "test-instance".to_string(),
            "test-host".to_string(),
            "0.28.20".to_string(),
            0,
            event_bus.clone(),
            String::new(),
        )),
        lsp_supervisor: Arc::new(crate::backend::lsp::LspSupervisor::new(event_bus.clone())),
        process_tracker,
        process_broker,
        dock_snapshots: Arc::new(crate::backend::dock_snapshot::DockSnapshotCache::new()),
        // Phase E.2c.2 — workspace RPC dispatches through reducer.
        // Tests get fresh state + a dummy broadcast bus.
        srv_state: Arc::new(tokio::sync::Mutex::new(crate::state::State::default())),
        srv_events_tx: tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(64).0,
        saga_id_alloc: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        // Saga durability (PR 1) — in-memory log so tests stay
        // hermetic. Production opens a file under the data dir.
        saga_log: Arc::new(crate::sagas::log::SagaLog::open_in_memory().unwrap()),
        auth_session_manager: Arc::new(crate::identity::auth_session::AuthSessionManager::new()),
        install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
        container_manager: Arc::new(crate::backend::container::ContainerRuntimeHandle::disabled()),
        shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
        cron_scheduler: crate::backend::cron::CronScheduler::new(
            None,
            reqwest::Client::new(),
            String::new(),
            "test-secret-key".to_string(),
            broker.clone(),
        ),
        editor_file_watcher: crate::backend::editor_file_watcher::EditorFileWatcher::new(
            fs_watch_pool.clone(),
            broker.clone(),
        ),
        media_file_watcher: crate::backend::media_file_watcher::MediaFileWatcher::new(
            fs_watch_pool.clone(),
            broker.clone(),
        ),
        fs_watch_pool: fs_watch_pool.clone(),
    }
}

fn test_router() -> Router {
    build_router(test_state())
}

#[tokio::test]
async fn health_returns_200() {
    let app = test_router();
    let req = Request::builder()
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], "0.28.20");
}

#[tokio::test]
async fn auth_rejects_bad_key() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "wrong-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"service":"client","method":"GetClientData"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_rejects_missing_key() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"service":"client","method":"GetClientData"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_accepts_valid_header() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"client","method":"GetClientData"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["success"].as_bool().unwrap());
}

#[tokio::test]
async fn auth_rejects_query_param_on_http_routes() {
    // 2026-05-11 audit (C3): the query-string `?authkey=` fallback
    // bypassed CORS preflight and leaked into logs / history. Only
    // /ws still honors it (browsers can't set headers on WS upgrade).
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service?authkey=test-secret-key")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"client","method":"GetClientData"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reactive_routes_require_auth_unauthenticated() {
    // Audit C1/C2 fix: /agentmux/reactive/* used to skip auth on the
    // "localhost is trusted" assumption. It isn't — same-host CSRF
    // via the permissive CORS layer could drive inject + poller-config.
    // These routes now require X-AuthKey. The previous test
    // (`reactive_routes_skip_auth`) asserted the bug; this one asserts
    // the fix.
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/reactive/agents")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reactive_routes_accept_valid_authkey() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/reactive/agents")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn cors_reflects_loopback_origin() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/")
        .header("Origin", "http://localhost:5173")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // 2026-05-11 audit (C3): reflect only loopback origins.
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(allow, "http://localhost:5173");
}

#[tokio::test]
async fn cors_rejects_non_loopback_origin() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/")
        .header("Origin", "https://attacker.example")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // Predicate denial means no Access-Control-Allow-Origin header is
    // emitted; browsers will block the cross-origin request as a result.
    assert!(resp
        .headers()
        .get("access-control-allow-origin")
        .is_none());
}

#[tokio::test]
async fn cors_exposes_zonefileinfo_header_to_cross_origin_callers() {
    // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1 follow-up —
    // without `expose_headers`, a cross-origin fetch() can't read
    // X-ZoneFileInfo even when the server sends it: `Response.headers.get()`
    // silently returns null for any header not in this list, indistinguishable
    // from the server never having sent it at all. Checked on an actual
    // response (not the OPTIONS preflight — Access-Control-Expose-Headers is
    // a real-response header per the CORS spec, preflight only negotiates
    // allowed methods/headers for the request itself).
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/agentmux/file?zoneid=nonexistent&name=term")
        .header("Origin", "http://localhost:5173")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let expose = resp
        .headers()
        .get("access-control-expose-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        expose.to_lowercase().contains("x-zonefileinfo"),
        "expected X-ZoneFileInfo in Access-Control-Expose-Headers, got {:?}",
        expose
    );
}

#[tokio::test]
async fn service_get_client_data() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"client","method":"GetClientData"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"]["oid"].is_string());
    assert!(json["data"]["windowids"].is_array());
}

#[tokio::test]
async fn service_list_workspaces() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"workspace","method":"ListWorkspaces"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"].is_array());
}

#[tokio::test]
async fn service_unknown_method_returns_error() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"foo","method":"Bar"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // success=false is skipped by serde (skip_serializing_if), so it's null
    assert!(!json["success"].as_bool().unwrap_or(false));
    assert!(json["error"].as_str().unwrap().contains("unknown"));
}

#[tokio::test]
async fn reactive_agents_returns_empty_list() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/reactive/agents")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array());
}

#[tokio::test]
async fn reactive_transcript_missing_agent_param_is_400() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/reactive/transcript")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reactive_transcript_unknown_agent_is_404() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/reactive/transcript?agent=nonexistent-agent")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reactive_supervisor_decision_missing_target_agent_is_rejected() {
    // `target_agent` is a required field on `SupervisorDecisionRequest`, so
    // an omitted key fails at axum's Json extractor (422), before the
    // handler's own empty-string check (400) ever runs.
    let app = test_router();
    let body = serde_json::json!({"action": "decline"});
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reactive_supervisor_decision_empty_target_agent_is_400() {
    let app = test_router();
    let body = serde_json::json!({"target_agent": "", "action": "decline"});
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reactive_supervisor_decision_unknown_action_is_400() {
    let app = test_router();
    let body = serde_json::json!({"target_agent": "some-agent", "action": "maybe"});
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reactive_supervisor_decision_nudge_without_message_is_400() {
    let app = test_router();
    let body = serde_json::json!({"target_agent": "some-agent", "action": "nudge"});
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reactive_supervisor_decision_decline_succeeds_for_unregistered_target() {
    // Decline never attempts delivery, so it succeeds even for a target
    // that isn't (or is no longer) registered — matching a Supervisor
    // deciding not to nudge an agent it can no longer see.
    let app = test_router();
    let body = serde_json::json!({
        "target_agent": "supervisor-decision-test-decline-target-http",
        "action": "decline",
        "reason": "target looks done"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["success"].as_bool().unwrap());
}

#[tokio::test]
async fn reactive_poller_status() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/reactive/poller/status")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_object());
}


#[tokio::test]
async fn wps_publish_accepts_persist_field() {
    // Regression for SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §6.1
    // (and PR β.B): WpsPublishRequest previously dropped the `persist`
    // field on deserialization, so every tool_chunk publish went to
    // the broker with persist=0. Late-subscribing frontends never got
    // replay history, defeating the per-block subscription strategy.
    //
    // This test exercises the deserialize + handler-200 path with a
    // 1024-persist body matching what agentmux-bashwrap actually
    // sends. Broker-level persistence semantics are covered by
    // wps.rs::tests::test_event_persistence.
    let app = test_router();
    let body = serde_json::json!({
        "event": "tool_chunk",
        "scopes": ["block:abc123"],
        "persist": 1024,
        "data": {
            "op": "chunk",
            "kind": "stdout",
            "content": "hello\n",
            "timestamp": 1_700_000_000_000_u64,
            "tool_id": "toolu_test"
        }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/wps/publish")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn wps_publish_omits_persist_defaults_to_zero() {
    // Defensive: a publisher that doesn't include the persist field
    // should still succeed (serde default 0 → pure fan-out, no replay).
    let app = test_router();
    let body = serde_json::json!({
        "event": "tool_chunk",
        "scopes": ["block:abc123"],
        "data": { "op": "chunk", "kind": "stdout", "content": "x", "timestamp": 0_u64, "tool_id": "t" }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/wps/publish")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---- First-class agent API (SPEC_AGENT_API_FIRST_CLASS_SURFACE) ----

/// `GET /api/v1/self` resolves the seeded agent block to its tab / window /
/// workspace. Read-only (no reducer), so it's hermetic in the test harness.
#[tokio::test]
async fn self_endpoint_resolves_seeded_agent() {
    let state = test_state();
    let wstore = state.wstore.clone();
    let tab = wstore
        .get_all::<crate::backend::obj::Tab>()
        .unwrap()
        .into_iter()
        .next()
        .expect("seeded tab");
    let block_id = tab.blockids.first().expect("seeded agent block").clone();
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/v1/self?block_id={block_id}"))
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["block_id"], block_id);
    assert_eq!(json["tab_id"], tab.oid);
    assert_eq!(json["workspace_name"], "Starter workspace");
    assert!(json["workspace_id"].is_string(), "workspace_id resolved");
    assert!(json["window_id"].is_string(), "window_id resolved via reverse lookup");
}

/// `/api/v1/self` is auth-gated like every other route.
#[tokio::test]
async fn self_endpoint_requires_auth() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/self?block_id=anything")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// `/api/v1/self` without a block_id is a client error, not a 500.
#[tokio::test]
async fn self_endpoint_missing_block_id_is_400() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/self")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── SPEC_WINDOW_NAME_API_HARDENING_2026_08_08 — phantom ids + status codes ──

fn window_name_request(body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/window/name")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// A well-formed UUID that matches no window must 404, not report
/// success — this used to sail through the guardless reducer arm into a
/// silent persist no-op (spec §2.1, found by live probe).
#[tokio::test]
async fn window_name_phantom_uuid_is_404() {
    let app = test_router();
    let resp = app
        .oneshot(window_name_request(serde_json::json!({
            "window_id": "00000000-dead-beef-0000-000000000000",
            "name": "phantom",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// A malformed (non-UUID) window id is a caller error, not a server fault.
#[tokio::test]
async fn window_name_malformed_id_is_400() {
    let app = test_router();
    let resp = app
        .oneshot(window_name_request(serde_json::json!({
            "window_id": "no-such-window-xyz",
            "name": "ghost",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Happy path: renaming the seeded window succeeds and persists
/// `window:displayname` in wstore. srv_state is hydrated from wstore via
/// the same `bootstrap_state_from_wstore` production runs, so the new
/// reducer existence guard sees the seeded window exactly as it would live.
#[tokio::test]
async fn window_name_renames_seeded_window_and_persists() {
    let state = test_state();
    crate::persist::bootstrap_state_from_wstore(&state.srv_state, &state.wstore).await;
    let wstore = state.wstore.clone();
    let window = wstore
        .get_all::<crate::backend::obj::Window>()
        .unwrap()
        .into_iter()
        .next()
        .expect("seeded window");
    let app = build_router(state);

    let resp = app
        .oneshot(window_name_request(serde_json::json!({
            "window_id": window.oid,
            "name": "renamed-by-test",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["name"], "renamed-by-test");

    let reread = wstore
        .get::<crate::backend::obj::Window>(&window.oid)
        .unwrap()
        .expect("window still exists");
    assert_eq!(
        reread.meta.get("window:displayname").and_then(|v| v.as_str()),
        Some("renamed-by-test"),
        "display name must be persisted in wstore meta"
    );
}

// ── SPEC_864 Phase 2 — UpdateObject routes layout pushes through the reducer ──

/// End-to-end over the HTTP service: a frontend-style full-row layout push
/// must (a) succeed, (b) bump `db_layout.version` exactly once (the legacy
/// path double-wrote: update_raw + the focus/magnify subscriber write),
/// (c) leave the reducer's `TabRecord.rootnode` equal to the persisted
/// rootnode (the Pillar-1 coherence invariant — TabRecord was previously a
/// passive shadow that diverged on every push), and (d) clear
/// `pendingbackendactions` when the push omits it (the frontend's ack path).
#[tokio::test]
async fn update_object_layout_push_single_write_and_coherent_reducer() {
    use agentmux_common::ipc::{Command, Event};
    use crate::backend::obj::{LayoutActionData, LayoutState, Tab};

    let state = test_state();
    let wstore = state.wstore.clone();
    let srv_state = state.srv_state.clone();

    // Seed workspace + tab through the reducer so BOTH reducer state and
    // wstore know the tab (mirrors production bootstrap).
    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }
    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "ws".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab {
            workspace_id: ws_id,
            name: "t".into(),
        },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    let tab = wstore.get::<Tab>(&tab_id).unwrap().unwrap();
    let layout_oid = tab.layoutstate.clone();

    // The push below references block "b-1" directly (a predetermined id
    // the test asserts on) rather than one the reducer would assign via
    // Command::CreateBlock, so it's seeded straight into reducer state.
    // Required since prune_dangling_block_refs (added alongside
    // LayoutSetTree — see
    // docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md)
    // now prunes any pushed leaf whose block_id isn't live, and this test's
    // "b-1" was never otherwise registered.
    {
        let mut s = srv_state.lock().await;
        s.blocks.insert(
            "b-1".to_string(),
            crate::state::BlockRecord { block_id: "b-1".to_string(), tab_id: tab_id.clone() },
        );
    }

    // Seed a pending backend action (as a redock would); the push below
    // omits pendingbackendactions — the ack must clear it.
    {
        let mut layout = wstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
        layout.pendingbackendactions = Some(vec![LayoutActionData {
            actiontype: "insert".into(),
            actionid: "a1".into(),
            blockid: "b-x".into(),
            nodesize: None,
            nodesizefraction: None,
            indexarr: None,
            focused: false,
            magnified: false,
            ephemeral: false,
            targetblockid: String::new(),
            position: String::new(),
        }]);
        wstore.update(&mut layout).unwrap();
    }
    let version_before = wstore
        .get::<LayoutState>(&layout_oid)
        .unwrap()
        .unwrap()
        .version;

    // Frontend-style full-row push (persistToBackend shape).
    let push = serde_json::json!({
        "service": "object",
        "method": "UpdateObject",
        "args": [{
            "otype": "layout",
            "oid": layout_oid,
            "version": version_before,
            "rootnode": {
                "id": "n-root",
                "flexDirection": "row",
                "size": 1,
                "data": { "blockId": "b-1" }
            },
            "focusednodeid": "n-root",
            "leaforder": [{ "nodeid": "n-root", "blockid": "b-1" }]
        }]
    });
    let app = build_router(state);
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(push.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["success"].as_bool().unwrap_or(false),
        "UpdateObject failed: {}",
        json["error"]
    );

    // (b) exactly ONE version bump — the double-write is collapsed.
    let layout = wstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
    assert_eq!(
        layout.version,
        version_before + 1,
        "layout push must produce exactly one db_layout write"
    );
    // Row content matches the push.
    assert_eq!(layout.rootnode.as_ref().unwrap().id, "n-root");
    assert_eq!(layout.focusednodeid, "n-root");
    assert_eq!(layout.leaforder.as_ref().unwrap()[0].blockid, "b-1");
    // (d) omitted pendingbackendactions = ack → cleared.
    assert!(
        layout.pendingbackendactions.is_none(),
        "push without pendingbackendactions must clear the queue (ack)"
    );

    // (c) the coherence invariant: TabRecord.rootnode == db_layout.rootnode.
    let s = srv_state.lock().await;
    let rec = s.tabs.get(&tab_id).expect("reducer knows the tab");
    assert_eq!(
        rec.rootnode, layout.rootnode,
        "TabRecord.rootnode must match db_layout mid-session (no stale shadow)"
    );
    assert_eq!(rec.focused_node_id, "n-root");
}

/// Reagent P1 (#1970 review 3) — the owned-row PARSE-FAILURE fallback must
/// keep the pre-Phase-2 Option-A behavior: legacy wholesale write PLUS the
/// focus/magnify reducer dispatch, so `TabRecord` focus state can't
/// silently diverge on the degenerate branch.
#[tokio::test]
async fn update_object_layout_parse_failure_falls_back_with_focus_dispatch() {
    use agentmux_common::ipc::{Command, Event};
    use crate::backend::obj::Tab;

    let state = test_state();
    let wstore = state.wstore.clone();
    let srv_state = state.srv_state.clone();

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }
    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "ws".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab {
            workspace_id: ws_id,
            name: "t".into(),
        },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let layout_oid = wstore.get::<Tab>(&tab_id).unwrap().unwrap().layoutstate;

    // rootnode.id must be a string — a numeric id fails the typed parse and
    // forces the legacy fallback branch.
    let push = serde_json::json!({
        "service": "object",
        "method": "UpdateObject",
        "args": [{
            "otype": "layout",
            "oid": layout_oid,
            "rootnode": { "id": 12345 },
            "focusednodeid": "n-fallback"
        }]
    });
    let app = build_router(state);
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(push.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["success"].as_bool().unwrap_or(false),
        "fallback push must still succeed: {}",
        json["error"]
    );

    // Row was written wholesale (raw JSON survives even though typed parse failed).
    let raw = wstore
        .get_raw("layout", &layout_oid)
        .unwrap()
        .expect("row present");
    assert_eq!(raw["focusednodeid"], "n-fallback");
    assert_eq!(raw["rootnode"]["id"], 12345);

    // The focus slice still reached the reducer (pre-Phase-2 Option-A behavior).
    let s = srv_state.lock().await;
    let rec = s.tabs.get(&tab_id).expect("reducer knows the tab");
    assert_eq!(
        rec.focused_node_id, "n-fallback",
        "parse-failure fallback must still dispatch focus to the reducer"
    );
}

/// SPEC_864 Phase 3 — the seeders route through the reducer: after
/// `seed_layout_via_reducer` / `setup_torn_off_block_layout`, the reducer's
/// `TabRecord.rootnode` and `db_layout.rootnode` are the same tree (single
/// writer), and the persisted row carries focus + leaforder.
#[tokio::test]
async fn layout_seeders_route_through_reducer_coherently() {
    use agentmux_common::ipc::{Command, Event};
    use crate::backend::obj::{LayoutState, Tab};

    let state = test_state();
    let wstore = state.wstore.clone();
    let srv_state = state.srv_state.clone();

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }
    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "ws".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();

    // ── three-pane seed (the CreateWindow post-bootstrap path) ──
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
    )
    .await;
    let tab1 = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    // Seeded straight into reducer state (predetermined block ids the test
    // asserts on, not reducer-assigned) — required since
    // prune_dangling_block_refs (added alongside LayoutSetTree/the
    // reducer-routed seeders — see
    // docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md)
    // now prunes any seeded leaf whose block_id isn't live.
    for bid in ["b-agent", "b-sysinfo", "b-swarm"] {
        srv_state.lock().await.blocks.insert(
            bid.to_string(),
            crate::state::BlockRecord { block_id: bid.to_string(), tab_id: tab1.clone() },
        );
    }
    let (tree, focused, leaforder) =
        crate::backend::wcore::default_three_pane_tree("b-agent", "b-sysinfo", "b-swarm");
    crate::server::service::seed_layout_via_reducer(
        &state, &tab1, tree, focused, leaforder,
    )
    .await
    .expect("three-pane seed via reducer");

    let layout_oid = wstore.get::<Tab>(&tab1).unwrap().unwrap().layoutstate;
    let row = wstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
    assert_eq!(row.leaforder.as_ref().unwrap().len(), 3);
    assert!(!row.focusednodeid.is_empty(), "focus persisted");
    {
        let s = srv_state.lock().await;
        let rec = s.tabs.get(&tab1).expect("reducer knows tab1");
        assert_eq!(
            rec.rootnode, row.rootnode,
            "TabRecord == db_layout after three-pane seed"
        );
        assert_eq!(rec.focused_node_id, row.focusednodeid);
    }

    // ── single-leaf tear-off seed ──
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab {
            workspace_id: ws_id,
            name: "t2".into(),
        },
    )
    .await;
    let tab2 = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    srv_state.lock().await.blocks.insert(
        "b-moved".to_string(),
        crate::state::BlockRecord { block_id: "b-moved".to_string(), tab_id: tab2.clone() },
    );
    crate::server::service::setup_torn_off_block_layout(&state, &tab2, "b-moved")
        .await
        .expect("tear-off seed via reducer");

    let layout_oid = wstore.get::<Tab>(&tab2).unwrap().unwrap().layoutstate;
    let row = wstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
    let root = row.rootnode.as_ref().expect("single-leaf tree persisted");
    assert_eq!(root.data.as_ref().unwrap().block_id, "b-moved");
    assert_eq!(row.leaforder.as_ref().unwrap()[0].blockid, "b-moved");
    {
        let s = srv_state.lock().await;
        let rec = s.tabs.get(&tab2).expect("reducer knows tab2");
        assert_eq!(
            rec.rootnode, row.rootnode,
            "TabRecord == db_layout after tear-off seed"
        );
    }
}

/// Seeding a tab the reducer doesn't know must fail loudly (Error event),
/// not silently write db_layout — the pre-bootstrap first-launch seed is
/// the only sanctioned store-direct path.
#[tokio::test]
async fn layout_seed_unknown_tab_errors() {
    let state = test_state();
    let (tree, focused, leaforder) =
        crate::backend::wcore::default_three_pane_tree("a", "b", "c");
    let err = crate::server::service::seed_layout_via_reducer(
        &state, "ghost-tab", tree, focused, leaforder,
    )
    .await
    .expect_err("unknown tab must error");
    assert!(err.contains("unknown tab"), "got: {err}");
}

/// SPEC_864 Phase 5, DoD #3 — `TabRecord.rootnode` (reducer) must equal
/// `db_layout.rootnode` (persisted) after EVERY layout-mutating op, not
/// just immediately post-seed. This is the capstone coherence check for
/// the single-writer collapse: chains split → resize → swap → replace →
/// Phase-4 queue-append → frontend-ack (LayoutSetTree) → delete-by-block
/// (twice, ending in a root-orphan clear), asserting coherence after
/// every step. A regression that only manifests a few ops into a real
/// session (e.g. a granular arm's `new_tree` going stale, or the queue's
/// append/replace semantics interacting badly) wouldn't be caught by a
/// single seed-then-check test.
#[tokio::test]
async fn layout_stays_coherent_across_full_mutation_lifecycle() {
    use agentmux_common::ipc::{Command, Event};
    use agentmux_common::{LayoutNode, LayoutNodeData, LayoutClientSlices, ResizeOp, SplitPosition};
    use crate::backend::obj::{LayoutState, Tab};

    let state = test_state();
    let wstore = state.wstore.clone();
    let srv_state = state.srv_state.clone();

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "unexpected reducer error: {:?}",
            events
        );
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }

    async fn assert_coherent(state: &AppState, tab_id: &str, step: &str) {
        let tab = state.wstore.get::<Tab>(tab_id).unwrap().unwrap();
        let db_layout = state
            .wstore
            .get::<LayoutState>(&tab.layoutstate)
            .unwrap()
            .unwrap();
        let s = state.srv_state.lock().await;
        let rec = s.tabs.get(tab_id).expect("reducer knows tab");
        assert_eq!(
            rec.rootnode, db_layout.rootnode,
            "TabRecord.rootnode != db_layout.rootnode after {step}"
        );
    }

    fn leaf(id: &str, block_id: &str) -> LayoutNode {
        LayoutNode {
            id: id.into(),
            data: Some(LayoutNodeData {
                block_id: block_id.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "ws".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(
        &state,
        Command::CreateTab {
            workspace_id: ws_id,
            name: "t".into(),
        },
    )
    .await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // Every block id this test's layout tree ever references, seeded
    // straight into reducer state up front (predetermined ids the test
    // asserts on throughout, not reducer-assigned) — required since
    // prune_dangling_block_refs (added alongside every reducer-routed
    // layout-tree write in this lifecycle — see
    // docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md)
    // now prunes any leaf whose block_id isn't live.
    for bid in ["b1", "b2", "b3", "b-fe"] {
        srv_state.lock().await.blocks.insert(
            bid.to_string(),
            crate::state::BlockRecord { block_id: bid.to_string(), tab_id: tab_id.clone() },
        );
    }

    // Seed: single leaf via the reducer-routed tear-off-style seeder.
    crate::server::service::setup_torn_off_block_layout(&state, &tab_id, "b1")
        .await
        .unwrap();
    assert_coherent(&state, &tab_id, "seed").await;
    let root_id = {
        let s = srv_state.lock().await;
        s.tabs[&tab_id].rootnode.as_ref().unwrap().id.clone()
    };

    // Split: wrap root in a new group, adding a second leaf.
    dispatch_apply(
        &state,
        Command::LayoutSplitVertical {
            tab_id: tab_id.clone(),
            target_id: root_id.clone(),
            new_node: leaf("n2", "b2"),
            position: SplitPosition::After,
            focus_after: true,
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "split").await;

    // Resize both leaves.
    dispatch_apply(
        &state,
        Command::LayoutResizeNodes {
            tab_id: tab_id.clone(),
            ops: vec![
                ResizeOp { node_id: root_id.clone(), size: 3.0 },
                ResizeOp { node_id: "n2".into(), size: 7.0 },
            ],
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "resize").await;

    // Swap the two leaves.
    dispatch_apply(
        &state,
        Command::LayoutSwapNodes {
            tab_id: tab_id.clone(),
            node1_id: root_id.clone(),
            node2_id: "n2".into(),
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "swap").await;

    // Replace the second leaf with a brand-new one (b2 -> b3).
    dispatch_apply(
        &state,
        Command::LayoutReplaceNode {
            tab_id: tab_id.clone(),
            target_id: "n2".into(),
            new_node: leaf("n3", "b3"),
            focus_after: false,
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "replace").await;

    // Phase 4: queue-append a backend action (does not touch the tree).
    let action = serde_json::json!([{
        "actiontype": "insert",
        "actionid": "a-fe",
        "blockid": "b-fe",
        "nodesize": null,
        "nodesizefraction": null,
        "indexarr": null,
        "focused": true,
        "magnified": false,
        "ephemeral": false,
        "targetblockid": "",
        "position": "",
    }]);
    crate::server::service::queue_layout_actions_via_reducer(
        &state,
        &tab_id,
        serde_json::from_value(action).unwrap(),
    )
    .await
    .unwrap();
    assert_coherent(&state, &tab_id, "queue-append").await;
    {
        let tab = wstore.get::<Tab>(&tab_id).unwrap().unwrap();
        let layout = wstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
        assert_eq!(
            layout.pendingbackendactions.as_ref().map(|a| a.len()),
            Some(1),
            "queued action must persist before the frontend acks it"
        );
    }

    // Frontend ack: push the tree with the new block inserted + an empty
    // pending-actions slice (REPLACE-clear, matching the real ack path).
    let acked_tree = LayoutNode {
        id: "root-acked".into(),
        children: vec![leaf(&root_id, "b1"), leaf("n3", "b3"), leaf("n4", "b-fe")],
        ..Default::default()
    };
    dispatch_apply(
        &state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(acked_tree),
            correlation_id: String::new(),
            slices: Some(LayoutClientSlices {
                leaforder: None,
                focused_node_id: "n4".into(),
                magnified_node_id: String::new(),
                pending_backend_actions: None,
            }),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "frontend-ack").await;
    {
        let tab = wstore.get::<Tab>(&tab_id).unwrap().unwrap();
        let layout = wstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
        assert!(
            layout.pendingbackendactions.is_none(),
            "ack slice must clear the queue"
        );
    }

    // Delete the newly-acked block by id — tree survives (2 leaves left).
    dispatch_apply(
        &state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.clone(),
            block_id: "b-fe".into(),
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "delete-by-block (b-fe)").await;

    // Delete b1 — one leaf left.
    dispatch_apply(
        &state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.clone(),
            block_id: "b1".into(),
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "delete-by-block (b1)").await;

    // Delete the last block — root-orphan clear (rootnode -> None on both sides).
    dispatch_apply(
        &state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.clone(),
            block_id: "b3".into(),
            correlation_id: String::new(),
        },
    )
    .await;
    assert_coherent(&state, &tab_id, "delete-by-block (b3, root orphan)").await;
    let tab = wstore.get::<Tab>(&tab_id).unwrap().unwrap();
    let layout = wstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
    assert!(layout.rootnode.is_none(), "tree must be fully empty at the end");
}

#[test]
fn clean_name_trims_clamps_and_rejects_empty() {
    use super::clean_name;
    assert_eq!(clean_name("  hi  ", 64).as_deref(), Some("hi"));
    assert_eq!(clean_name("   ", 64), None);
    assert_eq!(clean_name("", 64), None);
    // clamps to `max` chars (counted by char, not byte)
    let long = "x".repeat(200);
    assert_eq!(clean_name(&long, 64).unwrap().chars().count(), 64);
}

/// `GET /api/v1/layout` returns the seeded window → workspace → tab → pane
/// tree. Read-only, hermetic.
#[tokio::test]
async fn layout_endpoint_returns_seeded_tree() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/layout")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let windows = json["windows"].as_array().expect("windows array");
    assert_eq!(windows.len(), 1, "one seeded window");
    let win = &windows[0];
    assert_eq!(win["workspace_name"], "Starter workspace");
    let tabs = win["tabs"].as_array().expect("tabs array");
    assert!(!tabs.is_empty(), "seeded tab present");
    // The seeded tab carries the default agent/sysinfo/swarm panes.
    let panes = tabs[0]["panes"].as_array().expect("panes array");
    assert!(panes.iter().any(|p| p["view"] == "agent"), "agent pane present");
}

/// Introspection list endpoints are auth-gated and return their wrapper keys.
#[tokio::test]
async fn introspection_lists_return_collections() {
    for (path, key) in [
        ("/api/v1/windows", "windows"),
        ("/api/v1/workspaces", "workspaces"),
        ("/api/v1/tabs", "tabs"),
    ] {
        let app = test_router();
        let req = Request::builder()
            .method(Method::GET)
            .uri(path)
            .header("X-AuthKey", "test-secret-key")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json[key].is_array(), "{path} should return a {key} array");
        assert!(!json[key].as_array().unwrap().is_empty(), "{path} non-empty");
    }
}

/// `resolve_agent_definition_id` maps the S1-authenticated agent's
/// `AGENTMUX_AGENT_ID` — the persisted `slug` column, NOT `instance_name`
/// (see `instance_get_by_slug`'s doc comment,
/// SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md follow-up — reagentx
/// P1 on PR #2428 caught this same slug/display-name conflation baked
/// into this test itself: it used to pass the display name and call it
/// "slug") — to the definition id that keys `db_agent_identity_links`,
/// passes real definition ids through unchanged, and errors on unknown
/// ids. Guards the #1624 PR-C fix for
/// identity.account.upsert/self.accounts/self.unlink writing the slug
/// into the link table (FK failure on per-channel stores; silent
/// resolver-invisible rows on the shared store).
#[tokio::test]
async fn resolve_agent_definition_id_maps_slug_and_passes_through_def_id() {
    let state = test_state();

    // `slug` and `name`/`instance_name` deliberately differ (a stable
    // routing slug vs. a renameable display name) so this test can't
    // accidentally pass by conflating the two, the way it used to.
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "def-test-1", // caller-assigned; agent_def_insert stores it as-is
        "slug": "testslug",
        "name": "Test Slug Display",
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "created_at": 1,
    }))
    .expect("definition fixture");
    state.wstore.agent_def_insert(&mut def).expect("insert definition");

    let inst: crate::backend::storage::AgentInstance = serde_json::from_value(serde_json::json!({
        "id": "inst-1",
        "definition_id": def.id,
        "status": "running",
        "started_at": 1,
        "created_at": 1,
        "identity_id": "",
        "memory_id": "",
        "instance_name": "Test Slug Display",
        "working_directory": "",
    }))
    .expect("instance fixture");
    state.wstore.instance_create(&inst).expect("create instance");

    // Slug (AGENTMUX_AGENT_ID, what a real MCP-tool caller actually sends)
    // → definition id. NOT the display name — that's a different,
    // deliberately-mismatched value in this fixture.
    let resolved = super::app_api::resolve_agent_definition_id(&state, "testslug")
        .expect("slug resolves");
    assert_eq!(resolved, def.id);

    // A definition id passes through unchanged (internal non-S1 callers).
    let passthrough = super::app_api::resolve_agent_definition_id(&state, &def.id)
        .expect("definition id passes through");
    assert_eq!(passthrough, def.id);

    // The literal display name must NOT resolve — it lives in a
    // different namespace than the slug (see instance_get_by_slug's doc
    // comment for why merging the two was unsafe).
    assert!(
        super::app_api::resolve_agent_definition_id(&state, "Test Slug Display").is_err(),
        "the display name is not the slug and must not resolve"
    );

    // Unknown ids error instead of silently writing bogus link rows.
    assert!(super::app_api::resolve_agent_definition_id(&state, "no-such-agent").is_err());
}

// ---- muxspect (docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md) ----

/// `GET /api/v1/muxspect/list` returns the full `ProcessStatus` collection
/// (not just `block_ids`, unlike `agent.tracked-blocks`) and is auth-gated
/// like every other `/api/v1/*` route.
#[tokio::test]
async fn muxspect_list_returns_full_process_status_collection() {
    let state = test_state();
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/muxspect/list")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["blocks"].is_array(), "response has a blocks array");
    // Every row must carry the complete `is_agent` classification, not just
    // the raw `is_agent_pane` flag a naive consumer might reimplement
    // incorrectly for subprocess/persistent/acp controllers (codex P2 on
    // PR #2380).
    for block in json["blocks"].as_array().unwrap() {
        assert!(block.get("is_agent").is_some(), "row is missing is_agent: {block}");
    }
}

/// `GET /api/v1/muxspect/list` rejects a request with no/wrong auth key —
/// same auth_middleware every other route already goes through.
#[tokio::test]
async fn muxspect_list_requires_auth() {
    let state = test_state();
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/muxspect/list")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// `GET /api/v1/muxspect/describe` composes ProcessBroker status +
/// controller status + process tree into one response — the "describe
/// everything about block X" query
/// REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md §5.4 named as
/// missing. For a block with no controller, this must still succeed and
/// report `Lifecycle::Unknown` rather than erroring — an unknown block is a
/// legitimate, common answer (e.g. a stale/closed pane), not a failure.
#[tokio::test]
async fn muxspect_describe_composes_status_for_an_unknown_block() {
    let state = test_state();
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/muxspect/describe?block_id=no-such-block")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["block_id"], "no-such-block");
    assert_eq!(json["process_status"]["lifecycle"], "unknown");
    assert_eq!(json["is_agent"], false);
    assert_eq!(json["controller_status"], serde_json::Value::Null);
    // `process_status.controller_status` must agree with the top-level
    // field — both must come from the SAME snapshot, not two independent
    // reads that could observe different controller states (codex P2 on
    // PR #2380).
    assert_eq!(json["process_status"]["controller_status"], json["controller_status"]);
    assert_eq!(json["tracking_confidence"], "none");
    assert!(json["processes"].as_array().unwrap().is_empty());
}

/// Missing `block_id` is a client error, not a panic or a silent empty
/// success — mirrors `/api/v1/self`'s identical guard.
#[tokio::test]
async fn muxspect_describe_requires_block_id() {
    let state = test_state();
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/muxspect/describe")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// `describe` on a block with NO controller (the exact "diagnostically
/// empty" state `muxspect_describe_composes_status_for_an_unknown_block`
/// pins above) must now also surface WHY, when the reason is knowable: a
/// persisted `error_during_execution` frame as the last line of the
/// block's own output — the durable signal `agent_handlers/input.rs`
/// already writes for exactly this case (identity-gate spawn refusal,
/// container-spawn failure, etc.). See
/// `docs/reports/REPORT_MUXSPECT_SPAWN_REFUSAL_DIAGNOSIS_EXTENSION_2026_08_03.md`
/// — this is the change that closes the gap the report was written about.
#[tokio::test]
async fn muxspect_describe_surfaces_last_error_for_a_wedged_block() {
    use crate::backend::storage::filestore::{FileMeta, FileOpts};

    let state = test_state();
    let filestore = state.filestore.clone();
    filestore
        .make_file("wedged-block", "output", FileMeta::new(), FileOpts::default())
        .unwrap();
    let error_line = serde_json::json!({
        "type": "result",
        "is_error": true,
        "subtype": "error_during_execution",
        "error": {"message": "[AgentMux] no credentials for claude: bind an account for this provider in the Armory."}
    })
    .to_string();
    filestore
        .write_file("wedged-block", "output", format!("{error_line}\n").as_bytes())
        .unwrap();

    let app = build_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/muxspect/describe?block_id=wedged-block")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Still correctly reports no live controller — this block never had one.
    assert_eq!(json["process_status"]["lifecycle"], "unknown");
    assert_eq!(json["controller_status"], serde_json::Value::Null);
    // ...but is no longer diagnostically empty about why.
    assert_eq!(
        json["last_error"]["message"],
        "[AgentMux] no credentials for claude: bind an account for this provider in the Armory."
    );
    assert_eq!(json["last_error"]["source"], "identity");
    assert!(json["last_error"]["written_ms"].as_u64().unwrap() > 0);
}

/// A block with no `output` file at all (never opened, or genuinely
/// healthy) must report `last_error: null`, not error or fabricate one —
/// this is the common case and must stay silent.
#[tokio::test]
async fn muxspect_describe_last_error_is_null_for_a_block_with_no_output() {
    let state = test_state();
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/muxspect/describe?block_id=never-opened")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["last_error"], serde_json::Value::Null);
}

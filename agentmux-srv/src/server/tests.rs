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

    AppState {
        auth_key: "test-secret-key".to_string(),
        version: "0.28.20".to_string(),
        app_path: String::new(),
        wstore: wstore.clone(),
        shared_store: None,
        id_store: wstore,
        filestore,
        global_transcript_store: None,
        event_bus: event_bus.clone(),
        broker,
        reactive_handler,
        poller,
        config_watcher,
        messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
        http_client: reqwest::Client::new(),
        local_web_url: String::new(),
        subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone())),
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
        ),
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

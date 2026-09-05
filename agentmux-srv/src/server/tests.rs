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
    // This one store backs wstore/id_store/identity_store below, so it has to
    // carry BOTH schemas. Without the identity schema, any handler touching a
    // table that lives only in the global identity store (`db_work_queue`)
    // failed with a bare "no such table" 500 in tests while being perfectly
    // correct in production. Additive and idempotent — every statement in that
    // schema is CREATE ... IF NOT EXISTS.
    wstore.apply_identity_schema_for_tests().unwrap();
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
        lan_key: "test-lan-key".to_string(),
        boot_id: std::sync::Arc::from("test-boot"),
        version: "0.28.20".to_string(),
        app_path: String::new(),
        wstore: wstore.clone(),
        shared_store: None,
        id_store: wstore.clone(),
        // Aliased to `wstore` on purpose — a lot of existing setup code seeds
        // through one store and reads through another. See the
        // `apply_identity_schema_for_tests` call above for why that single
        // store now carries the identity schema too.
        identity_store: wstore.clone(),
        filestore,
        global_transcript_store: None,
        event_bus: event_bus.clone(),
        broker: broker.clone(),
        reactive_handler,
        poller,
        config_watcher,
        messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
        http_client: reqwest::Client::new(),
        host_ipc: Arc::new(tokio::sync::Mutex::new(None)),
        host_reg_secret: Some("test-host-reg-secret".to_string()),
        local_web_url: String::new(),
        subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone(), wstore.clone(), wstore.clone(), wstore.clone())),
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
        pending_background_pids: Arc::new(crate::backend::pending_background_pids::PendingBackgroundPids::new()),
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

// ── UI automation (SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18) ───

/// Minimal fake `/agentmux/browser/*` responder — returns `response_body`
/// verbatim for any request whose `Authorization: Bearer <token>` matches
/// `expect_bearer`, otherwise a canned unauthorized error. Enough to drive
/// the srv-side proxy handlers without a real CEF host.
async fn spawn_fake_browser_api(response_body: &'static str, expect_bearer: &'static str) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let expected_auth = format!("Bearer {expect_bearer}");
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let Ok(n) = stream.read(&mut buf).await else { return };
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.contains(&expected_auth) {
                    response_body
                } else {
                    r#"{"ok":false,"error":"unauthorized"}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn host_ipc_register_sets_state() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"host_ipc","method":"Register","args":[9999,"tok-123","test-host-reg-secret"]}"#,
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

// Regression test for a P0 flagged in review (reagent, 2026-08-19, PR #2662,
// second re-review round): the conflicting-registration liveness check
// above only helps once `state.host_ipc` already holds something to probe.
// At every srv startup — and the window after any `restart_backend` before
// the host re-registers — `state.host_ipc` is `None`, so there was nothing
// to compare against and ANY caller won the race outright. An attacker
// agent could register first, then stand up its own always-200 `/health`
// responder to permanently defeat the liveness check on every future
// legitimate registration too. Fixed with `AGENTMUX_HOST_REG_SECRET`, a
// credential never given to agents — this test proves a caller without it
// cannot win even in the `None` state the earlier fix couldn't cover.
#[tokio::test]
async fn host_ipc_register_rejects_without_the_matching_secret_even_when_state_is_none() {
    let app = test_router(); // state.host_ipc starts None, host_reg_secret = "test-host-reg-secret"
    let no_secret = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"host_ipc","method":"Register","args":[9999,"attacker-token"]}"#,
        ))
        .unwrap();
    let resp = app.clone().oneshot(no_secret).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["success"].as_bool().unwrap_or(false),
        "a caller with no secret at all must be rejected, even against an empty state.host_ipc"
    );

    let wrong_secret = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"host_ipc","method":"Register","args":[9999,"attacker-token","not-the-real-secret"]}"#,
        ))
        .unwrap();
    let resp = app.oneshot(wrong_secret).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["success"].as_bool().unwrap_or(false),
        "a caller with the wrong secret must be rejected, even against an empty state.host_ipc"
    );
}

// Companion to the above: if srv itself was never given a secret to check
// against (a config/spawn bug rather than an attack), it must fail closed
// — reject every registration — rather than silently accepting the first
// caller as trusted.
#[tokio::test]
async fn host_ipc_register_rejects_everything_when_srv_has_no_secret_configured() {
    let mut state = test_state();
    state.host_reg_secret = None;
    let app = build_router(state);

    let req = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"host_ipc","method":"Register","args":[9999,"tok-123","anything"]}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["success"].as_bool().unwrap_or(false),
        "srv with no host_reg_secret configured must reject every registration attempt"
    );
}

// Regression test for a P0 flagged in review (reagent, 2026-08-19, PR #2662):
// `host_ipc.Register` unconditionally overwrote `state.host_ipc` with
// whatever the caller supplied. Every route on `/agentmux/service` shares
// ONE instance-wide `X-AuthKey` — including agent-spawned processes, which
// can read that key from their own environment via a shell command — so
// any agent could re-register a fake port/token and silently redirect
// every other agent's UIScreenshot/UIClick/UIQuery to an attacker-controlled
// endpoint for the rest of the session. Fixed: a conflicting re-registration
// is rejected UNLESS the currently-registered host is unreachable at its
// own /health route (see host_ipc.rs's handle_register for why an
// unconditional reject was ALSO wrong — it broke real crash-restart
// recovery, reagent P1 re-review same PR). This test uses a live fake
// server for the first registration so the liveness probe genuinely finds
// it alive; `host_ipc_register_recovers_when_old_host_is_unreachable`
// below covers the opposite (dead old host) case.
#[tokio::test]
async fn host_ipc_register_rejects_a_conflicting_second_registration_while_old_host_is_alive() {
    let state = test_state();
    let alive_port = spawn_fake_browser_api(r#"{"ok":true,"data":{}}"#, "irrelevant").await;
    let app = build_router(state);

    let first = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(format!(
            r#"{{"service":"host_ipc","method":"Register","args":[{alive_port},"real-host-token","test-host-reg-secret"]}}"#
        )))
        .unwrap();
    let resp = app.clone().oneshot(first).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["success"].as_bool().unwrap(), "first registration should succeed");

    // A spoofed re-registration with DIFFERENT credentials, while the real
    // host (alive_port) is STILL alive and responding — the attack reagent
    // flagged. Carries the correct secret (an agent could never actually
    // supply this — see the secret-gate tests above; this test isolates the
    // liveness-conflict check specifically) so it reaches that check and is
    // rejected there, not merely at the secret gate.
    let spoof = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"host_ipc","method":"Register","args":[6666,"attacker-token","test-host-reg-secret"]}"#,
        ))
        .unwrap();
    let resp = app.clone().oneshot(spoof).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["success"].as_bool().unwrap_or(false),
        "a conflicting re-registration must be rejected while the old host is still alive"
    );

    // An IDENTICAL re-registration (e.g. a legitimate retry) is still a
    // harmless no-op, not an error — no liveness probe needed for this case.
    let retry = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(format!(
            r#"{{"service":"host_ipc","method":"Register","args":[{alive_port},"real-host-token","test-host-reg-secret"]}}"#
        )))
        .unwrap();
    let resp = app.oneshot(retry).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["success"].as_bool().unwrap(),
        "an identical re-registration should be accepted as a no-op"
    );
}

// The crash-restart recovery path reagent's re-review specifically called
// out: the OLD registration points at a host that's no longer reachable
// (crashed; the launcher relaunched just the host per its Job Object
// sibling design, with a fresh ipc_port/ipc_token). The new registration
// must be accepted, not permanently rejected.
#[tokio::test]
async fn host_ipc_register_recovers_when_old_host_is_unreachable() {
    // A port nothing is listening on: bind then immediately drop the
    // listener, so the OS won't hand this port to anything else for the
    // duration of this fast test, and a connection attempt reliably fails
    // fast (loopback connection-refused) rather than hanging.
    let dead_port = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };

    let state = test_state();
    *state.host_ipc.lock().await = Some(HostIpc {
        port: dead_port,
        token: "stale-token".to_string(),
    });
    let app = build_router(state);

    let recovery = Request::builder()
        .uri("/agentmux/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"host_ipc","method":"Register","args":[7777,"relaunched-host-token","test-host-reg-secret"]}"#,
        ))
        .unwrap();
    let resp = app.oneshot(recovery).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["success"].as_bool().unwrap(),
        "a re-registration must be accepted once the previously-registered host is unreachable"
    );
}

/// Mint a jekt key for a fresh, uniquely-named test agent, register it
/// with the global `ReactiveHandler` under a fresh unique block_id (the
/// `block_id_hint` is just a readable label, uuid-suffixed for real
/// uniqueness), and return a valid `{agent_id, ts_secs, sig}` JSON
/// fragment for it. Both agent_id AND block_id must be unique per call
/// (not just agent_id) — `ReactiveHandler` is a process-global singleton
/// shared by every test in this binary, and two tests registering
/// different agents under the SAME literal block_id raced each other
/// (confirmed flaky under `cargo test`'s default parallelism, though
/// deterministic in isolation) until this was unique per call too.
fn signed_ui_auth(state: &AppState, block_id_hint: &str) -> (String, serde_json::Value) {
    let unique = uuid::Uuid::new_v4();
    let agent_id = format!("test-agent-{unique}");
    let block_id = format!("{block_id_hint}-{unique}");
    let key = state.wstore.agent_jekt_key_ensure(&agent_id).unwrap();
    crate::backend::reactive::handler::get_global_handler()
        .register_agent(&agent_id, &block_id, None)
        .unwrap();
    let ts_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let sig = agentmux_common::jekt_sign::sign_jekt(
        &key,
        "ui-automation-identity",
        &agent_id,
        "__srv__",
        ts_secs,
        "",
    );
    (
        agent_id.clone(),
        serde_json::json!({ "agent_id": agent_id, "ts_secs": ts_secs, "sig": sig }),
    )
}

#[tokio::test]
async fn ui_click_requires_host_registration_first() {
    let state = test_state();
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["selector"] = serde_json::json!("button");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/click")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn ui_click_rejects_an_unsigned_request() {
    let app = test_router();
    let req = Request::builder()
        .uri("/api/v1/ui/click")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"agent_id":"nobody","ts_secs":9999999999,"sig":"forged","selector":"button"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// The exact attack this fix closes (reagent + Codex, PR #2662, 2026-08-19):
// an agent that reads the shared X-AuthKey from its own environment claims
// to BE a different agent (whose pane it wants to read/click), but signs
// with its OWN key (it can never have another agent's key). Must be
// rejected — `agent_a`'s valid signature over `agent_a`'s own identity
// does not verify against `agent_b`'s claimed identity, regardless of the
// host being registered and ready to serve a legitimate request.
#[tokio::test]
async fn ui_click_rejects_a_forged_agent_identity() {
    let state = test_state();
    let port = spawn_fake_browser_api(r#"{"ok":true,"data":{}}"#, "tok-abc").await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });

    // Victim: a real agent with its own key and pane.
    let (victim_agent_id, _victim_auth) = signed_ui_auth(&state, "victim-block");

    // Attacker: mints its OWN key/registration, then signs with ITS OWN
    // key but substitutes the VICTIM's agent_id into the request — exactly
    // what a bypassing agent would try after reading the victim's agent_id
    // from e.g. Layout()/DiscoverAgents().
    let (_attacker_agent_id, attacker_auth) = signed_ui_auth(&state, "attacker-block");
    let mut forged = attacker_auth;
    forged["agent_id"] = serde_json::json!(victim_agent_id);
    forged["selector"] = serde_json::json!("button");

    let app = build_router(state);
    let req = Request::builder()
        .uri("/api/v1/ui/click")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(forged.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a signature valid for the attacker's own identity must not verify for a different claimed agent_id"
    );
}

#[tokio::test]
async fn ui_click_proxies_to_host_after_registration() {
    let state = test_state();
    let port = spawn_fake_browser_api(r#"{"ok":true,"data":{}}"#, "tok-abc").await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["selector"] = serde_json::json!("button");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/click")
        .method("POST")
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
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn ui_click_surfaces_a_host_side_error() {
    let state = test_state();
    let port = spawn_fake_browser_api(
        r#"{"ok":false,"error":"selector \"button\" matched no element"}"#,
        "tok-abc",
    )
    .await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["selector"] = serde_json::json!("button");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/click")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert!(json["error"].as_str().unwrap().contains("matched no element"));
}

#[tokio::test]
async fn ui_query_returns_host_matches() {
    let state = test_state();
    let port = spawn_fake_browser_api(
        r#"{"ok":true,"data":{"matches":[{"selector":"body > button:nth-of-type(1)","tag":"button","text":"Sign in","attrs":{},"rect":{"x":0,"y":0,"width":10,"height":10},"focused":false}]}}"#,
        "tok-abc",
    )
    .await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["selector"] = serde_json::json!("button");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/query")
        .method("POST")
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
    assert_eq!(json["data"]["matches"][0]["text"], "Sign in");
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

/// reagentx P1 on PR #2557: `SupervisorAction::Nudge` delivers a fixed,
/// server-owned message (`handler::NUDGE_MESSAGE`) — it no longer accepts
/// (or requires) caller-supplied text at all. A `message` field in the
/// request body, if a caller sends one out of habit, must be silently
/// ignored (serde's default unknown-field behavior), not error and not
/// influence what's delivered — this regression-guards against the
/// free-form-text contract reagentx flagged ever coming back.
#[tokio::test]
async fn reactive_supervisor_decision_nudge_ignores_a_stray_message_field() {
    let app = test_router();
    let body = serde_json::json!({
        "target_agent": "some-agent",
        "action": "nudge",
        "message": "do something completely different",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // "some-agent" has no AgentDefinition in this hermetic store, so the
    // entitlement gate refuses it — same as any other not-opted-in target.
    // The point of this test is that a stray `message` field didn't change
    // that outcome (e.g. by being misparsed into some other code path).
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
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

/// reagentx P1 on PR #2557: a Nudge must not deliver unless the target
/// has actually opted in via `auto_continue_enabled`. An agent with no
/// matching `AgentDefinition` at all (never opted in — the default) must
/// be refused with 403, not silently attempted.
#[tokio::test]
async fn reactive_supervisor_decision_nudge_rejected_when_not_opted_in() {
    let app = test_router();
    let body = serde_json::json!({
        "target_agent": "supervisor-nudge-test-not-opted-in",
        "action": "nudge",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["error"].as_str().unwrap().contains("auto_continue_enabled"));
}

/// The same gate must actively check the flag's value, not just presence
/// of a definition row — `auto_continue_enabled: 0` (the default) is
/// still a rejection.
#[tokio::test]
async fn reactive_supervisor_decision_nudge_rejected_when_opted_out() {
    let state = test_state();
    // `target_agent` is what every delivery path actually keys on:
    // AGENTMUX_AGENT_ID, which agent_open.rs sets to the stable `slug`, NOT
    // the renameable `name` (reagentx P0, round 3). `name` is deliberately
    // a different string here so this test also guards against the gate
    // matching on `name` again.
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "def-supervisor-nudge-opted-out",
        "slug": "supervisor-nudge-test-opted-out",
        "name": "Opted-Out Display Name",
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "created_at": 1,
        "auto_continue_enabled": 0,
    }))
    .expect("definition fixture");
    state.wstore.agent_def_insert(&mut def).expect("insert definition");

    let app = build_router(state);
    let body = serde_json::json!({
        "target_agent": "supervisor-nudge-test-opted-out",
        "action": "nudge",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// An opted-in target's Nudge must pass the entitlement gate (i.e. NOT get
/// refused with 403) — whatever happens next is ordinary delivery
/// machinery (in this hermetic test harness, delivery itself fails because
/// no input sender is wired into the shared global reactive handler, which
/// is a test-environment gap unrelated to the gate this test targets).
#[tokio::test]
async fn reactive_supervisor_decision_nudge_passes_gate_when_opted_in() {
    let state = test_state();
    // Same slug/name distinction as the opted-out test above — `name` is
    // deliberately not what `target_agent` matches.
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "def-supervisor-nudge-opted-in",
        "slug": "supervisor-nudge-test-opted-in",
        "name": "Opted-In Display Name",
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "created_at": 1,
        "auto_continue_enabled": 1,
    }))
    .expect("definition fixture");
    state.wstore.agent_def_insert(&mut def).expect("insert definition");

    let app = build_router(state);
    let body = serde_json::json!({
        "target_agent": "supervisor-nudge-test-opted-in",
        "action": "nudge",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "an opted-in target must not be blocked by the entitlement gate"
    );
}

/// reagentx P0, round 3: the exact cross-namespace collision the entitlement
/// gate must not fall into. Agent A's own SLUG is unrelated, but its
/// display NAME happens to equal Agent B's slug; Agent A is opted OUT.
/// Agent B is opted IN. A nudge for `target_agent = "collision"` (which is
/// how every delivery path — and thus this gate — must resolve it: as
/// Agent B's slug) must pass the gate, not get wrongly authorized OR
/// wrongly rejected off Agent A's unrelated definition via a name match.
/// Mirrors `agents.rs`'s own
/// `instance_get_by_name_and_by_slug_never_cross_the_others_namespace`.
#[tokio::test]
async fn reactive_supervisor_decision_nudge_slug_match_does_not_cross_into_a_colliding_display_name() {
    let state = test_state();
    let mut agent_a: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "def-collision-agent-a",
        "slug": "agent-a-unrelated-slug",
        "name": "collision",
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "created_at": 1,
        "auto_continue_enabled": 0,
    }))
    .expect("definition fixture");
    state.wstore.agent_def_insert(&mut agent_a).expect("insert agent a");

    let mut agent_b: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "def-collision-agent-b",
        "slug": "collision",
        "name": "Agent B Unrelated Display Name",
        "icon": "robot",
        "provider": "claude",
        "description": "test agent",
        "created_at": 1,
        "auto_continue_enabled": 1,
    }))
    .expect("definition fixture");
    state.wstore.agent_def_insert(&mut agent_b).expect("insert agent b");

    let app = build_router(state);
    let body = serde_json::json!({
        "target_agent": "collision",
        "action": "nudge",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/agentmux/reactive/supervisor-decision")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "must resolve to Agent B (slug match, opted in), not Agent A (name match, opted out)"
    );
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

// SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md LAN P0-1 — the scoped
// `lan_key` (broadcast via mDNS/UDP for LAN peer discovery) must be
// accepted by the two LAN-forwarding routes but rejected everywhere else,
// and the full `auth_key` must keep working on those same two routes
// (the normal, non-LAN case — e.g. agentmux-mcp's SendMessage tool).

#[tokio::test]
async fn lan_key_is_accepted_on_reactive_agent_lookup() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/agentmux/reactive/agent?id=nonexistent")
        .header("X-AuthKey", "test-lan-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // 404 (agent not found), not 401 — proves the lan_key cleared auth and
    // the request reached the handler.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn full_auth_key_still_works_on_reactive_agent_lookup() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/agentmux/reactive/agent?id=nonexistent")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn lan_key_is_accepted_on_reactive_inject() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/agentmux/reactive/inject")
        .header("X-AuthKey", "test-lan-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"target_agent": "nonexistent", "message": "hi"}).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // The route itself always returns 200 with a success:false body for an
    // unknown target (see handle_reactive_inject) — the point here is just
    // that it's not 401.
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn garbage_key_is_rejected_on_reactive_inject() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/agentmux/reactive/inject")
        .header("X-AuthKey", "not-a-real-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({"target_agent": "nonexistent", "message": "hi"}).to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// The whole point of LAN P0-1: a captured `lan_key` must NOT grant access
/// to the general API surface, only the two LAN-forwarding routes.
#[tokio::test]
async fn lan_key_is_rejected_on_the_general_service_route() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/self?block_id=anything")
        .header("X-AuthKey", "test-lan-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn lan_key_is_rejected_on_other_reactive_routes() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::GET)
        .uri("/agentmux/reactive/agents")
        .header("X-AuthKey", "test-lan-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
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

    // ── four-pane seed (the CreateWindow post-bootstrap path) ──
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
    for bid in ["b-agent", "b-swarm", "b-armory", "b-sysinfo"] {
        srv_state.lock().await.blocks.insert(
            bid.to_string(),
            crate::state::BlockRecord { block_id: bid.to_string(), tab_id: tab1.clone() },
        );
    }
    let (tree, focused, leaforder) =
        crate::backend::wcore::default_four_pane_tree("b-agent", "b-swarm", "b-armory", "b-sysinfo");
    crate::server::service::seed_layout_via_reducer(
        &state, &tab1, tree, focused, leaforder, String::new(),
    )
    .await
    .expect("four-pane seed via reducer");

    let layout_oid = wstore.get::<Tab>(&tab1).unwrap().unwrap().layoutstate;
    let row = wstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
    assert_eq!(row.leaforder.as_ref().unwrap().len(), 4);
    assert!(!row.focusednodeid.is_empty(), "focus persisted");
    {
        let s = srv_state.lock().await;
        let rec = s.tabs.get(&tab1).expect("reducer knows tab1");
        assert_eq!(
            rec.rootnode, row.rootnode,
            "TabRecord == db_layout after four-pane seed"
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
        crate::backend::wcore::default_four_pane_tree("a", "b", "c", "d");
    let err = crate::server::service::seed_layout_via_reducer(
        &state, "ghost-tab", tree, focused, leaforder, String::new(),
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

// ---------------------------------------------------------------------------
// Fleet control — docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md
// ---------------------------------------------------------------------------

mod fleet_tests {
    use super::*;
    use crate::backend::rpc_types::{
        FleetActionResult, FleetGroup, FleetGroupListResult, StagePlanInput,
        COMMAND_FLEET_GROUP_CREATE, COMMAND_FLEET_GROUP_DELETE, COMMAND_FLEET_GROUP_LIST,
        COMMAND_FLEET_GROUP_UPDATE,
    };
    use crate::server::app_api::fleet::{fleet_broadcast_impl, fleet_bulk_stop_impl};

    #[tokio::test]
    async fn broadcast_reports_a_failure_for_a_block_with_no_registered_agent() {
        let state = test_state();
        let unique = uuid::Uuid::new_v4();
        let unregistered_block = format!("no-such-block-{unique}");

        let result = fleet_broadcast_impl(
            &state,
            vec![unregistered_block.clone()],
            "hello fleet".to_string(),
            None,
        ).await;

        assert!(result.succeeded.is_empty());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].id, unregistered_block);
        assert!(
            result.failed[0].error.contains("no registered agent"),
            "expected a clear resolution error, got: {}",
            result.failed[0].error
        );
    }

    // A registered block must resolve past the "no registered agent" check —
    // whatever inject_message itself then does (it has no real terminal to
    // deliver into under test) is a separate concern already covered by
    // reactive::handler's own test suite. This test's job is narrower:
    // prove block_id -> agent_id resolution actually ran for a real
    // registration, not the generic "unregistered" failure path above.
    #[tokio::test]
    async fn broadcast_resolves_a_registered_block_past_the_unregistered_check() {
        let state = test_state();
        let unique = uuid::Uuid::new_v4();
        let agent_id = format!("fleet-test-agent-{unique}");
        let block_id = format!("fleet-test-block-{unique}");
        state.reactive_handler.register_agent(&agent_id, &block_id, None).unwrap();

        let result = fleet_broadcast_impl(&state, vec![block_id.clone()], "hi".to_string(), None).await;

        let outcome_ids: Vec<&str> = result
            .succeeded
            .iter()
            .map(|s| s.as_str())
            .chain(result.failed.iter().map(|f| f.id.as_str()))
            .collect();
        assert_eq!(outcome_ids, vec![block_id.as_str()], "exactly one outcome for the one target");
        if let Some(failure) = result.failed.first() {
            assert!(
                !failure.error.contains("no registered agent"),
                "a registered block must not fail resolution: {}",
                failure.error
            );
        }
    }

    // Multi-target: one registered, one not — proves partial results are
    // reported per-target (never a single aggregate outcome for the whole
    // call), per the spec's §3/§5.4 "never a single aggregate toast" rule.
    #[tokio::test]
    async fn broadcast_reports_independent_outcomes_per_target() {
        let state = test_state();
        let unique = uuid::Uuid::new_v4();
        let agent_id = format!("fleet-test-agent-{unique}");
        let good_block = format!("fleet-test-block-good-{unique}");
        let bad_block = format!("fleet-test-block-bad-{unique}");
        state.reactive_handler.register_agent(&agent_id, &good_block, None).unwrap();

        let result = fleet_broadcast_impl(
            &state,
            vec![good_block.clone(), bad_block.clone()],
            "hi".to_string(),
            None,
        ).await;

        let bad_failure = result.failed.iter().find(|f| f.id == bad_block);
        assert!(bad_failure.is_some(), "unregistered target must be reported as failed");
        assert!(bad_failure.unwrap().error.contains("no registered agent"));
        // The good block produced exactly one outcome (succeeded or failed
        // for a DIFFERENT reason than "unregistered") — never silently
        // dropped just because a sibling target failed.
        let good_outcome_count = result.succeeded.iter().filter(|s| **s == good_block).count()
            + result.failed.iter().filter(|f| f.id == good_block).count();
        assert_eq!(good_outcome_count, 1, "the registered target must produce exactly one outcome");
    }

    async fn stop_result(state: &AppState, targets: Vec<String>, staged: Option<StagePlanInput>) -> FleetActionResult {
        fleet_bulk_stop_impl(state, targets, None, staged).await
    }

    #[tokio::test]
    async fn bulk_stop_reports_a_failure_per_target_with_no_live_controller() {
        let state = test_state();
        let targets = vec!["blk-a".to_string(), "blk-b".to_string(), "blk-c".to_string()];
        let result = stop_result(&state, targets.clone(), None).await;

        assert!(result.succeeded.is_empty());
        assert_eq!(result.failed.len(), 3);
        assert!(!result.aborted_early);
        let failed_ids: Vec<&str> = result.failed.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(failed_ids, vec!["blk-a", "blk-b", "blk-c"]);
        for f in &result.failed {
            assert!(f.error.contains("NOT_RUNNING"), "expected a controller-lookup failure, got: {}", f.error);
        }
    }

    // Every target fails under test (no live controller reachable), so a
    // staged plan with any max_fail_percentage below 100 must abort after
    // its first batch and record the untried remainder distinctly (never
    // silently drop them — succeeded+failed must always equal the original
    // target count).
    #[tokio::test]
    async fn bulk_stop_staged_aborts_early_and_accounts_for_every_target() {
        let state = test_state();
        let targets: Vec<String> = (0..5).map(|i| format!("blk-{i}")).collect();
        let result = stop_result(
            &state,
            targets.clone(),
            Some(StagePlanInput { batch_size: 2, max_fail_percentage: 50 }),
        ).await;

        assert!(result.aborted_early);
        assert!(result.succeeded.is_empty());
        assert_eq!(result.failed.len(), 5, "every target must be accounted for, tried or skipped");

        // First batch (blk-0, blk-1) actually attempted.
        let attempted: Vec<&str> = result.failed[..2].iter().map(|f| f.id.as_str()).collect();
        assert_eq!(attempted, vec!["blk-0", "blk-1"]);
        for f in &result.failed[..2] {
            assert!(f.error.contains("NOT_RUNNING"));
        }
        // Remaining three were skipped, not attempted.
        for f in &result.failed[2..] {
            assert!(f.error.contains("skipped"), "expected a skip marker, got: {}", f.error);
        }
    }

    // max_fail_percentage=100 never trips (100% failure is never > 100%),
    // so a staged plan with that threshold runs every batch to completion.
    #[tokio::test]
    async fn bulk_stop_staged_with_max_fail_percentage_100_never_aborts() {
        let state = test_state();
        let targets: Vec<String> = (0..5).map(|i| format!("blk-{i}")).collect();
        let result = stop_result(
            &state,
            targets.clone(),
            Some(StagePlanInput { batch_size: 2, max_fail_percentage: 100 }),
        ).await;

        assert!(!result.aborted_early);
        assert_eq!(result.failed.len(), 5);
        for f in &result.failed {
            assert!(f.error.contains("NOT_RUNNING"), "expected every target actually attempted, got: {}", f.error);
        }
    }

    // Regression for reagent's P2 (PR #2687 review): tripping the failure
    // threshold on the LAST batch must not report aborted_early, since
    // nothing was actually left unattempted — the whole target list ran.
    #[tokio::test]
    async fn bulk_stop_does_not_report_aborted_early_when_the_failing_batch_is_the_last_one() {
        let state = test_state();
        // batch_size=5 with exactly 5 targets → a single batch. All 5 fail
        // (no live controller under test), which exceeds any threshold
        // below 100 — but there's no second batch left to skip.
        let targets: Vec<String> = (0..5).map(|i| format!("blk-{i}")).collect();
        let result = stop_result(
            &state,
            targets.clone(),
            Some(StagePlanInput { batch_size: 5, max_fail_percentage: 50 }),
        ).await;

        assert_eq!(result.failed.len(), 5, "every target was still attempted");
        for f in &result.failed {
            assert!(
                f.error.contains("NOT_RUNNING"),
                "every target must show a real attempt error, not a skip marker: {}",
                f.error
            );
        }
        assert!(
            !result.aborted_early,
            "the whole list ran (it was the last/only batch) — this must not read as an early abort"
        );
    }

    // Regression for reagent/Codex P2 (PR #2687 review): bulk-stop must be
    // visible in Warden's Audit tab, same as an ordinary jekt injection —
    // this was missing entirely before this fix.
    #[tokio::test]
    async fn bulk_stop_writes_an_audit_entry_per_target() {
        let state = test_state();
        // ACTIVE_LOGIN-style caveat: get_audit_log reads a process-global
        // ring buffer shared by every test in this binary (same singleton
        // as ReactiveHandler itself) — filter by this test's own
        // uuid-suffixed block_id rather than assuming a before/after
        // length diff or `.last()` reflects only this test's own write.
        let unique = uuid::Uuid::new_v4();
        let agent_id = format!("fleet-audit-agent-{unique}");
        let block_id = format!("fleet-audit-block-{unique}");
        state.reactive_handler.register_agent(&agent_id, &block_id, None).unwrap();

        let _ = stop_result(&state, vec![block_id.clone()], None).await;
        let matching: Vec<_> = state
            .reactive_handler
            .get_audit_log(10_000)
            .into_iter()
            // register_agent's own setup call above now also audits a
            // "register" event (event_kind, #2694) alongside the stop's
            // own delivery-attempt entry — both share this test's unique
            // block_id, so filter to the delivery entry this test is
            // actually about.
            .filter(|e| e.block_id == block_id && e.event_kind == "delivery")
            .collect();

        assert_eq!(matching.len(), 1, "exactly one audit entry for this test's own block_id");
        let entry = &matching[0];
        assert_eq!(entry.target_agent, agent_id, "audit entry should resolve the block's registered agent name");
        assert!(!entry.success, "the stop itself failed under test (no live controller) — audit must reflect that");
    }

    // Exercises the actual registered RPC handlers (deserialize -> store call
    // -> serialize), not just the store layer directly (already covered by
    // `agent_groups.rs`'s own unit tests) — same
    // `WshRpcEngine::new()` + `handle_message` + response-channel pattern
    // `agent_handlers::core`'s `exportagents` tests already use.
    #[tokio::test]
    async fn group_crud_roundtrips_through_the_rpc_handlers() {
        use crate::backend::rpc::engine::WshRpcEngine;
        use crate::backend::rpc_types::RpcMessage;

        let state = test_state();
        let (engine, mut output_rx) = WshRpcEngine::new();
        crate::server::app_api::fleet::register(&engine, &state);

        async fn call(
            engine: &std::sync::Arc<WshRpcEngine>,
            output_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
            command: &str,
            data: serde_json::Value,
        ) -> RpcMessage {
            engine.handle_message(RpcMessage {
                command: command.to_string(),
                reqid: uuid::Uuid::new_v4().to_string(),
                data: Some(data),
                ..Default::default()
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
                .await
                .expect("handler should respond within 2s")
                .expect("response channel should not close")
        }

        let created = call(
            &engine,
            &mut output_rx,
            COMMAND_FLEET_GROUP_CREATE,
            serde_json::json!({ "name": "backend", "member_ids": ["b1", "b2"] }),
        ).await;
        assert!(created.error.is_empty(), "create failed: {}", created.error);
        let created: FleetGroup = serde_json::from_value(created.data.unwrap()).unwrap();
        assert_eq!(created.name, "backend");
        assert_eq!(created.member_ids, vec!["b1".to_string(), "b2".to_string()]);

        let listed = call(&engine, &mut output_rx, COMMAND_FLEET_GROUP_LIST, serde_json::json!({})).await;
        assert!(listed.error.is_empty());
        let listed: FleetGroupListResult = serde_json::from_value(listed.data.unwrap()).unwrap();
        assert_eq!(listed.groups.len(), 1);
        assert_eq!(listed.groups[0].id, created.id);

        let updated = call(
            &engine,
            &mut output_rx,
            COMMAND_FLEET_GROUP_UPDATE,
            serde_json::json!({ "id": created.id, "name": "backend-2" }),
        ).await;
        assert!(updated.error.is_empty(), "update failed: {}", updated.error);
        let updated: FleetGroup = serde_json::from_value(updated.data.unwrap()).unwrap();
        assert_eq!(updated.name, "backend-2");
        assert_eq!(updated.member_ids, vec!["b1".to_string(), "b2".to_string()], "untouched field must survive");

        let deleted = call(
            &engine,
            &mut output_rx,
            COMMAND_FLEET_GROUP_DELETE,
            serde_json::json!({ "id": created.id }),
        ).await;
        assert!(deleted.error.is_empty());
        assert_eq!(deleted.data.unwrap()["ok"], serde_json::Value::Bool(true));

        let listed_after = call(&engine, &mut output_rx, COMMAND_FLEET_GROUP_LIST, serde_json::json!({})).await;
        let listed_after: FleetGroupListResult = serde_json::from_value(listed_after.data.unwrap()).unwrap();
        assert!(listed_after.groups.is_empty());
    }

    #[tokio::test]
    async fn group_update_on_a_missing_id_errors_instead_of_silently_no_opping() {
        use crate::backend::rpc::engine::WshRpcEngine;
        use crate::backend::rpc_types::RpcMessage;

        let state = test_state();
        let (engine, mut output_rx) = WshRpcEngine::new();
        crate::server::app_api::fleet::register(&engine, &state);

        engine.handle_message(RpcMessage {
            command: COMMAND_FLEET_GROUP_UPDATE.to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({ "id": "does-not-exist", "name": "x" })),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!resp.error.is_empty(), "updating a nonexistent group must error, not silently no-op");
    }

    // ── Cross-channel bulk-stop forward
    // (SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md) ──────────────────

    use crate::server::app_api::fleet::forward_stop_to_shared_channel;

    /// Serializes every test in this sub-block that mutates the
    /// process-global `AGENTMUX_HOME_OVERRIDE` env var (which
    /// `resolve_shared_reactive_dir` reads) — same lock already used
    /// elsewhere in this crate for this exact var (e.g.
    /// `identity::resolver::inject`'s tests), not a new one, per
    /// `test_support::ISOLATED_AUTH_ENV_LOCK`'s own doc comment about
    /// avoiding a proliferation of module-local locks around shared env
    /// vars.
    fn home_override_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Write an `AgentEntry` directly (not via `registry::write_shared`,
    /// which always stamps `local_auth_key()` — a process-global `OnceLock`
    /// this test can't control) so tests can assert on a SPECIFIC auth_key
    /// value being forwarded. Layout only needs to satisfy
    /// `list_all_shared`'s generic "any `*.json` file under a subdirectory
    /// of `shared_dir`" walk, not `write_shared`'s own path convention.
    fn write_raw_shared_entry(
        shared_dir: &std::path::Path,
        agent_id: &str,
        local_url: &str,
        block_id: &str,
        auth_key: &str,
    ) {
        let dir = shared_dir.join(agent_id);
        std::fs::create_dir_all(&dir).unwrap();
        let entry = crate::backend::reactive::registry::AgentEntry {
            agent_id: agent_id.to_string(),
            local_url: local_url.to_string(),
            block_id: block_id.to_string(),
            pid: std::process::id(),
            updated_at: 0,
            auth_key: auth_key.to_string(),
            channel: "test-channel".to_string(),
            registration_nonce: 0,
            jekt_public_key: String::new(),
        };
        std::fs::write(dir.join("test-channel.json"), serde_json::to_string(&entry).unwrap()).unwrap();
    }

    /// Fake `/agentmux/agent/stop` responder — records the `X-AuthKey`
    /// header it received (or `None`) and returns `response_body` verbatim.
    /// Mirrors `spawn_fake_browser_api`'s established pattern (raw TCP +
    /// hand-built HTTP response — no real CEF host needed).
    async fn spawn_fake_agent_stop_endpoint(
        response_body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Option<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let received_auth: std::sync::Arc<std::sync::Mutex<Option<String>>> = std::sync::Arc::new(std::sync::Mutex::new(None));
        let received_auth_clone = received_auth.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let received_auth = received_auth_clone.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else { return };
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let auth = req
                        .lines()
                        .find_map(|l| l.strip_prefix("X-AuthKey: ").or_else(|| l.strip_prefix("x-authkey: ")))
                        .map(|v| v.trim_end_matches('\r').to_string());
                    *received_auth.lock().unwrap() = auth;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        (format!("http://127.0.0.1:{port}"), received_auth)
    }

    #[tokio::test]
    async fn forward_stop_returns_none_when_shared_registry_has_no_matching_entry() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let state = test_state();

        let result = forward_stop_to_shared_channel(&state, "no-such-block", None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        assert!(result.is_none(), "no matching shared entry must fall through to the caller's local-error path");
    }

    #[tokio::test]
    async fn forward_stop_ignores_a_non_loopback_entry() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let unique = uuid::Uuid::new_v4();
        let block_id = format!("blk-remote-{unique}");
        // A real IP, not loopback — must never be treated as a same-host
        // cross-channel peer (this is the same defense-in-depth check
        // `server/reactive.rs`'s inject cascade already applies).
        write_raw_shared_entry(&shared_dir, "remote-agent", "http://203.0.113.5:9999", &block_id, "some-key");
        let state = test_state();

        let result = forward_stop_to_shared_channel(&state, &block_id, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        assert!(result.is_none(), "a non-loopback entry must never be forwarded to");
    }

    #[tokio::test]
    async fn forward_stop_ignores_a_stale_self_entry() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let unique = uuid::Uuid::new_v4();
        let block_id = format!("blk-self-{unique}");
        let mut state = test_state();
        state.local_web_url = "http://127.0.0.1:12345".to_string();
        // An entry whose local_url IS this instance's own — a stale
        // self-registration from a prior crash, not a real peer.
        write_raw_shared_entry(&shared_dir, "self-agent", &state.local_web_url.clone(), &block_id, "some-key");

        let result = forward_stop_to_shared_channel(&state, &block_id, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        assert!(result.is_none(), "a stale self-entry must never be forwarded to (would be forwarding to itself)");
    }

    #[tokio::test]
    async fn forward_stop_succeeds_against_a_real_loopback_peer_with_its_own_auth_key() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let unique = uuid::Uuid::new_v4();
        let block_id = format!("blk-peer-{unique}");
        let (peer_url, received_auth) =
            spawn_fake_agent_stop_endpoint(r#"{"success":true,"result":{"block_id":"x","status":"done"}}"#).await;
        write_raw_shared_entry(&shared_dir, "peer-agent", &peer_url, &block_id, "peer-secret-key");
        let state = test_state();

        let result = forward_stop_to_shared_channel(&state, &block_id, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        let (agent_name, outcome) = result.expect("a loopback entry must be forwarded to");
        assert_eq!(agent_name, "peer-agent");
        assert!(outcome.is_ok(), "a success:true response must resolve Ok: {outcome:?}");
        assert_eq!(
            received_auth.lock().unwrap().as_deref(),
            Some("peer-secret-key"),
            "the peer's OWN auth_key (from the registry entry) must be sent, not this instance's"
        );
    }

    #[tokio::test]
    async fn forward_stop_propagates_a_peer_side_failure() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let unique = uuid::Uuid::new_v4();
        let block_id = format!("blk-fail-{unique}");
        let (peer_url, _received_auth) =
            spawn_fake_agent_stop_endpoint(r#"{"success":false,"error":"NOT_RUNNING: no controller for block x"}"#).await;
        write_raw_shared_entry(&shared_dir, "peer-agent", &peer_url, &block_id, "peer-secret-key");
        let state = test_state();

        let result = forward_stop_to_shared_channel(&state, &block_id, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        let (_agent_name, outcome) = result.expect("a loopback entry must still be forwarded to");
        let err = outcome.expect_err("a success:false response must resolve Err");
        assert!(err.contains("NOT_RUNNING"), "the peer's own error text must propagate: {err}");
    }

    #[tokio::test]
    async fn bulk_stop_reaches_a_target_only_present_in_the_shared_cross_channel_registry() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let unique = uuid::Uuid::new_v4();
        let block_id = format!("blk-e2e-{unique}");
        let (peer_url, _received_auth) =
            spawn_fake_agent_stop_endpoint(r#"{"success":true,"result":{"block_id":"x","status":"done"}}"#).await;
        write_raw_shared_entry(&shared_dir, "peer-agent-e2e", &peer_url, &block_id, "peer-secret-key");
        let state = test_state();

        let result = fleet_bulk_stop_impl(&state, vec![block_id.clone()], None, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        assert_eq!(result.succeeded, vec![block_id], "a target absent from THIS instance's own registry, but present cross-channel, must succeed via the forward — not fail with NOT_RUNNING");
        assert!(result.failed.is_empty());
    }
}

/// Regression for reagent P2 on PR #2674 (re-review): a caller supplying
/// `provenance.source` but omitting `detail` must deserialize `detail` as
/// `{}`, not `Value::Null` — a bare `#[serde(default)]` on a
/// `serde_json::Value` field yields `Null`, whose `.to_string()` is the
/// literal string `"null"`, not the `"{}"` every no-provenance write path
/// already uses. This is the same bug class already fixed once in this
/// PR's review history for the WS-RPC sibling
/// (`NativeMemoryWriteProvenance` in rpc_types/memory.rs) — recurring here
/// in the HTTP/App-API request struct that backs `handle_agent_memory_write`
/// (the `MemoryWrite` MCP tool's actual write path).
#[test]
fn agent_memory_write_provenance_req_defaults_a_missing_detail_to_an_empty_object() {
    let req: AgentMemoryWriteProvenanceReq = serde_json::from_str(r#"{"source":"human"}"#).unwrap();
    assert_eq!(req.detail, serde_json::json!({}));
    assert_eq!(req.detail.to_string(), "{}");
}

// ── Muxqueue HTTP surface ───────────────────────────────────────────────────
// docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md, slice 2.

/// reagent P1 on PR #2902 — the blocking one. `DELETE /agentmux/cron/:id`, the
/// sibling route registered a few lines away in the same router, takes NO body.
/// A caller that follows that adjacent convention for `DELETE
/// /agentmux/work/:id` must not be rejected by axum's `Json` extractor before
/// the handler ever runs. Drives the real router, so it fails if the extractor
/// is ever tightened back to a required body.
#[tokio::test]
async fn work_cancel_accepts_a_delete_with_no_body_like_its_cron_sibling() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/work/does-not-exist")
        .method("DELETE")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // CONFLICT = the handler ran and found no open/claimed item, which is the
    // point: anything in the 400/415 family would mean the body extractor
    // rejected the request before the handler was ever reached.
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "a bodyless DELETE must reach the handler, not be rejected by the Json extractor"
    );
}

/// The body is optional, not ignored — a supplied reason must still parse.
#[tokio::test]
async fn work_cancel_still_accepts_a_json_body_when_one_is_sent() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/work/does-not-exist")
        .method("DELETE")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"reason":"superseded"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// End-to-end through the router: enqueue, then claim it back out. Also pins
/// that the claim response carries `attempt` at the TOP level, which every
/// later holder call must echo as its fence.
#[tokio::test]
async fn work_enqueue_then_claim_round_trips_and_exposes_the_fence() {
    let app = test_router();

    let enq = Request::builder()
        .uri("/agentmux/work")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"title":"repro the thing","payload":"steps go here","created_by":"tester"}"#,
        ))
        .unwrap();
    let resp = app.clone().oneshot(enq).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let claim = Request::builder()
        .uri("/agentmux/work/claim")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"a1"}"#))
        .unwrap();
    let resp = app.oneshot(claim).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["claimed"], true);
    assert_eq!(
        v["attempt"], 1,
        "the fence must be at the top level of the claim response, not only nested in item"
    );
    assert_eq!(v["item"]["title"], "repro the thing");
}

/// An empty queue answers "nothing available" with 200, not an error — callers
/// are expected to poll speculatively when they have spare capacity.
#[tokio::test]
async fn work_claim_on_an_empty_queue_is_a_success_not_an_error() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/work/claim")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"a1"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["claimed"], false);
}

/// Targeting both an agent and a group is ambiguous; the handler rejects it
/// rather than letting one silently win.
#[tokio::test]
async fn work_enqueue_rejects_both_target_agent_and_target_group() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/work")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"title":"t","payload":"p","target_agent":"a1","target_group":"g1"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Codex P2 on PR #2902: a release must report the RESULTING state, because a
/// release on the final allowed attempt parks the item as `failed` rather than
/// reopening it. Without this the MCP layer tells the caller another agent can
/// pick the item up when nobody ever will.
#[tokio::test]
async fn work_release_reports_the_resulting_state_not_just_ok() {
    let app = test_router();

    // max_attempts 1, so the very first release is also the final attempt.
    let enq = Request::builder()
        .uri("/agentmux/work")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"title":"t","payload":"p","max_attempts":1}"#))
        .unwrap();
    let resp = app.clone().oneshot(enq).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let claim = Request::builder()
        .uri("/agentmux/work/claim")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"a1"}"#))
        .unwrap();
    let resp = app.clone().oneshot(claim).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let attempt = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["attempt"]
        .as_i64()
        .unwrap();

    let rel = Request::builder()
        .uri(format!("/agentmux/work/{id}/release"))
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(format!(
            r#"{{"agent_id":"a1","attempt":{attempt},"result":"cannot do this"}}"#
        )))
        .unwrap();
    let resp = app.oneshot(rel).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(
        v["state"], "failed",
        "a release on the final attempt parks the item; the response must say so"
    );
}

/// A stale fence is 409 CONFLICT, not 404: the row still exists, it just moved
/// on without this caller. The MCP layer depends on that distinction to tell an
/// agent to re-claim rather than to treat the item as gone.
#[tokio::test]
async fn work_complete_with_a_wrong_fence_is_conflict_not_not_found() {
    let app = test_router();

    let enq = Request::builder()
        .uri("/agentmux/work")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"title":"t","payload":"p"}"#))
        .unwrap();
    let resp = app.clone().oneshot(enq).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let claim = Request::builder()
        .uri("/agentmux/work/claim")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"a1"}"#))
        .unwrap();
    app.clone().oneshot(claim).await.unwrap();

    // attempt 99 was never issued by any claim.
    let done = Request::builder()
        .uri(format!("/agentmux/work/{id}/complete"))
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"a1","attempt":99,"result":"nope"}"#))
        .unwrap();
    let resp = app.oneshot(done).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

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
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    // This one store backs mstore/id_store/identity_store below, so it has to
    // carry BOTH schemas. Without the identity schema, any handler touching a
    // table that lives only in the global identity store (`db_work_queue`)
    // failed with a bare "no such table" 500 in tests while being perfectly
    // correct in production. Additive and idempotent — every statement in that
    // schema is CREATE ... IF NOT EXISTS.
    mstore.apply_identity_schema_for_tests().unwrap();
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
    wcore::ensure_initial_data(&mstore).unwrap();

    let config_watcher = Arc::new(wconfig::ConfigState::new());

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
        hostname: "test-host".to_string(),
        app_path: String::new(),
        mstore: mstore.clone(),
        shared_store: None,
        id_store: mstore.clone(),
        // Aliased to `mstore` on purpose — a lot of existing setup code seeds
        // through one store and reads through another. See the
        // `apply_identity_schema_for_tests` call above for why that single
        // store now carries the identity schema too.
        identity_store: mstore.clone(),
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
        subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone(), mstore.clone(), mstore.clone(), mstore.clone())),
        history_service: Arc::new(crate::backend::history::HistoryService::new()),
        lan_discovery: Arc::new(crate::backend::lan_discovery::LanDiscoveryController::new(
            "test-instance".to_string(),
            "test-host".to_string(),
            "0.28.20".to_string(),
            0,
            event_bus.clone(),
            String::new(),
        )),
        lan_listeners: Arc::new(crate::backend::lan_listeners::LanListenerSupervisor::new(0, 0)),
        lsp_supervisor: Arc::new(crate::backend::lsp::LspSupervisor::new(event_bus.clone())),
        process_tracker,
        process_broker,
        dock_snapshots: Arc::new(crate::backend::dock_snapshot::DockSnapshotCache::new()),
        pending_background_pids: Arc::new(crate::backend::pending_background_pids::PendingBackgroundPids::new()),
        narrated_events: Arc::new(crate::backend::narrated_events::NarratedEvents::new()),
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
        dev_proxy: crate::backend::dev_proxy::DevProxyRegistry::new(),
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

/// `local_url` is always loopback, so `host.hostname` is the only field that
/// tells a client (e.g. the mobile app's discovery screen) which machine it is
/// actually talking to. Regression guard for it being dropped from the
/// response or never plumbed onto `AppState` — the state it was in before.
#[tokio::test]
async fn discovery_reports_host_hostname() {
    let app = test_router();
    let req = Request::builder()
        .uri("/agentmux/discovery")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["host"]["hostname"], "test-host");
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
    let key = state.mstore.agent_jekt_key_ensure(&agent_id).unwrap();
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

// ── RegisterDevServer (SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md) ──────

#[tokio::test]
async fn register_dev_server_rejects_an_unsigned_request() {
    let app = test_router();
    let req = Request::builder()
        .uri("/api/v1/agent/dev_server/register")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"agent_id":"nobody","ts_secs":9999999999,"sig":"forged","project":"pulse","port":3000}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn register_dev_server_rejects_an_empty_project_before_touching_docker() {
    let state = test_state();
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["project"] = serde_json::json!("   ");
    body["port"] = serde_json::json!(3000);
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/agent/dev_server/register")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_dev_server_rejects_a_non_hostname_safe_project_before_touching_docker() {
    // reagent P2, PR #3439: a `project` that isn't a valid DNS label
    // (contains '.', whitespace, etc.) must 400 clearly instead of
    // registering successfully and handing back a URL that can never
    // actually route.
    let state = test_state();
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["project"] = serde_json::json!("pulse.app");
    body["port"] = serde_json::json!(3000);
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/agent/dev_server/register")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_dev_server_rejects_a_zero_port() {
    let state = test_state();
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["project"] = serde_json::json!("pulse");
    body["port"] = serde_json::json!(0);
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/agent/dev_server/register")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// `test_state()` wires a `disabled()` `ContainerRuntimeHandle` (no real
/// Docker) — confirms a well-formed, correctly signed request still fails
/// cleanly (503, not a panic or a 200 with a bogus address) when Docker
/// itself isn't available. The real-Docker path (network attach, IP
/// resolution, and registration reaching the routing table) is covered by
/// `backend::container::tests::itest_dev_proxy_network_attach_and_ip_resolution`
/// (Docker-gated, `#[ignore]`) and `backend::dev_proxy::tests`' in-process
/// HTTP-through-proxy test.
#[tokio::test]
async fn register_dev_server_requires_docker_available() {
    let state = test_state();
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["project"] = serde_json::json!("pulse");
    body["port"] = serde_json::json!(3000);
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/agent/dev_server/register")
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

// ── Browser-pane deep control (SPEC_AGENT_BROWSER_PANE_DEEP_CONTROL_2026_09_20) ──

#[tokio::test]
async fn ui_browser_navigate_proxies_to_host_after_registration() {
    let state = test_state();
    let port = spawn_fake_browser_api(r#"{"ok":true,"data":{"ok":true}}"#, "tok-abc").await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["url"] = serde_json::json!("https://example.com");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/browser/navigate")
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

/// The exact scenario the §3.1 fix in `browser_api::routes` exists for
/// (an agent whose own pane isn't a dedicated browser pane calling
/// `BrowserNavigate`): the CEF host rejects it with a clear error, and
/// this must surface intact through srv, not get swallowed or turned into
/// a generic 500.
#[tokio::test]
async fn ui_browser_navigate_surfaces_the_host_side_shared_target_rejection() {
    let state = test_state();
    let port = spawn_fake_browser_api(
        r#"{"ok":false,"error":"navigate: block \"b1\" is not a dedicated browser pane."}"#,
        "tok-abc",
    )
    .await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["url"] = serde_json::json!("https://example.com");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/browser/navigate")
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
    assert!(json["error"].as_str().unwrap().contains("not a dedicated browser pane"));
}

#[tokio::test]
async fn ui_browser_eval_returns_host_result() {
    let state = test_state();
    let port = spawn_fake_browser_api(
        r#"{"ok":true,"data":{"result":"hello","type":"string","exception":null}}"#,
        "tok-abc",
    )
    .await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (_agent_id, auth) = signed_ui_auth(&state, "b1");
    let mut body = auth;
    body["script"] = serde_json::json!("document.title");
    let app = build_router(state);

    let req = Request::builder()
        .uri("/api/v1/ui/browser/eval")
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
    assert_eq!(json["data"]["result"], "hello");
}

#[tokio::test]
async fn ui_browser_navigate_rejects_a_forged_agent_identity() {
    let state = test_state();
    let port = spawn_fake_browser_api(r#"{"ok":true,"data":{"ok":true}}"#, "tok-abc").await;
    *state.host_ipc.lock().await = Some(HostIpc {
        port,
        token: "tok-abc".to_string(),
    });
    let (victim_agent_id, _victim_auth) = signed_ui_auth(&state, "victim-block");
    let (_attacker_agent_id, attacker_auth) = signed_ui_auth(&state, "attacker-block");
    let mut forged = attacker_auth;
    forged["agent_id"] = serde_json::json!(victim_agent_id);
    forged["url"] = serde_json::json!("https://example.com");

    let app = build_router(state);
    let req = Request::builder()
        .uri("/api/v1/ui/browser/navigate")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(forged.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "browser-pane deep-control routes must use the same identity verification as UIClick/UIQuery"
    );
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
    state.mstore.agent_def_insert(&mut def).expect("insert definition");

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
    state.mstore.agent_def_insert(&mut def).expect("insert definition");

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
    state.mstore.agent_def_insert(&mut agent_a).expect("insert agent a");

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
    state.mstore.agent_def_insert(&mut agent_b).expect("insert agent b");

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
    // mps.rs::tests::test_event_persistence.
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
    let mstore = state.mstore.clone();
    let tab = mstore
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
// accepted by the three LAN-forwarding routes but rejected everywhere else,
// and the full `auth_key` must keep working on those same routes
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
/// to the general API surface, only the three LAN-forwarding routes.
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

/// The LAN peer agent-name list (2026-09-08): a `lan_key` holder may
/// enumerate agent NAMES, so a peer can show which agents live on another
/// host instead of the empty `agents: []` it reported before.
#[tokio::test]
async fn lan_key_can_list_agent_names() {
    let state = test_state();
    let unique = uuid::Uuid::new_v4();
    let agent_id = format!("names-test-agent-{unique}");
    state
        .reactive_handler
        .register_agent(&agent_id, &format!("names-test-block-{unique}"), None)
        .unwrap();

    let app = build_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/agentmux/reactive/agent-names")
        .header("X-AuthKey", "test-lan-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let names: Vec<&str> = json["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(names.contains(&agent_id.as_str()), "got {names:?}");
}

/// The security scoping that made the route above acceptable: it returns
/// NAMES and nothing else. `AgentRegistration` also carries `block_id`,
/// `tab_id`, `registered_at`, `last_seen` and `registration_nonce` —
/// internal delivery/routing detail that must stay behind full auth (that's
/// why this is a separate route from `/agentmux/reactive/agents`, which
/// `lan_key_is_rejected_on_other_reactive_routes` pins at 401).
#[tokio::test]
async fn agent_names_exposes_names_only_not_registration_internals() {
    let state = test_state();
    let unique = uuid::Uuid::new_v4();
    state
        .reactive_handler
        .register_agent(
            &format!("names-only-agent-{unique}"),
            &format!("names-only-block-{unique}"),
            Some("tab-should-not-leak"),
        )
        .unwrap();

    let app = build_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/agentmux/reactive/agent-names")
        .header("X-AuthKey", "test-lan-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let raw = String::from_utf8_lossy(&body);

    for leaked in [
        "block_id",
        "tab_id",
        "tab-should-not-leak",
        "registration_nonce",
        "registered_at",
        "last_seen",
    ] {
        assert!(!raw.contains(leaked), "{leaked} leaked over the LAN route: {raw}");
    }
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
/// `window:displayname` in mstore. srv_state is hydrated from mstore via
/// the same `bootstrap_state_from_mstore` production runs, so the new
/// reducer existence guard sees the seeded window exactly as it would live.
#[tokio::test]
async fn window_name_renames_seeded_window_and_persists() {
    let state = test_state();
    crate::persist::bootstrap_state_from_mstore(&state.srv_state, &state.mstore).await;
    let mstore = state.mstore.clone();
    let window = mstore
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

    let reread = mstore
        .get::<crate::backend::obj::Window>(&window.oid)
        .unwrap()
        .expect("window still exists");
    assert_eq!(
        reread.meta.get("window:displayname").and_then(|v| v.as_str()),
        Some("renamed-by-test"),
        "display name must be persisted in mstore meta"
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
    let mstore = state.mstore.clone();
    let srv_state = state.srv_state.clone();

    // Seed workspace + tab through the reducer so BOTH reducer state and
    // mstore know the tab (mirrors production bootstrap).
    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
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

    let tab = mstore.get::<Tab>(&tab_id).unwrap().unwrap();
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
        let mut layout = mstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
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
        mstore.update(&mut layout).unwrap();
    }
    let version_before = mstore
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
    let layout = mstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
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
    let mstore = state.mstore.clone();
    let srv_state = state.srv_state.clone();

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
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
    let layout_oid = mstore.get::<Tab>(&tab_id).unwrap().unwrap().layoutstate;

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
    let raw = mstore
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
    let mstore = state.mstore.clone();
    let srv_state = state.srv_state.clone();

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
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
    for bid in ["b-agent", "b-swarm", "b-sysinfo"] {
        srv_state.lock().await.blocks.insert(
            bid.to_string(),
            crate::state::BlockRecord { block_id: bid.to_string(), tab_id: tab1.clone() },
        );
    }
    let (tree, focused, leaforder) =
        crate::backend::wcore::default_three_pane_tree("b-agent", "b-swarm", "b-sysinfo");
    crate::server::service::seed_layout_via_reducer(
        &state, &tab1, tree, focused, leaforder, String::new(),
    )
    .await
    .expect("three-pane seed via reducer");

    let layout_oid = mstore.get::<Tab>(&tab1).unwrap().unwrap().layoutstate;
    let row = mstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
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

    let layout_oid = mstore.get::<Tab>(&tab2).unwrap().unwrap().layoutstate;
    let row = mstore.get::<LayoutState>(&layout_oid).unwrap().unwrap();
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
    let mstore = state.mstore.clone();
    let srv_state = state.srv_state.clone();

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "unexpected reducer error: {:?}",
            events
        );
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        events
    }

    async fn assert_coherent(state: &AppState, tab_id: &str, step: &str) {
        let tab = state.mstore.get::<Tab>(tab_id).unwrap().unwrap();
        let db_layout = state
            .mstore
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
        let tab = mstore.get::<Tab>(&tab_id).unwrap().unwrap();
        let layout = mstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
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
        let tab = mstore.get::<Tab>(&tab_id).unwrap().unwrap();
        let layout = mstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
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
    let tab = mstore.get::<Tab>(&tab_id).unwrap().unwrap();
    let layout = mstore.get::<LayoutState>(&tab.layoutstate).unwrap().unwrap();
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
    state.mstore.agent_def_insert(&mut def).expect("insert definition");

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
    state.mstore.instance_create(&inst).expect("create instance");

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

    // ── An agent's cross-channel FleetBulkStop asks the target instance's
    // user (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5) ──────────────────────

    /// A fake other instance: `stop-pending` opens request `remote-req-1`,
    /// whose status is `status_body`; `/agentmux/agent/stop` is the plain
    /// forward. `with_pending: false` is an instance that predates the route.
    async fn spawn_fake_peer(with_pending: bool, status_body: serde_json::Value) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use axum::routing::{get, post};
        let hits: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let (h1, h2, h3) = (hits.clone(), hits.clone(), hits.clone());
        let mut app = axum::Router::new()
            .route(
                "/agentmux/agent/stop",
                post(move || async move {
                    h1.lock().unwrap().push("stop".into());
                    axum::Json(serde_json::json!({ "success": true }))
                }),
            )
            .route(
                "/api/v1/agent/shutdown/{id}",
                get(move |headers: axum::http::HeaderMap| async move {
                    let key = headers.get("X-AuthKey").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                    h2.lock().unwrap().push(format!("status:{key}"));
                    axum::Json(status_body)
                }),
            );
        if with_pending {
            app = app.route(
                "/agentmux/agent/stop-pending",
                post(move |axum::Json(body): axum::Json<serde_json::Value>| async move {
                    h3.lock().unwrap().push(format!("stop-pending:{}", body["by"].as_str().unwrap_or("")));
                    axum::Json(serde_json::json!({
                        "success": true,
                        "pending": { "request_id": "remote-req-1", "by": "Korp", "via": "FleetBulkStop", "deadline_ms": 42, "joined": false },
                    }))
                }),
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (url, hits)
    }

    #[tokio::test]
    async fn a_cross_channel_target_waits_for_its_own_instance_s_user_and_its_status_is_proxied() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let block_id = format!("blk-xchan-{}", uuid::Uuid::new_v4());
        let (peer_url, hits) = spawn_fake_peer(true, serde_json::json!({ "request_id": "remote-req-1", "status": "kept_by_user" })).await;
        write_raw_shared_entry(&shared_dir, "peer-agent-x", &peer_url, &block_id, "peer-secret-key");
        let state = test_state();

        let (now, pending) =
            crate::server::app_api::fleet::fleet_bulk_stop_with_override(&state, "Korp", vec![block_id.clone()], None, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        assert!(now.succeeded.is_empty() && now.failed.is_empty(), "nothing is stopped at once");
        assert_eq!(pending.len(), 1);
        assert_eq!((pending[0].block_id.as_str(), pending[0].request_id.as_str()), (block_id.as_str(), "remote-req-1"));
        assert_eq!(hits.lock().unwrap().clone(), vec!["stop-pending:Korp".to_string()], "asked there, never the plain stop");

        // The MCP polls THIS instance; the answer comes from the peer.
        let resp = crate::server::app_api::pane::handle_shutdown_status(
            axum::extract::State(state.clone()),
            axum::extract::Path("remote-req-1".to_string()),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "kept_by_user");
        assert!(hits.lock().unwrap().contains(&"status:peer-secret-key".to_string()), "with the peer's own key");
    }

    #[tokio::test]
    async fn an_instance_that_predates_the_window_is_stopped_at_once_as_before() {
        let _guard = home_override_guard();
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let shared_dir = tmp.path().join("shared").join("agents").join("reactive");
        let block_id = format!("blk-xchan-old-{}", uuid::Uuid::new_v4());
        let (peer_url, hits) = spawn_fake_peer(false, serde_json::Value::Null).await;
        write_raw_shared_entry(&shared_dir, "peer-agent-old", &peer_url, &block_id, "peer-secret-key");
        let state = test_state();

        let (now, pending) =
            crate::server::app_api::fleet::fleet_bulk_stop_with_override(&state, "Korp", vec![block_id.clone()], None, None).await;
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

        assert!(pending.is_empty());
        assert_eq!(now.succeeded, vec![block_id]);
        assert_eq!(hits.lock().unwrap().clone(), vec!["stop".to_string()]);
        let entry = state.reactive_handler.get_audit_log(20).into_iter().find(|e| e.block_id == now.succeeded[0]).unwrap();
        assert!(entry.reason.unwrap_or_default().contains("predates the user override"), "audited as such");
    }

    /// The target instance's side: its own window, not an immediate stop.
    #[tokio::test]
    async fn stop_pending_opens_this_instance_s_window_for_a_block_running_here() {
        struct Running(String, std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl crate::backend::blockcontroller::Controller for Running {
            fn start(&self, _: crate::backend::obj::MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
                Ok(())
            }
            fn stop(&self, _: bool, _: &str) -> Result<(), String> {
                self.1.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            fn get_runtime_status(&self) -> crate::backend::blockcontroller::BlockControllerRuntimeStatus {
                crate::backend::blockcontroller::BlockControllerRuntimeStatus { blockid: self.0.clone(), ..Default::default() }
            }
            fn send_input(&self, _: crate::backend::blockcontroller::BlockInputUnion, _: Option<u64>) -> Result<(), String> {
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
        let state = test_state();
        let block_id = format!("blk-stop-pending-{}", uuid::Uuid::new_v4());
        let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        crate::backend::blockcontroller::register_controller(&block_id, std::sync::Arc::new(Running(block_id.clone(), stopped.clone())));

        let resp = super::super::handle_agent_stop_pending_forward(
            axum::extract::State(state.clone()),
            axum::Json(super::super::AgentStopPendingForwardRequest { block_id: block_id.clone(), signal: None, by: "Korp".into() }),
        )
        .await
        .into_response();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["success"], true);
        let request_id = v["pending"]["request_id"].as_str().unwrap().to_string();
        let here = crate::sagas::pending_shutdown::status(&request_id).unwrap();
        assert_eq!((here.status, here.by.as_str(), here.via.as_str()), ("pending", "Korp", "FleetBulkStop"));
        assert!(!stopped.load(std::sync::atomic::Ordering::SeqCst), "not stopped during the window");
        assert_eq!(crate::sagas::pending_shutdown::keep(&state, &block_id, &request_id), "kept_by_user");
        crate::backend::blockcontroller::delete_controller(&block_id);

        // Not running here: refused, nothing opened.
        let resp = super::super::handle_agent_stop_pending_forward(
            axum::extract::State(state.clone()),
            axum::Json(super::super::AgentStopPendingForwardRequest { block_id: "nowhere".into(), signal: None, by: "Korp".into() }),
        )
        .await
        .into_response();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["success"], false);
        assert!(v["error"].as_str().unwrap().starts_with("NOT_RUNNING"));
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

/// `GET /api/v1/agent/shutdown/{id}` must reach its handler through the real
/// router: the MCP tools poll it for the user's answer (SPEC_AGENT_SELF_QUIT
/// §6.5). It was registered with axum 0.8's `{request_id}` syntax, a literal
/// path in axum 0.7, so every poll got a 404 from the router.
#[tokio::test]
async fn shutdown_status_route_reaches_its_handler() {
    let state = test_state();
    let v = crate::sagas::pending_shutdown::request(
        &state,
        "blk-status-route",
        "Korp",
        "FleetBulkStop",
        "",
        crate::sagas::pending_shutdown::Action::Stop { signal: None },
    );
    let app = build_router(state.clone());
    let req = Request::builder()
        .uri(format!("/api/v1/agent/shutdown/{}", v.request_id))
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["request_id"], v.request_id.as_str());
    assert_eq!(json["status"], "pending");

    // An unknown id is the handler's own 404, with its error body.
    let req = Request::builder()
        .uri("/api/v1/agent/shutdown/no-such-request")
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "no such shutdown request");
    crate::sagas::pending_shutdown::keep(&state, "blk-status-route", &v.request_id);
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

// ── POST /api/v1/agent/open (REPORT_AGENT_OPEN_API_GAP_2026_09_06.md) ────────
//
// The route exists so automation can START an agent, not just stop one
// (`/api/v1/fleet/bulk-stop`). These pin the wiring: the route is present,
// behind the same X-AuthKey gate as every other /api/v1 route, and maps the
// impl's caller-fault vocabulary (AGENT_NOT_FOUND) to 400 rather than 500.
// A full successful-open test needs a real agent definition + installed CLI
// + reducer round-trip and lives with the end-to-end coverage, not here.

#[tokio::test]
async fn agent_open_rejects_a_bad_auth_key() {
    let app = test_router();
    let req = Request::builder()
        .uri("/api/v1/agent/open")
        .method("POST")
        .header("X-AuthKey", "wrong-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"anything"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_open_unknown_agent_is_a_400_not_a_500() {
    let app = test_router();
    let req = Request::builder()
        .uri("/api/v1/agent/open")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"agent_id":"no-such-agent-xyz"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "an unknown agent id is the caller's fault, not a server error"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["error"].as_str().unwrap_or("").starts_with("AGENT_NOT_FOUND"),
        "error should carry the impl's AGENT_NOT_FOUND vocabulary, got: {}",
        json["error"]
    );
}

/// `agent.open` opens My Agents agents only (identity spec §6.5.8): a
/// template is the caller's fault — a 400 — and nothing is spawned.
#[tokio::test]
async fn agent_open_of_a_template_is_a_400() {
    let state = test_state();
    let mut tpl = crate::backend::storage::agents::test_agent_def(
        "tpl-open-test", "Template Open Test", "claude", "agent", 1, "",
    );
    tpl.is_seeded = 1;
    state.mstore.agent_def_insert(&mut tpl).expect("insert template");
    let app = build_router(state);
    for wanted in ["tpl-open-test", "template open test"] {
        let req = Request::builder()
            .uri("/api/v1/agent/open")
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "agent_id": wanted }).to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{wanted}");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["error"].as_str().unwrap_or("").starts_with("TEMPLATE_NOT_OPENABLE"),
            "{wanted}: {}",
            json["error"]
        );
    }
}

// ---------------------------------------------------------------------------
// PTY shell handlers (docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md)
// ---------------------------------------------------------------------------

async fn post_json(app: &Router, uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .uri(uri)
        .method(Method::POST)
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// `#[ignore]` (2026-09-15): this test's own comment below already notes it
/// spawns "a REAL, persistent interactive shell process (unlike every other
/// non-`#[ignore]`d test in this file, which never spawns one at all)" — it
/// should have followed this file's own convention for real-process tests
/// (e.g. `ptyshell_create_input_read_stop_round_trips_through_a_real_pty`
/// right below, `backend::container`'s Docker-gated test) from the start.
/// The gap was invisible before: `ci-pr.yml`'s Windows leg previously timed
/// out compiling `agentmux-cef` long before the test suite got this far.
/// Once this PR scoped that job to the CEF-free crates and let the suite
/// actually reach this test, it hung for 7+ minutes on GitHub-hosted
/// `windows-latest` runners specifically — passes instantly (0.11s) on a
/// local Windows machine,
/// never observed to hang on the (non-blocking) ubuntu-latest leg either.
/// Likely ConPTY needing a real interactive console session that a
/// GH-hosted Windows runner's non-interactive session doesn't provide, but
/// not confirmed — root-causing the actual hang is separate follow-up work,
/// not blocking on it here matches this file's existing policy for
/// real-process tests. Run manually with: `cargo test -p agentmux-srv --bin
/// agentmux-srv ptyshell_create_returns_a_shell_id_and_inserts_a_real_term_block
/// -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn ptyshell_create_returns_a_shell_id_and_inserts_a_real_term_block() {
    // Built from a kept `AppState` handle (not `test_router()`, which
    // discards its state) so the block the request created can be read back
    // afterward through the same store the request went through.
    let state = test_state();
    let app = build_router(state.clone());

    // A real UUID, not a human-readable fixture id: `create`'s success path
    // now persists `term:shellsubblockid` back onto the agent block via
    // `update_object_meta`, which parses its oref through `ORef::parse` —
    // strict about the oid being a real UUID (matching every genuine block
    // in the app, always minted via `Uuid::new_v4()`). Also has to actually
    // exist in the store, unlike the plain `mstore.get`-based checks
    // elsewhere in this file that tolerate a fixture id referring to
    // nothing at all.
    let agent_block_id = uuid::Uuid::new_v4().to_string();
    let mut agent_block = crate::backend::obj::Block {
        oid: agent_block_id.clone(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": agent_block_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let shell_id = json["shell_id"].as_str().expect("shell_id present").to_string();
    assert!(!shell_id.is_empty());

    // The block this created is a real `view: "term", controller: "shell"`
    // sub-block — same shape createsubblock's WS handler builds for
    // AgentShellSubblock.tsx — not a bespoke record type.
    let block: crate::backend::obj::Block =
        state.mstore.must_get(&shell_id).expect("block was inserted");
    assert_eq!(block.meta.get("view").and_then(|v| v.as_str()), Some("term"));
    assert_eq!(
        block.meta.get(blockcontroller::META_KEY_CONTROLLER).and_then(|v| v.as_str()),
        Some(blockcontroller::BLOCK_CONTROLLER_SHELL)
    );
    assert_eq!(block.parentoref, format!("block:{agent_block_id}"));

    // The pointer is persisted on the AGENT's own block too — this is what
    // lets a drawer opened afterward (or a human who already has one open)
    // attach to the same shell instead of getting an independent one.
    let agent_block: crate::backend::obj::Block =
        state.mstore.must_get(&agent_block_id).expect("agent block exists");
    assert_eq!(
        agent_block.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()),
        Some(shell_id.as_str())
    );

    // `create` spawns a REAL, persistent interactive shell process (unlike
    // every other non-`#[ignore]`d test in this file, which never spawns
    // one at all) — clean it up directly rather than leaving it running for
    // the rest of the test binary's life. Confirmed live: an earlier
    // version of this test (and `ptyshell_create_defaults_cwd_from_the_agent_block`
    // below) omitted this and left the CI Linux job's `cargo test --workspace`
    // step hanging for hours (a stuck run had to be cancelled). Calling the
    // backend function directly, not the HTTP `stop` endpoint, so this
    // cleanup can't itself be broken by a bug in that handler.
    blockcontroller::delete_controller(&shell_id);
}

/// `#[ignore]` (2026-09-15): same real-process-spawn hang on GitHub-hosted
/// `windows-latest` as `ptyshell_create_returns_a_shell_id_and_inserts_a_real_term_block`
/// above — see that test's doc comment for the full explanation. The
/// handler's own doc comment notes `resync_controller` runs for the reuse
/// path too ("Either way, resync_controller runs before returning"), so
/// this one spawns a real process just as much as the fresh-create path
/// does.
#[tokio::test]
#[ignore]
async fn ptyshell_create_reuses_the_pane_s_existing_shell_instead_of_spawning_a_second_one() {
    // The core of the visible-shell feature: whichever side (agent or
    // human-opened drawer) creates the shell first, the other attaches to
    // the SAME one. This simulates the "human already has the drawer open"
    // direction — a real `view:"term"` sub-block created the way
    // `AgentShellSubblock.tsx` does it (through `createsubblock`, not
    // `PtyShell`), with `term:shellsubblockid` already pointing at it on
    // the agent's own block, exactly as `agent-view.tsx`'s
    // `onSubBlockCreated` callback leaves it.
    let state = test_state();
    let app = build_router(state.clone());

    // Real UUIDs, not human-readable fixture ids: the agent block gets
    // written to via `update_object_meta` (parses its oref through
    // `ORef::parse`, strict about a real-UUID oid, matching every genuine
    // block in the app).
    let agent_block_id = uuid::Uuid::new_v4().to_string();
    let human_shell_id = uuid::Uuid::new_v4().to_string();

    let mut agent_block = crate::backend::obj::Block {
        oid: agent_block_id.clone(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let mut human_shell = crate::backend::obj::Block {
        oid: human_shell_id.clone(),
        parentoref: format!("block:{agent_block_id}"),
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("term"));
            m.insert(
                blockcontroller::META_KEY_CONTROLLER.to_string(),
                serde_json::json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
            );
            m
        },
        ..Default::default()
    };
    state.mstore.insert(&mut human_shell).expect("insert human shell block");
    let mut agent_meta = crate::backend::obj::MetaMapType::new();
    agent_meta.insert(META_KEY_SHELL_SUBBLOCK_ID.to_string(), serde_json::json!(human_shell_id));
    crate::server::service::object_helpers::update_object_meta(
        &state.mstore,
        &format!("block:{agent_block_id}"),
        &agent_meta,
    )
    .expect("point agent block at the human's shell");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": agent_block_id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["shell_id"].as_str(),
        Some(human_shell_id.as_str()),
        "PtyShell must attach to the pane's existing shell, not create a new one"
    );

    // The agent block's pointer is unchanged — `create` didn't mint a
    // second, independent shell and repoint it there instead of reusing.
    let agent_block: crate::backend::obj::Block = state.mstore.must_get(&agent_block_id).unwrap();
    assert_eq!(
        agent_block.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()),
        Some(human_shell_id.as_str())
    );

    blockcontroller::delete_controller(&human_shell_id);
}

#[tokio::test]
async fn ptyshell_rejects_operating_on_a_block_outside_the_calling_agents_own_pane() {
    // Codex P1 on PR #3177, reframed after the reuse feature (see
    // `is_owned_by_agent`'s doc comment): `shell_id` lives in the SAME
    // namespace as every other pane block, and `Layout` exposes block ids
    // for ordinary panes — without this check, an agent could point
    // PtyShellInput/PtyShellStop/etc. at a completely unrelated pane
    // (another agent's own CLI pane, a human's terminal). This inserts a
    // real "shell"-controller block parented to a DIFFERENT agent and
    // asserts every mutating/reading endpoint refuses to touch it when the
    // request claims a different `agent_block_id`.
    let state = test_state();
    let app = build_router(state.clone());

    let mut foreign = crate::backend::obj::Block {
        oid: "someone-elses-real-pane".to_string(),
        parentoref: "block:someone-elses-agent".to_string(),
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("term"));
            m.insert(
                blockcontroller::META_KEY_CONTROLLER.to_string(),
                serde_json::json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
            );
            m
        },
        ..Default::default()
    };
    state.mstore.insert(&mut foreign).expect("insert foreign block");

    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": "someone-elses-real-pane", "agent_block_id": "attacking-agent", "text": "hi" }),
    )
    .await;
    assert_eq!(json["written"], serde_json::json!(false));
    assert!(json["error"].as_str().unwrap_or("").contains("not a shell belonging to this agent's own pane"));

    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/resize",
        serde_json::json!({ "shell_id": "someone-elses-real-pane", "agent_block_id": "attacking-agent", "rows": 30, "cols": 100 }),
    )
    .await;
    assert_eq!(json["resized"], serde_json::json!(false));

    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/read",
        serde_json::json!({ "shell_id": "someone-elses-real-pane", "agent_block_id": "attacking-agent" }),
    )
    .await;
    assert_eq!(json["content"], serde_json::json!(""));

    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/stop",
        serde_json::json!({ "shell_id": "someone-elses-real-pane", "agent_block_id": "attacking-agent" }),
    )
    .await;
    assert_eq!(json["released"], serde_json::json!(false));

    // The foreign block must still exist and be untouched.
    let still_there: Result<crate::backend::obj::Block, _> =
        state.mstore.must_get("someone-elses-real-pane");
    assert!(still_there.is_ok(), "ptyshell/* must not delete a block outside the caller's own pane");

    // The LEGITIMATE owner (matching agent_block_id) can still act on it —
    // proves the rejection above is about ownership, not a broken check.
    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/resize",
        serde_json::json!({ "shell_id": "someone-elses-real-pane", "agent_block_id": "someone-elses-agent", "rows": 30, "cols": 100 }),
    )
    .await;
    assert_eq!(json["resized"], serde_json::json!(false), "no live controller yet, but ownership check must pass");
    assert!(!json["error"].as_str().unwrap_or("").contains("not a shell belonging"));
}

/// `#[ignore]` (2026-09-15): same real-process-spawn hang on GitHub-hosted
/// `windows-latest` as `ptyshell_create_returns_a_shell_id_and_inserts_a_real_term_block`
/// above — see that test's doc comment for the full explanation. This one
/// hits the identical `/api/v1/ptyshell/create` code path.
#[tokio::test]
#[ignore]
async fn ptyshell_create_defaults_cwd_from_the_agent_block() {
    // Codex P1 on PR #3177: without this fallback, the PTY spawns in
    // agentmux-srv's own cwd rather than the agent's worktree — mirrors
    // `handle_shell_create`'s identical fallback.
    let state = test_state();
    let app = build_router(state.clone());

    let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
    let mut agent_block = crate::backend::obj::Block {
        oid: "agent-with-a-cwd".to_string(),
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert("cmd:cwd".to_string(), serde_json::json!(cwd.clone()));
            m
        },
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": "agent-with-a-cwd" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let shell_id = json["shell_id"].as_str().unwrap().to_string();

    let block: crate::backend::obj::Block = state.mstore.must_get(&shell_id).unwrap();
    assert_eq!(
        block.meta.get(blockcontroller::META_KEY_CMD_CWD).and_then(|v| v.as_str()),
        Some(cwd.as_str())
    );

    // See the identical cleanup note in
    // `ptyshell_create_returns_a_shell_id_and_inserts_a_real_term_block`.
    blockcontroller::delete_controller(&shell_id);
}

#[tokio::test]
async fn ptyshell_status_on_unknown_id_is_a_plain_not_running_answer() {
    let app = test_router();
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/status",
        serde_json::json!({ "shell_id": "no-such-shell", "agent_block_id": "whoever" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["running"], serde_json::json!(false));
    assert!(json["exit_code"].is_null());
}

#[tokio::test]
async fn ptyshell_input_on_unknown_id_reports_written_false_with_a_reason() {
    let app = test_router();
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": "no-such-shell", "agent_block_id": "whoever", "text": "hello" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["written"], serde_json::json!(false));
    // Caught by the ownership check before ever reaching
    // `blockcontroller::send_input` — an unrecognized id is
    // indistinguishable from "not this agent's own pane" by design.
    assert!(json["error"].as_str().unwrap_or("").contains("not a shell belonging to this agent's own pane"));
}

#[tokio::test]
async fn ptyshell_stop_on_unknown_id_reports_not_released() {
    let app = test_router();
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/stop",
        serde_json::json!({ "shell_id": "no-such-shell", "agent_block_id": "whoever" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["released"], serde_json::json!(false));
}

#[tokio::test]
async fn ptyshell_input_locks_the_shell_and_stop_releases_it_early() {
    // The locking feature: input/resize stamp an expiry timestamp the
    // frontend reads to gate human keyboard input, BEFORE attempting the
    // write itself (ReAgent P1 on PR #3194 — locking only on success left
    // the lock unset for the write's entire duration, exactly the window
    // controllerinput's own lock check needs it to already exist to catch
    // anything). PtyShellStop clears it immediately rather than waiting
    // for AGENT_LOCK_WINDOW_MS to lapse.
    let state = test_state();
    let app = build_router(state.clone());

    // Real UUIDs — the lock write goes through `update_object_meta`, strict
    // about a real-UUID oid (see the identical note on the reuse test above).
    let agent_block_id = uuid::Uuid::new_v4().to_string();
    let shell_id = uuid::Uuid::new_v4().to_string();

    let mut agent_block = crate::backend::obj::Block {
        oid: agent_block_id.clone(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");
    let mut shell = crate::backend::obj::Block {
        oid: shell_id.clone(),
        parentoref: format!("block:{agent_block_id}"),
        meta: {
            let mut m = crate::backend::obj::MetaMapType::new();
            m.insert("view".to_string(), serde_json::json!("term"));
            m
        },
        ..Default::default()
    };
    state.mstore.insert(&mut shell).expect("insert shell block");

    let before = agentmux_common::time::now_ms();
    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": agent_block_id, "text": "hi" }),
    )
    .await;
    // No live controller for this synthetic block, so the PTY write itself
    // fails — but the ownership check passed (a different error, not the
    // "not a shell belonging..." rejection), which is what proves the lock
    // write below came from the REAL request path, not a bypassed check.
    assert!(!json["error"].as_str().unwrap_or("").contains("not a shell belonging"));

    // The lock must be set by the real HTTP call itself — unconditionally,
    // before the (here, failing) write is even attempted — not merely
    // something this test simulates after the fact.
    let locked_block: crate::backend::obj::Block = state.mstore.must_get(&shell_id).unwrap();
    let until = locked_block.meta.get(META_KEY_AGENT_LOCK_UNTIL).and_then(|v| v.as_i64()).unwrap();
    assert!(until > before, "lock expiry must be in the future");
    assert!(until <= before + AGENT_LOCK_WINDOW_MS + 1000, "lock window should be short, not indefinite");

    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/stop",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": agent_block_id }),
    )
    .await;
    assert_eq!(json["released"], serde_json::json!(true));

    let unlocked_block: crate::backend::obj::Block = state.mstore.must_get(&shell_id).unwrap();
    assert!(
        unlocked_block.meta.get(META_KEY_AGENT_LOCK_UNTIL).is_none(),
        "stop must clear the lock, not just let it expire"
    );

    // The block itself must still exist — stop() never deletes it (this
    // shell is shared/pane-scoped now, not a private disposable one).
    assert!(state.mstore.get::<crate::backend::obj::Block>(&shell_id).unwrap().is_some());
}

/// Real end-to-end PTY spawn: create a shell, type a command, read the
/// output back, confirm it actually ran. `#[ignore]` because it spawns a
/// genuine OS process (matches this codebase's existing convention for
/// real-process/real-daemon tests, e.g. `backend::container`'s Docker-gated
/// test) rather than the mocked `ConnInterface` every other PTY test in
/// `blockcontroller::shell::tests` uses — that mocking happens at the
/// `ShellController` level, which this test intentionally exercises through
/// its real HTTP handlers end-to-end instead.
///
/// Run manually with: `cargo test -p agentmux-srv --bin agentmux-srv
/// ptyshell_create_input_read_stop_round_trips_through_a_real_pty --
/// --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn ptyshell_create_input_read_stop_round_trips_through_a_real_pty() {
    // Kept `AppState` handle (not `test_router()`) so the lock meta can be
    // asserted directly against the store below.
    let state = test_state();
    let app = build_router(state.clone());

    // A real block, not just a fixture id: `create`'s atomic claim-or-lose
    // step (Codex P2 on PR #3194) does a hard `must_get` on the agent
    // block to check/set its shellsubblockid pointer inside one
    // transaction — unlike the old best-effort parent-link, a genuinely
    // missing agent block is now a real error, matching every actual
    // caller in production (agent_block_id always names a real, live pane).
    let mut agent_block = crate::backend::obj::Block {
        oid: "test-agent-block".to_string(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let shell_id = json["shell_id"].as_str().unwrap().to_string();

    // Give the PTY a moment to spawn its shell before typing into it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let marker = "ptyshell-live-test-marker-4f9a";
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block", "text": format!("echo {marker}\r\n") }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["written"], serde_json::json!(true), "input write failed: {json:?}");

    // A successful write must lock the shell out from human input — the
    // whole point of the feature. Assert it directly against the store
    // rather than adding a frontend round trip to this backend test.
    let locked_block: crate::backend::obj::Block = state.mstore.must_get(&shell_id).unwrap();
    let lock_until = locked_block.meta.get(META_KEY_AGENT_LOCK_UNTIL).and_then(|v| v.as_i64());
    assert!(
        lock_until.is_some_and(|until| until > agentmux_common::time::now_ms()),
        "a successful PtyShellInput must set a future term:agentlockuntil"
    );

    // Poll rather than a single fixed sleep — shell startup + echo latency
    // varies by machine/OS.
    let mut content = String::new();
    for i in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, json) = post_json(
            &app,
            "/api/v1/ptyshell/read",
            serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block" }),
        )
        .await;
        content = json["content"].as_str().unwrap_or("").to_string();
        let (_, status_json) = post_json(
            &app,
            "/api/v1/ptyshell/status",
            serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block" }),
        )
        .await;
        eprintln!("[poll {i}] status={status_json:?} content={content:?}");
        if content.contains(marker) {
            break;
        }
    }
    assert!(
        content.contains(marker),
        "expected echoed marker in PTY output, got: {content:?}"
    );

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/status",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["running"], serde_json::json!(true));

    // stop() releases the lock but must NOT kill the shell — it's shared
    // with whatever human drawer might attach to it later.
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/stop",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["released"], serde_json::json!(true));

    let (_, json) = post_json(
        &app,
        "/api/v1/ptyshell/status",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(json["running"], serde_json::json!(true), "stop must not kill the shared shell");

    // Actually kill it now, for real, so this test doesn't leave a live
    // process running for the rest of the binary's life — see the
    // identical cleanup note on the other real-spawn tests in this file.
    blockcontroller::delete_controller(&shell_id);
}

/// Regression test for the "typing `exit` loops the pane" bug
/// (`docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md`): once a pane's
/// shell has actually exited, `PtyShellCreate` must NOT silently respawn it
/// in place — `try_attach_to_existing_shell` used to call
/// `resync_controller` unconditionally, which respawns any `STATUS_DONE`
/// controller, so every later `PtyShellCreate` against the same pane kept
/// resurrecting the same block and appending a fresh shell-startup banner
/// to its already-`exit`-terminated, append-only `term` scrollback —
/// visually indistinguishable from the exited shell's own output looping.
/// The fix: treat an exited shell like a stale pointer and fall through to
/// creating a genuinely fresh shell block instead.
///
/// `#[ignore]` for the same reason as the other real-PTY tests in this
/// file (see `ptyshell_create_input_read_stop_round_trips_through_a_real_pty`'s
/// doc comment) — spawns a real, interactive shell process, which hangs on
/// GitHub-hosted `windows-latest` runners. Run manually with: `cargo test
/// -p agentmux-srv --bin agentmux-srv
/// ptyshell_create_does_not_respawn_a_shell_that_already_exited --
/// --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn ptyshell_create_does_not_respawn_a_shell_that_already_exited() {
    let state = test_state();
    let app = build_router(state.clone());

    let mut agent_block = crate::backend::obj::Block {
        oid: "test-agent-block".to_string(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_shell_id = json["shell_id"].as_str().unwrap().to_string();

    // Give the PTY a moment to spawn its shell before typing into it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": first_shell_id, "agent_block_id": "test-agent-block", "text": "exit\r\n" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["written"], serde_json::json!(true), "input write failed: {json:?}");

    // Poll until the controller actually reports done — shell teardown
    // latency varies by machine/OS, same as the round-trip test above.
    // Budgeted generously (was 40x250ms=10s, uncomfortably tight): STATUS_DONE
    // isn't published until the wait/cleanup task's flusher-drain wait
    // resolves, and that hits its full 10s `FLUSHER_DRAIN_TIMEOUT` on
    // essentially every ordinary `cmd.exe` exit on Windows (discovered live
    // while adding `ptyshell_shell_pane_closes_itself_after_exit` — see its
    // own comment, and the close-on-exit trigger's comment in lifecycle.rs).
    let mut done = false;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let (_, status_json) = post_json(
            &app,
            "/api/v1/ptyshell/status",
            serde_json::json!({ "shell_id": first_shell_id, "agent_block_id": "test-agent-block" }),
        )
        .await;
        if status_json["running"] == serde_json::json!(false) {
            done = true;
            break;
        }
    }
    assert!(done, "expected the shell to report not-running after `exit`");

    // The pane's one shell already exited — a subsequent PtyShellCreate
    // must create a fresh shell, not respawn the dead one in place.
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second_shell_id = json["shell_id"].as_str().unwrap().to_string();
    assert_ne!(
        second_shell_id, first_shell_id,
        "PtyShellCreate must not silently respawn an already-exited shell in the same block"
    );

    // The pane's pointer must now follow the fresh shell, not the dead one.
    let agent_block: crate::backend::obj::Block =
        state.mstore.must_get("test-agent-block").expect("agent block exists");
    assert_eq!(
        agent_block.meta.get(META_KEY_SHELL_SUBBLOCK_ID).and_then(|v| v.as_str()),
        Some(second_shell_id.as_str())
    );

    blockcontroller::delete_controller(&first_shell_id);
    blockcontroller::delete_controller(&second_shell_id);
}

/// A shell that has EXITED must not still hold an agent lease over the
/// human's keyboard (`blockcontroller::agent_lock`, wired from the
/// wait/cleanup task in `shell/lifecycle.rs`).
///
/// Why this matters beyond tidiness: the lease drops human `controllerinput`
/// keystrokes outright. Leaving one attached to a dead PTY means the pane
/// silently swallows whatever the human types next — including `exit` itself,
/// the one key sequence the close-on-exit work
/// (`SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §10) exists to honour.
///
/// The lease seeded here is deliberately far longer than the real
/// `AGENT_LOCK_WINDOW_MS` (4s). A real lease would lapse on its own during
/// the multi-second wait for the shell to be reaped below, so this test
/// would pass with the release wiring deleted — it would be measuring the
/// clock, not the fix. A 10-minute lease can only be cleared by the exit
/// path itself.
///
/// `#[ignore]`d like this file's other real-PTY tests (they hang on
/// GitHub-hosted `windows-latest` runners; tracked, unrelated cause). Run
/// manually: `cargo test -p agentmux-srv --bin agentmux-srv
/// an_exited_shell_does_not_keep_the_agent_lease -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn an_exited_shell_does_not_keep_the_agent_lease() {
    let state = test_state();
    let app = build_router(state.clone());

    let mut agent_block = crate::backend::obj::Block {
        oid: "lease-exit-agent-block".to_string(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": "lease-exit-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let shell_id = json["shell_id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // The agent types `exit` into the shell it's driving. This write takes a
    // real lease through the production path (`lock_shell_for_agent`) as a
    // side effect — proving the lease is genuinely live at the moment the
    // shell starts going away, not just something this test invented.
    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": "lease-exit-agent-block", "text": "exit\r\n" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["written"], serde_json::json!(true), "input write failed: {json:?}");
    assert!(
        blockcontroller::agent_lock::is_locked(&shell_id),
        "precondition: an agent write must take a lease"
    );

    // Extend it past the reap wait — see the doc comment.
    blockcontroller::agent_lock::lock_until(&shell_id, agentmux_common::time::now_ms() + 600_000);

    let mut done = false;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let (_, status_json) = post_json(
            &app,
            "/api/v1/ptyshell/status",
            serde_json::json!({ "shell_id": shell_id, "agent_block_id": "lease-exit-agent-block" }),
        )
        .await;
        if status_json["running"] == serde_json::json!(false) {
            done = true;
            break;
        }
    }
    assert!(done, "expected the shell to report not-running after `exit`");

    assert!(
        !blockcontroller::agent_lock::is_locked(&shell_id),
        "an exited shell must release its agent lease, or the pane keeps dropping the human's keystrokes"
    );

    blockcontroller::agent_lock::release(&shell_id);
    blockcontroller::delete_controller(&shell_id);
}

/// Regression test for close-on-exit's PARENT exclusion
/// (`docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §11): a
/// `PtyShellCreate`-spawned block is always a SUB-block (`parentoref`
/// points at the agent block that owns it) — an agent's composer-drawer
/// shell or a headless agent-driven shell, not an independent top-level
/// pane. Closing one of these via `sagas::delete_block::run` leaves the
/// parent's own `term:shellsubblockid` pointer dangling (unrelated to,
/// and untouched by, that generic saga), and the drawer's own "attach or
/// create" logic then silently recreates a fresh shell moments later —
/// reproducing the original respawn loop via a different path, with EXTRA
/// latency, not fixing anything. Live-tested and confirmed exactly this
/// regression before this exclusion was added (§11's own writeup).
///
/// Installs a LOCAL close-on-exit handler directly via
/// `blockcontroller::set_close_on_exit_handler`, bypassing
/// `bootstrap::install_close_on_exit_handler` (needs a full saga-capable
/// `AppState` — `main.rs`'s real boot sequence, not `test_state()`). This
/// verifies `lifecycle.rs`'s decision logic specifically — that it does
/// NOT call the hook at all for a parented block — independent of
/// `sagas::delete_block::run`'s own separately-tested correctness.
/// See `shell_pane_with_no_parent_closes_itself_after_exit` (below) for
/// the positive case (a real top-level pane DOES close).
///
/// `set_close_on_exit_handler`'s `OnceLock` is PROCESS-global and can only
/// be set once — once this test installs its handler, it stays installed
/// for the rest of this test binary's life. Like this file's other
/// real-PTY tests, run this one individually
/// (`cargo test -p agentmux-srv --bin agentmux-srv
/// ptyshell_create_does_not_close_the_parented_shell_pane_after_exit --
/// --ignored --nocapture`), not as part of a bulk `--ignored` run alongside
/// some future test that also wants to install its own handler.
#[tokio::test]
#[ignore]
async fn ptyshell_create_does_not_close_the_parented_shell_pane_after_exit() {
    let state = test_state();
    let app = build_router(state.clone());

    let mut agent_block = crate::backend::obj::Block {
        oid: "test-agent-block".to_string(),
        ..Default::default()
    };
    state.mstore.insert(&mut agent_block).expect("insert agent block");

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/create",
        serde_json::json!({ "agent_block_id": "test-agent-block" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let shell_id = json["shell_id"].as_str().unwrap().to_string();

    // Sanity: this block really is parented — the property the exclusion
    // is keyed on.
    let shell_block: crate::backend::obj::Block = state.mstore.must_get(&shell_id).unwrap();
    assert_eq!(shell_block.parentoref, "block:test-agent-block");

    let closed: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let closed_for_handler = closed.clone();
    blockcontroller::set_close_on_exit_handler(std::sync::Arc::new(
        move |tab_id: String, block_id: String| {
            *closed_for_handler.lock().unwrap() = Some((tab_id, block_id));
        },
    ));

    // Give the PTY a moment to spawn its shell before typing into it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let (status, json) = post_json(
        &app,
        "/api/v1/ptyshell/input",
        serde_json::json!({ "shell_id": shell_id, "agent_block_id": "test-agent-block", "text": "exit\r\n" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["written"], serde_json::json!(true), "input write failed: {json:?}");

    // Budget generously — see the sibling test's comment on why (the
    // wait/cleanup task's flusher-drain wait routinely eats the full 10s
    // `FLUSHER_DRAIN_TIMEOUT` on Windows before the close-on-exit decision
    // is even reached).
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    assert!(
        closed.lock().unwrap().is_none(),
        "close-on-exit must NOT fire for a parented (sub-block) shell pane"
    );

    blockcontroller::delete_controller(&shell_id);
}

/// Regression test for close-on-exit's positive case
/// (`docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §10): a real
/// top-level pane (no `parentoref` — the `Terminal` widget, opened
/// directly in a tab, matching how the WS `ControllerResyncCommand` path
/// creates one) DOES close itself once its shell exits. Drives the
/// controller directly via `blockcontroller::resync_controller` +
/// `send_input` (not the `/api/v1/ptyshell/*` HTTP family, which always
/// creates a PARENTED sub-block — see the sibling test above) to get a
/// genuinely parent-less block with a REAL tab id, matching what a plain
/// Terminal pane looks like in production.
///
/// Same process-global `OnceLock` caveat as the sibling test — run this
/// one individually too
/// (`cargo test -p agentmux-srv --bin agentmux-srv
/// shell_pane_with_no_parent_closes_itself_after_exit -- --ignored
/// --nocapture`).
#[tokio::test]
#[ignore]
async fn shell_pane_with_no_parent_closes_itself_after_exit() {
    let state = test_state();

    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("term"));
    meta.insert(
        blockcontroller::META_KEY_CONTROLLER.to_string(),
        serde_json::json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
    );
    #[cfg(windows)]
    {
        meta.insert(blockcontroller::META_KEY_CMD.to_string(), serde_json::json!("cmd.exe"));
        meta.insert("cmd:interactive".to_string(), serde_json::json!(true));
    }
    let mut block = crate::backend::obj::Block {
        oid: "test-toplevel-shell".to_string(),
        // A REAL top-level pane is parented to its TAB, not to nothing —
        // `wcore::block` / `persist_subscriber` both write
        // `format!("tab:{tab_id}")`. An earlier version of this test used
        // `String::new()` here, which never occurs in production and let a
        // `!parentoref.is_empty()` bug ship that disabled close-on-exit
        // for every real pane (§12). Keep this realistic.
        parentoref: "tab:test-real-tab-1".to_string(),
        meta,
        ..Default::default()
    };
    state.mstore.insert(&mut block).expect("insert block");

    let closed: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let closed_for_handler = closed.clone();
    blockcontroller::set_close_on_exit_handler(std::sync::Arc::new(
        move |tab_id: String, block_id: String| {
            *closed_for_handler.lock().unwrap() = Some((tab_id, block_id));
        },
    ));

    let registry = state.mstore.shared_agent_registry();
    blockcontroller::resync_controller(
        &block,
        "test-real-tab-1",
        None,
        false,
        true,
        Some(state.broker.clone()),
        Some(state.event_bus.clone()),
        Some(state.mstore.clone()),
        Some(state.filestore.clone()),
        None,
        None,
        registry,
        state.boot_id.clone(),
        &state.auth_key,
        Some(state.config_watcher.clone()),
    )
    .expect("resync/start shell");

    // Give the PTY a moment to spawn its shell before typing into it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // cmd.exe queries cursor position (`ESC[6n`, Device Status Report) on
    // interactive startup and blocks waiting for a reply — normally
    // answered automatically by a real terminal emulator (xterm.js, over
    // the WS path a genuine Terminal pane uses) or by
    // `answer_conpty_handshake_if_seen` (the `/api/v1/ptyshell/*` HTTP
    // family's own answer for the same query, since those callers have no
    // frontend attached either). This test has neither, so it answers the
    // query itself the same way: `ESC[1;1R`, a Cursor Position Report.
    blockcontroller::send_input(
        "test-toplevel-shell",
        blockcontroller::BlockInputUnion::data(b"\x1b[1;1R".to_vec()),
        None,
    )
    .expect("answer cursor-position query");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    blockcontroller::send_input(
        "test-toplevel-shell",
        blockcontroller::BlockInputUnion::data(b"exit\r\n".to_vec()),
        None,
    )
    .expect("send exit");

    // Budget generously — see the sibling test's comment on why
    // (flusher-drain-timeout on Windows).
    let mut fired = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Some(v) = closed.lock().unwrap().clone() {
            fired = Some(v);
            break;
        }
    }
    let (fired_tab_id, fired_block_id) =
        fired.expect("close-on-exit handler should have fired for a parent-less shell pane");
    assert_eq!(fired_tab_id, "test-real-tab-1");
    assert_eq!(fired_block_id, "test-toplevel-shell");

    blockcontroller::delete_controller("test-toplevel-shell");
}

/// Codex P1 on PR #3230: a process exit is not by itself grounds to close
/// the pane — the child also dies when the controller is deliberately
/// REPLACED. `resync_controller`'s force path (the terminal's own refresh
/// action, `forcerestart: true`) calls `stop_for_replace`, killing this
/// child, then registers a replacement for the same block. The old wait
/// task still carries `close_on_exit == true`, so without the
/// still-the-current-controller check it would reap that killed child and
/// delete the pane — taking the freshly registered replacement with it —
/// instead of completing the restart.
///
/// Same real-PTY caveats and process-global `OnceLock` caveat as the
/// sibling tests above; run individually
/// (`cargo test -p agentmux-srv --bin agentmux-srv
/// force_restart_does_not_close_the_pane -- --ignored --nocapture`).
#[tokio::test]
#[ignore]
async fn force_restart_does_not_close_the_pane_via_the_old_controllers_exit() {
    let state = test_state();

    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("term"));
    meta.insert(
        blockcontroller::META_KEY_CONTROLLER.to_string(),
        serde_json::json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
    );
    #[cfg(windows)]
    {
        meta.insert(blockcontroller::META_KEY_CMD.to_string(), serde_json::json!("cmd.exe"));
        meta.insert("cmd:interactive".to_string(), serde_json::json!(true));
    }
    let mut block = crate::backend::obj::Block {
        oid: "test-restart-shell".to_string(),
        parentoref: "tab:test-restart-tab".to_string(),
        meta,
        ..Default::default()
    };
    state.mstore.insert(&mut block).expect("insert block");

    let closed: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let closed_for_handler = closed.clone();
    blockcontroller::set_close_on_exit_handler(std::sync::Arc::new(
        move |tab_id: String, block_id: String| {
            *closed_for_handler.lock().unwrap() = Some((tab_id, block_id));
        },
    ));

    let start = |force: bool| {
        blockcontroller::resync_controller(
            &block,
            "test-restart-tab",
            None,
            force,
            true,
            Some(state.broker.clone()),
            Some(state.event_bus.clone()),
            Some(state.mstore.clone()),
            Some(state.filestore.clone()),
            None,
            None,
            state.mstore.shared_agent_registry(),
            state.boot_id.clone(),
            &state.auth_key,
            Some(state.config_watcher.clone()),
        )
    };

    start(false).expect("initial resync/start");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Force-restart: kills this child and registers a replacement — the
    // exact sequence the terminal's refresh action drives.
    start(true).expect("force restart");

    // Well past the old child's reap + the (0ms default) close delay.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    assert!(
        closed.lock().unwrap().is_none(),
        "a force-restart must NOT close the pane — the old controller's child \
         dying is a replacement, not a natural exit"
    );

    blockcontroller::delete_controller("test-restart-shell");
}

/// A plain Terminal-widget pane's shell must receive AGENTMUX_AUTH_KEY (not
/// just AGENTMUX_LOCAL_URL) so a human can run `muxsh`/`muxspect`/`muxopen`/
/// `muxlog` from it — the same role Wave Terminal's `wsh` played, and
/// exactly what those tools' own help text and
/// docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md promise ("run from a
/// pane AgentMux opened, agent or shell"). Before this fix, `ShellController`
/// never injected AGENTMUX_AUTH_KEY at all — only agent panes got it, via
/// PersistentSpawnConfig/SubprocessSpawnConfig's own env_vars — so muxsh
/// failed from every plain Terminal pane with "AGENTMUX_LOCAL_URL /
/// AGENTMUX_AUTH_KEY not set" even though it was being run exactly as
/// documented.
///
/// Same real-PTY caveats and process-global `OnceLock` caveat as the sibling
/// tests above; run individually
/// (`cargo test -p agentmux-srv --bin agentmux-srv
/// plain_terminal_pane_shell_receives_the_instance_auth_key -- --ignored
/// --nocapture`).
#[tokio::test]
#[ignore]
async fn plain_terminal_pane_shell_receives_the_instance_auth_key() {
    let state = test_state();

    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("term"));
    meta.insert(
        blockcontroller::META_KEY_CONTROLLER.to_string(),
        serde_json::json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
    );
    #[cfg(windows)]
    {
        meta.insert(blockcontroller::META_KEY_CMD.to_string(), serde_json::json!("cmd.exe"));
        meta.insert("cmd:interactive".to_string(), serde_json::json!(true));
    }
    let mut block = crate::backend::obj::Block {
        oid: "test-authkey-shell".to_string(),
        parentoref: "tab:test-authkey-tab".to_string(),
        meta,
        ..Default::default()
    };
    state.mstore.insert(&mut block).expect("insert block");

    let registry = state.mstore.shared_agent_registry();
    blockcontroller::resync_controller(
        &block,
        "test-authkey-tab",
        None,
        false,
        true,
        Some(state.broker.clone()),
        Some(state.event_bus.clone()),
        Some(state.mstore.clone()),
        Some(state.filestore.clone()),
        None,
        None,
        registry,
        state.boot_id.clone(),
        &state.auth_key,
        Some(state.config_watcher.clone()),
    )
    .expect("resync/start shell");

    // Give the PTY a moment to spawn its shell before typing into it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Same cursor-position-query answer as the sibling tests (cmd.exe only).
    #[cfg(windows)]
    {
        blockcontroller::send_input(
            "test-authkey-shell",
            blockcontroller::BlockInputUnion::data(b"\x1b[1;1R".to_vec()),
            None,
        )
        .expect("answer cursor-position query");
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Dump every AGENTMUX_* var so it lands in the pane's own scrollback,
    // the same place a human's `muxsh` invocation would read it from.
    // Deliberately `env`/`set` (not `printenv NAME` or `echo %NAME%`) — the
    // point is proving AGENTMUX_AUTH_KEY is actually present in the child's
    // real environment, not just testing one specific shell builtin's
    // handling of an unset variable.
    #[cfg(windows)]
    let print_cmd: &[u8] = b"set | findstr AGENTMUX\r\n";
    #[cfg(not(windows))]
    let print_cmd: &[u8] = b"env | grep AGENTMUX\r\n";
    blockcontroller::send_input(
        "test-authkey-shell",
        blockcontroller::BlockInputUnion::data(print_cmd.to_vec()),
        None,
    )
    .expect("send env dump");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let scrollback = state
        .filestore
        .read_file("test-authkey-shell", "term")
        .expect("read term scrollback")
        .unwrap_or_default();
    let scrollback = String::from_utf8_lossy(&scrollback);
    assert!(
        scrollback.contains(&state.auth_key),
        "the shell's own env should have included AGENTMUX_AUTH_KEY=\"{}\" \
         into its scrollback, got: {scrollback:?}",
        state.auth_key
    );

    blockcontroller::delete_controller("test-authkey-shell");
}

/// The global `cmd:env` setting reaches a real Terminal pane's shell, through
/// the same `resync_controller` → `ShellController::start` path a pane takes
/// in production, with the block's own `cmd:env` still winning and a global
/// `AGENTMUX_AGENT_ID` still kept out. Before the fix the spawn read a fresh
/// default config instead of `AppState::config_watcher`, so no global value
/// ever arrived.
///
/// Uses the interactive-shell branch (no `cmd`), the only branch that applies
/// `cmd:env`: `$SHELL` on Unix, PowerShell on Windows. Same real-PTY and
/// process-global `OnceLock` caveats as the sibling tests above; run
/// individually (`cargo test -p agentmux-srv --bin agentmux-srv
/// terminal_pane_shell_receives_the_global_cmd_env_setting -- --ignored
/// --nocapture`).
#[tokio::test]
#[ignore]
async fn terminal_pane_shell_receives_the_global_cmd_env_setting() {
    let state = test_state();
    let block_id = "test-global-cmd-env-shell";

    // Loaded the way `config_watcher_fs` loads a saved settings.json.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(wconfig::SETTINGS_FILE);
    std::fs::write(
        &path,
        r#"{ "cmd:env": {
            "AMUX_E2E_GLOBAL": "global-value-7f3a",
            "AMUX_E2E_SHARED": "global-loses-1d8e",
            "AGENTMUX_AGENT_ID": "global-id-leak-5e0b"
        } }"#,
    )
    .unwrap();
    let (settings, errors): (wconfig::SettingsType, _) = wconfig::read_config_file(&path);
    assert!(errors.is_empty(), "{errors:?}");
    state.config_watcher.update_settings(settings);

    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("term"));
    meta.insert(
        blockcontroller::META_KEY_CONTROLLER.to_string(),
        serde_json::json!(blockcontroller::BLOCK_CONTROLLER_SHELL),
    );
    meta.insert(
        blockcontroller::META_KEY_CMD_ENV.to_string(),
        serde_json::json!({ "AMUX_E2E_SHARED": "block-wins-91c2" }),
    );
    let mut block = crate::backend::obj::Block {
        oid: block_id.to_string(),
        parentoref: "tab:test-global-cmd-env-tab".to_string(),
        meta,
        ..Default::default()
    };
    state.mstore.insert(&mut block).expect("insert block");

    let registry = state.mstore.shared_agent_registry();
    blockcontroller::resync_controller(
        &block,
        "test-global-cmd-env-tab",
        None,
        false,
        true,
        Some(state.broker.clone()),
        Some(state.event_bus.clone()),
        Some(state.mstore.clone()),
        Some(state.filestore.clone()),
        None,
        None,
        registry,
        state.boot_id.clone(),
        &state.auth_key,
        Some(state.config_watcher.clone()),
    )
    .expect("resync/start shell");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // Answer ConPTY's startup cursor-position query, as the siblings do.
    #[cfg(windows)]
    blockcontroller::send_input(
        block_id,
        blockcontroller::BlockInputUnion::data(b"\x1b[1;1R".to_vec()),
        None,
    )
    .expect("answer cursor-position query");

    // The values never appear in the typed command itself, so finding them
    // in the scrollback means the shell's own environment had them.
    #[cfg(windows)]
    let print_cmd: &[u8] =
        b"echo \"[$env:AMUX_E2E_GLOBAL|$env:AMUX_E2E_SHARED|$env:AGENTMUX_AGENT_ID]\"\r\n";
    #[cfg(not(windows))]
    let print_cmd: &[u8] = b"echo \"[$AMUX_E2E_GLOBAL|$AMUX_E2E_SHARED|$AGENTMUX_AGENT_ID]\"\r\n";
    let expected = "[global-value-7f3a|block-wins-91c2|]";

    // Shell startup time varies (PowerShell especially), so poll, re-sending
    // the command now and then in case the first one arrived too early.
    let mut scrollback = String::new();
    for attempt in 0..60 {
        if attempt % 10 == 0 {
            blockcontroller::send_input(
                block_id,
                blockcontroller::BlockInputUnion::data(print_cmd.to_vec()),
                None,
            )
            .expect("send env print");
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let bytes = state
            .filestore
            .read_file(block_id, "term")
            .expect("read term scrollback")
            .unwrap_or_default();
        scrollback = String::from_utf8_lossy(&bytes).into_owned();
        if scrollback.contains(expected) {
            break;
        }
    }
    blockcontroller::delete_controller(block_id);

    assert!(
        scrollback.contains(expected),
        "expected the shell to print {expected:?} from its own env, got: {scrollback:?}"
    );
    assert!(
        !scrollback.contains("global-loses-1d8e"),
        "the block's cmd:env must override the global value"
    );
    assert!(
        !scrollback.contains("global-id-leak-5e0b"),
        "a global AGENTMUX_AGENT_ID must never reach the pane"
    );
}

// ---- identity M4a: the Caller middleware, end to end through the router ----

async fn fallbacks_as(state: AppState, headers: &[(&str, &str)]) -> serde_json::Value {
    let mut req = Request::builder()
        .uri("/agentmux/identity/fallbacks")
        .method("GET");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = build_router(state)
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// A request carrying a token this srv minted is attributed to that UID;
/// the same request without it, or with an unknown one, is unattributed and
/// still served — attribution, never refusal (spec §6.5.3).
#[tokio::test]
async fn m4a_a_minted_token_attributes_the_request_and_nothing_is_refused() {
    let state = test_state();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-agenty").unwrap();

    let v = fallbacks_as(
        state.clone(),
        &[("X-AuthKey", "test-secret-key"), ("X-Agent-Token", &token)],
    )
    .await;
    assert_eq!(v["caller"], "uid-agenty", "{v}");

    let v = fallbacks_as(state.clone(), &[("X-AuthKey", "test-secret-key")]).await;
    assert!(v["caller"].is_null(), "{v}");

    let v = fallbacks_as(
        state,
        &[
            ("X-AuthKey", "test-secret-key"),
            ("X-Agent-Token", "not-a-token"),
        ],
    )
    .await;
    assert!(
        v["caller"].is_null(),
        "unknown token: unattributed, not 401 — {v}"
    );
}

/// The token never stands in for the instance key.
#[tokio::test]
async fn m4a_a_token_without_the_auth_key_is_still_unauthorized() {
    let state = test_state();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-agenty").unwrap();
    let req = Request::builder()
        .uri("/agentmux/identity/fallbacks")
        .header("X-Agent-Token", &token)
        .body(Body::empty())
        .unwrap();
    let resp = build_router(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Through the real LAN-forward route group: a valid token on a LAN-key
/// request is ignored (counted), never lifted to host trust.
#[tokio::test]
async fn m4a_a_token_on_a_lan_key_request_is_not_attributed() {
    let state = test_state();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-agenty").unwrap();
    let count = || {
        crate::backend::agent_resolve::uid_fallback_counts()
            .into_iter()
            .find(|(s, _)| *s == "m4.token_on_lan_key")
            .map_or(0, |(_, n)| n)
    };
    let before = count();
    let req = Request::builder()
        .uri("/agentmux/reactive/agent-names")
        .header("X-AuthKey", "test-lan-key")
        .header("X-Agent-Token", &token)
        .body(Body::empty())
        .unwrap();
    let resp = build_router(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(count() > before);
}

// ---- identity M4a-2: actor-name mismatches, end to end through the router ----

/// Every actor site: `(site, method, uri, body)` naming `actor`. Bodies fail
/// validation *after* the check where they can (bad filename, empty title,
/// bad cron expression, stale UI signature), so no test writes anything
/// real.
fn m4a2_actor_requests(actor: &str) -> Vec<(&'static str, Method, String, serde_json::Value)> {
    use serde_json::json;
    let q = |path: &str, rest: &str| format!("{path}?agent_id={actor}{rest}");
    let ui_auth = json!({"agent_id": actor, "ts_secs": 0, "sig": "", "selector": "body"});
    vec![
        (
            "memory_list",
            Method::GET,
            q("/api/v1/agent/memory/list", ""),
            json!(null),
        ),
        (
            "memory_read",
            Method::GET,
            q("/api/v1/agent/memory/read", "&filename=..%2Fm4a2"),
            json!(null),
        ),
        (
            "memory_write",
            Method::POST,
            "/api/v1/agent/memory/write".into(),
            json!({"agent_id": actor, "filename": "../m4a2", "content": ""}),
        ),
        (
            "memory_history",
            Method::GET,
            q("/api/v1/agent/memory/history", "&filename=..%2Fm4a2"),
            json!(null),
        ),
        (
            "memory_diff",
            Method::GET,
            q(
                "/api/v1/agent/memory/diff",
                "&from_version_id=m4a2-a&to_version_id=m4a2-b",
            ),
            json!(null),
        ),
        (
            "memory_revert",
            Method::POST,
            "/api/v1/agent/memory/revert".into(),
            json!({"agent_id": actor, "filename": "../m4a2", "target_version_id": "m4a2"}),
        ),
        (
            "globalmemory_write",
            Method::POST,
            "/api/v1/agent/globalmemory/write".into(),
            json!({"agent_id": actor, "name": "", "content": ""}),
        ),
        (
            "globalmemory_revert",
            Method::POST,
            "/api/v1/agent/globalmemory/revert".into(),
            json!({"agent_id": actor, "id": "m4a2-none", "version_id": "m4a2-none"}),
        ),
        (
            "preset_get",
            Method::GET,
            q("/api/v1/agent/preset/get", ""),
            json!(null),
        ),
        (
            "identity_accounts",
            Method::GET,
            q("/api/v1/agent/identity/accounts", ""),
            json!(null),
        ),
        (
            "identity_validate",
            Method::POST,
            "/api/v1/agent/identity/validate".into(),
            json!({"agent_id": actor, "account_id": "m4a2-none"}),
        ),
        (
            "history_search",
            Method::GET,
            format!("/agentmux/reactive/history/search?agent={actor}&query="),
            json!(null),
        ),
        (
            "work_enqueue",
            Method::POST,
            "/agentmux/work".into(),
            json!({"title": "", "payload": "", "created_by": actor}),
        ),
        (
            "work_claim",
            Method::POST,
            "/agentmux/work/claim".into(),
            json!({"agent_id": actor, "kind": "m4a2-no-such-kind"}),
        ),
        (
            "work_heartbeat",
            Method::POST,
            "/agentmux/work/m4a2-none/heartbeat".into(),
            json!({"agent_id": actor, "attempt": 1}),
        ),
        (
            "work_complete",
            Method::POST,
            "/agentmux/work/m4a2-none/complete".into(),
            json!({"agent_id": actor, "attempt": 1}),
        ),
        (
            "work_release",
            Method::POST,
            "/agentmux/work/m4a2-none/release".into(),
            json!({"agent_id": actor, "attempt": 1}),
        ),
        (
            "cron_create",
            Method::POST,
            "/agentmux/cron".into(),
            json!({"name": "m4a2", "expression": "not cron", "prompt": "", "target": "m4a2-nobody",
                   "created_by": actor, "max_fires": null}),
        ),
        (
            "inject",
            Method::POST,
            "/agentmux/reactive/inject".into(),
            json!({"target_agent": "m4a2-nobody", "message": "hi", "source_agent": actor}),
        ),
        (
            "supervisor_decision",
            Method::POST,
            "/agentmux/reactive/supervisor-decision".into(),
            json!({"target_agent": "", "action": "decline", "source_agent": actor}),
        ),
        (
            "bus_send",
            Method::POST,
            "/api/bus/send".into(),
            json!({"from": actor, "to": "m4a2-nobody", "payload": "hi"}),
        ),
        (
            "bus_inject",
            Method::POST,
            "/api/bus/inject".into(),
            json!({"from": actor, "target": "m4a2-nobody", "message": "hi"}),
        ),
        (
            "bus_broadcast",
            Method::POST,
            "/api/bus/broadcast".into(),
            json!({"from": actor, "payload": "hi"}),
        ),
        ("ui_auth", Method::POST, "/api/v1/ui/query".into(), ui_auth),
    ]
}

fn m4a2_count(counter: &str) -> u64 {
    crate::backend::agent_resolve::uid_fallback_counts()
        .into_iter()
        .find(|(s, _)| *s == counter)
        .map_or(0, |(_, n)| n)
}

/// Every M4a-2 outcome counter for `site`, so a test can assert that exactly
/// one moved.
fn m4a2_site_counts(site: &str) -> [u64; 4] {
    ["mismatch", "ambiguous", "absent", "unchecked"]
        .map(|outcome| m4a2_count(&format!("m4.actor_{outcome}.{site}")))
}

async fn m4a2_send(
    state: &AppState,
    token: Option<&str>,
    method: Method,
    uri: &str,
    body: &serde_json::Value,
) -> StatusCode {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let body = if body.is_null() {
        Body::empty()
    } else {
        Body::from(body.to_string())
    };
    build_router(state.clone())
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap()
        .status()
}

/// Serializes the tests that move the per-site `m4.actor_*` counters with a
/// token: M4a-2's test asserts their exact values, so another test's
/// attributed request to the same site in parallel would break it.
static M4_ACTOR_COUNTERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The colliding-names fixture ("AgentY" agenty, "AGENTY" agenty-2) plus a
/// third agent, "AgentZ"; the caller is the second agent. At every actor
/// site: naming AgentZ is a mismatch; naming `agenty` or `AGENTY` — which
/// select the caller's row by display name but also the first agent's — is
/// ambiguous; naming its own slug counts nothing. An unattributed request is
/// never checked, and nothing is refused on account of any of it: the same
/// request is served the same with or without the token.
#[tokio::test]
async fn m4a2_every_actor_site_counts_a_name_that_is_not_plainly_the_callers() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    for (id, name, slug) in [
        ("uid-m4a2-y", "AgentY", "agenty"),
        ("uid-m4a2-y2", "AGENTY", "agenty-2"),
        ("uid-m4a2-z", "AgentZ", "agentz"),
    ] {
        let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-m4a2-y2").unwrap();

    for (actor, outcome) in [("agentz", 0), ("agenty", 1), ("AGENTY", 1)] {
        for (site, method, uri, body) in m4a2_actor_requests(actor) {
            let mut expected = m4a2_site_counts(site);
            expected[outcome] += 1;
            let with = m4a2_send(&state, Some(&token), method.clone(), &uri, &body).await;
            assert_eq!(m4a2_site_counts(site), expected, "{site}: {actor}");
            let without = m4a2_send(&state, None, method, &uri, &body).await;
            assert_eq!(
                m4a2_site_counts(site),
                expected,
                "{site}: unattributed is not checked"
            );
            // M4a-2's counting changes no response. Since M4c-2 the owner of
            // a self-scoped site is the Caller when attributed (§6.5.9), so
            // there the token — not the count — does change it, by design.
            let owner_is_the_caller = site.starts_with("memory_")
                || matches!(
                    site,
                    "identity_accounts" | "identity_validate" | "preset_get" | "history_search"
                );
            if !owner_is_the_caller {
                assert_eq!(
                    with, without,
                    "{site}: counting changes nothing about the response"
                );
            }
        }
    }
    for (site, method, uri, body) in m4a2_actor_requests("agenty-2") {
        let before = m4a2_site_counts(site);
        m4a2_send(&state, Some(&token), method, &uri, &body).await;
        assert_eq!(m4a2_site_counts(site), before, "{site}: its own slug");
    }

    // A "Claude" made from the "Claude" template is `claude-2`, but a stub
    // launch has its MCP send `claude` (#3573) — the template's slug. That
    // is ambiguous, not the caller's own name, even though it is the
    // caller's display name.
    let mut tpl = test_agent_def("tpl-m4a2-claude", "Claude", "claude", "agent", 1, "");
    tpl.slug = "claude".to_string();
    tpl.is_seeded = 1;
    state.mstore.agent_def_insert(&mut tpl).unwrap();
    let mut stub = test_agent_def("uid-m4a2-claude", "Claude", "claude", "agent", 2, "");
    stub.slug = "claude-2".to_string();
    state.mstore.agent_def_insert(&mut stub).unwrap();
    let stub_token = state.mstore.agent_token_ensure("uid-m4a2-claude").unwrap();
    let before = m4a2_site_counts("memory_list");
    m4a2_send(
        &state,
        Some(&stub_token),
        Method::GET,
        "/api/v1/agent/memory/list?agent_id=claude",
        &serde_json::Value::Null,
    )
    .await;
    assert_eq!(
        m4a2_site_counts("memory_list")[1],
        before[1] + 1,
        "template stub: ambiguous"
    );

    // An attributed request that names no actor is counted as absent; a
    // token whose row is gone is counted as unchecked, never as a mismatch.
    // In this test, not its own, so no parallel test moves these counters.
    let norow = state.mstore.agent_token_ensure("uid-m4a2-norow").unwrap();
    let inject = serde_json::json!({"target_agent": "m4a2-nobody", "message": "hi"});
    let before = m4a2_site_counts("inject");
    m4a2_send(
        &state,
        Some(&norow),
        Method::POST,
        "/agentmux/reactive/inject",
        &inject,
    )
    .await;
    assert_eq!(m4a2_site_counts("inject")[2], before[2] + 1, "absent");

    let named =
        serde_json::json!({"target_agent": "m4a2-nobody", "message": "hi", "source_agent": "x"});
    let before = m4a2_site_counts("inject");
    m4a2_send(
        &state,
        Some(&norow),
        Method::POST,
        "/agentmux/reactive/inject",
        &named,
    )
    .await;
    let after = m4a2_site_counts("inject");
    assert_eq!(
        after[3],
        before[3] + 1,
        "no row for the token's UID: unchecked"
    );
    assert_eq!(after[0], before[0], "not a mismatch");
}

/// A work claim's carried `agent_uid` is compared with the token's UID —
/// through the real handler, so dropping the call fails this test.
#[tokio::test]
async fn m4a2_a_claims_carried_uid_that_is_not_the_tokens_is_counted() {
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    let mut def = test_agent_def("uid-m4a2-claim", "Claimer", "claude", "agent", 1, "");
    def.slug = "claimer".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-m4a2-claim").unwrap();
    let claim = |uid: &str| serde_json::json!({"agent_id": "claimer", "agent_uid": uid, "kind": "m4a2-no-such-kind"});
    let counter = "m4.actor_uid_mismatch.work_claim";
    let before = m4a2_count(counter);
    m4a2_send(
        &state,
        Some(&token),
        Method::POST,
        "/agentmux/work/claim",
        &claim("uid-m4a2-claim"),
    )
    .await;
    m4a2_send(
        &state,
        None,
        Method::POST,
        "/agentmux/work/claim",
        &claim("uid-other"),
    )
    .await;
    assert_eq!(m4a2_count(counter), before, "its own UID, or unattributed");
    m4a2_send(
        &state,
        Some(&token),
        Method::POST,
        "/agentmux/work/claim",
        &claim("uid-other"),
    )
    .await;
    assert_eq!(m4a2_count(counter), before + 1);

    // Identity M4c-1 (review of #3597): with a token AND a different carried
    // `agent_uid`, the token's UID wins — it takes the item addressed to the
    // token's UID and is what `claimed_by_uid` records. (In this test so the
    // mismatch counter above is not raced by a parallel test.)
    let item = crate::backend::storage::work_queue::WorkItem {
        id: "w-m4c1-precedence".into(),
        title: "precedence".into(),
        payload: "do it".into(),
        kind: "m4c1-precedence".into(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: crate::backend::storage::work_queue::work_state::OPEN.into(),
        claimed_by: String::new(),
        target_agent_uid: "uid-m4a2-claim".into(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: String::new(),
        created_by_uid: String::new(),
        created_at: 1000,
        updated_at: 1000,
        not_before: None,
        result: String::new(),
    };
    state.identity_store.work_queue_enqueue(&item).unwrap();
    let body = serde_json::json!({
        "agent_id": "claimer",
        "agent_uid": "uid-other",
        "kind": "m4c1-precedence"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/agentmux/work/claim")
        .header("X-AuthKey", "test-secret-key")
        .header("X-Agent-Token", &token)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let stored = state
        .identity_store
        .work_queue_get("w-m4c1-precedence")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.claimed_by_uid, "uid-m4a2-claim",
        "the token's UID, not the carried one"
    );
}

// ---- identity M4c-1: the actor's UID is dual-written from the Caller ----

/// Sends `body` as JSON, with the instance key and, when given, `token` as
/// `X-Agent-Token`; returns the status and the JSON response body.
async fn m4c1_send(
    state: &AppState,
    token: Option<&str>,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// A state with one agent, `uid-m4c1` (slug `m4c1-agent`), whose token the
/// index knows; returns the token. Every body below names that agent's own
/// slug, so M4a-2's actor check counts nothing — the `m4.actor_*` counters
/// are global, and a parallel M4a-2 test asserts them exactly.
fn m4c1_state() -> (AppState, String) {
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    let mut def = test_agent_def("uid-m4c1", "M4c1 Agent", "claude", "agent", 1, "");
    def.slug = "m4c1-agent".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-m4c1").unwrap();
    (state, token)
}

/// An attributed enqueue records the token's UID as `created_by_uid`; an
/// Unattributed one records `""`. A UID in the body is never read — the
/// field does not exist on the request, and a forged one changes nothing.
#[tokio::test]
async fn m4c1_work_enqueue_records_the_callers_uid_as_created_by_uid() {
    let (state, token) = m4c1_state();
    let body = serde_json::json!({
        "title": "m4c1", "payload": "do it", "created_by": "m4c1-agent",
        "created_by_uid": "uid-forged",
    });

    let (status, v) = m4c1_send(&state, Some(&token), "/agentmux/work", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let item = state
        .identity_store
        .work_queue_get(v["id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(item.created_by_uid, "uid-m4c1");
    assert_eq!(item.created_by, "m4c1-agent", "the name is never rewritten");

    let (status, v) = m4c1_send(&state, None, "/agentmux/work", body).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let item = state
        .identity_store
        .work_queue_get(v["id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        item.created_by_uid, "",
        "Unattributed: unknown, never guessed"
    );
}

/// An attributed claim writes the token's UID as `claimed_by_uid` even when
/// the body carries no `agent_uid` — and, since that one UID is also M3's
/// eligibility match, it can claim an item addressed to its UID. The same
/// claim Unattributed, with no carried UID and no block, has no UID: it
/// cannot reach the UID-addressed item, and an untargeted one records `""`.
#[tokio::test]
async fn m4c1_an_attributed_claim_records_the_tokens_uid_without_a_carried_one() {
    use crate::backend::storage::work_queue::{work_state, WorkItem};
    let (state, token) = m4c1_state();
    let item = |id: &str, target_uid: &str, created_at: i64| WorkItem {
        id: id.into(),
        title: id.into(),
        payload: "do it".into(),
        kind: "m4c1-claim".into(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: work_state::OPEN.into(),
        claimed_by: String::new(),
        target_agent_uid: target_uid.into(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: String::new(),
        created_by_uid: String::new(),
        created_at,
        updated_at: created_at,
        not_before: None,
        result: String::new(),
    };
    state
        .identity_store
        .work_queue_enqueue(&item("w-m4c1-uid", "uid-m4c1", 1000))
        .unwrap();
    let claim =
        serde_json::json!({"agent_id": "m4c1-agent", "agent_uid": "", "kind": "m4c1-claim"});

    let (status, v) = m4c1_send(&state, None, "/agentmux/work/claim", claim.clone()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        v["claimed"], false,
        "Unattributed, no UID: not eligible — {v}"
    );

    let (status, v) = m4c1_send(&state, Some(&token), "/agentmux/work/claim", claim.clone()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["claimed"], true, "{v}");
    assert_eq!(v["item"]["id"], "w-m4c1-uid");
    assert_eq!(v["item"]["claimed_by_uid"], "uid-m4c1");
    let stored = state
        .identity_store
        .work_queue_get("w-m4c1-uid")
        .unwrap()
        .unwrap();
    assert_eq!(stored.claimed_by_uid, "uid-m4c1");
    assert_eq!(
        stored.claimed_by, "m4c1-agent",
        "the name is never rewritten"
    );

    state
        .identity_store
        .work_queue_enqueue(&item("w-m4c1-open", "", 2000))
        .unwrap();
    let (_, v) = m4c1_send(&state, None, "/agentmux/work/claim", claim).await;
    assert_eq!(v["item"]["id"], "w-m4c1-open", "{v}");
    assert_eq!(
        v["item"]["claimed_by_uid"], "",
        "Unattributed, nothing carried"
    );
}

/// An attributed cron create records the token's UID as `created_by_uid`;
/// an Unattributed one records `""`.
#[tokio::test]
async fn m4c1_cron_create_records_the_callers_uid_as_created_by_uid() {
    let (mut state, token) = m4c1_state();
    // The cron handlers use the shared store; the test store carries the
    // identity schema, which has its own `db_cron_jobs`.
    state.shared_store = Some(state.mstore.clone());
    let body = serde_json::json!({
        "name": "m4c1", "expression": "0 0 1 1 *", "prompt": "hi",
        "target": "m4c1-nobody", "created_by": "m4c1-agent",
        "created_by_uid": "uid-forged",
    });
    let shared = state.shared_store.clone().unwrap();

    let (status, v) = m4c1_send(&state, Some(&token), "/agentmux/cron", body.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    let job = shared.cron_get(&id).unwrap().unwrap();
    assert_eq!(job.created_by_uid, "uid-m4c1");
    assert_eq!(job.created_by, "m4c1-agent", "the name is never rewritten");

    let (status, v) = m4c1_send(&state, None, "/agentmux/cron", body).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["job"]["id"].as_str().unwrap().to_string();
    state.cron_scheduler.cancel_job(&id);
    assert_eq!(shared.cron_get(&id).unwrap().unwrap().created_by_uid, "");
}

/// A Global Memory write and a revert record the token's UID as the
/// version's `written_by_uid`, beside the `written_by` name; Unattributed,
/// both record `""`.
#[tokio::test]
async fn m4c1_global_memory_write_and_revert_record_the_callers_uid() {
    let (state, token) = m4c1_state();
    let write = |id: Option<&str>, content: &str| serde_json::json!({"agent_id": "m4c1-agent", "id": id, "name": "M4c1", "content": content});
    let latest = |id: &str| {
        let v = state.id_store.bundle_version_list(id).unwrap();
        (
            v[0].id.clone(),
            v[0].written_by.clone(),
            v[0].written_by_uid.clone(),
        )
    };

    let (status, v) = m4c1_send(
        &state,
        Some(&token),
        "/api/v1/agent/globalmemory/write",
        write(None, "one"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let (first, by, uid) = latest(&id);
    assert_eq!((by.as_str(), uid.as_str()), ("m4c1-agent", "uid-m4c1"));

    let (status, v) = m4c1_send(
        &state,
        None,
        "/api/v1/agent/globalmemory/write",
        write(Some(&id), "two"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(latest(&id).2, "", "Unattributed write");

    let revert = serde_json::json!({"agent_id": "m4c1-agent", "id": id, "version_id": first});
    let (status, v) = m4c1_send(
        &state,
        Some(&token),
        "/api/v1/agent/globalmemory/revert",
        revert.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(latest(&id).2, "uid-m4c1", "attributed revert");
    // M4c-2d: returned beside `written_by`, by revert and by history.
    assert_eq!(v["version"]["written_by_uid"], "uid-m4c1", "{v}");
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/v1/agent/globalmemory/history?id={id}"))
        .header("X-AuthKey", "test-secret-key")
        .body(Body::empty())
        .unwrap();
    let resp = build_router(state.clone()).oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let history: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let uids: Vec<&str> = history["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["written_by_uid"].as_str().unwrap())
        .collect();
    assert_eq!(uids, ["uid-m4c1", "", "uid-m4c1"], "{history}");

    let (status, v) = m4c1_send(&state, None, "/api/v1/agent/globalmemory/revert", revert).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(latest(&id).2, "", "Unattributed revert");
}

// ---- identity M4c-2: holder checks by UID ----

/// Two agents both answer to `agenty`. The holder's token drives its item;
/// the other's token, sending the same name, is refused with the existing
/// 409; an Unattributed request keeps the name path and is counted.
#[tokio::test]
async fn m4c2_a_work_holder_is_checked_by_uid_when_both_are_known() {
    let state = test_state();
    state.mstore.attach_token_index().unwrap();
    let holder = state.mstore.agent_token_ensure("uid-m4c2-y").unwrap();
    let other = state.mstore.agent_token_ensure("uid-m4c2-y2").unwrap();
    let item = crate::backend::storage::work_queue::WorkItem {
        id: "w-m4c2-holder".into(),
        title: "holder".into(),
        payload: "do it".into(),
        kind: "m4c2-holder".into(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: crate::backend::storage::work_queue::work_state::OPEN.into(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: String::new(),
        created_by_uid: String::new(),
        created_at: 1000,
        updated_at: 1000,
        not_before: None,
        result: String::new(),
    };
    state.identity_store.work_queue_enqueue(&item).unwrap();
    let claim = serde_json::json!({"agent_id": "agenty", "kind": "m4c2-holder"});
    let (status, v) = m4c1_send(&state, Some(&holder), "/agentmux/work/claim", claim).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let attempt = v["attempt"].as_i64().unwrap();
    let body = serde_json::json!({"agent_id": "agenty", "attempt": attempt, "result": "r"});
    // Other work tests take the name path in parallel, so the counter can
    // only be checked to move; that a UID match is not counted is pinned by
    // the store's `HolderMatch` (work_queue.rs tests).
    let counter = "m4c.holder_by_name";

    for op in ["heartbeat", "complete", "release"] {
        let uri = format!("/agentmux/work/w-m4c2-holder/{op}");
        let (status, v) = m4c1_send(&state, Some(&other), &uri, body.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{op}: {v}");
    }
    let uri = "/agentmux/work/w-m4c2-holder/heartbeat";
    let (status, v) = m4c1_send(&state, Some(&holder), uri, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{v}");

    let before = m4a2_count(counter);
    let (status, v) = m4c1_send(&state, None, uri, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(m4a2_count(counter) > before, "Unattributed: by name, counted");

    let uri = "/agentmux/work/w-m4c2-holder/complete";
    let (status, v) = m4c1_send(&state, Some(&holder), uri, body).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let stored = state.identity_store.work_queue_get("w-m4c2-holder").unwrap().unwrap();
    assert_eq!(stored.result, "r");
}

// ---- identity M4c-2b: the personal-memory owner is the Caller ----

/// The second of two agents answering to `agenty` writes and reads with the
/// first one's slug. With its token, the owner is its own row — its file,
/// not the first agent's; Unattributed, the slug decides, counted.
#[tokio::test]
async fn m4c2b_an_attributed_memory_request_is_the_callers_own() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    let (tmp_y, tmp_y2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for (id, name, slug, dir) in [
        ("uid-m4c2b-y", "AgentY", "agenty", tmp_y.path()),
        ("uid-m4c2b-y2", "AGENTY", "agenty-2", tmp_y2.path()),
    ] {
        let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
        def.slug = slug.to_string();
        def.working_directory = dir.to_string_lossy().into_owned();
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
            .mstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: id.to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
                updated_at: 0,
            })
            .unwrap();
    }
    state.mstore.attach_token_index().unwrap();
    let y2 = state.mstore.agent_token_ensure("uid-m4c2b-y2").unwrap();
    let write = |content: &str| {
        serde_json::json!({"agent_id": "agenty", "filename": "MEMORY.md", "content": content})
    };

    let (status, v) = m4c1_send(&state, Some(&y2), "/api/v1/agent/memory/write", write("y2's")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let counter = "m4c.memory_owner_by_name";
    let before = m4a2_count(counter);
    let (status, v) = m4c1_send(&state, None, "/api/v1/agent/memory/write", write("y's")).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(m4a2_count(counter) > before, "Unattributed: by slug, counted");

    let versions = |uid: &str| {
        state
            .id_store
            .agent_native_memory_version_list(uid, "MEMORY.md")
            .unwrap()
            .into_iter()
            .map(|v| v.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(versions("uid-m4c2b-y2").len(), 1, "the token's row");
    assert_eq!(versions("uid-m4c2b-y").len(), 1, "the slug's row");

    let uri = "/api/v1/agent/memory/read?agent_id=agenty&filename=MEMORY.md";
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("X-AuthKey", "test-secret-key")
        .header("X-Agent-Token", &y2)
        .body(Body::empty())
        .unwrap();
    let resp = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"], "y2's", "the token's row, not the slug's");
}

// ---- identity M4c-2c: identity, preset and history owners are the Caller ----

/// Each handler takes its owner from the token, not from `agent_id`: a token
/// whose row is gone is refused with the caller named, while the same
/// request Unattributed resolves the slug as before and is counted.
#[tokio::test]
async fn m4c2c_self_handlers_take_the_owner_from_the_token() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let mut state = test_state();
    // An empty index: the Unattributed search below must not scan the
    // developer's real transcripts.
    state.history_service = std::sync::Arc::new(crate::backend::history::HistoryService::from_index(
        crate::backend::history::index::SessionIndex::new(vec![]),
    ));
    state.mstore.attach_token_index().unwrap();
    let gone = state.mstore.agent_token_ensure("uid-m4c2c-gone").unwrap();
    let get = |uri: &'static str, token: Option<String>| {
        let state = state.clone();
        async move {
            let mut req = Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header("X-AuthKey", "test-secret-key");
            if let Some(t) = token {
                req = req.header("X-Agent-Token", t);
            }
            let resp = build_router(state)
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = resp.status();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8_lossy(&body).into_owned())
        }
    };
    for (uri, counter) in [
        ("/api/v1/agent/identity/accounts?agent_id=m4c2c-nobody", "m4c.identity_owner_by_name"),
        ("/api/v1/agent/preset/get?agent_id=m4c2c-nobody", "m4c.preset_owner_by_name"),
        // History search no longer resolves a name at all: Unattributed, it's
        // refused (see `history_search_without_an_identity_token_is_refused_not_guessed`).
        (
            "/agentmux/reactive/history/search?agent=m4c2c-nobody&query=x",
            "history.search_refused_unattributed",
        ),
    ] {
        let (status, body) = get(uri, Some(gone.clone())).await;
        assert!(status.is_client_error(), "{uri}: {status} {body}");
        assert!(body.contains("calling agent uid-m4c2c-gone not found"), "{uri}: {body}");
        let before = m4a2_count(counter);
        let (_, body) = get(uri, None).await;
        assert!(!body.contains("calling agent"), "{uri}: {body}");
        assert!(m4a2_count(counter) > before, "{uri}: Unattributed is counted");
    }
}

// ---- SearchHistory: owner by token only, found wherever it was written ----

async fn history_search_get(
    state: &AppState,
    token: Option<&str>,
    query: &str,
) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(format!("/agentmux/reactive/history/search?{query}"))
        .header("X-AuthKey", "test-secret-key");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
}

/// A search without a per-agent token is refused — never answered by
/// resolving the name it declares. On 2026-09-24 that kind of guess answered
/// "no history" with `truncated: false` for an agent whose sessions were on
/// disk.
#[tokio::test]
async fn history_search_without_an_identity_token_is_refused_not_guessed() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let counter = "history.search_refused_unattributed";
    let before = m4a2_count(counter);

    let (status, v) = history_search_get(&state, None, "agent=agent3&query=x").await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"].as_str().unwrap_or_default().contains("X-Agent-Token"), "{v}");
    assert!(m4a2_count(counter) > before, "refusals are counted");
}

/// The whole path, with the real Claude adapter and parser: an attributed
/// caller finds a session written in its working directory under an identity
/// bundle created after the history service was built (an account switched
/// mid-run — the 2026-09-24 incident); a bare `SendMessage` filter finds the
/// MCP-prefixed call; `since`/`until` in seconds select by message time; and
/// the answer says it's complete. No `agent` parameter is needed.
#[tokio::test]
async fn an_attributed_history_search_finds_a_session_in_a_bundle_created_after_start() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    use crate::backend::history::{claude_adapter::ClaudeHistoryAdapter, index::SessionIndex, HistoryService};
    use crate::backend::storage::agents::test_agent_def;
    let tmp = tempfile::tempdir().unwrap();
    let shared = tmp.path().join("shared");
    std::fs::create_dir_all(&shared).unwrap();
    let mut state = test_state();
    state.history_service = std::sync::Arc::new(HistoryService::from_index(SessionIndex::new(vec![
        Box::new(ClaudeHistoryAdapter::with_roots(None, Some(shared.clone()), None)),
    ])));

    let cwd = "/work/history-agent";
    let mut def = test_agent_def("uid-history-search", "HistoryAgent", "claude", "agent", 1, "");
    def.slug = "historyagent".to_string();
    def.working_directory = cwd.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-history-search").unwrap();

    // The bundle appears only now, after the service was built.
    let project = shared
        .join("identities")
        .join("account-added-later")
        .join("claude")
        .join("projects")
        .join("-work-history-agent");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("sess-after-switch.jsonl"),
        [
            r#"{"type":"user","message":{"role":"user","content":"please ping Agent4"},"timestamp":"2026-09-24T10:00:00Z","cwd":"/work/history-agent","sessionId":"sess-after-switch"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus","content":[{"type":"text","text":"sending"},{"type":"tool_use","name":"mcp__agentmux__SendMessage","input":{"message":"ping","to":"Agent4"}}]},"timestamp":"2026-09-24T10:00:05Z","cwd":"/work/history-agent"}"#,
        ]
        .join("\n"),
    )
    .unwrap();

    let (status, v) = history_search_get(&state, Some(&token), "query=&tool=SendMessage").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["hits"].as_array().map(Vec::len), Some(1), "{v}");
    assert_eq!(v["hits"][0]["tool_name"], "mcp__agentmux__SendMessage", "{v}");
    assert_eq!(v["hits"][0]["session_id"], "sess-after-switch", "{v}");
    assert_eq!(v["complete"], true, "{v}");
    assert_eq!(v["total_sessions"], 1, "{v}");
    assert_eq!(v["sessions_scanned"], 1, "{v}");

    let secs = |t: &str| chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp();
    let (_, v) = history_search_get(
        &state,
        Some(&token),
        &format!("query=ping&until={}", secs("2026-09-24T10:00:02Z")),
    )
    .await;
    let snippets: Vec<&str> = v["hits"].as_array().unwrap().iter().filter_map(|h| h["snippet"].as_str()).collect();
    assert_eq!(snippets, vec!["please ping Agent4"], "only the message before `until`: {v}");
    assert_eq!(v["complete"], true, "{v}");

    let (_, v) = history_search_get(
        &state,
        Some(&token),
        &format!("query=ping&since={}", secs("2026-09-24T11:00:00Z")),
    )
    .await;
    assert_eq!(v["hits"].as_array().map(Vec::len), Some(0), "nothing after `since`: {v}");
}

// ---- identity M4c-2d: the sender's UID is exposed beside its name ----

/// An attributed inject is audited with its token's UID; an attributed bus
/// send carries it as `from_uid`. Unattributed, both stay empty.
#[tokio::test]
async fn m4c2d_the_senders_uid_is_audited_and_carried() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure("uid-m4c2d").unwrap();

    // A registered target, so the inject resolves at once instead of
    // searching for peers to forward to: the audit ring is shared by every
    // test and holds 100 entries, so the entry is read back straight away,
    // by its own request id.
    let target = format!("m4c2d-target-{}", uuid::Uuid::new_v4());
    let request_id = format!("m4c2d-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&target, &format!("{target}-block"), None).unwrap();
    let inject = serde_json::json!({
        "target_agent": target, "message": "hi", "source_agent": "m4c2d-sender",
        "request_id": request_id,
    });
    // The rate limiter is shared by every test too (10/s); a limited
    // request writes no audit entry, so wait for a token.
    let mut resp = serde_json::Value::Null;
    for _ in 0..25 {
        resp = m4c1_send(&state, Some(&token), "/agentmux/reactive/inject", inject.clone()).await.1;
        if resp["error"] != "rate limit exceeded" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let audited = state.reactive_handler.get_audit_log(100);
    let entry = audited.iter().find(|e| e.request_id == request_id);
    assert_eq!(entry.map(|e| e.audit_source_uid.as_str()), Some("uid-m4c2d"), "{resp}");

    m4c1_send(&state, None, "/api/bus/register", serde_json::json!({"agent_id": "m4c2d-inbox"})).await;
    let send = |payload: &str| {
        serde_json::json!({"from": "m4c2d-sender", "to": "m4c2d-inbox", "payload": payload})
    };
    m4c1_send(&state, Some(&token), "/api/bus/send", send("attributed")).await;
    m4c1_send(&state, None, "/api/bus/send", send("unattributed")).await;
    let messages = state.messagebus.read_messages("m4c2d-inbox", 10);
    let uid_of = |payload: &str| {
        messages.iter().find(|m| m.payload == payload).map(|m| m.from_uid.clone())
    };
    assert_eq!(uid_of("attributed").as_deref(), Some("uid-m4c2d"), "{messages:?}");
    assert_eq!(uid_of("unattributed").as_deref(), Some(""), "{messages:?}");
}

// ---- identity M4c-3: cron fires in process ----

/// Through the real installed delivery: a fire reaches the local inject path
/// as `"cron"` and is audited with its creator's UID.
#[tokio::test]
async fn m4c3_a_cron_fire_is_delivered_in_process_and_audited_with_its_creator() {
    let state = test_state();
    crate::bootstrap::install_cron_delivery(&state);
    let target = format!("m4c3-target-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&target, &format!("{target}-block"), None).unwrap();

    // The reactive handler, its audit ring (100) and its rate limiter (10/s)
    // are shared by every test: fire until this target's entry appears.
    let mut entry = None;
    for _ in 0..25 {
        state.cron_scheduler.fire("m4c3-job", "tick", &target, "", "uid-m4c3-creator").await;
        entry = state
            .reactive_handler
            .get_audit_log(100)
            .into_iter()
            .find(|e| e.target_agent == target);
        if entry.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let entry = entry.expect("the fire was audited");
    assert_eq!(entry.source_agent.as_deref(), Some("cron"));
    assert_eq!(entry.audit_source_uid, "uid-m4c3-creator");
}

// ---- durable jekt (SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md) ----

/// A jekt to a known agent that is not running anywhere is held once, by
/// its UID; an unknown name and a `cron` fire keep today's error; a replay
/// pass leaves a still-absent target's message where it is, with no attempt
/// counted.
#[tokio::test]
async fn a_jekt_to_an_absent_known_agent_is_held_and_waits() {
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    let slug = format!("heldy-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let uid = format!("uid-{slug}");
    let mut def = test_agent_def(&uid, "Held Y", "claude", "agent", 1, "");
    def.slug = slug.clone();
    state.mstore.agent_def_insert(&mut def).unwrap();

    let send = |target: &str, source: &str, id: &str| {
        serde_json::json!({"target_agent": target, "message": "hi", "source_agent": source, "request_id": id})
    };
    let id = format!("req-{slug}");
    // The global handler's rate limiter is shared by every test (10/s).
    let mut v = serde_json::Value::Null;
    for _ in 0..25 {
        v = m4c1_send(&state, None, "/agentmux/reactive/inject", send(&slug, "sender", &id)).await.1;
        if v["error"] != "rate limit exceeded" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(v["held"], true, "{v}");
    assert_eq!(v["success"], false, "held is not delivered: {v}");
    let rows = state.mstore.jekt_held_for_target(&uid, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, id);

    // Same id again: the same answer, nothing stored twice.
    let v = m4c1_send(&state, None, "/agentmux/reactive/inject", send(&slug, "sender", &id)).await.1;
    if v["error"] != "rate limit exceeded" {
        assert_eq!(v["held"], true, "{v}");
    }
    assert_eq!(state.mstore.jekt_held_for_target(&uid, 10).unwrap().len(), 1);

    // Not held: a name no agent has, and a periodic cron fire.
    for (target, source) in [(format!("nobody-{slug}"), "sender"), (slug.clone(), "cron")] {
        let v = m4c1_send(&state, None, "/agentmux/reactive/inject", send(&target, source, &format!("x-{target}-{source}"))).await.1;
        assert!(v.get("held").is_none(), "{target} from {source}: {v}");
    }
    assert_eq!(state.mstore.jekt_held_for_target(&uid, 10).unwrap().len(), 1);

    // Still absent: the replay pass does not touch it.
    let outcomes = crate::server::jekt_held::replay_pass(&state).await;
    assert!(outcomes.iter().all(|(r, _)| r != &id), "{outcomes:?}");
    let rows = state.mstore.jekt_held_for_target(&uid, 10).unwrap();
    assert_eq!(rows[0].attempts, 0);
}

/// Durable jekt §2.1 condition 5 (review of #3632): a target running under
/// another of its names — registered with its UID but not bound to the name
/// addressed — is delivered by UID at once, never held as "not running".
#[tokio::test]
async fn a_target_running_under_another_name_is_delivered_by_uid_not_held() {
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    let slug = format!("c5-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let uid = format!("uid-{slug}");
    let mut def = test_agent_def(&uid, "C5", "claude", "agent", 1, "");
    def.slug = slug.clone();
    state.mstore.agent_def_insert(&mut def).unwrap();
    // Registered under a display binding other than the slug addressed.
    state
        .reactive_handler
        .register_agent_full(&format!("{slug}-display"), &format!("{slug}-block"), None, 0, None, Some(&uid), "test")
        .unwrap();

    let body = serde_json::json!({"target_agent": slug, "message": "hi", "source_agent": "sender"});
    let mut v = serde_json::Value::Null;
    for _ in 0..25 {
        v = m4c1_send(&state, None, "/agentmux/reactive/inject", body.clone()).await.1;
        if v["error"] != "rate limit exceeded" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(v.get("held").is_none(), "running, so not held: {v}");
    assert!(state.mstore.jekt_held_for_target(&uid, 10).unwrap().is_empty());
    let err = v["error"].as_str().unwrap_or("");
    assert!(!err.starts_with("agent not found"), "delivered to the UID's block: {v}");
}

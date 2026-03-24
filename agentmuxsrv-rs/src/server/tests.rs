use super::*;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use crate::backend::reactive as backend_reactive;
use crate::backend::wconfig;
use crate::backend::wcore;

fn test_state() -> AppState {
    let wstore = Arc::new(WaveStore::open_in_memory().unwrap());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let event_bus = Arc::new(EventBus::new());
    let broker = Arc::new(Broker::new());
    let reactive_handler = backend_reactive::get_global_handler();
    let poller = Arc::new(Poller::new(
        backend_reactive::PollerConfig {
            agentmux_url: None,
            agentmux_token: None,
            poll_interval_secs: 30,
        },
        reactive_handler,
    ));

    // Bootstrap initial data
    wcore::ensure_initial_data(&wstore).unwrap();

    let config_watcher = Arc::new(wconfig::ConfigWatcher::new());

    AppState {
        auth_key: "test-secret-key".to_string(),
        version: "0.28.20".to_string(),
        app_path: String::new(),
        wstore,
        filestore,
        event_bus: event_bus.clone(),
        broker,
        reactive_handler,
        poller,
        config_watcher,
        messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
        http_client: reqwest::Client::new(),
        local_web_url: String::new(),
        subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus)),
        history_service: Arc::new(crate::backend::history::HistoryService::new()),
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
        .uri("/wave/service")
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
        .uri("/wave/service")
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
        .uri("/wave/service")
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
async fn auth_accepts_query_param() {
    let app = test_router();
    let req = Request::builder()
        .uri("/wave/service?authkey=test-secret-key")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"client","method":"GetClientData"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn reactive_routes_skip_auth() {
    let app = test_router();
    let req = Request::builder()
        .uri("/wave/reactive/agents")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn cors_headers_present() {
    let app = test_router();
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/")
        .header("Origin", "http://localhost:5173")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp
        .headers()
        .contains_key("access-control-allow-origin"));
}

#[tokio::test]
async fn service_get_client_data() {
    let app = test_router();
    let req = Request::builder()
        .uri("/wave/service")
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
        .uri("/wave/service")
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
        .uri("/wave/service")
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
        .uri("/wave/reactive/agents")
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
        .uri("/wave/reactive/poller/status")
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
async fn workspace_colors_and_icons() {
    let state = test_state();
    let app = build_router(state);
    let req = Request::builder()
        .uri("/wave/service")
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"service":"workspace","method":"GetColors"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"].is_array());
    assert!(json["data"].as_array().unwrap().len() > 0);
}

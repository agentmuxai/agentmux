// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use axum::extract::RawQuery;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::Router;

const LIST: &str = "/api/v1/muxspect/list";

fn record(channel: &str, port: u16, key: &str) -> InstanceRecord {
    let now = agentmux_common::time::now_ms_u64();
    InstanceRecord {
        channel: channel.to_string(),
        version: "0.60.27".to_string(),
        pid: 1,
        local_url: format!("http://127.0.0.1:{port}"),
        auth_key: key.to_string(),
        started_at: now - 9_000,
        updated_at: now,
        agent_count: 3,
    }
}

async fn serve(app: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

/// What the other instance saw: the key it was sent and the query it got.
async fn echo(headers: HeaderMap, RawQuery(q): RawQuery) -> Json<serde_json::Value> {
    let key = headers.get(agentmux_common::AUTH_KEY_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    Json(json!({ "saw_key": key, "saw_query": q.unwrap_or_default() }))
}

fn live_of(records: Vec<InstanceRecord>) -> impl FnOnce(&Path) -> Vec<InstanceRecord> + Send + 'static {
    move |_| records
}

async fn body_json(resp: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn the_channel_parameter_is_split_from_the_rest_of_the_query() {
    assert_eq!(split_channel(None), None);
    assert_eq!(split_channel(Some("block_id=abc")), None);
    assert_eq!(split_channel(Some("channel=&block_id=abc")), None);
    assert_eq!(split_channel(Some("channel=posa-dev")), Some(("posa-dev".into(), String::new())));
    assert_eq!(
        split_channel(Some("block_id=a%20b&channel=posa%2Fdev&tab_id=t")),
        Some(("posa/dev".into(), "block_id=a+b&tab_id=t".into()))
    );
}

#[tokio::test]
async fn a_request_is_forwarded_with_the_other_instances_key_and_without_the_channel() {
    let port = serve(Router::new().route(LIST, get(echo))).await;
    let resp = forward(Path::new("."), "dev", LIST, "block_id=abc", live_of(vec![record("dev", port, "dev-key")])).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["x-agentmux-forwarded-channel"], "dev");
    assert_eq!(body_json(resp).await, json!({ "saw_key": "dev-key", "saw_query": "block_id=abc" }));
}

#[tokio::test]
async fn the_answer_comes_from_the_named_instance_not_a_neighbour() {
    let one = serve(Router::new().route(LIST, get(|| async { Json(json!({ "who": "one" })) }))).await;
    let two = serve(Router::new().route(LIST, get(|| async { Json(json!({ "who": "two" })) }))).await;
    let records = vec![record("one", one, "k1"), record("two", two, "k2")];
    let resp = forward(Path::new("."), "two", LIST, "", live_of(records)).await;
    assert_eq!(body_json(resp).await, json!({ "who": "two" }));
}

#[tokio::test]
async fn the_other_instances_error_status_and_body_pass_through() {
    let app = Router::new().route(LIST, get(|| async { (StatusCode::NOT_FOUND, Json(json!({ "error": "no such block" }))) }));
    let port = serve(app).await;
    let resp = forward(Path::new("."), "dev", LIST, "", live_of(vec![record("dev", port, "k")])).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await, json!({ "error": "no such block" }));
}

#[tokio::test]
async fn an_unknown_channel_is_a_404_that_names_the_live_ones() {
    let resp = forward(Path::new("."), "nope", LIST, "", live_of(vec![record("dev", 1, "k"), record("stable", 2, "k")])).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert!(body["error"].as_str().unwrap().contains("'nope'"));
    assert_eq!(body["live_channels"], json!(["dev", "stable"]));
}

#[tokio::test]
async fn a_listed_instance_that_does_not_answer_is_a_502() {
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let resp = forward(Path::new("."), "dev", LIST, "", live_of(vec![record("dev", port, "k")])).await;
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    assert!(body_json(resp).await["error"].as_str().unwrap().contains("did not answer"));
}

#[tokio::test]
async fn the_layer_serves_a_request_for_its_own_channel_here() {
    let app = Router::new()
        .route(LIST, get(|| async { Json(json!({ "who": "here" })) }))
        .layer(axum::middleware::from_fn(forward_to_channel));
    let port = serve(app).await;
    let own = crate::backend::reactive::registry::local_channel_id();
    for query in ["", &format!("?channel={own}"), "?channel="] {
        let body: serde_json::Value = reqwest::get(format!("http://127.0.0.1:{port}{LIST}{query}")).await.unwrap().json().await.unwrap();
        assert_eq!(body, json!({ "who": "here" }), "query {query:?}");
    }
}

#[test]
fn the_listing_has_every_instance_and_no_key() {
    let records = vec![record("dev", 5, "secret-dev-key")];
    let body = listing_body(&records, 1, agentmux_common::time::now_ms_u64());
    assert!(!body.to_string().contains("secret-dev-key"));
    assert_eq!(body["instances"][0]["channel"], "dev");
    assert_eq!(body["instances"][0]["agent_count"], 3);
    assert_eq!(body["instances"][0]["self"], true);
    assert!(body["own_channel"].is_string());
}

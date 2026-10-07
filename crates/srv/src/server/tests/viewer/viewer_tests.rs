// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The viewer routes and their credential (agentmux-mobile's
//! SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §9, §13.2, §13.3): pairing,
//! what the viewer token opens and what it doesn't, revoke, and the feed.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use futures_util::StreamExt;
use tower::ServiceExt;

use super::*;
use crate::backend::viewer::feed::FeedHub;
use crate::backend::viewer::{pairing, ViewerService};
use crate::server::tests::test_state;

type DataStream = axum::body::BodyDataStream;

fn viewer_router(state: &AppState) -> Router {
    build_viewer_router(state.clone())
}

fn request(method: Method, uri: &str, auth: Option<(&str, &str)>, body: Option<serde_json::Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some((name, value)) = auth {
        b = b.header(name, value);
    }
    match body {
        Some(json) => b.header("Content-Type", "application/json").body(Body::from(json.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

fn bearer(token: &str) -> Option<(&'static str, String)> {
    Some(("Authorization", format!("Bearer {token}")))
}

async fn send(app: &Router, method: Method, uri: &str, auth: Option<(&str, String)>) -> StatusCode {
    let auth = auth.as_ref().map(|(k, v)| (*k, v.as_str()));
    app.clone().oneshot(request(method, uri, auth, None)).await.unwrap().status()
}

async fn json_of(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// A paired device, recorded the way the pair route records one.
fn pair_device(state: &AppState, name: &str) -> (String, String) {
    let token = pairing::new_token();
    let device_id = uuid::Uuid::new_v4().to_string();
    state
        .mstore
        .viewer_device_insert(&device_id, &pairing::token_hash(&token), name, "", 1)
        .unwrap();
    (device_id, token)
}

fn pair_request(code: &str, from: [u8; 4], body: Option<serde_json::Value>) -> Request<Body> {
    let body = body.unwrap_or_else(|| serde_json::json!({ "code": code, "device_name": "Pixel 9" }));
    let mut req = request(Method::POST, "/agentmux/viewer/pair", None, Some(body));
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((from, 40000))));
    req
}

// ---- Pairing ----

#[tokio::test]
async fn a_code_pairs_a_device_once() {
    let state = test_state();
    let app = viewer_router(&state);
    let (code, _) = state.viewer.pairing.start();

    let resp = app.clone().oneshot(pair_request(&code, [198, 51, 100, 2], None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = json_of(resp).await;
    let token = json["token"].as_str().unwrap();
    assert!(token.starts_with("amxv_") && token.len() == 5 + 43, "{token}");
    assert_eq!(json["hostname"], "test-host");
    assert_eq!(json["version"], "0.28.20");
    assert!(json["channel"].as_str().is_some_and(|c| !c.is_empty()));
    let device_id = json["device_id"].as_str().unwrap();

    let devices = state.mstore.viewer_device_list().unwrap();
    let device = devices.iter().find(|d| d.device_id == device_id).expect("recorded");
    assert_eq!(device.device_name, "Pixel 9");
    let hashes = state.mstore.viewer_device_token_hashes().unwrap();
    assert!(hashes.iter().any(|(id, h)| id == device_id && *h == pairing::token_hash(token)));
    assert!(!hashes.iter().any(|(_, h)| h == token), "the token itself is never stored");

    // The token works.
    assert_eq!(send(&app, Method::GET, "/agentmux/viewer/hello", bearer(token)).await, StatusCode::OK);
    // The code doesn't, a second time.
    let again = app.oneshot(pair_request(&code, [198, 51, 100, 2], None)).await.unwrap();
    assert_eq!(again.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_pair_request_is_validated_before_the_code_is_spent() {
    let state = test_state();
    let app = viewer_router(&state);
    let (code, _) = state.viewer.pairing.start();
    for body in [
        serde_json::json!({ "code": code, "device_name": "" }),
        serde_json::json!({ "code": code, "device_name": "x".repeat(65) }),
        serde_json::json!({ "code": code, "device_name": "Pixel", "device_key": "not base64!" }),
        serde_json::json!({ "code": code, "device_name": "Pixel", "device_key": "AAAA" }),
        serde_json::json!({ "device_name": "Pixel" }),
    ] {
        let resp = app.clone().oneshot(pair_request(&code, [198, 51, 100, 3], Some(body.clone()))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    use base64::Engine as _;
    let key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let ok = serde_json::json!({ "code": code, "device_name": "x".repeat(64), "device_key": key });
    let resp = app.oneshot(pair_request(&code, [198, 51, 100, 3], Some(ok))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "the code survived the bad requests");
    let device_id = json_of(resp).await["device_id"].as_str().unwrap().to_string();
    let device = state.mstore.viewer_device_list().unwrap().into_iter().find(|d| d.device_id == device_id).unwrap();
    assert_eq!(device.device_key, key);
}

#[tokio::test]
async fn wrong_codes_are_rate_limited_per_address() {
    let state = test_state();
    let app = viewer_router(&state);
    let (code, _) = state.viewer.pairing.start();
    for _ in 0..pairing::WRONG_PER_ADDRESS {
        let resp = app.clone().oneshot(pair_request("AAAAAAAAAA", [198, 51, 100, 9], None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
    let resp = app.clone().oneshot(pair_request(&code, [198, 51, 100, 9], None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let resp = app.oneshot(pair_request(&code, [198, 51, 100, 10], None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "another address still pairs");
}

#[tokio::test]
async fn pair_start_builds_the_qr_url() {
    let mut state = test_state();
    let dir = tempfile::tempdir().unwrap();
    state.viewer = ViewerService::new(Some(dir.path().to_path_buf()), &state.broker);
    assert_eq!(
        super::super::app_api::viewer::pair_start_impl(&state).err().as_deref(),
        Some(super::super::app_api::viewer::NOT_LISTENING),
        "no QR while the listener is down"
    );

    state.viewer.set_bound(vec![std::net::IpAddr::from([198, 51, 100, 7])], 29702);
    let before = agentmux_common::time::now_ms();
    let started = super::super::app_api::viewer::pair_start_impl(&state).unwrap();
    assert!(started.expires_ms >= before + 119_000 && started.expires_ms <= before + 121_000);
    let url = url::Url::parse(&started.url).unwrap();
    assert_eq!(url.scheme(), "agentmux");
    assert_eq!(url.host_str(), Some("pair"));
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["v"], "1");
    assert_eq!(q["host"], "198.51.100.7");
    assert_eq!(q["port"], "29702");
    assert_eq!(q["fp"], state.viewer.tls().unwrap().fingerprint);
    assert_eq!(q["fp"].len(), 64);
    assert_eq!(q["hostname"], "test-host");
    assert!(!q["channel"].is_empty());
    let code = &q["code"];
    assert_eq!(code.len(), 10);

    // That code pairs, and the next QR cancels it.
    let next = super::super::app_api::viewer::pair_start_impl(&state).unwrap();
    assert_ne!(next.url, started.url);
    assert_eq!(state.viewer.pairing.redeem(code, None), pairing::Redeem::Refused);
}

// ---- What the token opens ----

#[tokio::test]
async fn the_viewer_token_opens_the_viewer_routes_and_nothing_else() {
    let state = test_state();
    let (device_id, token) = pair_device(&state, "Pixel");
    let viewer = viewer_router(&state);
    let resp = viewer.clone().oneshot(request(Method::GET, "/agentmux/viewer/hello", None, None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "no token");
    let auth = format!("Bearer {token}");
    let resp = viewer
        .clone()
        .oneshot(request(Method::GET, "/agentmux/viewer/hello", Some(("Authorization", &auth)), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let hello = json_of(resp).await;
    assert_eq!(hello["device_id"], device_id.as_str());
    assert_eq!(hello["hostname"], "test-host");
    assert_eq!(hello["version"], "0.28.20");
    assert_eq!(send(&viewer, Method::GET, "/agentmux/viewer/agents", bearer(&token)).await, StatusCode::OK);

    // Not on the loopback or LAN routers, in either header.
    let routers = crate::server::build_routers(state.clone());
    for (label, app) in [("full", routers.full), ("lan", routers.lan)] {
        for (method, uri) in [
            (Method::POST, "/agentmux/reactive/inject"),
            (Method::GET, "/agentmux/reactive/agent-names"),
            (Method::GET, "/agentmux/fleet"),
            (Method::GET, "/agentmux/discovery"),
            (Method::POST, "/agentmux/service"),
        ] {
            for auth in [bearer(&token), Some(("X-AuthKey", token.clone()))] {
                let status = send(&app, method.clone(), uri, auth).await;
                assert!(
                    status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND,
                    "{label} {uri} answered {status} to the viewer token"
                );
            }
        }
        // And the viewer routes aren't served there at all.
        let status = send(&app, Method::GET, "/agentmux/viewer/hello", Some(("X-AuthKey", "test-secret-key".into()))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{label} must not serve viewer routes");
    }
}

#[tokio::test]
async fn the_lan_key_and_the_instance_key_open_no_viewer_route() {
    let state = test_state();
    let viewer = viewer_router(&state);
    for uri in ["/agentmux/viewer/hello", "/agentmux/viewer/agents", "/agentmux/viewer/agents/anyone/feed"] {
        for key in ["test-secret-key", "test-lan-key"] {
            assert_eq!(send(&viewer, Method::GET, uri, Some(("X-AuthKey", key.into()))).await, StatusCode::UNAUTHORIZED);
            assert_eq!(send(&viewer, Method::GET, uri, bearer(key)).await, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(send(&viewer, Method::GET, uri, bearer("amxv_not-a-paired-token")).await, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn hello_stamps_last_seen() {
    let state = test_state();
    let (device_id, token) = pair_device(&state, "Pixel");
    assert_eq!(send(&viewer_router(&state), Method::GET, "/agentmux/viewer/hello", bearer(&token)).await, StatusCode::OK);
    let device = state.mstore.viewer_device_list().unwrap().into_iter().find(|d| d.device_id == device_id).unwrap();
    assert!(device.last_seen_ms > 1, "stamped on use");
}

// ---- Agents ----

fn register(state: &AppState, prefix: &str) -> (String, String) {
    let unique = uuid::Uuid::new_v4();
    let name = format!("{prefix}-{unique}");
    let block_id = uuid::Uuid::new_v4().to_string();
    state.reactive_handler.register_agent(&name, &block_id, None).unwrap();
    (name, block_id)
}

/// An agent whose block names its definition, so it can be hidden.
fn register_defined(state: &AppState, prefix: &str) -> (String, String, String) {
    let (name, block_id) = register(state, prefix);
    let def_id = uuid::Uuid::new_v4().to_string();
    let mut def = crate::backend::storage::agents::test_agent_def(&def_id, &name, "claude", "standalone", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut block = crate::backend::obj::Block { oid: block_id.clone(), ..Default::default() };
    block.meta.insert("agentId".into(), serde_json::json!(def_id));
    block.meta.insert("agentProvider".into(), serde_json::json!("claude"));
    state.mstore.insert(&mut block).unwrap();
    (name, block_id, def_id)
}

#[tokio::test]
async fn agents_lists_this_channels_agents_without_hidden_ones() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (shown, _, _) = register_defined(&state, "viewer-shown");
    let (hidden, _, hidden_def) = register_defined(&state, "viewer-hidden");
    assert!(state.mstore.agent_hide_from_devices_set(&hidden_def, true).unwrap());

    let auth = format!("Bearer {token}");
    let resp = viewer_router(&state)
        .oneshot(request(Method::GET, "/agentmux/viewer/agents", Some(("Authorization", &auth)), None))
        .await
        .unwrap();
    let json = json_of(resp).await;
    assert!(json["now_ms"].as_i64().unwrap() > 0);
    let agents = json["agents"].as_array().unwrap();
    let entry = agents.iter().find(|a| a["name"] == shown.as_str()).expect("listed");
    assert_eq!(entry["kind"], "host");
    assert!(entry.get("state").is_none() && entry.get("since_ms").is_none(), "status comes with phase 1");
    assert!(!agents.iter().any(|a| a["name"] == hidden.as_str()), "hidden from paired devices");
}

#[tokio::test]
async fn a_hidden_or_unknown_agent_has_no_feed() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (hidden, _, def) = register_defined(&state, "viewer-hidden-feed");
    state.mstore.agent_hide_from_devices_set(&def, true).unwrap();
    let app = viewer_router(&state);
    assert_eq!(send(&app, Method::GET, &format!("/agentmux/viewer/agents/{hidden}/feed"), bearer(&token)).await, StatusCode::NOT_FOUND);
    assert_eq!(
        send(&app, Method::GET, "/agentmux/viewer/agents/no-such-agent-anywhere/feed", bearer(&token)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(state.viewer.open_feeds(), 0, "a refused feed holds no place");
}

// ---- The feed ----

fn user_line(text: &str) -> String {
    serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } }).to_string()
}

fn append(state: &AppState, block_id: &str, line: &str) {
    crate::backend::blockcontroller::shell::append_output_line(&state.broker, block_id, line, Some(&state.filestore), None);
}

async fn open_feed(state: &AppState, name: &str, token: &str, last_event_id: Option<&str>) -> (StatusCode, DataStream) {
    let mut b = Request::builder()
        .method(Method::GET)
        .uri(format!("/agentmux/viewer/agents/{name}/feed"))
        .header("Authorization", format!("Bearer {token}"));
    if let Some(id) = last_event_id {
        b = b.header("Last-Event-ID", id);
    }
    let resp = viewer_router(state).oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    (status, resp.into_body().into_data_stream())
}

async fn next_chunk(body: &mut DataStream) -> Option<String> {
    let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), body.next()).await.ok()??;
    Some(String::from_utf8(chunk.unwrap().to_vec()).unwrap())
}

/// The next event: `(name, id, data)`, heartbeats skipped.
async fn next_event(body: &mut DataStream) -> Option<(String, Option<String>, serde_json::Value)> {
    loop {
        let chunk = next_chunk(body).await?;
        if chunk.starts_with(':') || chunk.starts_with("retry:") {
            continue;
        }
        let mut name = String::new();
        let mut id = None;
        let mut data = serde_json::Value::Null;
        for line in chunk.lines() {
            if let Some(v) = line.strip_prefix("event: ") {
                name = v.to_string();
            } else if let Some(v) = line.strip_prefix("id: ") {
                id = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("data: ") {
                data = serde_json::from_str(v).unwrap();
            }
        }
        return Some((name, id, data));
    }
}

fn lines_of(data: &serde_json::Value) -> Vec<String> {
    data["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn a_feed_starts_with_a_snapshot_then_follows_appends_in_order() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-feed");
    for i in 0..3 {
        append(&state, &block, &user_line(&format!("turn {i}")));
    }
    let (status, mut body) = open_feed(&state, &name, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(next_chunk(&mut body).await.unwrap(), "retry: 3000\n\n");
    let (event, id, snap) = next_event(&mut body).await.unwrap();
    assert_eq!(event, "snapshot");
    assert_eq!(snap["provider"], "claude");
    assert_eq!(snap["from_line"], 0);
    assert_eq!(snap["next_line"], 3);
    let gen = snap["gen"].as_str().unwrap().to_string();
    assert!(!gen.is_empty());
    assert_eq!(id.as_deref(), Some(format!("{gen}:3").as_str()));
    assert_eq!(lines_of(&snap), (0..3).map(|i| user_line(&format!("turn {i}"))).collect::<Vec<_>>());

    append(&state, &block, "{\"type\":\"assistant\",\"n\":1}");
    append(&state, &block, "{\"type\":\"assistant\",\"n\":2}");
    let (event, id, a1) = next_event(&mut body).await.unwrap();
    assert_eq!(event, "append");
    assert_eq!((a1["gen"].as_str(), a1["line"].as_u64()), (Some(gen.as_str()), Some(3)));
    assert_eq!(lines_of(&a1), vec!["{\"type\":\"assistant\",\"n\":1}"]);
    assert_eq!(id.as_deref(), Some(format!("{gen}:4").as_str()));
    let (_, id, a2) = next_event(&mut body).await.unwrap();
    assert_eq!(a2["line"], 4);
    assert_eq!(id.as_deref(), Some(format!("{gen}:5").as_str()));
}

#[tokio::test]
async fn a_reconnect_resumes_without_gaps_or_duplicates() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-resume");
    for i in 0..5 {
        append(&state, &block, &format!("{{\"n\":{i}}}"));
    }
    let gen = state.filestore.line_state(&block, "output").unwrap().unwrap().counted.unwrap().gen;

    // Saw lines 0..2, missed 2..5.
    let (_, mut body) = open_feed(&state, &name, &token, Some(&format!("{gen}:2"))).await;
    let (event, id, data) = next_event(&mut body).await.unwrap();
    assert_eq!(event, "append", "no snapshot for a reconnect that still holds");
    assert_eq!(data["line"], 2);
    assert_eq!(lines_of(&data), vec!["{\"n\":2}", "{\"n\":3}", "{\"n\":4}"]);
    assert_eq!(id.as_deref(), Some(format!("{gen}:5").as_str()));
    append(&state, &block, "{\"n\":5}");
    let (_, _, next) = next_event(&mut body).await.unwrap();
    assert_eq!(next["line"], 5);
    assert_eq!(lines_of(&next), vec!["{\"n\":5}"]);
    drop(body);

    // Up to date: nothing until something new.
    let (_, mut body) = open_feed(&state, &name, &token, Some(&format!("{gen}:6"))).await;
    assert_eq!(next_chunk(&mut body).await.unwrap(), "retry: 3000\n\n");
    append(&state, &block, "{\"n\":6}");
    let (event, _, data) = next_event(&mut body).await.unwrap();
    assert_eq!((event.as_str(), data["line"].as_u64()), ("append", Some(6)));
    drop(body);

    // Another generation, or garbage: a snapshot.
    for stale in ["0000000000000000:3", "garbage", &format!("{gen}:99")] {
        let (_, mut body) = open_feed(&state, &name, &token, Some(stale)).await;
        let (event, _, data) = next_event(&mut body).await.unwrap();
        assert_eq!(event, "snapshot", "{stale}");
        assert_eq!(data["next_line"], 7);
    }
}

#[tokio::test]
async fn a_replaced_transcript_resets_and_sends_a_fresh_snapshot() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-replace");
    append(&state, &block, "{\"old\":1}");
    let (_, mut body) = open_feed(&state, &name, &token, None).await;
    let (_, _, first) = next_event(&mut body).await.unwrap();
    let old_gen = first["gen"].as_str().unwrap().to_string();

    crate::backend::blockcontroller::shell::with_transcript_order(&block, || {
        state.filestore.write_file(&block, "output", b"{\"new\":1}\n{\"new\":2}\n").unwrap();
        crate::backend::blockcontroller::shell::publish_transcript_changed(
            &state.broker,
            &block,
            crate::backend::mps::FILE_OP_REPLACE,
            &state.filestore,
        );
    });
    let (event, id, reset) = next_event(&mut body).await.unwrap();
    assert_eq!((event.as_str(), id), ("reset", None));
    assert_eq!(reset["reason"], "replace");
    let (event, _, snap) = next_event(&mut body).await.unwrap();
    assert_eq!(event, "snapshot");
    assert_ne!(snap["gen"].as_str().unwrap(), old_gen);
    assert_eq!(lines_of(&snap), vec!["{\"new\":1}", "{\"new\":2}"]);
    assert_eq!(snap["next_line"], 2);

    // Appends continue in the new generation.
    append(&state, &block, "{\"new\":3}");
    let (event, _, data) = next_event(&mut body).await.unwrap();
    assert_eq!((event.as_str(), data["line"].as_u64()), ("append", Some(2)));
    assert_eq!(data["gen"], snap["gen"]);
}

#[tokio::test]
async fn an_append_from_another_generation_resets_the_feed() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-gen");
    append(&state, &block, "{\"a\":1}");
    let (_, mut body) = open_feed(&state, &name, &token, None).await;
    next_event(&mut body).await.unwrap();
    // A replace nobody announced: the next append's position names a new
    // generation, which is the feed's cue to start over.
    state.filestore.write_file(&block, "output", b"{\"b\":1}\n").unwrap();
    append(&state, &block, "{\"b\":2}");
    let (event, _, reset) = next_event(&mut body).await.unwrap();
    assert_eq!((event.as_str(), reset["reason"].as_str()), ("reset", Some("gen")));
    let (event, _, snap) = next_event(&mut body).await.unwrap();
    assert_eq!(event, "snapshot");
    assert_eq!(lines_of(&snap), vec!["{\"b\":1}", "{\"b\":2}"]);
}

#[tokio::test]
async fn a_frame_over_64_kb_is_truncated_in_the_feed() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-trunc");
    let big = serde_json::json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "content": "x".repeat(5 * 1024 * 1024) }] } }).to_string();
    append(&state, &block, &big);
    let (_, mut body) = open_feed(&state, &name, &token, None).await;
    let (_, _, snap) = next_event(&mut body).await.unwrap();
    let lines = lines_of(&snap);
    assert_eq!(lines.len(), 1, "the newest line stays, however large");
    let frame: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(frame["type"], "amx_truncated");
    assert_eq!(frame["bytes"], big.len());
    assert_eq!(frame["head"].as_str().unwrap().chars().count(), 2048);

    append(&state, &block, &big);
    let (_, _, data) = next_event(&mut body).await.unwrap();
    let frame: serde_json::Value = serde_json::from_str(&lines_of(&data)[0]).unwrap();
    assert_eq!(frame["type"], "amx_truncated");
}

#[tokio::test]
async fn the_snapshot_is_bounded_by_turns_and_bytes() {
    let state = test_state();
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-bounded");
    for i in 0..60 {
        append(&state, &block, &user_line(&format!("turn {i}")));
        append(&state, &block, "{\"type\":\"assistant\"}");
    }
    let (_, mut body) = open_feed(&state, &name, &token, None).await;
    let (_, _, snap) = next_event(&mut body).await.unwrap();
    assert_eq!(snap["from_line"], 20, "the last 50 of 60 turns, two lines each");
    assert_eq!(snap["next_line"], 120);
    assert_eq!(lines_of(&snap)[0], user_line("turn 10"));
    drop(body);

    // 300 KB of output in a few turns: the 256 KB budget is the smaller bound.
    let (name, block, _) = register_defined(&state, "viewer-bounded-bytes");
    for i in 0..3 {
        append(&state, &block, &user_line(&format!("turn {i}")));
        for _ in 0..10 {
            append(&state, &block, &format!("{{\"pad\":\"{}\"}}", "p".repeat(10 * 1024)));
        }
    }
    let (_, mut body) = open_feed(&state, &name, &token, None).await;
    let (_, _, snap) = next_event(&mut body).await.unwrap();
    let bytes: usize = lines_of(&snap).iter().map(|l| l.len() + 1).sum();
    assert!(bytes <= 256 * 1024, "{bytes}");
    assert!(snap["from_line"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn feeds_are_capped_per_device_and_per_srv() {
    let mut state = test_state();
    state.viewer = ViewerService::with_limits(None, &state.broker, Arc::new(FeedHub::default()), 2, 3);
    let (_, a) = pair_device(&state, "A");
    let (_, b) = pair_device(&state, "B");
    let (name, block, _) = register_defined(&state, "viewer-caps");
    append(&state, &block, "{\"a\":1}");

    let (s1, b1) = open_feed(&state, &name, &a, None).await;
    let (s2, _b2) = open_feed(&state, &name, &a, None).await;
    assert_eq!((s1, s2), (StatusCode::OK, StatusCode::OK));
    let (s3, _) = open_feed(&state, &name, &a, None).await;
    assert_eq!(s3, StatusCode::SERVICE_UNAVAILABLE, "the device's cap");
    let (s4, _b4) = open_feed(&state, &name, &b, None).await;
    assert_eq!(s4, StatusCode::OK);
    let (s5, _) = open_feed(&state, &name, &b, None).await;
    assert_eq!(s5, StatusCode::SERVICE_UNAVAILABLE, "the srv's cap");
    drop(b1);
    let (s6, _b6) = open_feed(&state, &name, &a, None).await;
    assert_eq!(s6, StatusCode::OK, "a closed feed frees its place");
}

#[tokio::test]
async fn a_slow_reader_is_reset_and_closed_not_buffered() {
    let mut state = test_state();
    state.viewer = ViewerService::with_limits(None, &state.broker, Arc::new(FeedHub::with_max_behind(200)), 4, 16);
    let (_, token) = pair_device(&state, "Pixel");
    let (name, block, _) = register_defined(&state, "viewer-slow");
    append(&state, &block, "{\"a\":0}");
    let (_, mut body) = open_feed(&state, &name, &token, None).await;
    next_event(&mut body).await.unwrap();
    // The device reads nothing while 2 KB is written.
    for i in 0..20 {
        append(&state, &block, &format!("{{\"n\":{i},\"pad\":\"{}\"}}", "p".repeat(80)));
    }
    let mut events = Vec::new();
    while let Some((event, _, data)) = next_event(&mut body).await {
        events.push((event, data));
    }
    let (last, data) = events.last().unwrap();
    assert_eq!((last.as_str(), data["reason"].as_str()), ("reset", Some("behind")));
    assert!(events.len() <= 3, "at most what fit in 200 bytes was queued: {}", events.len());
    assert_eq!(state.viewer.open_feeds(), 0, "the stream ended and freed its place");
}

#[tokio::test]
async fn revoking_a_device_refuses_its_next_request_and_closes_its_feeds() {
    let state = test_state();
    let (device_id, token) = pair_device(&state, "Pixel");
    let (_, other) = pair_device(&state, "Tablet");
    let (name, block, _) = register_defined(&state, "viewer-revoke");
    append(&state, &block, "{\"a\":1}");
    let (_, mut mine) = open_feed(&state, &name, &token, None).await;
    let (_, mut theirs) = open_feed(&state, &name, &other, None).await;
    next_event(&mut mine).await.unwrap();
    next_event(&mut theirs).await.unwrap();

    let result = super::super::app_api::viewer::revoke_impl(&state, &device_id).unwrap();
    assert!(result.revoked);
    assert_eq!(result.feeds_closed, 1);
    assert!(next_event(&mut mine).await.is_none(), "the revoked device's feed ends");
    let app = viewer_router(&state);
    assert_eq!(send(&app, Method::GET, "/agentmux/viewer/hello", bearer(&token)).await, StatusCode::UNAUTHORIZED);

    // The other device is untouched.
    append(&state, &block, "{\"a\":2}");
    let (event, _, _) = next_event(&mut theirs).await.unwrap();
    assert_eq!(event, "append");
    assert_eq!(send(&app, Method::GET, "/agentmux/viewer/hello", bearer(&other)).await, StatusCode::OK);
    assert!(!super::super::app_api::viewer::revoke_impl(&state, &device_id).unwrap().revoked, "already gone");
}

async fn next_bytes<S>(stream: &mut S) -> Option<String>
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, std::convert::Infallible>> + Unpin,
{
    let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await.ok()??;
    Some(String::from_utf8(chunk.unwrap().to_vec()).unwrap())
}

#[tokio::test]
async fn an_agent_hidden_while_watched_is_reset_and_closed() {
    let state = test_state();
    let (_, block, def) = register_defined(&state, "viewer-hide-live");
    append(&state, &block, "{\"a\":1}");
    let lease = state.viewer.open_feed("dev").unwrap();
    let check = state.clone();
    let check_block = block.clone();
    let stream = feed_stream(
        FeedSource::of(&state, &block),
        "claude".into(),
        None,
        lease,
        std::time::Duration::from_millis(50),
        move || agent_hidden(&check, &check_block),
    );
    let mut stream = Box::pin(stream);
    assert_eq!(next_bytes(&mut stream).await.unwrap(), "retry: 3000

");
    assert!(next_bytes(&mut stream).await.unwrap().starts_with("event: snapshot"));
    assert_eq!(next_bytes(&mut stream).await.unwrap(), ": hb

", "a quiet feed sends heartbeats");
    state.mstore.agent_hide_from_devices_set(&def, true).unwrap();
    let mut last = String::new();
    while let Some(chunk) = next_bytes(&mut stream).await {
        last = chunk;
    }
    assert_eq!(last, "event: reset
data: {\"reason\":\"hidden\"}

");
    assert_eq!(state.viewer.open_feeds(), 0);
}

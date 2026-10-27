// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The files a Kanban task carries, through the router (SPEC_KANBAN_TASK_MEDIA_2026_10_25.md).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use sha2::Digest;
use tower::ServiceExt;

struct Api {
    router: axum::Router,
    state: crate::server::AppState,
}

fn app() -> Api {
    let state = crate::server::tests::test_state();
    Api { router: crate::server::build_router(state.clone()), state }
}

async fn send(app: &Api, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn post(app: &Api, uri: &str, body: Value) -> (StatusCode, Value) {
    let token = crate::server::tests::token_for(&app.state, &crate::server::tests::actor_in(&body));
    let req = Request::builder()
        .uri(uri)
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("X-Agent-Token", token)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, req).await
}

/// Upload a file as an agent's MCP does.
async fn upload(app: &Api, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let req = Request::builder()
        .uri(format!("/agentmux/work/media?name={name}"))
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("X-Agent-Token", crate::server::tests::token_for(&app.state, "a1"))
        .body(Body::from(bytes))
        .unwrap();
    send(app, req).await
}

fn png(shade: u8) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(6, 6, image::Rgba([shade, 20, 30, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A task claimed by `a1`: its number and attempt.
async fn claimed(app: &Api, extra: Value) -> (i64, i64) {
    let mut body = json!({"title": "t", "payload": "p", "review": true});
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    post(app, "/agentmux/work", body).await;
    let (_, c) = post(app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap())
}

#[tokio::test]
async fn an_upload_is_typed_by_its_content_and_hashed() {
    let app = app();
    let bytes = png(1);
    let (s, v) = upload(&app, "C:/shots/after.png", bytes.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["kind"].as_str(), v["mime"].as_str(), v["name"].as_str()), (Some("image"), Some("image/png"), Some("after.png")));
    assert_eq!(v["sha256"].as_str().unwrap(), hex::encode(sha2::Sha256::digest(&bytes)));
    assert_eq!(v["size"].as_u64(), Some(bytes.len() as u64));

    // Called .png, but text: refused, and nothing is left.
    let (s, v) = upload(&app, "shot.png", b"this is not a picture".to_vec()).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{v}");
    let (s, _) = upload(&app, "empty.png", Vec::new()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = upload(&app, "tool.exe", b"MZ\x90\0\x03\0\0\0".to_vec()).await;
    assert_eq!(s, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn an_image_over_its_limit_is_refused() {
    let app = app();
    let mut big = png(2);
    big.resize(crate::backend::task_media::MAX_IMAGE_BYTES as usize + 1, 0);
    let (s, v) = upload(&app, "big.png", big).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert!(v["error"].as_str().unwrap().contains("25 MiB"), "{v}");
}

#[tokio::test]
async fn submit_of_a_ui_task_that_waits_for_the_operator_needs_media_or_a_reason() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({"ui_change": true})).await;
    let submit = |extra: Value| {
        let mut b = json!({"agent_id": "a1", "attempt": attempt, "text": "PR up"});
        b.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        b
    };
    let uri = format!("/agentmux/work/{num}/submit");

    let (s, v) = post(&app, &uri, submit(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("no_media_reason"), "says the way through: {v}");
    let id = app.state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().item.id;
    assert_eq!(app.state.identity_store.work_queue_get(&id).unwrap().unwrap().state, "claimed", "nothing moved");

    // Media that was never uploaded fails the call whole.
    let (s, v) = post(&app, &uri, submit(json!({"media": [{"sha256": "0".repeat(64), "name": "x.png"}]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(app.state.identity_store.work_queue_get(&id).unwrap().unwrap().state, "claimed");

    let (_, up) = upload(&app, "after.png", png(3)).await;
    let (s, v) = post(&app, &uri, submit(json!({"media": [{"sha256": up["sha256"], "name": "after.png", "caption": "narrow"}]}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "review");
    assert_eq!(v["attached"][0]["caption"], "narrow");
    let card = app.state.identity_store.work_board_get(&id).unwrap().unwrap().card;
    assert_eq!(card.media.count, 1);
    assert!(card.media.ui_change);
}

#[tokio::test]
async fn complete_is_held_to_the_same_rule_as_submit() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({"ui_change": true})).await;
    let uri = format!("/agentmux/work/{num}/complete");
    let (s, v) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "result": "done"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("no_media_reason"), "tells how to get through: {v}");
    let id = app.state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().item.id;
    assert_eq!(app.state.identity_store.work_queue_get(&id).unwrap().unwrap().state, "claimed", "nothing moved");

    // A stale attempt is still a conflict, not a media refusal.
    let (s, _) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt + 1, "result": "done"})).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Media attached with update, then complete goes through.
    let (_, up) = upload(&app, "after.png", png(7)).await;
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/update"), json!({"agent_id": "a1", "attempt": attempt, "media": [{"sha256": up["sha256"]}]})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "result": "done"})).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("review")), "{v}");
}

#[tokio::test]
async fn a_reason_lets_a_ui_task_through_and_shows_on_the_card() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({"ui_change": true})).await;
    let (s, v) = post(
        &app,
        &format!("/agentmux/work/{num}/submit"),
        json!({"agent_id": "a1", "attempt": attempt, "text": "done", "no_media_reason": "headless host, nothing to see"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let card = app.state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().card;
    assert_eq!(card.media.no_media_reason, "headless host, nothing to see");
    assert_eq!(card.media.count, 0);
}

#[tokio::test]
async fn a_reason_holds_for_its_own_attempt_only() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({"ui_change": true})).await;
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/update"), json!({"agent_id": "a1", "attempt": attempt, "no_media_reason": "nothing to see"})).await;
    assert_eq!(s, StatusCode::OK);
    // Handed back and taken again: a new attempt.
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/release"), json!({"agent_id": "a1", "attempt": attempt, "result": "later"})).await;
    assert_eq!(s, StatusCode::OK);
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let second = c["attempt"].as_i64().unwrap();
    assert!(second > attempt, "a new attempt: {c}");
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": second, "text": "done"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the old reason does not carry over: {v}");
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/update"), json!({"agent_id": "a1", "attempt": second, "no_media_reason": "still nothing"})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": second, "text": "done"})).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("review")), "{v}");
}

#[tokio::test]
async fn a_task_not_marked_as_ui_is_handed_in_as_before() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({})).await;
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": attempt, "text": "done"})).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("review")), "{v}");

    // And one that needs no review needs no media, marked or not.
    let (num, attempt) = claimed(&app, json!({"ui_change": true, "review": false})).await;
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": attempt, "text": "done"})).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("done")), "{v}");
}

#[tokio::test]
async fn an_agent_marks_a_task_as_ui_while_working_and_a_pr_link_brings_a_reminder_not_a_refusal() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({})).await;
    let uri = format!("/agentmux/work/{num}/update");
    let (s, v) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "pr_url": "https://github.com/o/r/pull/1"})).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["reminder"].is_null(), "not marked: no reminder: {v}");

    let (s, _) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "ui_change": true})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "pr_url": "https://github.com/o/r/pull/1"})).await;
    assert_eq!(s, StatusCode::OK, "mid-work: never a refusal");
    assert!(v["reminder"].as_str().unwrap().contains("screenshots"), "{v}");

    let (_, up) = upload(&app, "a.png", png(4)).await;
    let (_, v) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "media": [{"sha256": up["sha256"]}], "pr_url": "https://github.com/o/r/pull/1"})).await;
    assert!(v["reminder"].is_null(), "attached: no reminder: {v}");
    assert_eq!(v["attached"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn only_the_holder_attaches_and_the_limit_per_call_holds() {
    let app = app();
    let (num, attempt) = claimed(&app, json!({})).await;
    let (_, up) = upload(&app, "a.png", png(5)).await;
    let uri = format!("/agentmux/work/{num}/update");
    let (s, _) = post(&app, &uri, json!({"agent_id": "a2", "attempt": attempt, "media": [{"sha256": up["sha256"]}]})).await;
    assert_eq!(s, StatusCode::CONFLICT, "not the holder");

    let many: Vec<Value> = (0..11).map(|_| json!({"sha256": up["sha256"]})).collect();
    let (s, v) = post(&app, &uri, json!({"agent_id": "a1", "attempt": attempt, "media": many})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("10 files"), "{v}");
    let id = app.state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().item.id;
    assert!(app.state.identity_store.work_media_list(&id).unwrap().is_empty(), "a refused call attaches nothing");
}

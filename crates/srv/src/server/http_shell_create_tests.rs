// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/shell/create` driven through the router: where the command runs and with which
//! environment, what the shell is listed as, and the requests refused before anything starts.

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn insert_agent_block(state: &AppState, block_id: &str, meta: &[(&str, serde_json::Value)]) {
    let mut m = crate::backend::obj::MetaMapType::new();
    for (k, v) in meta {
        m.insert(k.to_string(), v.clone());
    }
    let mut block = crate::backend::obj::Block { oid: block_id.to_string(), meta: m, ..Default::default() };
    state.mstore.insert(&mut block).unwrap();
}

async fn post_create(state: &AppState, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/shell/create")
        .header("X-AuthKey", "test-secret-key")
        .header("content-type", "application/json")
        .header("X-Agent-Token", crate::server::tests::token_on_block(state, body["agent_block_id"].as_str().unwrap_or_default()))
        .body(Body::from(body.to_string()))
        .unwrap();
    let call = crate::server::routes::build_router(state.clone()).oneshot(req);
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), call).await.expect("the create answered within 20s").unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

fn agent_id() -> String {
    format!("shell-agent-{}", uuid::Uuid::new_v4())
}

/// What the command wrote to `out.txt` in `dir`, once it has.
#[cfg(unix)]
async fn written_by_command(dir: &std::path::Path) -> String {
    let out = dir.join("out.txt");
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&out) {
            if !text.is_empty() {
                return text;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the command wrote nothing to {}", out.display());
}

/// With no cwd of its own the command starts in the agent block's `cmd:cwd`; its environment is the
/// block's `cmd:env` with the caller's values laid over it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_command_runs_in_the_agent_blocks_cwd_with_its_env_under_the_callers() {
    let state = crate::server::tests::test_state();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let agent = agent_id();
    insert_agent_block(
        &state,
        &agent,
        &[("cmd:cwd", json!(cwd.to_string_lossy())), ("cmd:env", json!({ "FROM_BLOCK": "b", "BOTH": "block" }))],
    );

    let body = json!({
        "agent_block_id": agent,
        "cmd": "printf '%s|%s|%s|%s' \"$(pwd -P)\" \"$FROM_BLOCK\" \"$BOTH\" \"$FROM_REQ\" > out.txt",
        "env": { "BOTH": "caller", "FROM_REQ": "r" },
    });
    let (status, reply) = post_create(&state, body).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(reply["shell_id"].as_str().is_some_and(|s| !s.is_empty()), "{reply}");

    let written = written_by_command(&cwd).await;
    assert_eq!(written, format!("{}|b|caller|r", cwd.display()));
}

/// A cwd in the request is used as given, ahead of the agent block's own.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_cwd_in_the_request_wins_over_the_agent_blocks() {
    let state = crate::server::tests::test_state();
    let block_dir = tempfile::tempdir().unwrap();
    let asked_dir = tempfile::tempdir().unwrap();
    let asked = asked_dir.path().canonicalize().unwrap();
    let agent = agent_id();
    insert_agent_block(&state, &agent, &[("cmd:cwd", json!(block_dir.path().to_string_lossy()))]);

    let body = json!({ "agent_block_id": agent, "cmd": "pwd -P > out.txt", "cwd": asked.to_string_lossy() });
    let (status, reply) = post_create(&state, body).await;
    assert_eq!(status, StatusCode::OK, "{reply}");

    assert_eq!(written_by_command(&asked).await.trim(), asked.to_string_lossy());
}

/// A running shell is listed against the agent's pane, titled with the command unless the caller
/// gave a title.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_running_shell_is_listed_with_its_command_and_title() {
    let state = crate::server::tests::test_state();
    let agent = agent_id();
    insert_agent_block(&state, &agent, &[]);

    let (status, plain) = post_create(&state, json!({ "agent_block_id": agent, "cmd": "sleep 30" })).await;
    assert_eq!(status, StatusCode::OK, "{plain}");
    let (status, titled) = post_create(&state, json!({ "agent_block_id": agent, "cmd": "sleep 31", "title": "dev server" })).await;
    assert_eq!(status, StatusCode::OK, "{titled}");
    let plain_id = plain["shell_id"].as_str().unwrap().to_string();
    let titled_id = titled["shell_id"].as_str().unwrap().to_string();
    assert_ne!(plain_id, titled_id);

    let mut listed = Vec::new();
    for _ in 0..100 {
        listed = state.shell_sessions.list_active();
        if listed.iter().filter(|s| s.block_id == agent).count() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let find = |id: &str| listed.iter().find(|s| s.shell_id == id).unwrap_or_else(|| panic!("{id} is listed"));
    assert_eq!((find(&plain_id).cmd.as_str(), find(&plain_id).title.as_str()), ("sleep 30", "sleep 30"));
    assert_eq!((find(&titled_id).cmd.as_str(), find(&titled_id).title.as_str()), ("sleep 31", "dev server"));
    assert_eq!(find(&plain_id).block_id, agent);

    assert!(state.shell_sessions.stop(&plain_id));
    assert!(state.shell_sessions.stop(&titled_id));
}

/// A connection that cannot be used fails the call with its reason, and nothing runs here instead.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_that_cannot_be_used_fails_the_call() {
    let state = crate::server::tests::test_state();
    let agent = agent_id();
    insert_agent_block(&state, &agent, &[]);

    let body = json!({ "agent_block_id": agent, "cmd": "true", "connection": "no-such-host://x" });
    let (status, reply) = post_create(&state, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(reply["error"].as_str().is_some_and(|e| !e.is_empty()), "{reply}");
    assert!(state.shell_sessions.list_active().iter().all(|s| s.block_id != agent));
}

/// stdin is closed once a Vault value is written to it, so the two cannot be asked for together.
#[tokio::test(flavor = "multi_thread")]
async fn a_stdin_secret_cannot_be_combined_with_capture_stdin() {
    let state = crate::server::tests::test_state();
    let agent = agent_id();
    insert_agent_block(&state, &agent, &[]);

    let body = json!({ "agent_block_id": agent, "cmd": "cat", "capture_stdin": true, "stdin_secret": "an-entry" });
    let (status, reply) = post_create(&state, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(reply["error"].as_str().unwrap().contains("capture_stdin"), "{reply}");
}

/// Asking for a Vault value without saying why is refused before anything is asked or run.
#[tokio::test(flavor = "multi_thread")]
async fn a_vault_secret_without_a_reason_is_refused() {
    let state = crate::server::tests::test_state();
    let agent = agent_id();
    insert_agent_block(&state, &agent, &[]);

    let body = json!({ "agent_block_id": agent, "cmd": "true", "secrets": { "TOKEN": "an-entry" } });
    let (status, reply) = post_create(&state, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{reply}");
    assert!(reply["error"].as_str().is_some_and(|e| !e.is_empty()), "{reply}");
    assert!(state.shell_sessions.list_active().iter().all(|s| s.block_id != agent));
}

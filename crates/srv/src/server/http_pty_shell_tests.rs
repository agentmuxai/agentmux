// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ptyshell/create` driven through the router, against a real local shell.

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
        .uri("/api/v1/ptyshell/create")
        .header("X-AuthKey", "test-secret-key")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let call = crate::server::routes::build_router(state.clone()).oneshot(req);
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), call).await.expect("the create answered within 20s").unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

fn block_of(state: &AppState, id: &str) -> crate::backend::obj::Block {
    state.mstore.get::<crate::backend::obj::Block>(id).unwrap().expect("the block exists")
}

/// Runs a test that starts a real shell on its own runtime, and gives up on the runtime's blocking
/// threads (the shell's PTY reader) a few seconds after the test body ends instead of waiting for them.
#[cfg(unix)]
fn run_with_shell<F: std::future::Future<Output = ()>>(test: F) {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    rt.block_on(test);
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
}

fn stop_shell(shell_id: &str) {
    let _ = blockcontroller::stop_block_controller(shell_id);
    blockcontroller::delete_controller(shell_id);
}

/// A fresh create builds the pane's shell sub-block (the headless `term` shape, the automatic
/// shell, the agent block's cwd, the caller's geometry), points the parent at it and starts the
/// controller; a second create attaches to that same shell instead of making another.
#[cfg(unix)]
#[test]
fn create_makes_the_panes_shell_and_a_second_create_attaches_to_it() {
    run_with_shell(async {
    let state = crate::server::tests::test_state();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap().to_string_lossy().to_string();
    let parent = format!("pty-parent-{}", uuid::Uuid::new_v4());
    insert_agent_block(&state, &parent, &[("cmd:cwd", json!(cwd))]);

    let (status, body) = post_create(&state, json!({ "agent_block_id": parent, "rows": 30, "cols": 100 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shell_id = body["shell_id"].as_str().expect("a shell id").to_string();

    let shell = block_of(&state, &shell_id);
    assert_eq!(shell.parentoref, format!("block:{parent}"));
    assert_eq!(shell.meta.get("view"), Some(&json!("term")));
    assert_eq!(shell.meta.get(blockcontroller::META_KEY_CONTROLLER), Some(&json!(blockcontroller::BLOCK_CONTROLLER_SHELL)));
    assert_eq!(shell.meta.get(META_KEY_PTYSHELL_BOOT_ID), Some(&json!("test-boot")));
    assert_eq!(shell.meta.get(blockcontroller::META_KEY_TERM_LOCAL_SHELL_OPTS), Some(&json!([])));
    assert_eq!(shell.meta.get(blockcontroller::META_KEY_CMD_CWD), Some(&json!(cwd)));
    assert!(shell.meta.contains_key(crate::backend::gh_guard::META_KEY_PTYSHELL_GH_CONFIG_DIR));
    let size = &shell.runtimeopts.as_ref().expect("the caller's geometry").termsize;
    assert_eq!((size.rows, size.cols), (30, 100));

    let parent_block = block_of(&state, &parent);
    assert_eq!(parent_block.meta.get(META_KEY_SHELL_SUBBLOCK_ID), Some(&json!(shell_id)));
    assert_eq!(parent_block.subblockids.as_deref(), Some(&[shell_id.clone()][..]));
    assert!(blockcontroller::get_block_controller_status(&shell_id).is_some(), "the controller was started");

    let (status, again) = post_create(&state, json!({ "agent_block_id": parent })).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["shell_id"], json!(shell_id), "the second create joins the same shell");

    stop_shell(&shell_id);
    });
}

/// A pointer that names a block that is gone is replaced by a fresh shell, not followed.
#[cfg(unix)]
#[test]
fn a_stale_pointer_is_replaced_by_a_fresh_shell() {
    run_with_shell(async {
    let state = crate::server::tests::test_state();
    let parent = format!("pty-stale-{}", uuid::Uuid::new_v4());
    insert_agent_block(&state, &parent, &[(META_KEY_SHELL_SUBBLOCK_ID, json!("ghost-shell"))]);

    let (status, body) = post_create(&state, json!({ "agent_block_id": parent })).await;
    // The claim sees the pointer already set, so it pivots to the winner, finds nothing, and inserts for real.
    assert_eq!(status, StatusCode::OK, "{body}");
    let shell_id = body["shell_id"].as_str().expect("a shell id").to_string();
    assert_ne!(shell_id, "ghost-shell");
    assert_eq!(block_of(&state, &parent).meta.get(META_KEY_SHELL_SUBBLOCK_ID), Some(&json!(shell_id)));
    assert_eq!(block_of(&state, &shell_id).parentoref, format!("block:{parent}"));

    stop_shell(&shell_id);
    });
}

#[tokio::test]
async fn create_for_a_pane_that_does_not_exist_fails_the_claim() {
    let state = crate::server::tests::test_state();
    let (status, body) = post_create(&state, json!({ "agent_block_id": "no-such-pane" })).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body["error"].as_str().unwrap_or("").starts_with("ptyshell.create: claim:"), "{body}");
}

#[tokio::test]
async fn create_on_a_connection_that_is_refused_fails_with_its_reason() {
    let state = crate::server::tests::test_state();
    let parent = format!("pty-conn-{}", uuid::Uuid::new_v4());
    insert_agent_block(&state, &parent, &[]);
    let (status, body) = post_create(&state, json!({ "agent_block_id": parent, "connection": "nonsense://nowhere" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()), "{body}");
    assert!(block_of(&state, &parent).meta.get(META_KEY_SHELL_SUBBLOCK_ID).is_none(), "no shell was made");
}

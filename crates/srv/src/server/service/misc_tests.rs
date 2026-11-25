// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Behaviour tests for the catch-all service handler.

use super::*;
use crate::server::tests::test_state;

fn save_terminal_state_call(block_id: &str, state_str: &str, pty_offset: i64) -> WebCallType {
    WebCallType {
        service: "block".to_string(),
        method: "SaveTerminalState".to_string(),
        uicontext: None,
        args: vec![
            serde_json::json!(block_id),
            serde_json::json!(state_str),
            serde_json::json!("full"),
            serde_json::json!(pty_offset),
            serde_json::json!({ "rows": 24, "cols": 80 }),
        ],
    }
}

#[tokio::test]
async fn save_terminal_state_persists_content_and_meta() {
    // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.2 — the
    // previously-stubbed RPC must actually write the "cache:term:full"
    // blockfile the frontend's `loadInitialTerminalData()` reads back.
    let state = test_state();
    let call = save_terminal_state_call("block-1", "serialized-xterm-state", 42);

    let result = handle_misc_service(&state, &call).await;
    assert!(result.success, "expected success, got {:?}", result.error);

    let content = state
        .filestore
        .read_file("block-1", "cache:term:full")
        .expect("read_file ok")
        .expect("cache file should exist");
    assert_eq!(content, b"serialized-xterm-state");

    let file = state
        .filestore
        .stat("block-1", "cache:term:full")
        .expect("stat ok")
        .expect("file should exist");
    assert_eq!(file.meta["ptyoffset"], serde_json::json!(42));
    assert_eq!(file.meta["termsize"], serde_json::json!({ "rows": 24, "cols": 80 }));
}

#[tokio::test]
async fn save_terminal_state_overwrites_on_second_snapshot() {
    // Periodic snapshots replace the previous one wholesale, they don't
    // accumulate — matches the frontend's "one current snapshot" model.
    let state = test_state();

    let first = save_terminal_state_call("block-1", "first-snapshot", 10);
    assert!(handle_misc_service(&state, &first).await.success);

    let second = save_terminal_state_call("block-1", "second-snapshot-longer", 99);
    assert!(handle_misc_service(&state, &second).await.success);

    let content = state
        .filestore
        .read_file("block-1", "cache:term:full")
        .unwrap()
        .unwrap();
    assert_eq!(content, b"second-snapshot-longer");

    let file = state.filestore.stat("block-1", "cache:term:full").unwrap().unwrap();
    assert_eq!(file.meta["ptyoffset"], serde_json::json!(99));
}

#[tokio::test]
async fn save_terminal_state_missing_block_id_errors() {
    let state = test_state();
    let call = WebCallType {
        service: "block".to_string(),
        method: "SaveTerminalState".to_string(),
        uicontext: None,
        args: vec![],
    };
    let result = handle_misc_service(&state, &call).await;
    assert!(!result.success);
    assert!(result.error.is_some());
}

fn call(service: &str, method: &str, args: Vec<serde_json::Value>) -> WebCallType {
    WebCallType {
        service: service.to_string(),
        method: method.to_string(),
        uicontext: None,
        args,
    }
}

async fn run(service: &str, method: &str, args: Vec<serde_json::Value>) -> WebReturnType {
    let state = test_state();
    handle_misc_service(&state, &call(service, method, args)).await
}

#[tokio::test]
async fn unknown_method_names_the_service_and_method() {
    let result = run("block", "NoSuchMethod", vec![]).await;
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("unknown service method: block.NoSuchMethod"));
    let result = run("nosuch", "GetControllerStatus", vec![]).await;
    assert_eq!(result.error.as_deref(), Some("unknown service method: nosuch.GetControllerStatus"));
}

#[tokio::test]
async fn user_input_response_is_accepted_and_dropped() {
    let result = run("userinput", "SendUserInputResponse", vec![serde_json::json!({"x": 1})]).await;
    assert!(result.success);
    assert!(result.data.is_none());
}

#[tokio::test]
async fn block_send_command_is_an_empty_success() {
    let result = run("block", "SendCommand", vec![]).await;
    assert!(result.success);
    assert!(result.data.is_none());
}

#[tokio::test]
async fn controller_status_of_an_unknown_block_is_the_default_for_that_id() {
    let result = run("block", "GetControllerStatus", vec![serde_json::json!("block-unknown")]).await;
    assert!(result.success);
    let data = result.data.expect("status data");
    assert_eq!(data["blockid"], serde_json::json!("block-unknown"));
}

#[tokio::test]
async fn controller_status_without_a_block_id_is_an_argument_error() {
    let result = run("block", "GetControllerStatus", vec![]).await;
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("missing argument at index 0"));
}

#[tokio::test]
async fn save_terminal_state_without_a_state_is_an_argument_error() {
    let result = run("block", "SaveTerminalState", vec![serde_json::json!("block-1")]).await;
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("missing argument at index 1"));
}

#[tokio::test]
async fn save_terminal_state_defaults_offset_and_size_when_omitted() {
    let state = test_state();
    let args = vec![serde_json::json!("block-d"), serde_json::json!("snap")];
    let result = handle_misc_service(&state, &call("block", "SaveTerminalState", args)).await;
    assert!(result.success);
    let file = state.filestore.stat("block-d", "cache:term:full").unwrap().unwrap();
    assert_eq!(file.meta["ptyoffset"], serde_json::json!(0));
    assert_eq!(file.meta["termsize"], serde_json::Value::Null);
}

#[tokio::test]
async fn shell_and_cron_listings_are_empty_arrays_on_a_fresh_state() {
    let shells = run("shell", "ListActive", vec![]).await;
    assert!(shells.success);
    assert_eq!(shells.data, Some(serde_json::json!([])));
    let cron = run("cron", "ListActive", vec![]).await;
    assert!(cron.success);
    assert_eq!(cron.data, Some(serde_json::json!([])));
}

#[tokio::test]
async fn subagent_listings_are_empty_on_a_fresh_state() {
    for method in ["ListActive", "ListDispatches"] {
        let result = run("subagent", method, vec![]).await;
        assert!(result.success, "{method}");
        assert_eq!(result.data, Some(serde_json::json!([])), "{method}");
    }
    let history = run("subagent", "GetHistory", vec![serde_json::json!("nobody"), serde_json::json!(5)]).await;
    assert!(history.success);
    assert_eq!(history.data, Some(serde_json::json!([])));
    let info = run("subagent", "GetInfo", vec![serde_json::json!("nobody")]).await;
    assert!(info.success);
    assert_eq!(info.data, Some(serde_json::Value::Null));
}

#[tokio::test]
async fn subagent_lookups_need_an_agent_id() {
    for method in ["GetHistory", "GetInfo", "GenerateName", "WatchAgent"] {
        let result = run("subagent", method, vec![]).await;
        assert!(!result.success, "{method}");
        assert_eq!(result.error.as_deref(), Some("missing argument at index 0"), "{method}");
    }
}

#[tokio::test]
async fn watch_agent_needs_a_config_dir() {
    let result = run("subagent", "WatchAgent", vec![serde_json::json!("agent-x")]).await;
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("missing argument at index 1"));
}

#[tokio::test]
async fn resolve_unnamed_backlog_returns_at_once() {
    let result = run("subagent", "ResolveUnnamedBacklog", vec![]).await;
    assert!(result.success);
    assert!(result.data.is_none());
}

#[tokio::test]
async fn history_calls_that_name_a_session_need_the_id() {
    for method in ["ListForAgent", "Get"] {
        let result = run("history", method, vec![]).await;
        assert!(!result.success, "{method}");
        assert_eq!(result.error.as_deref(), Some("missing argument at index 0"), "{method}");
    }
}

#[tokio::test]
async fn history_list_filtered_to_an_unknown_provider_succeeds() {
    let args = vec![serde_json::json!("no-such-provider"), serde_json::Value::Null, serde_json::json!(0), serde_json::json!(5)];
    let result = run("history", "List", args).await;
    assert!(result.success, "{:?}", result.error);
    assert!(result.data.is_some());
}

#[tokio::test]
async fn history_get_of_an_unknown_session_still_answers() {
    let get = run("history", "Get", vec![serde_json::json!("no-such-session")]).await;
    assert!(get.success, "{:?}", get.error);
}

#[tokio::test]
async fn history_list_for_an_unknown_agent_succeeds() {
    let result = run("history", "ListForAgent", vec![serde_json::json!("agent-none")]).await;
    assert!(result.success, "{:?}", result.error);
}

#[tokio::test]
async fn the_route_refuses_to_delete_or_clear_history_and_to_define_an_agent_whatever_the_arguments() {
    let valid_define = serde_json::json!({"name": "intruder"});
    for (service, method, args) in [
        ("history", "Delete", vec![serde_json::json!("no-such-session")]),
        ("history", "Delete", vec![]),
        ("history", "Clear", vec![serde_json::json!("no-such-provider"), serde_json::json!("no-such-project")]),
        ("history", "Clear", vec![]),
        ("agent", "define", vec![valid_define]),
        ("agent", "define", vec![]),
    ] {
        let result = run(service, method, args).await;
        assert!(!result.success, "{service}.{method}");
        assert!(result.error.as_deref().is_some_and(|e| e.starts_with("FORBIDDEN: ")), "{service}.{method}: {:?}", result.error);
    }
}

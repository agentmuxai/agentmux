// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `restart_resume`: which parked agents come back after a restart.

use super::*;
use crate::backend::agent_session::agent_current_zone;
use crate::backend::storage::filestore::{FileMeta, FileOpts};
use crate::sagas::park_agents_tests::{dispatch_apply, seed_agent_block, seed_workspace, set_always_available};
use crate::server::app_api::agent_attach::{set_desired, DESIRED_STOPPED};
use crate::server::tests::test_state;
use agentmux_common::ipc::Command;

fn set_meta(state: &AppState, block_id: &str, meta: serde_json::Value) {
    let map: obj::MetaMapType = serde_json::from_value(meta).unwrap();
    crate::server::service::update_object_meta(&state.mstore, &format!("block:{block_id}"), &map).unwrap();
}

fn wrote_a_turn(state: &AppState, uid: &str) {
    let zone = agent_current_zone(uid);
    state.filestore.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    state.filestore.write_file(&zone, OUTPUT_FILE, b"{}\n").unwrap();
}

/// A parked agent that was wanted running, has a session and wrote a turn just now.
async fn resumable(state: &AppState, tab: &str, agent: &str) -> String {
    let block = seed_agent_block(state, tab, agent).await;
    dispatch_apply(state, Command::ParkBlock { tab_id: tab.to_string(), block_id: block.clone() }).await;
    set_desired(&state.mstore, &block, DESIRED_RUNNING);
    set_meta(state, &block, serde_json::json!({ "agent:sessionid": "sess-1" }));
    wrote_a_turn(state, agent);
    block
}

#[test]
fn an_agent_is_active_within_its_window_and_any_agent_that_ran_is_when_there_is_none() {
    assert!(active_within(1_000, 3_601_000, Some(3_600_000)), "exactly the window");
    assert!(!active_within(999, 3_601_000, Some(3_600_000)), "a millisecond past it");
    assert!(active_within(0, 99_000_000, None), "no window: never stopped for idleness");
    assert!(active_within(9_000, 1_000, Some(10)), "a clock step back is not a long idle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_a_parked_agent_wanted_running_with_a_session_and_a_recent_turn_comes_back() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let back = resumable(&state, &tabs[0], "ag-back").await;

    let stopped = resumable(&state, &tabs[0], "ag-stopped").await;
    set_desired(&state.mstore, &stopped, DESIRED_STOPPED);

    let lazy = resumable(&state, &tabs[0], "ag-lazy").await;
    set_meta(&state, &lazy, serde_json::json!({ "agent:sessionid": "" }));

    let silent = seed_agent_block(&state, &tabs[0], "ag-silent").await;
    dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: silent.clone() }).await;
    set_desired(&state.mstore, &silent, DESIRED_RUNNING);
    set_meta(&state, &silent, serde_json::json!({ "agent:sessionid": "sess-2" }));

    let signed_out = resumable(&state, &tabs[0], "ag-signed-out").await;
    set_meta(
        &state,
        &signed_out,
        serde_json::json!({ "agent:last_failure": { "code": "auth", "title": "t", "detail": "d", "retryable": false } }),
    );

    let in_terminal = resumable(&state, &tabs[0], "ag-term").await;
    set_meta(&state, &in_terminal, serde_json::json!({ "agentProvider": "aider" }));

    // Shown in a tab: a window (or the startup backstop) brings it back, not this.
    let shown = seed_agent_block(&state, &tabs[0], "ag-shown").await;
    set_desired(&state.mstore, &shown, DESIRED_RUNNING);
    set_meta(&state, &shown, serde_json::json!({ "agent:sessionid": "sess-3" }));
    wrote_a_turn(&state, "ag-shown");

    let got: Vec<String> = candidates(&state).into_iter().map(|c| c.block_id).collect();
    assert_eq!(got, vec![back], "{got:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_most_recently_active_agent_comes_back_first() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let older = resumable(&state, &tabs[0], "ag-older").await;
    std::thread::sleep(Duration::from_millis(20));
    let newer = resumable(&state, &tabs[0], "ag-newer").await;

    let got: Vec<String> = candidates(&state).into_iter().map(|c| c.block_id).collect();
    assert_eq!(got, vec![newer, older]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_resume_starts_nothing_with_the_setting_off_and_stops_at_the_cap_instead_of_making_room() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let block = resumable(&state, &tabs[0], "ag-resume").await;

    assert_eq!(run_once(&state).await, 0, "the setting is off");
    assert!(blockcontroller::get_controller(&block).is_none());

    // One agent already runs and the cap is one: the parked one stays stopped, and the running one stays.
    state.config_watcher.update_settings(
        serde_json::from_value(serde_json::json!({
            crate::backend::always_available::SETTING_ALWAYS_AVAILABLE: true,
            crate::server::agent_lifecycle::SETTING_MAX_RUNNING: 1,
        }))
        .unwrap(),
    );
    let running = seed_agent_block(&state, &tabs[0], "ag-running").await;
    crate::backend::agent_state::test_support::register_quiet(&running, 1000);
    assert_eq!(run_once(&state).await, 0, "at the cap");
    assert!(blockcontroller::get_controller(&block).is_none(), "nothing was started or stopped to make room");
    blockcontroller::delete_controller(&running);
    set_always_available(&state, false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_room_the_parked_agent_is_started_and_gets_no_pane() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let block = resumable(&state, &tabs[0], "ag-go").await;

    assert_eq!(run_once(&state).await, 1);

    assert_eq!(crate::sagas::park_agents_tests::in_tab(&state, &block).await.as_deref(), Some(""), "it stays parked");
    assert!(blockcontroller::get_controller(&block).is_some(), "its process was started");
    blockcontroller::delete_controller(&block);
}

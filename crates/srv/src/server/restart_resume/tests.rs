// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `restart_resume`: which parked agents come back after a restart.

use super::*;
use crate::backend::agent_session::agent_current_zone;
use crate::backend::storage::filestore::{FileMeta, FileOpts};
use crate::sagas::park_agents_tests::{dispatch_apply, seed_agent_block, seed_block, seed_workspace, set_always_available};
use crate::server::app_api::agent_attach::{set_desired, DESIRED_STOPPED};
use crate::server::tests::test_state;
use agentmux_common::ipc::Command;

fn set_meta(state: &AppState, block_id: &str, meta: serde_json::Value) {
    let map: obj::MetaMapType = serde_json::from_value(meta).unwrap();
    crate::server::service::update_object_meta(&state.mstore, &format!("block:{block_id}"), &map).unwrap();
}

fn write_turn(store: &FileStore, zone: &str) {
    store.make_file(zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    store.write_file(zone, OUTPUT_FILE, b"{}\n").unwrap();
}

/// A turn as the controller writes it: into the block's own zone of this channel's store.
fn wrote_a_turn(state: &AppState, block_id: &str) {
    write_turn(&state.filestore, block_id);
}

/// A parked agent that was wanted running, has a session and wrote a turn just now.
async fn resumable(state: &AppState, tab: &str, agent: &str) -> String {
    let block = seed_agent_block(state, tab, agent).await;
    dispatch_apply(state, Command::ParkBlock { tab_id: tab.to_string(), block_id: block.clone() }).await;
    set_desired(&state.mstore, &block, DESIRED_RUNNING);
    set_meta(state, &block, serde_json::json!({ "agent:sessionid": "sess-1" }));
    wrote_a_turn(state, &block);
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
    wrote_a_turn(&state, &shown);

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

    set_always_available(&state, false);
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

/// The newest write of a turn counts wherever it was kept: the global transcript store holds the agent's
/// zone, the channel's store the block's, and an older layout the agent's zone there too.
#[test]
fn the_last_turn_is_the_newest_write_in_any_of_the_stores() {
    let global = FileStore::open_in_memory().unwrap();
    let local = FileStore::open_in_memory().unwrap();
    assert_eq!(latest_write_ms(&[(&local, "block-1"), (&global, "agent:a:current")]), None, "nothing written");

    write_turn(&global, "agent:a:current");
    let from_global = latest_write_ms(&[(&local, "block-1"), (&global, "agent:a:current")]).expect("the global store counts");

    std::thread::sleep(Duration::from_millis(20));
    write_turn(&local, "block-1");
    let newest = latest_write_ms(&[(&local, "block-1"), (&global, "agent:a:current")]).expect("both");
    assert!(newest > from_global, "the block's zone is newer: {newest} vs {from_global}");
}

/// An agent whose only transcript is the agent's zone in this channel's store (an older layout) still counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_layouts_agent_zone_in_the_channel_store_still_counts() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let block = seed_agent_block(&state, &tabs[0], "ag-legacy").await;
    dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: block.clone() }).await;
    set_desired(&state.mstore, &block, DESIRED_RUNNING);
    set_meta(&state, &block, serde_json::json!({ "agent:sessionid": "sess-1" }));
    write_turn(&state.filestore, &agent_current_zone("ag-legacy"));

    let got: Vec<String> = candidates(&state).into_iter().map(|c| c.block_id).collect();
    assert_eq!(got, vec![block]);
}

/// Two parked blocks of one agent with another agent's between them in time: the agent starts once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_with_two_parked_blocks_is_started_once_whatever_lies_between_them_in_time() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let _older = resumable(&state, &tabs[0], "ag-twice").await;
    std::thread::sleep(Duration::from_millis(20));
    let _other = resumable(&state, &tabs[0], "ag-between").await;
    std::thread::sleep(Duration::from_millis(20));
    let newer = seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "ag-twice" })).await;
    dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: newer.clone() }).await;
    set_desired(&state.mstore, &newer, DESIRED_RUNNING);
    set_meta(&state, &newer, serde_json::json!({ "agent:sessionid": "sess-9" }));
    wrote_a_turn(&state, &newer);

    let mut uids: Vec<String> = candidates(&state).into_iter().map(|c| c.uid).collect();
    uids.sort();
    assert_eq!(uids, vec!["ag-between".to_string(), "ag-twice".to_string()], "once each");
}

/// An agent with several parked blocks is judged on the one a start runs (the lowest id among them), not
/// on a newer one that passes: the other block is never resumed in its place.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_block_that_is_checked_is_the_block_a_start_runs() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let first = resumable(&state, &tabs[0], "ag-pick").await;
    let second = seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "ag-pick" })).await;
    dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: second.clone() }).await;
    set_desired(&state.mstore, &second, DESIRED_RUNNING);
    set_meta(&state, &second, serde_json::json!({ "agent:sessionid": "sess-2" }));
    wrote_a_turn(&state, &second);
    let (chosen, other) = if first < second { (first, second) } else { (second, first) };

    set_desired(&state.mstore, &chosen, DESIRED_STOPPED);
    assert!(candidates(&state).is_empty(), "the block a start runs is stopped; the other is not resumed instead");

    set_desired(&state.mstore, &chosen, DESIRED_RUNNING);
    let got: Vec<String> = candidates(&state).into_iter().map(|c| c.block_id).collect();
    assert_eq!(got, vec![chosen], "only the block a start runs, never both ({other} is left)");
}

/// An agent stopped after the list was made is not started from it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_stopped_while_the_list_was_being_worked_is_not_started() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let block = resumable(&state, &tabs[0], "ag-late-stop").await;
    let queued = candidates(&state).into_iter().next().expect("a candidate");
    assert!(still_wanted(&state, &queued));

    set_desired(&state.mstore, &block, DESIRED_STOPPED);

    assert!(!still_wanted(&state, &queued), "the stop is seen right before the start");
}


/// A claimed task for `slug`/`uid`, held until the test ends.
fn claim_a_task(state: &AppState, uid: &str) {
    use crate::backend::storage::work_queue::{work_state, ClaimFilter, WorkItem};
    let item = WorkItem {
        id: format!("w-{uid}"),
        title: "t".into(),
        payload: "p".into(),
        kind: String::new(),
        target_agent: String::new(),
        target_group: String::new(),
        priority: 0,
        state: work_state::OPEN.into(),
        claimed_by: String::new(),
        target_agent_uid: String::new(),
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: "tester".into(),
        created_by_uid: String::new(),
        created_at: 1000,
        updated_at: 1000,
        not_before: None,
        result: String::new(),
    };
    state.work_store.work_queue_enqueue(&item).unwrap();
    let slug = state.mstore.agent_def_get(uid).unwrap().unwrap().slug;
    let filter = ClaimFilter { kind: None, agent_id: slug, agent_uid: uid.into(), groups: vec![] };
    state.work_store.work_queue_claim(&filter, 2000, 600_000).unwrap().expect("claimed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_quiet_past_its_window_still_comes_back_if_it_was_running_at_exit_or_the_sweep_keeps_it() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let quiet = resumable(&state, &tabs[0], "ag-quiet").await;
    let ran = resumable(&state, &tabs[0], "ag-ran").await;
    let claims = resumable(&state, &tabs[0], "ag-claims").await;
    // A window of about a millisecond: every turn written above is older than it a moment later.
    for block in [&quiet, &ran, &claims] {
        set_meta(&state, block, serde_json::json!({ "agent:idle_stop_minutes": 0.00002 }));
    }
    claim_a_task(&state, "ag-claims");
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;

    // Nothing says they ran: only the one that holds a claimed task (the sweep never stops it) is back.
    let got: Vec<String> = candidates(&state).into_iter().map(|c| c.block_id).collect();
    assert_eq!(got, vec![claims.clone()], "{got:?}");
    // The exit left ag-ran running: back, whatever its last turn; the quiet one is still not.
    let ran_at_exit: HashSet<String> = ["ag-ran".to_string()].into_iter().collect();
    let mut got: Vec<String> = candidates_with(&state, &ran_at_exit).into_iter().map(|c| c.block_id).collect();
    got.sort();
    let mut want = vec![claims, ran];
    want.sort();
    assert_eq!(got, want, "{got:?}");
    assert!(!got.contains(&quiet));
}

#[test]
fn the_agents_running_at_exit_stay_on_file_until_the_resume_has_run() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_running(dir.path()).is_empty(), "no file: none");
    write_running(dir.path(), &["a".to_string(), "b".to_string()]);
    assert_eq!(read_running(dir.path()), vec!["a".to_string(), "b".to_string()]);
    assert_eq!(read_running(dir.path()), vec!["a".to_string(), "b".to_string()], "a crash before the resume ran loses nothing");
    clear_running(dir.path());
    assert!(read_running(dir.path()).is_empty(), "spent");
    // An exit with nothing running leaves nothing from an earlier one.
    write_running(dir.path(), &["a".to_string()]);
    write_running(dir.path(), &[]);
    assert!(read_running(dir.path()).is_empty());
    std::fs::write(dir.path().join(RUNNING_FILE), b"not json").unwrap();
    assert!(read_running(dir.path()).is_empty(), "an unreadable list is none");
}

#[test]
fn an_exit_before_the_resume_ran_carries_the_agents_still_waiting() {
    let running = vec!["c".to_string(), "a".to_string()];
    let pending = vec!["b".to_string(), "a".to_string()];
    assert_eq!(with_pending(running, &pending), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert!(with_pending(Vec::new(), &[]).is_empty());
}

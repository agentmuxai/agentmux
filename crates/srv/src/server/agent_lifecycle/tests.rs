// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `agent_lifecycle`: the idle stop and the cap on running agents.

use super::*;
use crate::backend::agent_state::test_support::register_quiet;
use crate::backend::storage::work_queue::{work_state, ClaimFilter, WorkItem};
use crate::sagas::park_agents_tests::{dispatch_apply, seed_agent_block, seed_workspace, set_always_available};
use crate::server::tests::test_state;
use agentmux_common::ipc::Command;

fn extra(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn live(parked: bool, state: AgentState, since_ms: u64) -> Live {
    Live { block_id: "b".into(), agent_id: "a".into(), uid: "u".into(), parked, state: Some(state), since_ms, idle_override: None }
}

#[test]
fn the_defaults_are_an_hour_twelve_agents_and_85_percent_and_zero_turns_each_off() {
    let p = Policy::from_extra(&extra(&[]));
    assert_eq!((p.idle_stop_ms, p.max_running, p.commit_limit_pct), (Some(3_600_000), Some(24), Some(85.0)));
    let p = Policy::from_extra(&extra(&[
        (SETTING_IDLE_STOP_MINUTES, serde_json::json!(0)),
        (SETTING_MAX_RUNNING, serde_json::json!(0)),
        (SETTING_COMMIT_LIMIT_PCT, serde_json::json!(0)),
    ]));
    assert_eq!((p.idle_stop_ms, p.max_running, p.commit_limit_pct), (None, None, None));
    let p = Policy::from_extra(&extra(&[(SETTING_IDLE_STOP_MINUTES, serde_json::json!(5)), (SETTING_MAX_RUNNING, serde_json::json!(3))]));
    assert_eq!((p.idle_stop_ms, p.max_running), (Some(300_000), Some(3)));
}

#[test]
fn only_an_idle_agent_with_no_view_is_stopped_and_only_after_its_window() {
    let hour = Some(3_600_000);
    let now = 10_000_000;
    assert!(live(true, AgentState::Idle, now - 3_600_000).idle_expired(now, hour));
    assert!(!live(true, AgentState::Idle, now - 3_599_999).idle_expired(now, hour), "not yet");
    assert!(!live(false, AgentState::Idle, 0).idle_expired(now, hour), "a shown agent has a view");
    for busy in [AgentState::Working, AgentState::Waiting, AgentState::Limited, AgentState::SignedOut] {
        assert!(!live(true, busy, 0).idle_expired(now, hour), "{busy:?} is not idle");
    }
    assert!(!live(true, AgentState::Idle, 0).idle_expired(now, None), "the window is off");
}

#[test]
fn an_agents_own_window_replaces_the_default() {
    let now = 10_000_000;
    let mut a = live(true, AgentState::Idle, now - 600_000);
    a.idle_override = Some(Some(300_000));
    assert!(a.idle_expired(now, Some(3_600_000)), "its own 5 minutes, not the hour");
    a.idle_override = Some(None);
    assert!(!a.idle_expired(now, Some(1)), "0 = never stop it");
    assert!(!a.evictable(), "and never stopped to make room either");
    a.idle_override = Some(Some(3_600_000));
    assert!(!a.idle_expired(now, Some(1)), "a longer window of its own");
    assert!(a.evictable());
}

#[test]
fn the_build_slot_holder_is_the_agent_line_of_its_owner_file() {
    assert_eq!(holder_in("agent=posa\npid=2241\ntoken=x\n").as_deref(), Some("posa"));
    assert_eq!(holder_in("pid=1\n"), None);
    assert_eq!(holder_in("agent=\n"), None);
}

#[test]
fn the_cap_and_the_commit_limit_are_each_a_reason_to_make_room() {
    let p = Policy::from_extra(&extra(&[(SETTING_MAX_RUNNING, serde_json::json!(2))]));
    assert!(over_limit(&p, 1, None).is_none());
    assert!(matches!(over_limit(&p, 2, None), Some(Over::Cap(why)) if why.contains("limit is 2")));
    assert!(over_limit(&p, 1, Some(84.0)).is_none());
    assert!(matches!(over_limit(&p, 1, Some(86.0)), Some(Over::Commit(why)) if why.contains("86%")));
    let off = Policy::from_extra(&extra(&[(SETTING_MAX_RUNNING, serde_json::json!(0)), (SETTING_COMMIT_LIMIT_PCT, serde_json::json!(0))]));
    assert!(over_limit(&off, 99, Some(99.0)).is_none());
}

/// A parked agent whose process is a running stub, quiet between turns.
async fn parked_running(state: &AppState, tab: &str, agent: &str) -> String {
    let block = seed_agent_block(state, tab, agent).await;
    dispatch_apply(state, Command::ParkBlock { tab_id: tab.to_string(), block_id: block.clone() }).await;
    register_quiet(&block, 1000);
    block
}

/// A stop leaves the controller registered (finished); what it records is the wish to stay stopped.
fn was_stopped(state: &AppState, block_id: &str) -> bool {
    let block = state.mstore.get::<obj::Block>(block_id).unwrap().unwrap();
    crate::server::app_api::agent_attach::desired_of(&block).as_deref() == Some("stopped")
}

fn set_limits(state: &AppState, max: u32) {
    state.config_watcher.update_settings(
        serde_json::from_value(serde_json::json!({
            crate::backend::always_available::SETTING_ALWAYS_AVAILABLE: true,
            SETTING_MAX_RUNNING: max,
        }))
        .unwrap(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_the_cap_a_start_stops_the_agent_idle_longest_and_a_running_agent_needs_no_room() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let first = parked_running(&state, &tabs[0], "ag-cap-1").await;
    let second = parked_running(&state, &tabs[0], "ag-cap-2").await;
    set_limits(&state, 2);

    admit_start_with(&state, "ag-cap-1", || None).await.expect("already running: no new process");
    assert!(!was_stopped(&state, &first) && !was_stopped(&state, &second), "an agent that already runs makes no room");

    admit_start_with(&state, "ag-cap-3", || None).await.expect("room made");
    let stopped = [&first, &second].iter().filter(|b| was_stopped(&state, b)).count();
    assert_eq!(stopped, 1, "one of the two idle agents was stopped to make room");

    for b in [&first, &second] {
        blockcontroller::delete_controller(b);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_the_cap_with_nothing_to_stop_a_start_is_refused_and_off_it_is_admitted() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    // A shown agent has a view; it is never stopped to make room.
    let shown = seed_agent_block(&state, &tabs[0], "ag-shown").await;
    register_quiet(&shown, 1000);
    set_limits(&state, 1);

    let err = admit_start_with(&state, "ag-new", || None).await.unwrap_err();
    assert!(err.starts_with("capacity:"), "{err}");
    assert!(!was_stopped(&state, &shown), "the agent with a view was left running");

    set_always_available(&state, false);
    admit_start_with(&state, "ag-new", || None).await.expect("the setting is off: no lifecycle policy");
    blockcontroller::delete_controller(&shown);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_that_holds_a_claimed_task_or_a_held_message_is_not_stopped_to_make_room() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let busy = parked_running(&state, &tabs[0], "ag-claims").await;
    set_limits(&state, 1);
    let item = WorkItem {
        id: "w1".into(),
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
    let slug = state.mstore.agent_def_get("ag-claims").unwrap().unwrap().slug;
    let filter = ClaimFilter { kind: None, agent_id: slug, agent_uid: "ag-claims".into(), groups: vec![] };
    state.work_store.work_queue_claim(&filter, 2000, 600_000).unwrap().expect("claimed");

    let err = admit_start_with(&state, "ag-new", || None).await.unwrap_err();
    assert!(err.starts_with("capacity:"), "{err}");
    assert!(!was_stopped(&state, &busy), "an agent with a claimed task keeps running");
    blockcontroller::delete_controller(&busy);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sweep_stops_nothing_with_the_setting_off_and_nothing_that_is_not_yet_idle_long_enough() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let block = parked_running(&state, &tabs[0], "ag-sweep").await;
    set_always_available(&state, false);
    assert_eq!(sweep_once(&state).await, 0, "off");
    set_limits(&state, 12);
    assert_eq!(sweep_once(&state).await, 0, "idle for a moment, not an hour");
    assert!(!was_stopped(&state, &block));
    blockcontroller::delete_controller(&block);
}

/// Starts admitted a moment apart count each other until the first is seen running, so a burst cannot
/// each see room that the others are already using.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_starts_counts_the_ones_already_admitted() {
    let state = test_state();
    set_limits(&state, 2);
    admit_start_with(&state, "ag-burst-1", || None).await.expect("room for the first");
    admit_start_with(&state, "ag-burst-2", || None).await.expect("room for the second");
    let err = admit_start_with(&state, "ag-burst-3", || None).await.unwrap_err();
    assert!(err.starts_with("capacity:") && err.contains("2 agents are running"), "{err}");
}

/// Over the commit limit one idle agent is stopped, not all of them, for a figure that has not fallen yet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn over_the_commit_limit_one_agent_is_stopped_not_every_idle_one() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let blocks = [
        parked_running(&state, &tabs[0], "ag-commit-1").await,
        parked_running(&state, &tabs[0], "ag-commit-2").await,
        parked_running(&state, &tabs[0], "ag-commit-3").await,
    ];
    set_limits(&state, 99);

    let err = admit_start_with(&state, "ag-commit-new", || Some(99.0)).await.unwrap_err();

    assert!(err.starts_with("capacity:") && err.contains("did not bring it down"), "{err}");
    assert_eq!(blocks.iter().filter(|b| was_stopped(&state, b)).count(), 1);
    for b in &blocks {
        blockcontroller::delete_controller(b);
    }
}

/// A reservation ends when its agent is seen running: evicting that agent then frees its room, where
/// before the reservation counted again and the start was refused after both idle agents were stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reservation_ends_when_its_agent_runs_so_stopping_it_frees_room() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_limits(&state, 2);
    admit_start_with(&state, "ag-res-1", || None).await.expect("room for the first");
    admit_start_with(&state, "ag-res-2", || None).await.expect("room for the second");
    let blocks = [parked_running(&state, &tabs[0], "ag-res-1").await, parked_running(&state, &tabs[0], "ag-res-2").await];

    admit_start_with(&state, "ag-res-3", || None).await.expect("one stop makes room");

    assert_eq!(blocks.iter().filter(|b| was_stopped(&state, b)).count(), 1, "one agent was stopped, not both");
    for b in &blocks {
        blockcontroller::delete_controller(b);
    }
}

/// A start that did not happen gives its room back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_that_failed_gives_its_room_back() {
    let state = test_state();
    set_limits(&state, 1);
    admit_start_with(&state, "ag-fail-1", || None).await.expect("room");
    admit_start_with(&state, "ag-fail-2", || None).await.unwrap_err();
    release_start(&state, "ag-fail-1");
    admit_start_with(&state, "ag-fail-2", || None).await.expect("the failed start no longer holds the room");
}

/// The idle window runs from when the agent lost its view, not from when it went idle in its pane.
#[test]
fn the_idle_window_starts_when_the_view_is_lost() {
    let mut lives = vec![live(true, AgentState::Idle, 1_000), live(false, AgentState::Idle, 1_000)];
    lives[0].block_id = "b-lost-1".into();
    lives[1].block_id = "b-lost-2".into();
    let mut lost = HashMap::new();
    idle_from_view_lost(&mut lost, &mut lives, 5_000_000);
    assert_eq!(lives[0].since_ms, 5_000_000, "parked just now, idle in its pane for much longer");
    assert_eq!(lives[1].since_ms, 1_000, "a shown agent keeps its own clock");
    assert!(!lives[0].idle_expired(5_000_000 + 3_599_999, Some(3_600_000)));
    assert!(lives[0].idle_expired(5_000_000 + 3_600_000, Some(3_600_000)));

    lives[0].since_ms = 1_000;
    idle_from_view_lost(&mut lost, &mut lives, 9_000_000);
    assert_eq!(lives[0].since_ms, 5_000_000, "first seen parked stays the start of the window");
}

/// A start that must not cost another agent its process is refused at the cap, where the ordinary start
/// stops an idle agent with no view to make room.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_without_eviction_is_refused_at_the_cap_and_stops_nobody() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let idle = parked_running(&state, &tabs[0], "ag-keep").await;
    set_limits(&state, 1);

    let err = admit_start_without_eviction(&state, "ag-more").await.unwrap_err();

    assert!(err.starts_with("capacity:") && err.contains("does not stop other agents"), "{err}");
    assert!(!was_stopped(&state, &idle), "the idle agent was left running");
    blockcontroller::delete_controller(&idle);
}

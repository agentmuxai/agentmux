// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The quiet-agent watch (SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §2 to §5, phase 1).

use super::*;
use crate::backend::reactive::AuditLogEntry;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::work_queue::{ClaimFilter, WorkItem};
use crate::test_support::CoreParts;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

const MIN: u64 = 60_000;
const T0: u64 = 1_000_000_000;

fn lim() -> Limits {
    limits_from(&HashMap::new()).expect("on by default")
}

fn idle(activity_ms: u64) -> Reading {
    Reading { activity_ms, turn_active: false }
}

fn running(activity_ms: u64) -> Reading {
    Reading { activity_ms, turn_active: true }
}

#[test]
fn the_times_come_from_settings_and_zero_turns_the_watch_off() {
    assert_eq!(lim(), Limits { quiet_ms: 15 * MIN, running_ms: 30 * MIN, escalate_ms: 5 * MIN });
    let set = |pairs: &[(&str, serde_json::Value)]| pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<HashMap<_, _>>();
    assert_eq!(
        limits_from(&set(&[(SETTING_QUIET_MINUTES, json!(10)), (SETTING_QUIET_RUNNING_MINUTES, json!(20)), (SETTING_QUIET_ESCALATE_MINUTES, json!(2))])),
        Some(Limits { quiet_ms: 10 * MIN, running_ms: 20 * MIN, escalate_ms: 2 * MIN })
    );
    assert_eq!(limits_from(&set(&[(SETTING_QUIET_MINUTES, json!(0))])), None, "0 turns it off");
    assert_eq!(limits_from(&set(&[(SETTING_QUIET_MINUTES, json!(-3))])), Some(lim()), "a negative number is the default");
    assert_eq!(limits_from(&set(&[(SETTING_QUIET_MINUTES, json!("soon"))])), Some(lim()), "so is text");
}

/// Q1 with a fake clock: nothing while it moves, one nudge at the quiet time, the watchers five minutes later, and
/// no second note about the same quiet for an hour.
#[test]
fn an_idle_agent_is_nudged_at_the_quiet_time_and_its_watchers_told_five_minutes_later() {
    let mut w = Watch::default();
    let l = lim();
    assert_eq!(w.step("a", idle(T0), &l, T0), None, "just seen");
    assert_eq!(w.step("a", idle(T0), &l, T0 + 14 * MIN), None, "not yet quiet");
    assert_eq!(w.step("a", idle(T0), &l, T0 + 15 * MIN), Some(Action::Nudge));
    assert_eq!(w.step("a", idle(T0), &l, T0 + 16 * MIN), None, "nudged once");
    assert_eq!(w.step("a", idle(T0), &l, T0 + 19 * MIN), None, "the grace is not over");
    assert_eq!(w.step("a", idle(T0), &l, T0 + 20 * MIN), Some(Action::Tell { running: false, idle_ms: 20 * MIN }));
    // A note nobody could receive (no watcher open, or the one-a-minute gap held it) is tried again at the next sweep.
    assert_eq!(w.step("a", idle(T0), &l, T0 + 20 * MIN + 30_000), Some(Action::Tell { running: false, idle_ms: 20 * MIN + 30_000 }), "not delivered: again");
    w.mark_told("a", T0 + 20 * MIN + 30_000);
    assert_eq!(w.step("a", idle(T0), &l, T0 + 40 * MIN), None, "told once");
    assert_eq!(w.step("a", idle(T0), &l, T0 + 81 * MIN), Some(Action::Tell { running: false, idle_ms: 81 * MIN }), "an hour later, still quiet");
}

/// The nudge is allowed once an hour: an agent that answers it and goes quiet again within the hour is not
/// nudged again; its watchers are told at the quiet time.
#[test]
fn no_nudge_twice_in_an_hour() {
    let mut w = Watch::default();
    let l = lim();
    w.step("a", idle(T0), &l, T0);
    assert_eq!(w.step("a", idle(T0), &l, T0 + 15 * MIN), Some(Action::Nudge));
    // It answers the nudge (output), then says nothing for the quiet time.
    let answered = T0 + 16 * MIN;
    assert_eq!(w.step("a", idle(answered), &l, T0 + 17 * MIN), None, "it moved: a new episode");
    assert_eq!(w.step("a", idle(answered), &l, answered + 14 * MIN), None);
    assert_eq!(
        w.step("a", idle(answered), &l, answered + 15 * MIN),
        Some(Action::Tell { running: false, idle_ms: 15 * MIN }),
        "no second nudge within the hour: its watchers hear at the quiet time"
    );
    // Another episode more than an hour after the nudge nudges again.
    let later = T0 + 90 * MIN;
    assert_eq!(w.step("a", idle(later), &l, later + 1), None);
    assert_eq!(w.step("a", idle(later), &l, later + 15 * MIN), Some(Action::Nudge));
}

/// Q2: a turn that is running and has written nothing is quiet only after the longer time, with no nudge.
#[test]
fn a_running_turn_is_quiet_after_the_running_time_and_is_not_nudged() {
    let mut w = Watch::default();
    let l = lim();
    w.step("a", running(T0), &l, T0);
    assert_eq!(w.step("a", running(T0), &l, T0 + 15 * MIN), None, "a long build is healthy at 15");
    assert_eq!(w.step("a", running(T0), &l, T0 + 34 * MIN), None);
    assert_eq!(w.step("a", running(T0), &l, T0 + 35 * MIN), Some(Action::Tell { running: true, idle_ms: 35 * MIN }));
    w.mark_told("a", T0 + 35 * MIN);
    assert_eq!(w.step("a", running(T0), &l, T0 + 50 * MIN), None, "told once");
}

/// Something only the agent does resets the clock; an agent seen for the first time, or after a restart of srv,
/// gets a whole quiet time whatever its old activity says.
#[test]
fn its_own_activity_resets_the_clock_and_a_new_episode_starts_with_a_whole_quiet_time() {
    let mut w = Watch::default();
    let l = lim();
    // Last wrote output days ago, just given work.
    let days_ago = T0 - 3 * 24 * 60 * MIN;
    assert_eq!(w.step("a", idle(days_ago), &l, T0), None, "first seen: the clock starts now");
    assert_eq!(w.step("a", idle(days_ago), &l, T0 + 14 * MIN), None);
    assert_eq!(w.step("a", idle(days_ago), &l, T0 + 15 * MIN), Some(Action::Nudge));
    // Output after the nudge: a new episode.
    assert_eq!(w.step("a", idle(T0 + 16 * MIN), &l, T0 + 21 * MIN), None, "it moved, so its watchers are not told");
    // Forgotten when it owes nothing: the next episode starts again with a whole quiet time.
    w.retain(&HashSet::new());
    assert_eq!(w.step("a", idle(days_ago), &l, T0 + 200 * MIN), None);
    assert_eq!(w.step("a", idle(days_ago), &l, T0 + 214 * MIN), None);
    assert_eq!(w.step("a", idle(days_ago), &l, T0 + 215 * MIN), Some(Action::Nudge), "a whole quiet time again, and the last nudge is over an hour old");
    // Two agents have their own clocks.
    assert_eq!(w.step("b", idle(0), &l, T0 + 215 * MIN), None);
}

#[test]
fn only_a_working_or_idle_agent_can_be_called_quiet() {
    assert!(callable(Some(AgentState::Working)));
    assert!(callable(Some(AgentState::Idle)));
    for state in [Some(AgentState::Waiting), Some(AgentState::Stopped), Some(AgentState::Error), Some(AgentState::Limited), Some(AgentState::SignedOut), None] {
        assert!(!callable(state), "{state:?}");
    }
}

/// The Kanban's store, as srv opens it.
fn work_store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap(), dir)
}

fn item(id: &str, filer_uid: &str) -> WorkItem {
    WorkItem {
        id: id.into(),
        title: format!("task {id}"),
        payload: "do it".into(),
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
        created_by: "someone".into(),
        created_by_uid: filer_uid.into(),
        created_at: 1,
        updated_at: 1,
        not_before: None,
        result: String::new(),
    }
}

/// The tasks an agent owes are the ones it holds claimed: not an open one, a held one, one handed in for review, or
/// one waiting on an answer.
#[test]
fn an_agent_owes_the_tasks_it_holds_claimed_and_no_others() {
    let (work, _dir) = work_store();
    let now = 2_000_000_000_000i64;
    let holder = ClaimFilter { agent_id: "helper".into(), agent_uid: "uid-helper".into(), ..ClaimFilter::default() };
    work.work_queue_enqueue(&item("claimed", "uid-filer")).unwrap();
    let got = work.work_queue_claim(&holder, now, 120_000).unwrap().expect("claimed");
    assert_eq!(got.id, "claimed");
    work.work_queue_enqueue(&item("open", "uid-filer")).unwrap();
    let mut held = item("held", "uid-filer");
    held.state = work_state::HELD.into();
    work.work_queue_enqueue(&held).unwrap();

    let def = test_agent_def("uid-helper", "Helper", "claude", "host", 1, "");
    let other = test_agent_def("uid-other", "Other", "claude", "host", 1, "");
    let claimed = claimed_by_holder(&read_claimed(&work, now));
    let owed = owed_by(&claimed, &def);
    assert_eq!(owed.len(), 1, "{owed:?}: only the claimed one");
    assert_eq!(owed[0].filer_uid, "uid-filer");
    assert!(owed[0].num > 0);
    assert!(owed_by(&claimed, &other).is_empty(), "another agent holds nothing");
}

fn dispatch_apply(state: &CoreParts, cmd: Command) -> impl std::future::Future<Output = Vec<Event>> + '_ {
    state.dispatch_apply(cmd)
}

async fn pane(state: &CoreParts, tab: &str, def: &str) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab.into(), meta: json!({ "view": "agent", "agentId": def }) })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

async fn tab(state: &CoreParts) -> String {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

fn mark_worker(state: &CoreParts, worker_id: &str, lead: &crate::backend::storage::agents::AgentDefinition) {
    state
        .mstore
        .agent_content_set(&crate::backend::storage::store::AgentContent {
            agent_id: worker_id.to_string(),
            content_type: crate::backend::agent_seat::WORKER_OF_CONTENT.into(),
            content: json!({ "agent": lead.name, "id": lead.id }).to_string(),
            updated_at: 1,
        })
        .unwrap();
}

/// The watchers are the worker's leader and the agents that filed its tasks: each once, never the agent itself,
/// and only one with a pane open.
#[tokio::test]
async fn the_watchers_are_the_leader_and_the_filers_of_its_tasks_each_once() {
    let state = crate::test_support::core_parts();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (lead_id, worker_id, filer_id, closed_id) = (format!("lead-{id}"), format!("worker-{id}"), format!("filer-{id}"), format!("closed-{id}"));
    let mut lead = test_agent_def(&lead_id, &format!("Lead {}", &id[..8]), "claude", "host", 1, "");
    let mut worker = test_agent_def(&worker_id, &format!("Helper {}", &id[..8]), "claude", "host", 1, "");
    let mut filer = test_agent_def(&filer_id, &format!("Filer {}", &id[..8]), "claude", "host", 1, "");
    let mut closed = test_agent_def(&closed_id, &format!("Closed {}", &id[..8]), "claude", "host", 1, "");
    for d in [&mut lead, &mut worker, &mut filer, &mut closed] {
        state.mstore.agent_def_insert(d).unwrap();
    }
    mark_worker(&state, &worker_id, &lead);
    let (lead_pane, filer_pane) = (pane(&state, &tab, &lead_id).await, pane(&state, &tab, &filer_id).await);
    let worker_pane = pane(&state, &tab, &worker_id).await;

    assert_eq!(watcher_panes(&state.mstore, &worker_id, &[]), vec![lead_pane.clone()], "its leader");
    assert_eq!(
        watcher_panes(&state.mstore, &worker_id, &[filer_id.clone()]),
        vec![lead_pane.clone(), filer_pane.clone()],
        "the leader, and the filer of a task it holds: a second watcher is told too"
    );
    assert_eq!(watcher_panes(&state.mstore, &worker_id, &[lead_id.clone(), filer_id.clone(), filer_id.clone()]), vec![lead_pane.clone(), filer_pane.clone()], "each once");
    assert_eq!(watcher_panes(&state.mstore, &worker_id, &[worker_id.clone(), String::new(), closed_id.clone()]), vec![lead_pane], "not itself, not an empty filer, not one with no pane");
    assert!(!worker_pane.is_empty());
}

/// An agent that is not a spawned worker but holds a claimed task is watched by the filer of the task: its
/// failure, exit and waiting question reach the filer's pane, and the text says "agent".
#[tokio::test]
async fn an_agent_holding_a_claimed_task_is_watched_by_its_filer() {
    let state = crate::test_support::core_parts();
    let (work, _dir) = work_store();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (holder_id, filer_id) = (format!("holder-{id}"), format!("filer-{id}"));
    let mut holder = test_agent_def(&holder_id, &format!("Holder {}", &id[..8]), "claude", "host", 1, "");
    let mut filer = test_agent_def(&filer_id, &format!("Filer {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut holder).unwrap();
    state.mstore.agent_def_insert(&mut filer).unwrap();
    let filer_pane = pane(&state, &tab, &filer_id).await;
    let holder_pane = pane(&state, &tab, &holder_id).await;
    let failed = crate::backend::worker_notes::WorkerEvent::Failed { block_id: holder_pane.clone(), title: "Not authenticated".into() };

    assert_eq!(crate::backend::worker_notes::note_for(&state.mstore, Some(&work), &state.filestore, &failed), None, "it holds nothing yet");

    work.work_queue_enqueue(&item("t1", &filer_id)).unwrap();
    let claim = ClaimFilter { agent_id: holder.slug.clone(), agent_uid: holder_id.clone(), ..ClaimFilter::default() };
    work.work_queue_claim(&claim, agentmux_common::time::now_ms(), 120_000).unwrap().expect("claimed");

    let (to, text, wait) = crate::backend::worker_notes::note_for(&state.mstore, Some(&work), &state.filestore, &failed).expect("the filer is told");
    assert_eq!(to, vec![filer_pane]);
    assert_eq!(wait, None);
    assert!(text.starts_with(&format!("[AgentMux] Your agent {} stopped with an error: Not authenticated.", holder.name)), "{text}");
}

/// The filer of a task a non-worker agent holds is told by the agent's own failure note, so the separate note the
/// sender's failure would add is skipped; an agent that did not file it still gets that one.
#[tokio::test]
async fn the_filers_own_failure_note_stands_in_for_the_senders() {
    let state = crate::test_support::core_parts();
    let (work, _dir) = work_store();
    let tab = tab(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (holder_id, filer_id, other_id) = (format!("holder-{id}"), format!("filer-{id}"), format!("other-{id}"));
    let mut holder = test_agent_def(&holder_id, &format!("Holder {}", &id[..8]), "claude", "host", 1, "");
    let mut filer = test_agent_def(&filer_id, &format!("Filer {}", &id[..8]), "claude", "host", 1, "");
    let mut other = test_agent_def(&other_id, &format!("Other {}", &id[..8]), "claude", "host", 1, "");
    for d in [&mut holder, &mut filer, &mut other] {
        state.mstore.agent_def_insert(d).unwrap();
    }
    pane(&state, &tab, &filer_id).await;
    pane(&state, &tab, &other_id).await;
    let holder_pane = pane(&state, &tab, &holder_id).await;
    work.work_queue_enqueue(&item("t1", &filer_id)).unwrap();
    let claim = ClaimFilter { agent_id: holder.slug.clone(), agent_uid: holder_id.clone(), ..ClaimFilter::default() };
    work.work_queue_claim(&claim, agentmux_common::time::now_ms(), 120_000).unwrap().expect("claimed");

    crate::backend::worker_failure_note::record_failure_admission(&holder_pane, true);
    assert!(notes::already_tells(&state.mstore, Some(&work), &holder_pane, &filer.slug), "the filer is a watcher whose note went out");
    assert!(!notes::already_tells(&state.mstore, Some(&work), &holder_pane, &other.slug), "another agent is not");
    crate::backend::worker_failure_note::record_failure_admission(&holder_pane, false);
    assert!(!notes::already_tells(&state.mstore, Some(&work), &holder_pane, &filer.slug), "the note was held by the gap: the sender's own goes through");
}

/// A claimed task is found however many newer rows the board has: a row limit would cut off the oldest, which are
/// the tasks of the agents that have been quiet longest.
#[test]
fn an_old_claimed_task_is_found_among_many_newer_rows() {
    let (work, _dir) = work_store();
    let now = 2_000_000_000_000i64;
    work.work_queue_enqueue(&item("old", "uid-filer")).unwrap();
    let holder = ClaimFilter { agent_id: "helper".into(), agent_uid: "uid-helper".into(), ..ClaimFilter::default() };
    work.work_queue_claim(&holder, now, 120_000).unwrap().expect("claimed");
    for n in 0..620 {
        let mut newer = item(&format!("newer-{n}"), "uid-filer");
        newer.updated_at = now + 1_000_000 + n;
        newer.created_at = newer.updated_at;
        work.work_queue_enqueue(&newer).unwrap();
    }
    let def = test_agent_def("uid-helper", "Helper", "claude", "host", 1, "");
    let owed = owed_by(&claimed_by_holder(&read_claimed(&work, now)), &def);
    assert_eq!(owed.len(), 1, "{owed:?}");
}

fn delivery(source: &str, block: &str, at: u64) -> AuditLogEntry {
    serde_json::from_value(json!({
        "timestamp": at, "source_agent": source, "target_agent": "x", "block_id": block,
        "message_hash": "h", "message_length": 1, "success": true, "request_id": "r"
    }))
    .unwrap()
}

/// A message the agent sends to anyone is its own activity; one sent to it is not.
#[test]
fn a_message_the_agent_sends_is_activity_and_one_sent_to_it_is_not() {
    let me = format!("quiet-sender-{}", uuid::Uuid::new_v4());
    assert_eq!(notes::last_message_by(&me), None);
    notes::absorb(&[delivery(&me.to_uppercase(), "some-pane", 5_000), delivery(&me, "another-pane", 9_000), delivery("leader-q", &me, 20_000)]);
    assert_eq!(notes::last_message_by(&me), Some(9_000), "the newest it sent, to any pane, in any case; the leader's message to it does not count");
}

#[test]
fn the_note_to_a_watcher_says_who_what_and_how_long() {
    let tasks = vec![OwedTask { num: 1234, filer_uid: String::new(), updated_ms: 0 }, OwedTask { num: 1240, filer_uid: String::new(), updated_ms: 0 }];
    assert_eq!(quiet_body("Pulp", false, &tasks, 22, false, ""), "Your agent Pulp has been quiet for 22 min on #1234, #1240. It is idle.");
    assert_eq!(
        quiet_body("Pulp", true, &[], 20, false, "next I run the check"),
        "Your worker Pulp has been quiet for 20 min without reporting to you. It is idle after a turn that ended: \"next I run the check\""
    );
    assert_eq!(quiet_body("Pulp", false, &tasks[..1], 35, true, ""), "Your agent Pulp has been quiet for 35 min on #1234. Its turn is still running but has written nothing.");
    assert_eq!(nudge_text(&tasks[..1], 15), "You hold #1234 and nothing has run for 15 minutes. Carry on, or say what you are waiting on.");
    assert_eq!(nudge_text(&[], 15), "You have not reported to your leader and nothing has run for 15 minutes. Carry on, or say what you are waiting on.");
}

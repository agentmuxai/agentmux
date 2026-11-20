// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::backend::obj::Tab;
use crate::backend::storage::store::AgentContent;
use crate::registry::DefinitionStore;

const NOW: u64 = 1_795_173_000_000;
const RESET_S: u64 = 1_795_180_200;
const RESET_MS: u64 = RESET_S * 1000;

fn account() -> String {
    format!("acct-{}", uuid::Uuid::new_v4())
}

/// A frame as Claude Code 2.1.301 sends it on an ordinary request.
fn allowed(five: f64, seven: f64) -> Value {
    json!({"type": "rate_limit_event", "rate_limit_info": {
        "status": "allowed", "resetsAt": RESET_S, "rateLimitType": "five_hour",
        "overageStatus": "rejected", "overageDisabledReason": "org_level_disabled", "isUsingOverage": false,
        "unifiedWindows": {
            "five_hour": {"utilization": five, "resetsAt": RESET_S},
            "seven_day": {"utilization": seven, "resetsAt": 1_795_251_600u64}}},
        "uuid": "f2b11406-76f9-48f4-8c25-2135f8f18cd9", "session_id": "efe7d7bf-b6f1-410c-8162-39863bbdc6ba"})
}

fn seen(account: &str, frame: &Value, now_ms: u64) -> Vec<Crossing> {
    observe(account, &windows_of_frame(frame), now_ms)
}

#[test]
fn a_frame_gives_each_window_its_use_and_reset() {
    let mut got = windows_of_frame(&allowed(0.6, 0.44));
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got, vec![("five_hour".to_string(), 0.6, RESET_S), ("seven_day".to_string(), 0.44, 1_795_251_600)]);
}

#[test]
fn a_rejected_frame_reads_the_same_way_and_other_frames_give_nothing() {
    let rejected = json!({"type": "rate_limit_event", "rate_limit_info": {
        "status": "rejected", "resetsAt": 1792886400u64, "rateLimitType": "seven_day",
        "overageStatus": "rejected", "unifiedWindows": {"seven_day": {"utilization": 1, "resetsAt": 1792886400u64}}}});
    assert_eq!(windows_of_frame(&rejected), vec![("seven_day".to_string(), 1.0, 1_792_886_400)]);
    assert!(windows_of_frame(&json!({"type": "result"})).is_empty());
    let none = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1}});
    assert!(windows_of_frame(&none).is_empty(), "no unifiedWindows");
    let broken = json!({"type": "rate_limit_event", "rate_limit_info": {"unifiedWindows": {
        "five_hour": {"resetsAt": 1}, "seven_day": {"utilization": "high", "resetsAt": 1}, "x": {"utilization": -1, "resetsAt": 1},
        "y": {"utilization": 0.5}}}});
    assert!(windows_of_frame(&broken).is_empty(), "a window with no number, a negative one or no reset is skipped");
}

#[test]
fn the_newest_reading_of_an_account_wins_whichever_pane_sent_it_and_carries_its_age() {
    let a = account();
    seen(&a, &allowed(0.6, 0.44), NOW);
    seen(&a, &allowed(0.62, 0.44), NOW + 30_000);
    let got = of_account(&a, NOW + 90_000);
    assert_eq!(got["five_hour"], UsageWindow { used_percent: 62.0, resets_at_ms: RESET_MS, observed_at_ms: NOW + 30_000, age_ms: 60_000 });
    assert_eq!(got["seven_day"].used_percent, 44.0);
    assert!(of_account(&account(), NOW).is_empty(), "another account has no reading");
}

#[test]
fn a_window_is_dropped_when_its_reset_passes() {
    let a = account();
    seen(&a, &allowed(0.6, 0.44), NOW);
    assert_eq!(of_account(&a, RESET_MS - 1).len(), 2);
    let after = of_account(&a, RESET_MS);
    assert_eq!(after.keys().collect::<Vec<_>>(), vec!["seven_day"], "the 5-hour window is over, the weekly one is not");
    assert!(of_account(&a, 1_795_251_600_000).is_empty());
}

#[test]
fn a_frame_from_the_window_before_does_not_replace_the_current_one() {
    let a = account();
    seen(&a, &allowed(0.3, 0.1), NOW);
    let older = json!({"type": "rate_limit_event", "rate_limit_info": {"unifiedWindows": {"five_hour": {"utilization": 0.99, "resetsAt": RESET_S - 18_000}}}});
    assert!(seen(&a, &older, NOW + 1).is_empty());
    assert_eq!(of_account(&a, NOW + 2)["five_hour"].used_percent, 30.0);
}

#[test]
fn each_threshold_tells_once_per_window_and_a_new_window_tells_again() {
    let a = account();
    assert!(seen(&a, &allowed(0.7, 0.1), NOW).is_empty(), "below both");
    let first = seen(&a, &allowed(0.76, 0.1), NOW + 1);
    assert_eq!(first, vec![Crossing { window: "five_hour".into(), threshold: 75, used_percent: 76.0, resets_at_ms: RESET_MS }]);
    assert!(seen(&a, &allowed(0.8, 0.1), NOW + 2).is_empty(), "75 is told");
    let high = seen(&a, &allowed(0.91, 0.1), NOW + 3);
    assert_eq!(high.iter().map(|c| c.threshold).collect::<Vec<_>>(), vec![90]);
    assert!(seen(&a, &allowed(0.95, 0.1), NOW + 4).is_empty(), "90 is told");
    // A reading that dips (a plan change) and rises again does not repeat either.
    assert!(seen(&a, &allowed(0.5, 0.1), NOW + 5).is_empty());
    assert!(seen(&a, &allowed(0.8, 0.1), NOW + 6).is_empty());
    // The next window, with a later reset, tells again.
    let next = json!({"type": "rate_limit_event", "rate_limit_info": {"unifiedWindows": {"five_hour": {"utilization": 0.8, "resetsAt": RESET_S + 18_000}}}});
    assert_eq!(seen(&a, &next, RESET_MS + 1).len(), 1);
}

#[test]
fn a_jump_over_both_thresholds_tells_the_high_one_only_and_a_weekly_window_is_its_own() {
    let a = account();
    let got = seen(&a, &allowed(0.95, 0.8), NOW);
    let mut got: Vec<(String, u64)> = got.into_iter().map(|c| (c.window, c.threshold)).collect();
    got.sort();
    assert_eq!(got, vec![("five_hour".to_string(), 90), ("seven_day".to_string(), 75)]);
    assert!(seen(&a, &allowed(0.96, 0.8), NOW + 1).is_empty(), "75 was skipped over, not owed");
}

#[test]
fn only_claude_reports_and_the_others_say_so() {
    let a = account();
    assert_eq!(AccountUsage::of("codex", &a, NOW).status, UsageStatus::NotReported);
    assert_eq!(AccountUsage::of("gemini", &a, NOW).status, UsageStatus::NotReported);
    assert_eq!(AccountUsage::of("claude", &a, NOW).status, UsageStatus::NotYet);
    seen(&a, &allowed(0.6, 0.44), NOW);
    let usage = AccountUsage::of("claude", &a, NOW + 1_000);
    assert_eq!(usage.status, UsageStatus::Reported);
    assert_eq!(usage.windows["five_hour"].age_ms, 1_000);
    assert_eq!(serde_json::to_value(&usage).unwrap()["status"], "reported");
    assert_eq!(serde_json::to_value(UsageStatus::NotReported).unwrap(), "not_reported");
    assert_eq!(AccountUsage::of("codex", &a, NOW).windows.len(), 0, "a not-reported provider shows no windows even on a shared id");
}

#[test]
fn the_note_names_the_window_the_use_and_the_reset_and_asks_nothing() {
    let c = Crossing { window: "five_hour".into(), threshold: 75, used_percent: 76.4, resets_at_ms: RESET_MS };
    let body = note_body(&c);
    assert!(body.contains("76%") && body.contains("5-hour usage window") && body.contains("resets "), "{body}");
    assert!(!body.contains("90"), "{body}");
    let high = note_body(&Crossing { threshold: 90, used_percent: 93.0, window: "seven_day".into(), ..c });
    assert!(high.contains("93%") && high.contains("weekly") && high.contains("hold new work"), "{high}");
}

fn store_with_registry() -> (Store, tempfile::TempDir) {
    let store = Store::open_in_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    store.set_def_registry(Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap()));
    (store, tmp)
}

fn pane(store: &Store, agent: &str, account: Option<&str>, running: bool) -> Block {
    let mut b = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    b.meta.insert("agentId".into(), json!(agent));
    if let Some(account) = account {
        b.meta.insert(RUN_ACCOUNT_META_KEY.into(), json!(account));
    }
    store.insert(&mut b).unwrap();
    if running {
        crate::backend::agent_state::test_support::register_stub(&b.oid, crate::backend::blockcontroller::BLOCK_CONTROLLER_PERSISTENT, crate::backend::blockcontroller::STATUS_RUNNING, false);
    }
    b
}

/// The leaders of the agents running on an account hear of it, each once; an agent with no leader
/// hears itself; a pane that is not running, and an account's neighbour, do not count.
#[test]
fn the_leaders_of_the_agents_running_on_the_account_are_the_ones_told() {
    let (store, _tmp) = store_with_registry();
    for (id, name) in [("lead-1", "Leader"), ("work-1", "WorkerOne"), ("work-2", "WorkerTwo"), ("solo", "Solo"), ("stopped", "Stopped"), ("other", "Other")] {
        let mut def = crate::backend::storage::agents::test_agent_def(id, name, "claude", "host", 1, "");
        store.agent_def_insert(&mut def).unwrap();
    }
    for worker in ["work-1", "work-2", "stopped"] {
        store
            .agent_content_set(&AgentContent {
                agent_id: worker.into(),
                content_type: crate::backend::agent_seat::WORKER_OF_CONTENT.into(),
                content: json!({ "agent": "Leader", "id": "lead-1" }).to_string(),
                updated_at: 1,
            })
            .unwrap();
    }
    let a = account();
    let leader = pane(&store, "lead-1", None, false);
    let mut tab = Tab { blockids: vec![leader.oid.clone()], ..Default::default() };
    tab.oid = uuid::Uuid::new_v4().to_string();
    store.insert(&mut tab).unwrap();
    let w1 = pane(&store, "work-1", Some(&a), true);
    let w2 = pane(&store, "work-2", Some(&a), true);
    let solo = pane(&store, "solo", Some(&a), true);
    pane(&store, "stopped", Some(&a), false);
    pane(&store, "other", Some("acct-elsewhere"), true);

    let mut targets = note_targets(&store, &a);
    targets.sort();
    let mut want = vec![leader.oid.clone(), solo.oid.clone()];
    want.sort();
    assert_eq!(targets, want, "one note for the leader of both workers, one for the agent with no leader");

    let crossing = Crossing { window: "five_hour".into(), threshold: 75, used_percent: 76.0, resets_at_ms: RESET_MS };
    notify_leaders(&store, &a, &crossing);
    for block in [&leader, &solo] {
        let (message, _held) = crate::backend::reactive::held_notes::attach(&block.oid, "next");
        assert!(message.contains("[AgentMux]") && message.contains("76%") && message.contains("next"), "{message}");
    }
    let (plain, _) = crate::backend::reactive::held_notes::attach(&w1.oid, "next");
    assert_eq!(plain, "next", "a worker is not told: its leader is");
    for id in [&w1.oid, &w2.oid, &solo.oid] {
        crate::backend::blockcontroller::delete_controller(id);
    }
}

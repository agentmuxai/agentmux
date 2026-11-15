// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::connectivity::ANTHROPIC;

const T0: i64 = 1_800_000_000_000;

fn api() -> Service {
    Service::ModelApi(&ANTHROPIC)
}

fn down(error: &str) -> Outcome {
    Outcome::Unreachable(error.to_string())
}

fn state_of(t: &Tracker, id: &str) -> (Reachability, i64) {
    let status = t.status();
    let s = status.services.iter().find(|s| s.service == id).unwrap_or_else(|| panic!("{id} in {status:?}"));
    (s.state, s.since_ms)
}

#[test]
fn transport_failures_in_a_row_make_a_service_offline() {
    let mut t = Tracker::new(0, T0);
    let first = t.report(api(), down("connection refused"), T0 + 1_000).expect("left online");
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Degraded, T0 + 1_000));
    assert_eq!(first.services[1].last_error.as_deref(), Some("connection refused"));
    assert_eq!(t.report(api(), down("connection refused"), T0 + 2_000), None, "still degraded: nothing to send");
    let offline = t.report(api(), down("connection refused"), T0 + 3_000).expect("went offline");
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Offline, T0 + 3_000));
    assert!(offline.revision > first.revision);
    assert_eq!(t.report(api(), down("connection refused"), T0 + 4_000), None);
    assert_eq!(state_of(&t, "model_api:anthropic").1, T0 + 3_000, "since is when it went offline");
}

#[test]
fn any_answer_brings_an_offline_service_back_online() {
    let mut t = Tracker::new(0, T0);
    for i in 0..OFFLINE_AFTER {
        t.report(Service::Muxbus, down("dns"), T0 + i64::from(i));
    }
    assert_eq!(state_of(&t, "muxbus").0, Reachability::Offline);
    t.report(Service::Muxbus, Outcome::Answered, T0 + 10_000).expect("back");
    assert_eq!(state_of(&t, "muxbus"), (Reachability::Online, T0 + 10_000));
    assert_eq!(t.status().services[1].last_error, None);
}

#[test]
fn some_failing_and_some_answered_is_degraded() {
    let mut t = Tracker::new(0, T0);
    t.report(Service::Muxbus, Outcome::Answered, T0);
    t.report(Service::Muxbus, down("reset"), T0 + 1_000);
    t.report(Service::Muxbus, Outcome::Answered, T0 + 2_000);
    t.report(Service::Muxbus, down("reset"), T0 + 3_000);
    t.report(Service::Muxbus, Outcome::Answered, T0 + 4_000);
    assert_eq!(state_of(&t, "muxbus"), (Reachability::Degraded, T0 + 3_000), "two failures in the last minute");
    // Once the failures are more than a minute old, an answer is all there is.
    t.report(Service::Muxbus, Outcome::Answered, T0 + 3_000 + DEGRADED_WINDOW_MS + 1);
    assert_eq!(state_of(&t, "muxbus").0, Reachability::Online);
}

#[test]
fn one_stray_failure_then_an_answer_is_online_again() {
    let mut t = Tracker::new(0, T0);
    t.report(api(), down("reset"), T0);
    t.report(api(), Outcome::Answered, T0 + 500);
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Online, T0 + 500));
}

#[test]
fn a_first_answer_adds_the_service_without_a_change_to_send() {
    let mut t = Tracker::new(0, T0);
    assert_eq!(t.report(api(), Outcome::Answered, T0), None);
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Online, T0));
}

#[test]
fn the_internet_is_offline_only_when_two_services_are_and_none_answers() {
    let mut t = Tracker::new(0, T0);
    for i in 0..OFFLINE_AFTER {
        t.report(api(), down("refused"), T0 + i64::from(i));
    }
    assert_eq!(state_of(&t, "internet").0, Reachability::Online, "one host down is that host's problem");
    for i in 0..OFFLINE_AFTER {
        t.report(Service::Muxbus, down("refused"), T0 + 100 + i64::from(i));
    }
    assert_eq!(state_of(&t, "internet"), (Reachability::Offline, T0 + 102));
    t.report(Service::Muxbus, Outcome::Answered, T0 + 5_000);
    assert_eq!(state_of(&t, "internet"), (Reachability::Online, T0 + 5_000));
    assert_eq!(state_of(&t, "model_api:anthropic").0, Reachability::Offline);
}

#[test]
fn the_internet_row_comes_first() {
    let mut t = Tracker::new(0, T0);
    t.report(Service::Muxbus, Outcome::Answered, T0);
    t.report(api(), Outcome::Answered, T0);
    let ids: Vec<String> = t.status().services.into_iter().map(|s| s.service).collect();
    assert_eq!(ids, ["internet", "model_api:anthropic", "muxbus"]);
}

#[test]
fn an_online_service_is_never_probed() {
    let mut t = Tracker::new(0, T0);
    t.report(api(), Outcome::Answered, T0);
    assert_eq!(t.next_probe_due(), None);
    assert!(t.take_due_probes(T0 + 3_600_000).is_empty());
}

#[test]
fn probes_back_off_from_five_seconds_to_a_minute() {
    let mut t = Tracker::new(0, T0);
    t.report(api(), down("refused"), T0);
    assert_eq!(t.next_probe_due(), Some(T0 + PROBE_FIRST_MS));
    assert!(t.take_due_probes(T0 + PROBE_FIRST_MS - 1).is_empty(), "not due yet");
    let mut now = T0 + PROBE_FIRST_MS;
    let mut waits = Vec::new();
    for _ in 0..6 {
        let due = t.take_due_probes(now);
        assert_eq!(due.len(), 1);
        assert_eq!(t.next_probe_due(), None, "running, not scheduled");
        let (service, ticket) = due.into_iter().next().unwrap();
        t.probe_finished(&service, ticket, down("refused"), now);
        let next = t.next_probe_due().unwrap();
        waits.push(next - now);
        now = next;
    }
    assert_eq!(waits, [10_000, 20_000, 40_000, 60_000, 60_000, 60_000]);
}

#[test]
fn a_failed_probe_means_offline() {
    let mut t = Tracker::new(0, T0);
    t.report(Service::Muxbus, down("reset"), T0);
    assert_eq!(state_of(&t, "muxbus").0, Reachability::Degraded);
    let (service, ticket) = t.take_due_probes(T0 + PROBE_FIRST_MS).remove(0);
    t.probe_finished(&service, ticket, down("timed out"), T0 + PROBE_FIRST_MS + 5_000).expect("changed");
    assert_eq!(state_of(&t, "muxbus"), (Reachability::Offline, T0 + PROBE_FIRST_MS + 5_000));
}

#[test]
fn an_answered_probe_ends_the_probing() {
    let mut t = Tracker::new(0, T0);
    for i in 0..OFFLINE_AFTER {
        t.report(api(), down("refused"), T0 + i64::from(i));
    }
    let (service, ticket) = t.take_due_probes(T0 + 60_000).remove(0);
    t.probe_finished(&service, ticket, Outcome::Answered, T0 + 60_100).expect("back");
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Online, T0 + 60_100));
    assert_eq!(t.next_probe_due(), None);
}

#[test]
fn a_probe_that_outlived_its_schedule_changes_nothing() {
    let mut t = Tracker::new(0, T0);
    t.report(api(), down("refused"), T0);
    let (service, stale) = t.take_due_probes(T0 + PROBE_FIRST_MS).remove(0);
    // Meanwhile the CLI got through, then the connection dropped again.
    t.report(api(), Outcome::Answered, T0 + 6_000);
    t.report(api(), down("refused"), T0 + 7_000);
    assert_eq!(t.probe_finished(&service, stale, down("refused"), T0 + 8_000), None);
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Degraded, T0 + 7_000));
    assert_eq!(t.next_probe_due(), Some(T0 + 7_000 + PROBE_FIRST_MS), "the new schedule stands");
}

#[test]
fn the_status_serializes_snake_case() {
    let mut t = Tracker::new(41, T0);
    t.report(Service::Muxbus, down("dns error"), T0);
    let v = serde_json::to_value(t.status()).unwrap();
    assert_eq!(v["revision"], serde_json::json!(42));
    assert_eq!(
        v["services"][1],
        serde_json::json!({ "service": "muxbus", "label": "MuxBus", "state": "degraded", "since_ms": T0, "last_error": "dns error" })
    );
    assert_eq!(v["services"][0]["state"], "online");
}

#[test]
fn a_tracked_api_starts_not_checked_with_a_probe_due_at_once() {
    let mut t = Tracker::new(0, T0);
    let seeded = t.seed(api(), T0).expect("listed");
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Unknown, T0));
    assert_eq!(serde_json::to_value(&seeded.services[1]).unwrap()["state"], "unknown");
    // Not checked says nothing about the internet.
    assert_eq!(state_of(&t, "internet").0, Reachability::Online);
    assert_eq!(t.seed(api(), T0 + 1), None, "listed once");
    let (service, ticket) = t.take_due_probes(T0).remove(0);
    let online = t.probe_finished(&service, ticket, Outcome::Answered, T0 + 300).expect("checked");
    assert!(online.revision > seeded.revision);
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Online, T0 + 300));
    assert_eq!(t.next_probe_due(), None, "an answering API isn't probed again");
}

#[test]
fn a_tracked_api_whose_first_probe_fails_is_offline_and_keeps_being_probed() {
    let mut t = Tracker::new(0, T0);
    t.seed(api(), T0);
    let (service, ticket) = t.take_due_probes(T0).remove(0);
    t.probe_finished(&service, ticket, down("connection refused"), T0 + 200).expect("checked");
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Offline, T0 + 200));
    assert_eq!(t.next_probe_due(), Some(T0 + 200 + 2 * PROBE_FIRST_MS));
}

#[test]
fn a_service_that_already_reported_isnt_seeded_over() {
    let mut t = Tracker::new(0, T0);
    t.report(api(), Outcome::Answered, T0);
    assert_eq!(t.seed(api(), T0 + 1), None);
    assert_eq!(state_of(&t, "model_api:anthropic"), (Reachability::Online, T0));
}

#[test]
fn a_network_change_makes_every_probe_due_at_once_and_starts_the_backoff_over() {
    let mut t = Tracker::new(0, 0);
    t.report(api(), down("refused"), 0);
    let (_, ticket) = t.take_due_probes(5_000).remove(0);
    t.probe_finished(&api(), ticket, down("refused"), 5_000);
    assert_eq!(t.next_probe_due(), Some(15_000));
    assert!(t.network_changed(6_000));
    assert_eq!(t.next_probe_due(), Some(6_000));
    let (_, ticket) = t.take_due_probes(6_000).remove(0);
    t.probe_finished(&api(), ticket, down("refused"), 6_000);
    assert_eq!(t.next_probe_due(), Some(16_000), "ten seconds, as after the first probe");
}

#[test]
fn a_network_change_has_nothing_to_probe_before_any_service_is_known() {
    assert!(!Tracker::new(0, 0).network_changed(1_000));
}

#[test]
fn a_probe_of_an_online_service_that_answers_leaves_no_schedule() {
    let mut t = Tracker::new(0, 0);
    t.report(api(), Outcome::Answered, 0);
    assert_eq!(t.next_probe_due(), None);
    t.network_changed(1_000);
    let (_, ticket) = t.take_due_probes(1_000).remove(0);
    assert!(t.probe_finished(&api(), ticket, Outcome::Answered, 1_000).is_none(), "nothing changed");
    assert_eq!(t.next_probe_due(), None);
    assert_eq!(state_of(&t, "model_api:anthropic").0, Reachability::Online);
}

#[test]
fn a_probe_of_an_online_service_that_fails_is_a_doubt_with_a_probe_five_seconds_on() {
    let mut t = Tracker::new(0, 0);
    t.report(api(), Outcome::Answered, 0);
    t.network_changed(1_000);
    let (_, ticket) = t.take_due_probes(1_000).remove(0);
    assert!(t.probe_finished(&api(), ticket, down("no route"), 1_000).is_some());
    assert_eq!(state_of(&t, "model_api:anthropic").0, Reachability::Degraded);
    assert_eq!(t.next_probe_due(), Some(6_000));
}

#[test]
fn a_network_change_while_a_probe_runs_repeats_it_at_once() {
    let mut t = Tracker::new(0, 0);
    t.report(api(), down("refused"), 0);
    let (_, ticket) = t.take_due_probes(5_000).remove(0);
    t.network_changed(6_000);
    assert_eq!(t.next_probe_due(), None, "the running probe is not started a second time");
    t.probe_finished(&api(), ticket, down("refused"), 7_000);
    assert_eq!(t.next_probe_due(), Some(7_000));
}

#[test]
fn a_network_change_while_an_online_services_probe_runs_has_it_repeated_even_when_it_answers() {
    let mut t = Tracker::new(0, 0);
    t.report(api(), Outcome::Answered, 0);
    t.network_changed(1_000);
    let (_, ticket) = t.take_due_probes(1_000).remove(0);
    t.network_changed(2_000);
    t.probe_finished(&api(), ticket, Outcome::Answered, 3_000);
    assert_eq!(t.next_probe_due(), Some(3_000), "the answer was about the old network");
    let (_, ticket) = t.take_due_probes(3_000).remove(0);
    t.probe_finished(&api(), ticket, Outcome::Answered, 3_100);
    assert_eq!(t.next_probe_due(), None, "and the repeat leaves nothing scheduled");
}

#[test]
fn a_network_change_while_a_down_services_probe_runs_has_it_repeated_even_when_it_answers() {
    let mut t = Tracker::new(0, 0);
    t.report(api(), down("refused"), 0);
    let (_, ticket) = t.take_due_probes(5_000).remove(0);
    t.network_changed(6_000);
    let changed = t.probe_finished(&api(), ticket, Outcome::Answered, 7_000);
    assert!(changed.is_some(), "it is back, as far as that probe could tell");
    assert_eq!(t.next_probe_due(), Some(7_000), "but the answer was about the old network");
    let (_, ticket) = t.take_due_probes(7_000).remove(0);
    t.probe_finished(&api(), ticket, Outcome::Answered, 7_100);
    assert_eq!(t.next_probe_due(), None);
}

fn at(n: usize) -> Service {
    Service::ModelApiAt(&ANTHROPIC, format!("https://proxy{n}.example.com"))
}

fn has(t: &Tracker, service: &Service) -> bool {
    t.status().services.iter().any(|s| s.service == service.id())
}

#[test]
fn pane_addresses_are_bounded_and_the_oldest_fine_one_makes_room() {
    let mut t = Tracker::new(0, 0);
    for n in 0..MAX_PANE_ADDRESSES {
        t.seed(at(n), n as i64);
    }
    t.report(at(0), Outcome::Answered, 100);
    assert!(t.seed(at(99), 200).is_some());
    let rows = t.status().services.iter().filter(|s| s.service.contains('@')).count();
    assert_eq!(rows, MAX_PANE_ADDRESSES, "never more than the cap");
    assert!(has(&t, &at(99)));
    assert!(!has(&t, &at(1)), "the one fine for longest, not checked since 1 ms, went");
    assert!(has(&t, &at(0)), "one that answered later is newer than that");
}

#[test]
fn when_every_pane_address_is_in_trouble_a_new_one_is_not_tracked() {
    let mut t = Tracker::new(0, 0);
    for n in 0..MAX_PANE_ADDRESSES {
        for _ in 0..3 {
            t.report(at(n), down("refused"), n as i64);
        }
    }
    assert!(t.seed(at(99), 500).is_none());
    assert!(t.report(at(99), Outcome::Answered, 501).is_none());
    assert!(!has(&t, &at(99)));
    assert!(has(&t, &at(0)));
}

#[test]
fn the_providers_own_service_is_never_counted_against_the_cap() {
    let mut t = Tracker::new(0, 0);
    for n in 0..MAX_PANE_ADDRESSES {
        t.seed(at(n), 0);
    }
    assert!(t.seed(api(), 1).is_some());
    assert!(t.report(Service::Muxbus, Outcome::Answered, 2).is_some() || has(&t, &Service::Muxbus));
}

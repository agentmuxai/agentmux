// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const AFTER: u64 = 55 * 60_000;
const NOW: u64 = 1_000_000_000;
const MIN: u64 = 60_000;

fn status(availability: Availability) -> AgentStatus {
    AgentStatus { availability, pane: Pane::Shown, wanted: Wanted::Running, api: Reach::Online, controller: true }
}

fn every_availability() -> Vec<Availability> {
    vec![
        Availability::NotRunning { exited_with_error: false },
        Availability::NotRunning { exited_with_error: true },
        Availability::Idle { quiet_for_ms: None },
        Availability::Idle { quiet_for_ms: Some(5) },
        Availability::Idle { quiet_for_ms: Some(AFTER) },
        Availability::Idle { quiet_for_ms: Some(AFTER + 1) },
        Availability::Running { waiting_on_user: false },
        Availability::Running { waiting_on_user: true },
        Availability::Limited { resets_at_ms: None, since_ms: 1 },
        Availability::Limited { resets_at_ms: Some(50), since_ms: 1 },
        Availability::SignedOut { since_ms: 1 },
        Availability::Unknown,
    ]
}

fn every_pane_wanted_api() -> Vec<(Pane, Wanted, Reach)> {
    let mut all = Vec::new();
    for pane in [Pane::Shown, Pane::Parked, Pane::None] {
        for wanted in [Wanted::Running, Wanted::Stopped, Wanted::Unset] {
            for api in [Reach::Online, Reach::Degraded, Reach::Offline, Reach::Unchecked { for_ms: 0 }, Reach::Unchecked { for_ms: 60_000 }, Reach::NotTracked] {
                all.push((pane, wanted, api));
            }
        }
    }
    all
}

#[test]
fn a_message_is_decided_exactly_as_the_verdict_decides_it() {
    for av in every_availability() {
        for (pane, wanted, api) in every_pane_wanted_api() {
            for (urgent, routine, cold) in [(false, true, Some(AFTER)), (true, true, Some(AFTER)), (false, false, Some(AFTER)), (false, true, None)] {
                for hands_over in [false, true] {
                    let s = AgentStatus { availability: av, pane, wanted, api, controller: true };
                    let starts_a_turn = matches!(av, Availability::Idle { .. });
                    let want = match verdict(&av, urgent, routine, cold, || hands_over) {
                        // The one difference (spec D2): a turn that would start while the API is offline waits.
                        Verdict::Deliver if starts_a_turn && api == Reach::Offline => Decision::Wait { reason: WaitReason::ApiOffline, until_ms: None },
                        Verdict::Deliver => Decision::Deliver,
                        Verdict::Wait { reason, until_ms } => Decision::Wait { reason, until_ms },
                    };
                    let got = decide(&s, Intent::Message { urgent, routine, cold_after_ms: cold }, || hands_over);
                    assert_eq!(got, want, "{av:?} urgent={urgent} routine={routine} cold={cold:?} hands_over={hands_over}");
                }
            }
        }
    }
}

#[test]
fn a_message_waits_for_the_api_only_when_it_would_start_a_turn() {
    let offline = |av| AgentStatus { api: Reach::Offline, ..status(av) };
    let message = Intent::Message { urgent: true, routine: false, cold_after_ms: None };
    let wait = Decision::Wait { reason: WaitReason::ApiOffline, until_ms: None };
    assert_eq!(decide(&offline(Availability::Idle { quiet_for_ms: Some(5) }), message, || false), wait, "even an urgent one: the turn would fail");
    assert_eq!(decide(&offline(Availability::Running { waiting_on_user: false }), message, || false), Decision::Deliver, "it joins the turn");
    assert_eq!(decide(&offline(Availability::Unknown), message, || false), Decision::Deliver, "nothing tracks its turns");
    for api in [Reach::Online, Reach::Degraded, Reach::NotTracked, Reach::Unchecked { for_ms: 1 }] {
        let s = AgentStatus { api, ..status(Availability::Idle { quiet_for_ms: Some(5) }) };
        assert_eq!(decide(&s, message, || false), Decision::Deliver, "{api:?}");
    }
    // A limit or a sign-out is the reason given, as before.
    let limited = Availability::Limited { resets_at_ms: Some(50), since_ms: 1 };
    assert_eq!(
        decide(&offline(limited), message, || false),
        Decision::Wait { reason: WaitReason::UsageLimit, until_ms: Some(50) },
        "the limit stays the reason"
    );
}

#[test]
fn only_a_routine_message_to_a_cold_session_asks_whether_it_hands_over_work() {
    let never = || panic!("asked whether it hands over work, but it could not matter");
    for av in every_availability() {
        let cold = matches!(av, Availability::Idle { quiet_for_ms: Some(q) } if q > AFTER);
        if !cold {
            decide(&status(av), Intent::Message { urgent: false, routine: true, cold_after_ms: Some(AFTER) }, never);
        }
        decide(&status(av), Intent::Note, never);
        decide(&status(av), Intent::HeldInput, never);
        decide(&status(av), Intent::Start { allowed: true }, never);
        decide(&status(av), Intent::Lease { aimed_at_agent: false, now_ms: NOW }, never);
    }
}

#[test]
fn a_note_attaches_to_anything_with_a_process_and_to_a_stopped_agent_only_when_it_is_wanted_running() {
    for av in every_availability() {
        let with_process = !matches!(av, Availability::NotRunning { .. });
        for (_, wanted, api) in every_pane_wanted_api() {
            let s = AgentStatus { availability: av, pane: Pane::Shown, wanted, api, controller: true };
            let want = if with_process || wanted == Wanted::Running { Decision::Attach } else { Decision::Drop };
            assert_eq!(decide(&s, Intent::Note, || false), want, "{av:?} wanted={wanted:?}");
        }
    }
}

#[test]
fn held_input_waits_for_a_controller_a_known_limit_a_sign_in_and_the_api() {
    let wait_not_running = Decision::Wait { reason: WaitReason::NotRunning, until_ms: None };
    for av in every_availability() {
        for (pane, wanted, api) in every_pane_wanted_api() {
            let s = AgentStatus { availability: av, pane, wanted, api, controller: true };
            // An exited process under a controller is restarted by the text, so it is not a wait.
            let want = if let Availability::Limited { resets_at_ms: Some(reset), .. } = av {
                Decision::Wait { reason: WaitReason::UsageLimit, until_ms: Some(reset) }
            } else if matches!(av, Availability::SignedOut { .. }) {
                Decision::Wait { reason: WaitReason::NeedsLogin, until_ms: None }
            } else if !api.lets_input_go() {
                Decision::WaitApi
            } else {
                Decision::Deliver
            };
            assert_eq!(decide(&s, Intent::HeldInput, || false), want, "{av:?} api={api:?}");
        }
    }
    // Nothing holds the agent at all: nothing to send the text to.
    let no_controller = AgentStatus { controller: false, ..status(Availability::NotRunning { exited_with_error: false }) };
    assert_eq!(decide(&no_controller, Intent::HeldInput, || false), wait_not_running);
    // A parked agent out of quota until a known reset, or signed out, keeps the text (spec D1); one whose
    // reset is unknown is sent it and finds out, as a message is.
    let parked = |av| AgentStatus { pane: Pane::Parked, ..status(av) };
    assert_eq!(
        decide(&parked(Availability::Limited { resets_at_ms: Some(50), since_ms: 1 }), Intent::HeldInput, || false),
        Decision::Wait { reason: WaitReason::UsageLimit, until_ms: Some(50) }
    );
    assert_eq!(
        decide(&parked(Availability::SignedOut { since_ms: 1 }), Intent::HeldInput, || false),
        Decision::Wait { reason: WaitReason::NeedsLogin, until_ms: None }
    );
    assert_eq!(decide(&parked(Availability::Limited { resets_at_ms: None, since_ms: 1 }), Intent::HeldInput, || false), Decision::Deliver);
}

#[test]
fn the_api_lets_input_go_as_the_pane_rule_says() {
    assert!(Reach::Online.lets_input_go());
    assert!(Reach::Degraded.lets_input_go());
    assert!(Reach::NotTracked.lets_input_go());
    assert!(!Reach::Offline.lets_input_go());
    assert!(!Reach::Unchecked { for_ms: 0 }.lets_input_go());
    assert!(!Reach::Unchecked { for_ms: UNCHECKED_HOLD_MS }.lets_input_go(), "20 s exactly still holds");
    assert!(Reach::Unchecked { for_ms: UNCHECKED_HOLD_MS + 1 }.lets_input_go());
}

#[test]
fn a_stopped_agent_starts_only_when_a_start_is_allowed() {
    for av in every_availability() {
        let stopped = matches!(av, Availability::NotRunning { .. });
        let allowed = decide(&status(av), Intent::Start { allowed: true }, || false);
        let refused = decide(&status(av), Intent::Start { allowed: false }, || false);
        if stopped {
            assert_eq!(allowed, Decision::Start);
            assert_eq!(refused, Decision::Wait { reason: WaitReason::NotRunning, until_ms: None });
        } else {
            assert_eq!((allowed, refused), (Decision::Deliver, Decision::Deliver), "{av:?}");
        }
    }
}

/// The table of `work_tasks_tests.rs` `renews_lease`: a holder out of quota or signed out keeps a task
/// aimed at it, and keeps any task for 30 minutes and while its reset is within the hour.
#[test]
fn a_lease_lapses_as_renews_lease_decides() {
    let limited = |minutes_ago: u64, reset_in_minutes: Option<u64>| {
        status(Availability::Limited { resets_at_ms: reset_in_minutes.map(|m| NOW + m * MIN), since_ms: NOW - minutes_ago * MIN })
    };
    let lease = |s: &AgentStatus, aimed_at_agent| decide(s, Intent::Lease { aimed_at_agent, now_ms: NOW }, || false);
    for av in every_availability() {
        if !matches!(av, Availability::Limited { .. } | Availability::SignedOut { .. }) {
            assert_eq!(lease(&status(av), false), Decision::Renew, "{av:?}");
        }
    }
    assert_eq!(lease(&limited(29, Some(300)), false), Decision::Renew, "not yet 30 minutes");
    assert_eq!(lease(&limited(30, Some(300)), false), Decision::Lapse, "30 minutes, reset in 5 h");
    assert_eq!(lease(&limited(90, Some(59)), false), Decision::Renew, "the reset is within the hour: wait it out");
    assert_eq!(lease(&limited(90, Some(61)), false), Decision::Lapse);
    assert_eq!(lease(&limited(90, None), false), Decision::Lapse, "no reset known counts as far");
    assert_eq!(lease(&limited(90, None), true), Decision::Renew, "a task aimed at an agent stays with it");
    assert_eq!(lease(&status(Availability::SignedOut { since_ms: NOW - 31 * MIN }), false), Decision::Lapse);
    assert_eq!(lease(&status(Availability::SignedOut { since_ms: NOW - 29 * MIN }), false), Decision::Renew);
    assert_eq!(lease(&status(Availability::SignedOut { since_ms: NOW - 31 * MIN }), true), Decision::Renew);
}

#[test]
fn deciding_changes_nothing_and_the_same_status_decides_the_same() {
    let s = status(Availability::Limited { resets_at_ms: Some(50), since_ms: 1 });
    let again = s;
    let intent = Intent::Message { urgent: false, routine: true, cold_after_ms: Some(AFTER) };
    assert_eq!(decide(&s, intent, || false), decide(&again, intent, || false));
    assert_eq!(s, again);
}

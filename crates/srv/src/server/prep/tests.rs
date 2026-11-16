// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The window against the real ledger, with a window of one second instead of thirty: who ends it, in
//! what order, and that the action runs exactly once whichever way it ends.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ledger::{self, Answer};
use super::*;
use crate::server::tests::test_state;

/// The notes the tests' agents were sent, by route, instead of a real delivery.
pub(crate) mod sink {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};

    static NOTES: LazyLock<Mutex<HashMap<String, Vec<String>>>> = LazyLock::new(Default::default);

    /// A test names the route whose notes it wants to catch; every other route is delivered for real.
    pub fn expect(route: &str) {
        NOTES.lock().unwrap().insert(route.to_lowercase(), Vec::new());
    }

    pub fn deliver(route: &str, text: &str) -> Option<Result<(), String>> {
        NOTES.lock().unwrap().get_mut(&route.to_lowercase()).map(|notes| {
            notes.push(text.to_string());
            Ok(())
        })
    }

    pub fn notes(route: &str) -> Vec<String> {
        NOTES.lock().unwrap().get(&route.to_lowercase()).cloned().unwrap_or_default()
    }
}

fn store_installed() {
    crate::backend::agent_session::set_global_transcript_store(Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap()));
}

fn prep(def_id: &str, name: &str, target: &str) -> Prep {
    Prep {
        def_id: def_id.to_string(),
        agent_name: name.to_string(),
        blocks: Vec::new(),
        action: "account_switch",
        target: target.to_string(),
        what: "move you to the account b@y".to_string(),
        actor: "operator".to_string(),
        cuts_turn: false,
    }
}

type Ran = Arc<Mutex<Vec<How>>>;

fn recorder() -> (Ran, Proceed) {
    let ran: Ran = Default::default();
    let r = ran.clone();
    (ran, Box::new(move |how| r.lock().unwrap().push(how)))
}

/// Wait up to 6 s for `done`.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..600 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn wait(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test]
async fn the_agent_is_asked_once_and_the_closure_runs_when_it_answers() {
    store_installed();
    let state = test_state();
    sink::expect("prep-a");
    let (ran, proceed) = recorder();
    let started = Instant::now();
    open(&state, prep("prep-def-a", "Prep-A", "acct-2"), proceed);
    until("the note", || sink::notes("prep-a").len() == 1).await;
    let note = sink::notes("prep-a").remove(0);
    assert!(note.contains("Prepare for a change") && note.contains("move you to the account b@y"), "{note}");
    assert!(is_open("prep-def-a") && ran.lock().unwrap().is_empty(), "nothing runs while the agent has not answered");

    let said = ledger::answer(&state, "prep-def-a", "prep-a", Answer { handoff: Some("In flight: PR 7".into()), ..Answer::default() }).unwrap();
    assert_eq!(said["status"], "ready");
    until("the change", || !ran.lock().unwrap().is_empty()).await;
    assert_eq!(*ran.lock().unwrap(), vec![How::Ready], "its turn is over (no pane), so the change runs at once");
    assert!(started.elapsed() < WINDOW, "well before the deadline: {:?}", started.elapsed());
    assert!(!is_open("prep-def-a"));
    let held = crate::backend::continuity_handoff::take("prep-def-a").expect("the handoff waits for the new session");
    assert_eq!(held.text, "In flight: PR 7");
}

#[tokio::test]
async fn a_window_nobody_answers_ends_at_the_deadline_and_runs_the_closure_once() {
    let state = test_state();
    sink::expect("prep-b");
    let (ran, proceed) = recorder();
    let started = Instant::now();
    open(&state, prep("prep-def-b", "Prep-B", "acct-2"), proceed);
    until("the deadline", || !ran.lock().unwrap().is_empty()).await;
    assert!(started.elapsed() >= WINDOW - Duration::from_millis(50), "not before the deadline: {:?}", started.elapsed());
    assert_eq!(*ran.lock().unwrap(), vec![How::TimedOut]);
    wait(300).await;
    assert_eq!(ran.lock().unwrap().len(), 1, "once");
}

#[tokio::test]
async fn the_person_ends_the_window_at_once_and_a_late_answer_changes_nothing() {
    store_installed();
    let state = test_state();
    sink::expect("prep-c");
    let (ran, proceed) = recorder();
    open(&state, prep("prep-def-c", "Prep-C", "acct-2"), proceed);
    until("the note", || !sink::notes("prep-c").is_empty()).await;
    ledger::person_says_now("prep-def-c").unwrap();
    until("the change", || !ran.lock().unwrap().is_empty()).await;
    assert_eq!(*ran.lock().unwrap(), vec![How::PersonSaidNow]);
    let late = ledger::answer(&state, "prep-def-c", "prep-c", Answer { handoff: Some("late".into()), ..Answer::default() });
    assert!(late.unwrap_err().contains("no change is waiting on you"), "an answer after the change ran is refused");
    assert!(ledger::person_says_now("prep-def-c").is_err());
    assert_eq!(ran.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn more_time_moves_the_deadline_once_and_a_handoff_survives_the_timeout() {
    store_installed();
    let state = test_state();
    sink::expect("prep-d");
    let (ran, proceed) = recorder();
    let started = Instant::now();
    open(&state, prep("prep-def-d", "Prep-D", "acct-2"), proceed);
    until("the note", || !sink::notes("prep-d").is_empty()).await;
    let more = Answer { handoff: Some("Mid push: finish, then merge".into()), more_time: true, why: Some("finishing a push".into()) };
    assert_eq!(ledger::answer(&state, "prep-def-d", "prep-d", more).unwrap()["status"], "more_time");
    let again = Answer { more_time: true, ..Answer::default() };
    assert!(ledger::answer(&state, "prep-def-d", "prep-d", again).unwrap_err().contains("once"), "one extension only");
    wait(1_300).await;
    assert!(ran.lock().unwrap().is_empty(), "past the first deadline, the longer window is still open");
    until("the longer deadline", || !ran.lock().unwrap().is_empty()).await;
    assert!(started.elapsed() >= EXTENDED_WINDOW - Duration::from_millis(50), "{:?}", started.elapsed());
    assert_eq!(*ran.lock().unwrap(), vec![How::TimedOut]);
    assert_eq!(crate::backend::continuity_handoff::take("prep-def-d").unwrap().text, "Mid push: finish, then merge", "what it left is kept");
}

#[tokio::test]
async fn an_answer_with_nothing_in_it_is_refused() {
    let state = test_state();
    sink::expect("prep-e");
    let (_, proceed) = recorder();
    open(&state, prep("prep-def-e", "Prep-E", "acct-2"), proceed);
    let nothing = ledger::answer(&state, "prep-def-e", "prep-e", Answer::default());
    assert!(nothing.unwrap_err().contains("handoff"));
    cancel(&state, "prep-def-e");
}

#[tokio::test]
async fn another_target_replaces_the_closure_and_keeps_the_window_and_the_same_target_changes_nothing() {
    let state = test_state();
    sink::expect("prep-f");
    let (first, proceed_first) = recorder();
    let (second, proceed_second) = recorder();
    let (third, proceed_third) = recorder();
    let started = Instant::now();
    open(&state, prep("prep-def-f", "Prep-F", "acct-2"), proceed_first);
    wait(500).await;
    open(&state, prep("prep-def-f", "Prep-F", "acct-2"), proceed_third);
    open(&state, prep("prep-def-f", "Prep-F", "acct-3"), proceed_second);
    until("the deadline", || !second.lock().unwrap().is_empty()).await;
    assert!(started.elapsed() < WINDOW + Duration::from_millis(400), "the window kept its deadline: {:?}", started.elapsed());
    assert!(first.lock().unwrap().is_empty() && third.lock().unwrap().is_empty(), "neither the old target's closure nor the repeat runs");
    assert_eq!(*second.lock().unwrap(), vec![How::TimedOut]);
    assert_eq!(sink::notes("prep-f").len(), 1, "the agent was asked once, not once per request");
}

#[tokio::test]
async fn a_cancel_drops_the_closure_and_what_the_agent_left_and_tells_it_to_carry_on() {
    store_installed();
    let state = test_state();
    sink::expect("prep-g");
    let (ran, proceed) = recorder();
    open(&state, prep("prep-def-g", "Prep-G", "acct-2"), proceed);
    until("the note", || !sink::notes("prep-g").is_empty()).await;
    ledger::answer(&state, "prep-def-g", "prep-g", Answer { handoff: Some("x".into()), more_time: true, ..Answer::default() }).unwrap();
    cancel(&state, "prep-def-g");
    wait(EXTENDED_WINDOW.as_millis() as u64 + 300).await;
    assert!(ran.lock().unwrap().is_empty(), "the change did not happen");
    assert!(!is_open("prep-def-g"));
    assert!(crate::backend::continuity_handoff::take("prep-def-g").is_none(), "its handoff is taken back");
    let notes = sink::notes("prep-g");
    assert!(notes.last().unwrap().contains("was cancelled. Carry on."), "{notes:?}");
}

#[tokio::test]
async fn an_agent_that_cannot_be_asked_does_not_hold_the_change_back() {
    let state = test_state();
    // No sink for this route: the real delivery finds no such agent.
    let (ran, proceed) = recorder();
    open(&state, prep("prep-def-h", "Prep-Nobody-H", "acct-2"), proceed);
    until("the change", || !ran.lock().unwrap().is_empty()).await;
    assert_eq!(*ran.lock().unwrap(), vec![How::NotAsked]);
}

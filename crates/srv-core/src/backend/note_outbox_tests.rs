// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn held(agent: &str, text: &str) -> Line {
    Line { agent: agent.into(), text: text.into(), held_key: Some(format!("quiet:{agent}")) }
}

fn urgent(agent: &str, text: &str) -> Line {
    Line { agent: agent.into(), text: text.into(), held_key: None }
}

#[test]
fn a_lone_urgent_note_goes_at_once_and_opens_no_window() {
    let mut o = Outbox::default();
    let out = o.push("leader", urgent("a", "Your worker A stopped: its process exited."), 1_000).expect("sent now");
    assert_eq!(out.text, "[AgentMux] Your worker A stopped: its process exited.");
    assert_eq!(out.wait_key, None);
    assert!(o.is_empty());
}

#[test]
fn a_note_that_can_wait_opens_a_window_and_a_window_of_one_is_the_note_unchanged() {
    let mut o = Outbox::default();
    assert_eq!(o.push("leader", held("a", "Your agent A has been quiet for 22 min on #5."), 1_000), None);
    assert!(o.take_due(1_000 + WINDOW_MS - 1).is_empty(), "the window is not over");
    let due = o.take_due(1_000 + WINDOW_MS);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].text, "[AgentMux] Your agent A has been quiet for 22 min on #5.");
    assert_eq!(due[0].wait_key.as_deref(), Some("quiet:a"), "its own key, as before");
    assert!(o.is_empty());
}

#[test]
fn notes_in_one_window_leave_as_one_note_with_a_line_each() {
    let mut o = Outbox::default();
    assert_eq!(o.push("leader", held("a", "A is quiet."), 0), None);
    assert_eq!(o.push("leader", urgent("b", "B stopped."), 5_000), None, "joins the open window, even when urgent");
    assert_eq!(o.push("leader", held("c", "C is quiet."), 6_000), None);
    let due = o.take_due(WINDOW_MS);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].text, "[AgentMux] 3 of your agents need a look:\n- A is quiet.\n- B stopped.\n- C is quiet.");
    assert_eq!(due[0].wait_key, None, "one line must be heard now, so the note is not held");
}

#[test]
fn a_later_note_about_the_same_agent_replaces_its_line() {
    let mut o = Outbox::default();
    o.push("leader", held("a", "A is quiet."), 0);
    o.push("leader", held("b", "B is quiet."), 1_000);
    o.push("leader", urgent("a", "A stopped."), 2_000);
    let due = o.take_due(WINDOW_MS);
    assert_eq!(due[0].text, "[AgentMux] 2 of your agents need a look:\n- A stopped.\n- B is quiet.");
}

#[test]
fn a_group_of_notes_that_can_all_wait_waits_under_a_key_no_later_group_replaces() {
    let mut o = Outbox::default();
    o.push("leader", held("a", "A is quiet."), 0);
    o.push("leader", held("b", "B is quiet."), 1_000);
    let first = o.take_due(WINDOW_MS).remove(0);
    o.push("leader", held("c", "C is quiet."), 100_000);
    o.push("leader", held("d", "D is quiet."), 101_000);
    let second = o.take_due(100_000 + WINDOW_MS).remove(0);
    assert!(first.wait_key.is_some() && second.wait_key.is_some());
    assert_ne!(first.wait_key, second.wait_key);
}

#[test]
fn each_leader_has_its_own_window() {
    let mut o = Outbox::default();
    o.push("one", held("a", "A is quiet."), 0);
    o.push("two", held("a", "A is quiet."), 10_000);
    assert_eq!(o.take_due(WINDOW_MS).len(), 1, "only the first leader's window is over");
    assert_eq!(o.take_due(10_000 + WINDOW_MS).len(), 1);
    assert!(o.is_empty());
}

#[test]
fn an_urgent_note_after_a_window_closed_goes_at_once_again() {
    let mut o = Outbox::default();
    o.push("leader", held("a", "A is quiet."), 0);
    o.take_due(WINDOW_MS);
    assert!(o.push("leader", urgent("b", "B failed."), WINDOW_MS + 1).is_some());
}

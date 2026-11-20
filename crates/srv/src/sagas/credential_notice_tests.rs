// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const HOUR: i64 = REPEAT_MS;

#[test]
fn a_refusal_puts_one_line_up_and_the_same_cause_again_does_not() {
    let mut map = HashMap::new();
    let first = plan(&mut map, "pulp", Cause::Github, 1_000).expect("the first refusal shows");
    assert_eq!(first.take_down, None);
    assert_eq!(plan(&mut map, "pulp", Cause::Github, 1_000 + 5_000), None, "a retry loop does not stack lines");
    assert_eq!(plan(&mut map, "pulp", Cause::Github, 1_000 + HOUR - 1), None);
}

#[test]
fn another_cause_or_another_agent_has_its_own_line() {
    let mut map = HashMap::new();
    plan(&mut map, "pulp", Cause::Github, 1_000).unwrap();
    assert!(plan(&mut map, "pulp", Cause::Aws, 1_000).is_some(), "AWS is its own cause");
    assert!(plan(&mut map, "korp", Cause::Github, 1_000).is_some(), "each agent is told");
}

#[test]
fn after_an_hour_the_line_is_put_up_again_and_the_old_one_taken_down() {
    let mut map = HashMap::new();
    let first = plan(&mut map, "pulp", Cause::Aws, 1_000).unwrap();
    let again = plan(&mut map, "pulp", Cause::Aws, 1_000 + HOUR).expect("an hour on, it is shown again");
    assert_eq!(again.take_down.as_deref(), Some(first.put_up.as_str()));
    assert_ne!(again.put_up, first.put_up, "a new id, so a pane that dismissed the old one shows it");
}

#[test]
fn the_sentence_is_one_line_and_clipped() {
    assert_eq!(sentence("GitHub isn't connected:\n  connect it in Connectors"), "GitHub isn't connected: connect it in Connectors");
    let long = "word ".repeat(200);
    let said = sentence(&long);
    assert_eq!(said.chars().count(), MAX_SENTENCE);
    assert!(said.ends_with('…'));
}

#[test]
fn the_line_is_srvs_own_record_with_no_buttons_and_no_countdown() {
    let v = view("id-1", Cause::Aws, "block-1", "pulp", "The host's AWS login has lapsed.", "line", 5);
    assert_eq!((v.kind, v.status, v.deadline_ms, v.always_label.as_str(), v.quote.as_str(), v.body.as_str()), ("aws", "line", 0, "", "", ""));
    assert_eq!(view("id-2", Cause::Github, "b", "pulp", "", "cleared", 5).kind, "github");
}

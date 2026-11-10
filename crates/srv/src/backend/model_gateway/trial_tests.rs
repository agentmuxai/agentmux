// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::model_gateway::launch::Choice;
use crate::backend::model_gateway::switch::{move_to, Target};
use serde_json::json;

const FLASH: &str = "deepseek/deepseek-v4.1-flash-1017";
const PRO: &str = "deepseek/deepseek-v4-pro-0813";

fn store() -> Store {
    let store = Store::open_in_memory().unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def("a1", "a1", "claude", "host", 1, "");
    store.agent_def_insert(&mut def).unwrap();
    store
}

fn to(s: &Store, model: &str) {
    move_to(s, "a1", "claude", Target::Route { route: "openrouter", model }, |_| true).unwrap();
}

fn on(model: &str) -> Choice {
    Choice { route: "openrouter".into(), model: model.into() }
}

fn ok_frame() -> Value {
    json!({ "type": "result", "is_error": false, "result": "done" })
}

fn error_frame(text: &str) -> Value {
    json!({ "type": "result", "is_error": true, "result": text })
}

#[test]
fn a_refusal_of_the_request_undoes_the_change_and_other_failures_wait() {
    assert_eq!(verdict(&ok_frame()), Verdict::Commit);
    assert_eq!(verdict(&error_frame("API Error: 400 Mid-conversation reasoning effort (configuration_update) is not supported")), Verdict::Refused(400));
    assert_eq!(verdict(&error_frame("API Error: 404 model not found")), Verdict::Refused(404));
    assert_eq!(verdict(&error_frame("API Error: 413 request too large")), Verdict::Refused(413));
    for waits in ["API Error: 429 rate limited", "API Error: 529 overloaded", "API Error: 401 bad key", "API Error: 402 out of credit", "API Error: 500 oops", "something else broke"] {
        assert_eq!(verdict(&error_frame(waits)), Verdict::Wait, "{waits}");
    }
}

#[test]
fn a_change_is_undone_when_its_first_turn_is_refused_and_it_is_not_before_the_harness_is_relaunched() {
    let s = store();
    to(&s, FLASH);
    // The first change from the harness's own sign-in: a turn still running on the old place does not settle it.
    let refused = error_frame("API Error: 400 history refused");
    assert!(settle(&s, "a1", "blk", &refused).is_none(), "not armed: the turn was the old place's");
    assert_eq!(launch::choice_of(&s, "a1"), Some(on(FLASH)));
    arm(&s, "a1");
    let undone = settle(&s, "a1", "blk", &refused).unwrap();
    assert_eq!((undone.back_on.route.as_str(), undone.tried.model.as_str()), ("", FLASH));
    assert_eq!(launch::choice_of(&s, "a1"), None, "back on the harness's own sign-in");
    assert_eq!(switch::previous_of(&s, "a1"), Some(Previous { route: "openrouter".into(), model: FLASH.into() }), "the panel can still offer it");
    assert!(launch::leaving(&s, "a1").is_some(), "the route it was refused on is left like any other");
    assert!(pending(&s, "a1").is_none());
}

#[test]
fn a_change_whose_first_turn_worked_stands_and_one_that_met_a_limit_waits_for_another_turn() {
    let s = store();
    to(&s, FLASH);
    to(&s, PRO);
    assert_eq!(pending(&s, "a1").unwrap().from, Previous { route: String::new(), model: String::new() }, "a second change before a relaunch keeps the first place");
    arm(&s, "a1");
    assert!(settle(&s, "a1", "blk", &error_frame("API Error: 429 rate limited")).is_none());
    assert!(pending(&s, "a1").is_some(), "a limit says nothing about the history");
    assert!(settle(&s, "a1", "blk", &ok_frame()).is_none());
    assert!(pending(&s, "a1").is_none(), "committed");
    assert_eq!(launch::choice_of(&s, "a1"), Some(on(PRO)));
}

#[test]
fn a_refused_change_goes_back_to_the_model_the_agent_worked_on() {
    let s = store();
    to(&s, FLASH);
    arm(&s, "a1");
    settle(&s, "a1", "blk", &ok_frame());
    to(&s, PRO);
    arm(&s, "a1");
    let undone = settle(&s, "a1", "blk", &error_frame("API Error: 400 history refused")).unwrap();
    assert_eq!((undone.back_on.model.as_str(), undone.tried.model.as_str()), (FLASH, PRO));
    assert_eq!(launch::choice_of(&s, "a1"), Some(on(FLASH)));
    assert!(launch::leaving(&s, "a1").is_none(), "same route: nothing to end");
}

#[test]
fn a_change_back_to_where_the_agent_was_ends_the_trial() {
    let s = store();
    to(&s, FLASH);
    arm(&s, "a1");
    settle(&s, "a1", "blk", &ok_frame());
    to(&s, PRO);
    to(&s, FLASH);
    assert!(pending(&s, "a1").is_none());
}

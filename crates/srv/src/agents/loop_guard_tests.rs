// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

fn call(id: &str, name: &str, input: Value) -> Value {
    json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }] } })
}

fn answer(id: &str, is_error: bool) -> Value {
    json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "is_error": is_error, "content": "x" }] } })
}

/// One call and its answer; the guard's verdict on the answer.
fn round(g: &mut LoopGuard, n: usize, name: &str, input: Value, is_error: bool) -> Option<Stuck> {
    let id = format!("t{n}");
    assert_eq!(g.observe(&call(&id, name, input)), None);
    g.observe(&answer(&id, is_error))
}

#[test]
fn the_same_call_failing_five_times_in_a_turn_is_a_loop() {
    let mut g = LoopGuard::default();
    for n in 1..FAILING_CALLS as usize {
        assert_eq!(round(&mut g, n, "Tasks", json!({ "action": "claim" }), true), None, "failure {n}");
    }
    let stuck = round(&mut g, 5, "Tasks", json!({ "action": "claim" }), true);
    assert_eq!(stuck, Some(Stuck::FailingCall { tool: "Tasks".into(), count: 5 }));
    assert_eq!(stuck.unwrap().describe(), "repeated failing call: Tasks x5 (the same arguments)");
}

#[test]
fn calls_with_other_arguments_or_a_success_in_between_are_not_the_same_loop() {
    let mut g = LoopGuard::default();
    for n in 1..=4 {
        assert_eq!(round(&mut g, n, "Tasks", json!({ "action": "update", "attempt": n }), true), None, "every call differs");
    }
    for n in 5..=8 {
        assert_eq!(round(&mut g, n, "Bash", json!({ "command": "make" }), true), None);
    }
    assert_eq!(round(&mut g, 9, "Bash", json!({ "command": "make" }), false), None, "it worked: the run is over");
    assert_eq!(round(&mut g, 10, "Bash", json!({ "command": "make" }), true), None, "and starts again from one");
}

#[test]
fn another_call_between_failures_does_not_hide_the_loop() {
    let mut g = LoopGuard::default();
    let mut stuck = None;
    for n in 0..5 {
        stuck = stuck.or(round(&mut g, 2 * n, "Tasks", json!({ "action": "claim" }), true));
        assert_eq!(round(&mut g, 2 * n + 1, "Read", json!({ "file_path": "a" }), false), None);
    }
    assert!(matches!(stuck, Some(Stuck::FailingCall { count: 5, .. })));
}

#[test]
fn a_new_turn_starts_counting_again() {
    let mut g = LoopGuard::default();
    for n in 1..=4 {
        round(&mut g, n, "Tasks", json!({ "action": "claim" }), true);
    }
    assert_eq!(g.observe(&json!({ "type": "result", "is_error": false })), None);
    assert_eq!(round(&mut g, 5, "Tasks", json!({ "action": "claim" }), true), None, "the fifth failure, but the first of this turn");
}

#[test]
fn an_answer_to_a_call_it_never_saw_counts_for_nothing() {
    let mut g = LoopGuard::default();
    for _ in 0..10 {
        assert_eq!(g.observe(&answer("unknown", true)), None);
    }
}

fn thinking(text: &str) -> Value {
    json!({ "type": "assistant", "message": { "content": [{ "type": "thinking", "thinking": text }] } })
}

#[test]
fn a_thought_that_repeats_a_line_is_a_loop() {
    let mut g = LoopGuard::default();
    let looped = "Let me check.\n".to_string() + &"Hmm. Wait.\n".repeat(25);
    assert_eq!(g.observe(&thinking(&looped)), Some(Stuck::SameThought));
    assert_eq!(Stuck::SameThought.describe(), "stuck: repeating the same thought");
}

#[test]
fn a_run_of_hmm_on_one_line_is_a_loop() {
    let mut g = LoopGuard::default();
    assert_eq!(g.observe(&thinking(&"Hmm. ".repeat(200))), Some(Stuck::SameThought));
    assert_eq!(g.observe(&thinking(&"Hmm. Hmm. wait. ".repeat(40))), Some(Stuck::SameThought));
}

#[test]
fn ordinary_thinking_and_a_short_repeat_are_not() {
    let mut g = LoopGuard::default();
    let plain = "First read the config, then find where the default is set. The setting is read at launch, so a running agent keeps the old value. \
                 I will add the new key next to the others, write the test, and run it. If the test fails, look at the order of the checks. "
        .repeat(3);
    assert_eq!(g.observe(&thinking(&plain)), None);
    assert_eq!(g.observe(&thinking(&"ok\n".repeat(30))), None, "lines under three characters are not counted");
    assert_eq!(g.observe(&thinking(&"Hmm. ".repeat(10))), None, "ten is a habit, not a loop");
    let list = (1..=40).map(|n| format!("item {n}\n")).collect::<String>();
    assert_eq!(g.observe(&thinking(&list)), None, "a list whose lines differ");
}

fn delta(index: u64, kind: &str, text: &str) -> Value {
    let field = if kind == "thinking_delta" { "thinking" } else { "text" };
    json!({ "type": "stream_event", "event": { "type": "content_block_delta", "index": index, "delta": { "type": kind, field: text } } })
}

#[test]
fn a_thought_that_loops_while_it_streams_is_cut_before_it_finishes() {
    let mut g = LoopGuard::default();
    let mut stuck = None;
    let mut pieces = 0;
    for _ in 0..400 {
        pieces += 1;
        stuck = g.observe(&delta(0, "thinking_delta", "Hmm. Wait. "));
        if stuck.is_some() {
            break;
        }
    }
    assert_eq!(stuck, Some(Stuck::SameThought));
    assert!(pieces < 400, "found while it streamed, after {pieces} pieces");
}

#[test]
fn a_streamed_text_block_is_read_too_and_a_finished_block_starts_over() {
    let mut g = LoopGuard::default();
    let mut stuck = None;
    for _ in 0..400 {
        stuck = stuck.or_else(|| g.observe(&delta(1, "text_delta", "Let me retry the call.\n")));
    }
    assert_eq!(stuck, Some(Stuck::SameThought));

    let mut g = LoopGuard::default();
    for _ in 0..8 {
        assert_eq!(g.observe(&delta(0, "thinking_delta", "Hmm. ")), None);
    }
    let stop = json!({ "type": "stream_event", "event": { "type": "content_block_stop", "index": 0 } });
    assert_eq!(g.observe(&stop), None);
    for _ in 0..8 {
        assert_eq!(g.observe(&delta(0, "thinking_delta", "Hmm. ")), None, "the stopped block's text is not carried over");
    }
}

#[test]
fn a_streamed_thought_that_does_not_repeat_is_left_alone() {
    let mut g = LoopGuard::default();
    for n in 0..300 {
        assert_eq!(g.observe(&delta(0, "thinking_delta", &format!("Step {n}: read file number {n} and compare it with the next one. "))), None);
    }
}

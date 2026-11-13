// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests for module B, run state and the checkpoint encoding
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.2, §3.3, §7
//! "State", "Checkpoint and resume", "Taint").
//!
//! They fail until `state.rs` and `checkpoint.rs` are written, and are marked
//! `ignore` so main stays green; the module's PR deletes each `#[ignore]` line
//! and changes nothing else in this file. Run them with
//! `cargo test -p agentmux-srv --bin agentmux-srv drone::state_acceptance -- --ignored`.

use std::collections::BTreeMap;

use serde_json::json;

use super::checkpoint::{apply, decode, diff, encode, sample, Checkpoint, CheckpointError, MAX_STEPS, SCHEMA_VERSION};
use super::state::{Reducer, RunState, StateError, STATE_CAP_BYTES};
use super::value::Tv;

fn state_with(name: &str, reducer: Reducer) -> RunState {
    let mut s = RunState::new();
    s.declare(name, reducer).unwrap();
    s
}

// ── reducers ────────────────────────────────────────────────────────────

#[test]
fn replace_is_the_default_and_the_last_write_wins() {
    let mut s = RunState::new();
    assert_eq!(s.reducer_of("x"), Reducer::Replace);
    s.write("x", Tv::clean(json!(1))).unwrap();
    s.write("x", Tv::clean(json!({ "a": [1] }))).unwrap();
    assert_eq!(s.get("x"), Some(&Tv::clean(json!({ "a": [1] }))));
    assert_eq!(s.get("missing"), None);
}

#[test]
fn append_makes_a_list_and_pushes_each_write_as_one_element() {
    let mut s = state_with("log", Reducer::Append);
    s.write("log", Tv::clean(json!("a"))).unwrap();
    s.write("log", Tv::clean(json!(["b", "c"]))).unwrap();
    assert_eq!(s.get("log").unwrap().v, json!(["a", ["b", "c"]]));
    // Once a name has a value its reducer cannot change; declaring the same one again is fine.
    let mut once = RunState::new();
    once.write("n", Tv::clean(json!(1))).unwrap();
    assert!(matches!(once.declare("n", Reducer::Append), Err(StateError::Type { .. })));
    assert!(once.declare("n", Reducer::Replace).is_ok());
}

#[test]
fn merge_is_a_shallow_object_merge_where_the_write_wins() {
    let mut s = state_with("o", Reducer::Merge);
    s.write("o", Tv::clean(json!({ "a": 1, "b": { "x": 1 } }))).unwrap();
    s.write("o", Tv::clean(json!({ "b": { "y": 2 }, "c": 3 }))).unwrap();
    assert_eq!(s.get("o").unwrap().v, json!({ "a": 1, "b": { "y": 2 }, "c": 3 }));
    let err = s.write("o", Tv::clean(json!([1]))).unwrap_err();
    assert!(matches!(err, StateError::Type { reducer: Reducer::Merge, .. }), "{err:?}");
    assert_eq!(s.get("o").unwrap().v, json!({ "a": 1, "b": { "y": 2 }, "c": 3 }), "a refused write changes nothing");
}

#[test]
fn add_sums_numbers_and_integers_stay_integers() {
    let mut s = state_with("n", Reducer::Add);
    s.write("n", Tv::clean(json!(1))).unwrap();
    s.write("n", Tv::clean(json!(2))).unwrap();
    assert_eq!(s.get("n").unwrap().v, json!(3));
    s.write("n", Tv::clean(json!(0.5))).unwrap();
    assert_eq!(s.get("n").unwrap().v, json!(3.5));
    let err = s.write("n", Tv::clean(json!("4"))).unwrap_err();
    assert!(matches!(err, StateError::Type { reducer: Reducer::Add, .. }), "{err:?}");
}

#[test]
fn a_bad_variable_name_is_refused() {
    let mut s = RunState::new();
    for name in ["", "a.b", "a b", " "] {
        assert!(matches!(s.write(name, Tv::clean(json!(1))), Err(StateError::Name(_))), "{name:?}");
        assert!(matches!(s.declare(name, Reducer::Add), Err(StateError::Name(_))), "{name:?}");
    }
}

// ── taint through a write ───────────────────────────────────────────────

#[test]
fn a_variable_tainted_in_one_iteration_is_tainted_in_the_next() {
    // append keeps what was there: one tainted write taints the whole list for good.
    let mut s = state_with("items", Reducer::Append);
    s.write("items", Tv::tainted(json!("from outside"))).unwrap();
    s.write("items", Tv::clean(json!("from the design"))).unwrap();
    let v = s.get("items").unwrap();
    assert_eq!((v.tainted, v.derived), (true, false));
    // replace clears nothing either: a clean write over a tainted value stays tainted (spec 4.1).
    let mut r = RunState::new();
    r.write("x", Tv::tainted(json!("a"))).unwrap();
    r.write("x", Tv::clean(json!("b"))).unwrap();
    assert_eq!(r.get("x"), Some(&Tv::tainted(json!("b"))));
}

#[test]
fn add_and_merge_join_the_bits_and_a_derived_number_stays_derived() {
    let mut n = state_with("count", Reducer::Add);
    n.write("count", Tv::clean(json!(1))).unwrap();
    n.write("count", Tv::derived(json!(2))).unwrap();
    let v = n.get("count").unwrap();
    assert_eq!((v.v.clone(), v.tainted, v.derived), (json!(3), false, true));
    n.write("count", Tv::clean(json!(1))).unwrap();
    assert!(n.get("count").unwrap().derived, "adding a clean number does not clean it");
    let mut m = state_with("o", Reducer::Merge);
    m.write("o", Tv::clean(json!({ "a": 1 }))).unwrap();
    m.write("o", Tv::tainted(json!({ "b": "x" }))).unwrap();
    assert!(m.get("o").unwrap().tainted);
}

// ── the size cap ────────────────────────────────────────────────────────

#[test]
fn state_over_the_cap_fails_the_write_with_the_size_and_changes_nothing() {
    let mut s = RunState::new();
    s.write("small", Tv::clean(json!("ok"))).unwrap();
    let before = s.size_bytes();
    assert_eq!(before, serde_json::to_string(&s.snapshot()).unwrap().len());
    let err = s.write("big", Tv::clean(json!("x".repeat(STATE_CAP_BYTES)))).unwrap_err();
    match err {
        StateError::TooLarge { bytes, cap } => assert!(bytes > STATE_CAP_BYTES && cap == STATE_CAP_BYTES, "{bytes} {cap}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.size_bytes(), before);
    assert_eq!(s.get("big"), None);
    // An append that pushes the state over the cap fails the same way.
    let mut log = state_with("log", Reducer::Append);
    let chunk = Tv::clean(json!("y".repeat(60 * 1024)));
    let mut failed = false;
    for _ in 0..10 {
        if matches!(log.write("log", chunk.clone()), Err(StateError::TooLarge { .. })) {
            failed = true;
            break;
        }
    }
    assert!(failed, "five 60 KB writes pass 256 KB");
    assert!(log.size_bytes() <= STATE_CAP_BYTES);
}

// ── the checkpoint encoding ─────────────────────────────────────────────

#[test]
fn a_checkpoint_round_trips_with_every_taint_bit_and_reducer() {
    let c = sample(4);
    let bytes = encode(&c).unwrap();
    assert!(std::str::from_utf8(&bytes).is_ok(), "UTF-8 JSON");
    let back = decode(&bytes).unwrap();
    assert_eq!(back, c);
    assert_eq!(back.state.vars["title"], Tv::tainted(json!("fix the build")));
    assert_eq!(back.state.vars["attempts"], Tv::derived(json!(2)));
    assert_eq!(back.state.vars["owner"], Tv::clean(json!("agenty")));
    // A restored state keeps working under its reducers.
    let mut s = RunState::from_snapshot(back.state);
    s.write("attempts", Tv::clean(json!(1))).unwrap();
    let a = s.get("attempts").unwrap();
    assert_eq!((a.v.clone(), a.derived), (json!(3), true));
}

#[test]
fn a_restore_never_clears_a_bit_and_a_missing_bit_is_unreadable() {
    let mut value = serde_json::from_slice::<serde_json::Value>(&encode(&sample(1)).unwrap()).unwrap();
    value["state"]["vars"]["title"].as_object_mut().unwrap().remove("tainted");
    let err = decode(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(matches!(err, CheckpointError::Unreadable(_)), "{err:?}");
    assert!(matches!(decode(b"not json"), Err(CheckpointError::Unreadable(_))));
    assert!(matches!(decode(b"{}"), Err(CheckpointError::Unreadable(_))));
}

#[test]
fn a_newer_format_is_refused_before_the_rest_is_read() {
    let mut c = sample(0);
    c.schema_version = SCHEMA_VERSION + 1;
    let mut bytes = serde_json::to_value(&c).unwrap();
    // Even with fields this build does not know, the version decides.
    bytes["something_new"] = json!({ "a": 1 });
    bytes.as_object_mut().unwrap().remove("frontier");
    let err = decode(&serde_json::to_vec(&bytes).unwrap()).unwrap_err();
    assert_eq!(err, CheckpointError::UnsupportedVersion { found: SCHEMA_VERSION + 1, supported: SCHEMA_VERSION });
}

#[test]
fn a_run_has_at_most_the_step_limit_of_checkpoints() {
    assert!(encode(&sample(MAX_STEPS - 1)).is_ok());
    assert_eq!(encode(&sample(MAX_STEPS)).unwrap_err(), CheckpointError::TooManySteps { step: MAX_STEPS, max: MAX_STEPS });
}

#[test]
fn a_diff_applied_to_the_checkpoint_before_gives_the_next_one() {
    let prev = sample(4);
    let mut next = sample(5);
    next.state.vars.insert("title".into(), Tv::clean(json!("typed by a person")));
    next.state.vars.insert("new".into(), Tv::tainted(json!([1])));
    next.state.vars.remove("owner");
    next.frontier.ready = vec!["n4".into(), "n5".into()];
    next.loop_counters.insert("guard".into(), 3);
    next.cost_usd = 0.5;
    next.elapsed_ms = 2_500;
    next.deadlines = BTreeMap::new();
    let d = diff(&prev, &next).unwrap();
    assert_eq!((d.from_step, d.to_step), (4, 5));
    assert_eq!(d.removed_vars, vec!["owner".to_string()]);
    assert_eq!(apply(&prev, &d).unwrap(), next);
}

#[test]
fn a_diff_of_a_small_change_is_much_smaller_than_the_checkpoint() {
    let mut prev = sample(1);
    prev.state.vars.insert("big".into(), Tv::tainted(json!("z".repeat(50_000))));
    let mut next = prev.clone();
    next.step = 2;
    next.cost_usd += 0.01;
    next.elapsed_ms += 10;
    let whole = encode(&next).unwrap().len();
    let d = serde_json::to_vec(&diff(&prev, &next).unwrap()).unwrap().len();
    assert!(d * 20 < whole, "diff {d} bytes against {whole}");
    // Nothing changed but the step: no state at all in the diff.
    let mut same = prev.clone();
    same.step = 2;
    let none = diff(&prev, &same).unwrap();
    assert!(none.changed_state.vars.is_empty() && none.removed_vars.is_empty() && none.frontier.is_none());
}

#[test]
fn a_diff_applies_only_to_the_checkpoint_it_follows() {
    let prev = sample(4);
    let d = diff(&prev, &sample(5)).unwrap();
    assert!(matches!(apply(&sample(7), &d), Err(CheckpointError::DoesNotFollow(_))), "another step");
    let mut other_run = sample(4);
    other_run.run_id = "run-2".into();
    assert!(matches!(apply(&other_run, &d), Err(CheckpointError::DoesNotFollow(_))), "another run");
    assert!(matches!(diff(&sample(4), &sample(6)), Err(CheckpointError::DoesNotFollow(_))), "a diff is for the next step only");
    let _: Checkpoint = prev;
}

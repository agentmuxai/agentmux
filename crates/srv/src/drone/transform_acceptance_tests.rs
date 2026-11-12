// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests for module A, the Transform evaluator
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9, §7 "Transform").
//!
//! They fail until `transform::evaluate` is written, and are marked `ignore`
//! so main stays green; the module's PR deletes each `#[ignore]` line and
//! changes nothing else in this file. Run them with
//! `cargo test -p agentmux-srv --bin agentmux-srv drone::transform_acceptance -- --ignored`.

use serde_json::{json, Value};

use super::transform::{evaluate, BoundKind, Transform, TransformError, MAX_ITEMS, MAX_OPERATIONS, MAX_OUTPUT_BYTES};
use super::value::Tv;

fn chain(ops: Value) -> Transform {
    serde_json::from_value(json!({ "ops": ops })).expect("a valid chain")
}

fn run(ops: Value, input: Value) -> Result<Tv, TransformError> {
    evaluate(&chain(ops), &Tv::clean(input))
}

fn ok(ops: Value, input: Value) -> Value {
    run(ops, input).expect("the chain runs").v
}

fn people() -> Value {
    json!([
        { "name": "Ada", "age": 36, "team": "core", "tags": ["a", "b"] },
        { "name": "Bo", "age": 25, "team": "web", "tags": ["b"] },
        { "name": "Cy", "age": 41, "team": "core", "tags": [] }
    ])
}

#[test]
#[ignore = "module A"]
fn map_and_filter_use_item_and_index() {
    assert_eq!(ok(json!([{ "op": "map", "expr": "item.name" }]), people()), json!(["Ada", "Bo", "Cy"]));
    assert_eq!(ok(json!([{ "op": "map", "expr": "item.age + index" }]), people()), json!([36, 26, 43]));
    assert_eq!(ok(json!([{ "op": "filter", "expr": "item.age > 30 && item.team == 'core'" }, { "op": "map", "expr": "item.name" }]), people()), json!(["Ada", "Cy"]));
    assert_eq!(ok(json!([{ "op": "filter", "expr": "!(item.team == 'core') || item.age >= 41" }, { "op": "map", "expr": "item.name" }]), people()), json!(["Bo", "Cy"]));
}

#[test]
#[ignore = "module A"]
fn pick_and_omit_work_on_an_object_and_on_every_object_of_a_list() {
    assert_eq!(ok(json!([{ "op": "pick", "fields": ["name"] }]), json!({ "name": "Ada", "age": 36 })), json!({ "name": "Ada" }));
    assert_eq!(ok(json!([{ "op": "omit", "fields": ["tags", "team"] }, { "op": "first" }]), people()), json!({ "name": "Ada", "age": 36 }));
    assert_eq!(ok(json!([{ "op": "pick", "fields": ["name", "nope"] }, { "op": "last" }]), people()), json!({ "name": "Cy" }));
}

#[test]
#[ignore = "module A"]
fn sort_is_stable_and_can_be_descending() {
    assert_eq!(ok(json!([{ "op": "sort", "by": "item.age" }, { "op": "map", "expr": "item.name" }]), people()), json!(["Bo", "Ada", "Cy"]));
    assert_eq!(ok(json!([{ "op": "sort", "by": "item.age", "desc": true }, { "op": "map", "expr": "item.name" }]), people()), json!(["Cy", "Ada", "Bo"]));
    assert_eq!(ok(json!([{ "op": "sort", "by": "item.team" }, { "op": "map", "expr": "item.name" }]), people()), json!(["Ada", "Cy", "Bo"]), "equal keys keep their input order");
}

#[test]
#[ignore = "module A"]
fn group_unique_slice_first_last_flatten() {
    let (ada, bo, cy) = (people()[0].clone(), people()[1].clone(), people()[2].clone());
    assert_eq!(ok(json!([{ "op": "group", "by": "item.team" }]), people()), json!({ "core": [ada, cy], "web": [bo] }));
    assert_eq!(ok(json!([{ "op": "unique" }]), json!([1, 2, 1, 3, 2])), json!([1, 2, 3]));
    assert_eq!(ok(json!([{ "op": "unique", "by": "item.team" }, { "op": "map", "expr": "item.name" }]), people()), json!(["Ada", "Bo"]));
    assert_eq!(ok(json!([{ "op": "slice", "start": 1, "end": 3 }]), json!([1, 2, 3, 4])), json!([2, 3]));
    assert_eq!(ok(json!([{ "op": "slice", "start": 2 }]), json!([1, 2, 3, 4])), json!([3, 4]));
    assert_eq!(ok(json!([{ "op": "slice", "start": 9, "end": 20 }]), json!([1, 2])), json!([]), "ends are clamped");
    assert_eq!(ok(json!([{ "op": "first" }]), json!([7, 8])), json!(7));
    assert_eq!(ok(json!([{ "op": "last" }]), json!([7, 8])), json!(8));
    assert_eq!(ok(json!([{ "op": "first" }]), json!([])), Value::Null);
    assert_eq!(ok(json!([{ "op": "flatten" }]), json!([[1, 2], 3, [4]])), json!([1, 2, 3, 4]));
}

#[test]
#[ignore = "module A"]
fn join_split_and_merge() {
    assert_eq!(ok(json!([{ "op": "join", "sep": ", " }]), json!(["a", "b", "c"])), json!("a, b, c"));
    assert_eq!(ok(json!([{ "op": "join", "sep": "-" }]), json!([1, 2])), json!("1-2"));
    assert_eq!(ok(json!([{ "op": "split", "sep": "," }]), json!("a,b,,c")), json!(["a", "b", "", "c"]));
    assert_eq!(ok(json!([{ "op": "merge", "with": { "b": 2, "c": 3 } }]), json!({ "a": 1, "b": 0 })), json!({ "a": 1, "b": 2, "c": 3 }));
}

#[test]
#[ignore = "module A"]
fn count_sum_min_max_and_exists() {
    assert_eq!(ok(json!([{ "op": "count" }]), people()), json!(3));
    assert_eq!(ok(json!([{ "op": "count" }]), json!({ "a": 1, "b": 2 })), json!(2));
    assert_eq!(ok(json!([{ "op": "count" }]), json!("héllo")), json!(5));
    assert_eq!(ok(json!([{ "op": "sum" }]), json!([1, 2, 3])), json!(6));
    assert_eq!(ok(json!([{ "op": "sum", "by": "item.age" }]), people()), json!(102));
    assert_eq!(ok(json!([{ "op": "min", "by": "item.age" }]), people()), json!(25));
    assert_eq!(ok(json!([{ "op": "max", "by": "item.age" }]), people()), json!(41));
    assert_eq!(ok(json!([{ "op": "max" }]), json!([])), Value::Null);
    assert_eq!(ok(json!([{ "op": "exists", "expr": "item.age > 40" }]), people()), json!(true));
    assert_eq!(ok(json!([{ "op": "exists", "expr": "item.age > 50" }]), people()), json!(false));
    assert_eq!(ok(json!([{ "op": "exists" }]), json!([])), json!(false));
    assert_eq!(ok(json!([{ "op": "exists", "expr": "a.b" }]), json!({ "a": { "b": 1 } })), json!(true));
    assert_eq!(ok(json!([{ "op": "exists", "expr": "a.c" }]), json!({ "a": { "b": 1 } })), json!(false));
}

#[test]
#[ignore = "module A"]
fn the_expression_grammar_of_the_module_docs() {
    let one = |expr: &str, item: Value| ok(json!([{ "op": "map", "expr": expr }]), json!([item]))[0].clone();
    // arithmetic and its types
    assert_eq!(one("1 + 2 * 3", Value::Null), json!(7));
    assert_eq!(one("(1 + 2) * 3", Value::Null), json!(9));
    assert_eq!(one("7 % 4", Value::Null), json!(3));
    assert_eq!(one("7 / 2", Value::Null), json!(3.5));
    assert_eq!(one("-item", json!(5)), json!(-5));
    // comparison, containment, regex, sets
    assert_eq!(one("item == 'a'", json!("a")), json!(true));
    assert_eq!(one("item == 1", json!("1")), json!(false), "different types are not equal");
    assert_eq!(one("item != 1", json!("1")), json!(true));
    assert_eq!(one("item contains 'ell'", json!("hello")), json!(true));
    assert_eq!(one("item contains 2", json!([1, 2])), json!(true));
    assert_eq!(one("item matches '^v[0-9]+$'", json!("v12")), json!(true));
    assert_eq!(one("item matches '^v[0-9]+$'", json!("x12")), json!(false));
    assert_eq!(one("item in_set ['a', 'b']", json!("b")), json!(true));
    assert_eq!(one("item in_set ['a', 'b']", json!("c")), json!(false));
    // functions and missing paths
    assert_eq!(one("lower(item)", json!("AbC")), json!("abc"));
    assert_eq!(one("trunc(item, 3)", json!("abcdef")), json!("abc"));
    assert_eq!(one("default(item.x, 'none')", json!({})), json!("none"));
    assert_eq!(one("default(item.x, 'none')", json!({ "x": "set" })), json!("set"));
    assert_eq!(one("exists(item.x)", json!({ "x": 0 })), json!(true));
    assert_eq!(one("item.a.b.c", json!({})), Value::Null, "a missing path is null");
    // strings with quotes of either kind, and the escapes
    assert_eq!(one(r"'it\'s'", Value::Null), json!("it's"));
    assert_eq!(one(r#""a\"b""#, Value::Null), json!("a\"b"));
    assert_eq!(one(r"'a\\b'", Value::Null), json!("a\\b"));
}

#[test]
#[ignore = "module A"]
fn bad_expressions_and_ops_are_errors_naming_the_step() {
    let err = |ops: Value, input: Value| run(ops, input).unwrap_err();
    assert!(matches!(err(json!([{ "op": "map", "expr": "item +" }]), json!([1])), TransformError::Expr { index: 0, .. }), "a parse error");
    assert!(matches!(err(json!([{ "op": "map", "expr": "1 / 0" }]), json!([1])), TransformError::Expr { index: 0, .. }), "division by zero");
    assert!(matches!(err(json!([{ "op": "map", "expr": "item < 'a'" }]), json!([1])), TransformError::Expr { index: 0, .. }), "ordering across types");
    assert!(matches!(err(json!([{ "op": "map", "expr": "item matches '('" }]), json!(["a"])), TransformError::Expr { index: 0, .. }), "a bad regex");
    assert!(matches!(err(json!([{ "op": "count" }, { "op": "map", "expr": "item" }]), json!([1])), TransformError::Op { index: 1, .. }), "a map on a number");
    assert!(matches!(err(json!([{ "op": "sum" }]), json!(["a"])), TransformError::Op { index: 0, .. }), "a sum of text");
}

#[test]
#[ignore = "module A"]
fn there_is_no_way_to_write_a_loop_or_call_out() {
    for expr in [
        "exec('ls')",
        "sh('ls')",
        "fetch('http://example.com')",
        "while(true)",
        "x = 1",
        "item.map(item)",
        "f(f(1))",
        "lower",
        "1; 2",
        "eval(item)",
    ] {
        let r = run(json!([{ "op": "map", "expr": expr }]), json!([1]));
        assert!(matches!(r, Err(TransformError::Expr { index: 0, .. })), "{expr:?} must not run: {r:?}");
    }
}

#[test]
#[ignore = "module A"]
fn more_than_the_item_limit_in_fails_naming_the_items_bound() {
    let big: Vec<Value> = (0..=MAX_ITEMS).map(|i| json!(i)).collect();
    let err = run(json!([{ "op": "count" }]), Value::Array(big)).unwrap_err();
    assert_eq!(err, TransformError::Bound { kind: BoundKind::Items, limit: MAX_ITEMS as u64 });
    let exactly: Vec<Value> = (0..MAX_ITEMS).map(|i| json!(i)).collect();
    assert_eq!(ok(json!([{ "op": "count" }]), Value::Array(exactly)), json!(MAX_ITEMS));
}

#[test]
#[ignore = "module A"]
fn an_evaluation_over_the_operation_budget_fails_naming_the_operations_bound() {
    // 10,000 items through 11 maps is at least 110,000 element visits.
    let items: Vec<Value> = (0..MAX_ITEMS).map(|i| json!(i)).collect();
    let ops: Vec<Value> = (0..11).map(|_| json!({ "op": "map", "expr": "item + 1" })).collect();
    let err = run(Value::Array(ops), Value::Array(items)).unwrap_err();
    assert_eq!(err, TransformError::Bound { kind: BoundKind::Operations, limit: MAX_OPERATIONS });
}

#[test]
#[ignore = "module A"]
fn an_output_over_the_state_cap_fails_naming_the_output_bound() {
    let word = "x".repeat(300);
    let items: Vec<Value> = (0..1_000).map(|_| json!(word)).collect();
    let err = run(json!([{ "op": "map", "expr": "item" }]), Value::Array(items)).unwrap_err();
    assert_eq!(err, TransformError::Bound { kind: BoundKind::Output, limit: MAX_OUTPUT_BYTES as u64 });
}

#[test]
#[ignore = "module A"]
fn a_hostile_regex_cannot_stall_it() {
    let started = std::time::Instant::now();
    let hay = format!("{}!", "a".repeat(5_000));
    let r = run(json!([{ "op": "map", "expr": "item matches '(a+)+$'" }]), json!([hay]));
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "the regex engine is linear time");
    assert_eq!(r.unwrap().v, json!([false]));
    // A pattern too large to compile is an expression error, not a hang or a panic.
    let huge = "a{1000}".repeat(1_000);
    let r = run(json!([{ "op": "map", "expr": format!("item matches '{huge}'") }]), json!(["a"]));
    assert!(matches!(r, Err(TransformError::Expr { index: 0, .. })), "{r:?}");
}

#[test]
#[ignore = "module A"]
fn the_output_carries_the_taint_bits_of_the_chain() {
    let tainted = |ops: Value| evaluate(&chain(ops), &Tv::tainted(people())).unwrap();
    // A count over a tainted list is clean as data and derived.
    let c = tainted(json!([{ "op": "count" }]));
    assert_eq!((c.v, c.tainted, c.derived), (json!(3), false, true));
    // A map over it is tainted.
    let m = tainted(json!([{ "op": "map", "expr": "item.name" }]));
    assert_eq!((m.tainted, m.derived), (true, false));
    // A filter by in_set keeps the elements tainted: only the membership is clean.
    let f = tainted(json!([{ "op": "filter", "expr": "item.team in_set ['core']" }]));
    assert_eq!((f.v.as_array().map(Vec::len), f.tainted, f.derived), (Some(2), true, false));
    // A sum of a count stays derived; an exists is a derived boolean.
    let s = tainted(json!([{ "op": "count" }, { "op": "sum" }]));
    assert_eq!((s.tainted, s.derived), (false, true));
    let e = tainted(json!([{ "op": "exists", "expr": "item.age > 40" }]));
    assert_eq!((e.v, e.tainted, e.derived), (json!(true), false, true));
    // Over a clean list nothing is tainted or derived; over a derived value, derived stays.
    let clean = evaluate(&chain(json!([{ "op": "count" }])), &Tv::clean(people())).unwrap();
    assert_eq!((clean.tainted, clean.derived), (false, false));
    let derived = evaluate(&chain(json!([{ "op": "map", "expr": "item" }])), &Tv::derived(json!([1, 2]))).unwrap();
    assert_eq!((derived.tainted, derived.derived), (false, true));
}

#[test]
#[ignore = "module A"]
fn the_input_is_not_changed_and_the_same_chain_gives_the_same_answer() {
    let input = Tv::tainted(people());
    let t = chain(json!([{ "op": "sort", "by": "item.age", "desc": true }, { "op": "map", "expr": "item.name" }]));
    let a = evaluate(&t, &input).unwrap();
    let b = evaluate(&t, &input).unwrap();
    assert_eq!(a, b);
    assert_eq!(input, Tv::tainted(people()));
}

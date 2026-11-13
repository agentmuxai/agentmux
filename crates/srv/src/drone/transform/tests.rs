// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unit tests for the Transform parser and evaluator
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.9).

use super::*;
use serde_json::{json, Value};

fn after(op: &TransformOp, input: &Tv) -> (bool, bool) {
    taint_out(op, input)
}

/// One expression over one item, as a `map` would read it.
fn one(expr: &str, item: Value) -> Result<Value, TransformError> {
    let t: Transform = serde_json::from_value(json!({ "ops": [{ "op": "map", "expr": expr }] }))
        .expect("a valid chain");
    let out = evaluate(&t, &Tv::clean(json!([item])))?;
    Ok(out.v[0].clone())
}

fn value(expr: &str, item: Value) -> Value {
    one(expr, item).expect("the expression is valid")
}

/// Precedence and associativity as the module docs list them.
#[test]
fn the_parser_follows_the_precedence_of_the_grammar() {
    assert_eq!(value("1 + 2 * 3", Value::Null), json!(7));
    assert_eq!(value("(1 + 2) * 3", Value::Null), json!(9));
    assert_eq!(
        value("10 - 3 - 2", Value::Null),
        json!(5),
        "left associative"
    );
    assert_eq!(value("2 * 3 % 4", Value::Null), json!(2));
    assert_eq!(value("-3 * -2", Value::Null), json!(6));
    assert_eq!(value("!false", Value::Null), json!(true));
    assert_eq!(
        value("1 + 1 == 2", Value::Null),
        json!(true),
        "sum before comparison"
    );
    assert_eq!(value("'a' in_set ['a', 'b']", Value::Null), json!(true));
    assert_eq!(value("lower('AB')", Value::Null), json!("ab"));
}

#[test]
fn a_parse_error_stops_the_chain_and_names_its_step() {
    assert!(matches!(
        one("item +", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    assert!(matches!(
        one("1 2", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    assert!(matches!(
        one("1 < 2 < 3", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    assert!(matches!(
        one("lower(1, 2)", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    assert!(matches!(
        one("item in_set", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    assert!(matches!(
        one("item.", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    let later: Transform = serde_json::from_value(json!({ "ops": [
            { "op": "filter", "expr": "true" },
            { "op": "map", "expr": "1 / 0" }
        ] }))
    .expect("a valid chain");
    let error = evaluate(&later, &Tv::clean(json!([1]))).expect_err("the second step fails");
    assert!(
        matches!(error, TransformError::Expr { index: 1, .. }),
        "{error:?}"
    );
}

#[test]
fn a_missing_path_is_null_and_a_stray_name_is_an_error() {
    assert_eq!(value("item.a.b", json!({})), Value::Null);
    assert_eq!(value("item.a.b", json!({ "a": { "b": 2 } })), json!(2));
    assert_eq!(value("index", json!("x")), json!(0));
    assert!(matches!(
        one("other", Value::Null),
        Err(TransformError::Expr { index: 0, .. })
    ));
    assert_eq!(
        value("exists(item.a.b)", json!({ "a": { "b": null } })),
        json!(false)
    );
}

/// The taint table of §3.9 and the derived-value rule of §4.5.
#[test]
fn the_taint_table() {
    let tainted_list = Tv::tainted(json!(["a", "b"]));
    let clean_list = Tv::clean(json!([1, 2]));
    let derived_num = Tv::derived(json!(2));
    // Results that cannot carry text are clean over tainted input, and derived.
    for op in [
        TransformOp::Count,
        TransformOp::Sum { by: None },
        TransformOp::Min { by: None },
        TransformOp::Max { by: None },
        TransformOp::Exists { expr: None },
    ] {
        assert_eq!(
            after(&op, &tainted_list),
            (false, true),
            "{op:?} over tainted"
        );
        assert_eq!(after(&op, &clean_list), (false, false), "{op:?} over clean");
        assert_eq!(
            after(&op, &derived_num),
            (false, true),
            "{op:?} over derived"
        );
    }
    // Everything else carries the input's bits.
    for op in [
        TransformOp::Map {
            expr: "item".into(),
        },
        TransformOp::Filter {
            expr: "true".into(),
        },
        TransformOp::Pick { fields: vec![] },
        TransformOp::Omit { fields: vec![] },
        TransformOp::Sort {
            by: "item".into(),
            desc: false,
        },
        TransformOp::Group { by: "item".into() },
        TransformOp::Unique { by: None },
        TransformOp::Slice {
            start: 0,
            end: None,
        },
        TransformOp::First,
        TransformOp::Last,
        TransformOp::Flatten,
        TransformOp::Join { sep: ",".into() },
        TransformOp::Split { sep: ",".into() },
        TransformOp::Merge { with: json!({}) },
    ] {
        assert_eq!(
            after(&op, &tainted_list),
            (true, false),
            "{op:?} over tainted"
        );
        assert_eq!(after(&op, &clean_list), (false, false), "{op:?} over clean");
        assert_eq!(
            after(&op, &derived_num),
            (false, true),
            "{op:?} over derived"
        );
    }
}

#[test]
fn a_chain_keeps_the_derivation_after_a_count() {
    let tainted = Tv::tainted(json!(["a", "b"]));
    let ops = [
        TransformOp::Filter {
            expr: "true".into(),
        },
        TransformOp::Count,
    ];
    assert_eq!(chain_taint(&ops, &tainted), (false, true));
    // A sum of a count is still derived; text never comes back.
    let more = [TransformOp::Count, TransformOp::Sum { by: None }];
    assert_eq!(chain_taint(&more, &tainted), (false, true));
    assert_eq!(chain_taint(&ops, &Tv::clean(json!([1]))), (false, false));
    assert_eq!(chain_taint(&[], &tainted), (true, false));
}

#[test]
fn a_chain_reads_from_and_writes_to_json() {
    let t: Transform = serde_json::from_value(json!({ "ops": [
            { "op": "filter", "expr": "item.n > 1" },
            { "op": "sort", "by": "item.n", "desc": true },
            { "op": "slice", "start": 0, "end": 2 },
            { "op": "count" }
        ] }))
    .unwrap();
    assert_eq!(t.ops.len(), 4);
    assert_eq!(
        serde_json::from_value::<Transform>(serde_json::to_value(&t).unwrap()).unwrap(),
        t
    );
    assert!(
        serde_json::from_value::<Transform>(json!({ "ops": [{ "op": "eval", "expr": "1" }] }))
            .is_err(),
        "no op beyond the list"
    );
}

#[test]
fn every_op_has_a_name_and_the_expressions_of_a_chain_can_be_listed() {
    assert_eq!(op_names().len(), 19);
    let ops = vec![
        TransformOp::Map {
            expr: "item.a".into(),
        },
        TransformOp::Sort {
            by: "item.b".into(),
            desc: false,
        },
        TransformOp::Count,
    ];
    let exprs: Vec<&str> = ops.iter().flat_map(expressions_of).collect();
    assert_eq!(exprs, ["item.a", "item.b"]);
}

#[test]
fn an_expression_over_the_source_limit_is_an_error() {
    let long = format!("item{}", " + 1".repeat(20_000));
    assert!(
        matches!(
            one(&long, Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ),
        "a source over the limit must not reach the tokenizer"
    );
}

#[test]
fn an_operator_chain_deeper_than_the_evaluation_limit_is_an_error() {
    let chain = format!("{}1", "1 + ".repeat(1_000));
    assert!(
        matches!(
            one(&chain, Value::Null),
            Err(TransformError::Expr { index: 0, .. })
        ),
        "a left-deep chain must not recurse to the stack limit"
    );
}

#[test]
fn nesting_past_the_limit_is_an_error_not_a_crash() {
    for source in [
        format!("{}1{}", "(".repeat(200), ")".repeat(200)),
        format!("{}1{}", "[".repeat(200), "]".repeat(200)),
        format!("{}true", "!".repeat(200)),
        format!("{}1", "-".repeat(200)),
    ] {
        assert!(
            matches!(
                one(&source, Value::Null),
                Err(TransformError::Expr { index: 0, .. })
            ),
            "200 levels must be refused"
        );
    }
}

#[test]
fn the_smallest_integer_against_minus_one_does_not_panic() {
    let min = "(-9223372036854775807 - 1)";
    let remainder = value(&format!("{min} % -1"), Value::Null);
    assert_eq!(remainder.as_f64(), Some(0.0));
    assert!(one(&format!("{min} / -1"), Value::Null).is_ok());
}

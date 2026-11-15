// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Condition block's richer expressions run on the Transform expression language
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §4.1): `&&`, `||`, `!`, grouping, `contains`, `matches`,
//! `in_set`, `exists` and the filters, with every bound the language has (source size, nesting, operations, a
//! linear-time regex). The block turns each `{{reference}}` into a name bound to the value it holds, so the
//! expression is read as written and what a reference holds is never read as syntax.

use serde_json::{Map, Value};

use super::expr::{eval_at, parse_or, State};
use super::TransformError;

/// What the expression says, as true or false. `bound` gives the values the expression reads as `item.<name>`.
pub fn evaluate_condition(expr: &str, bound: &Map<String, Value>) -> Result<bool, String> {
    let ast = parse_or(expr, 0).map_err(message_of)?;
    let item = Value::Object(bound.clone());
    match eval_at(&ast, &item, 0, 0, &mut State::default()).map_err(message_of)? {
        Value::Bool(answer) => Ok(answer),
        other => Err(format!("the condition gave {}, not true or false", describe(&other))),
    }
}

fn message_of(error: TransformError) -> String {
    match error {
        TransformError::Expr { message, .. } | TransformError::Op { message, .. } => message,
        bound @ TransformError::Bound { .. } => bound.to_string(),
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "nothing",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn bound(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn eval(expr: &str, v: Value) -> Result<bool, String> {
        evaluate_condition(expr, &bound(v))
    }

    #[test]
    fn combines_with_and_or_not_and_grouping() {
        let v = json!({ "a": 5, "b": "red" });
        assert_eq!(eval("item.a > 3 && item.b == 'red'", v.clone()), Ok(true));
        assert_eq!(eval("item.a > 9 || item.b == 'red'", v.clone()), Ok(true));
        assert_eq!(eval("!(item.a > 9)", v.clone()), Ok(true));
        assert_eq!(eval("item.a > 9 && (item.b == 'red' || true)", v.clone()), Ok(false));
        assert_eq!(eval("item.a > 3 && !(item.b == 'red')", v), Ok(false));
    }

    #[test]
    fn reads_text_lists_and_membership() {
        let v = json!({ "log": "build failed: disk full", "area": "srv", "seats": ["srv", "ui"] });
        assert_eq!(eval("item.log contains 'disk full'", v.clone()), Ok(true));
        assert_eq!(eval("item.log matches 'fail(ed|ure)'", v.clone()), Ok(true));
        assert_eq!(eval("item.log matches '^ok'", v.clone()), Ok(false));
        assert_eq!(eval("item.seats contains item.area", v.clone()), Ok(true));
        assert_eq!(eval("item.area in_set ['srv', 'ui', 'cef']", v.clone()), Ok(true));
        assert_eq!(eval("item.area in_set ['docs']", v), Ok(false));
    }

    #[test]
    fn exists_is_false_for_a_missing_value_and_a_hostile_pattern_cannot_hang() {
        let v = json!({ "a": null, "b": 0 });
        assert_eq!(eval("exists(item.a)", v.clone()), Ok(false));
        assert_eq!(eval("exists(item.b)", v.clone()), Ok(true));
        assert_eq!(eval("exists(item.nope)", v), Ok(false));
        let hostile = json!({ "text": "a".repeat(5000) + "!" });
        assert_eq!(eval("item.text matches '(a+)+$'", hostile), Ok(false));
    }

    #[test]
    fn an_answer_that_is_not_a_boolean_is_refused_by_name() {
        assert_eq!(eval("item.a", json!({ "a": 3 })), Err("the condition gave a number, not true or false".into()));
        assert_eq!(eval("item.a", json!({})), Err("the condition gave nothing, not true or false".into()));
        assert_eq!(eval("item.a", json!({ "a": "yes" })), Err("the condition gave text, not true or false".into()));
    }

    #[test]
    fn a_bad_expression_says_why() {
        assert!(eval("item.a >", json!({ "a": 1 })).is_err());
        assert!(eval("item.a matches '('", json!({ "a": "x" })).is_err());
    }
}

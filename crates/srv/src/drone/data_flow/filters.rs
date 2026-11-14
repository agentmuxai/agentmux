// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Template filters: `{{a.log | tail:20 | lower}}`.
//!
//! A reference may be followed by `| filter` steps, each applied to the value the step before it left. They cut and
//! reshape a value (a log to its last lines, a field out of an API body); none of them runs code, reads another value, or
//! makes text clean: what a filter returns is still what the reference held, so the taint bits are the reference's own.
//!
//!   `tail:N` / `head:N`  the last / first N lines of text, or items of a list
//!   `json:path`          parse text as JSON (or take an object as it is), then walk `a.b.0.c`; missing is nothing
//!   `lower`              lowercase text
//!   `trunc:N`            the first N characters of text
//!   `default:x`          `x` when the value is missing, null or empty
//!
//! A step that is not one of these, or has an argument it cannot read, leaves the whole token unresolved and visible, as
//! an unknown reference does. Every step is bounded: a count is at most `MAX_COUNT`, parsed text at most `MAX_JSON`
//! bytes, a path at most `MAX_PATH` segments.

use serde_json::Value;

const MAX_COUNT: usize = 100_000;
const MAX_JSON: usize = 1024 * 1024;
const MAX_PATH: usize = 16;

/// Splits a token body into its reference path and the filter steps after the first `|`.
pub fn split(token: &str) -> (&str, Option<&str>) {
    match token.split_once('|') {
        Some((path, steps)) => (path.trim(), Some(steps)),
        None => (token.trim(), None),
    }
}

/// Runs `steps` (`tail:20 | lower`) over `value`, `None` being a reference that held nothing. `None` back means the token
/// stays unresolved.
pub fn apply(value: Option<Value>, steps: &str) -> Option<Value> {
    let mut value = value;
    for step in steps.split('|') {
        let (name, arg) = match step.trim().split_once(':') {
            Some((name, arg)) => (name.trim(), Some(arg.trim())),
            None => (step.trim(), None),
        };
        // A step over nothing stays nothing, so a later `default` can still fill it; a step that is not one of these,
        // or has an argument it cannot read, ends the whole token.
        value = match (name, arg) {
            ("default", Some(fallback)) => Some(match value {
                Some(v) if !is_empty(&v) => v,
                _ => Value::String(unquote(fallback).to_string()),
            }),
            ("lower", None) => value.map(|v| Value::String(text_of(&v).to_lowercase())),
            ("trunc", Some(n)) => {
                let n = count(n)?;
                value.map(|v| Value::String(text_of(&v).chars().take(n).collect()))
            }
            ("tail", Some(n)) => {
                let n = count(n)?;
                value.map(|v| cut(v, n, true))
            }
            ("head", Some(n)) => {
                let n = count(n)?;
                value.map(|v| cut(v, n, false))
            }
            ("json", Some(path)) => value.and_then(|v| json_at(v, path)),
            _ => return None,
        };
    }
    value
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

fn count(arg: &str) -> Option<usize> {
    arg.parse::<usize>().ok().filter(|n| *n <= MAX_COUNT)
}

fn unquote(s: &str) -> &str {
    let quoted = s.len() >= 2 && matches!((s.as_bytes()[0], s.as_bytes()[s.len() - 1]), (b'\'', b'\'') | (b'"', b'"'));
    if quoted {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The last or first `n` items of a list, or lines of anything else read as text.
fn cut(v: Value, n: usize, from_end: bool) -> Value {
    if let Value::Array(items) = v {
        let len = items.len();
        let kept: Vec<Value> = if from_end { items.into_iter().skip(len.saturating_sub(n)).collect() } else { items.into_iter().take(n).collect() };
        return Value::Array(kept);
    }
    let text = text_of(&v);
    let lines: Vec<&str> = text.lines().collect();
    let picked = if from_end { &lines[lines.len().saturating_sub(n)..] } else { &lines[..lines.len().min(n)] };
    Value::String(picked.join("\n"))
}

/// The value at `path` (`a.b.0.c`) inside `v`, which is parsed first when it is text. `None` for a path that leads
/// nowhere, text that is not JSON, or one over the bounds.
fn json_at(v: Value, path: &str) -> Option<Value> {
    let mut cur = match v {
        Value::String(s) if s.len() <= MAX_JSON => serde_json::from_str::<Value>(&s).ok()?,
        Value::String(_) => return None,
        other => other,
    };
    let segments: Vec<&str> = path.split('.').map(str::trim).filter(|s| !s.is_empty()).collect();
    if segments.len() > MAX_PATH {
        return None;
    }
    for key in segments {
        cur = match cur {
            Value::Object(mut m) => m.remove(key)?,
            Value::Array(mut items) => {
                let at = key.parse::<usize>().ok().filter(|i| *i < items.len())?;
                items.swap_remove(at)
            }
            _ => return None,
        };
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(v: Value, steps: &str) -> Option<Value> {
        apply(Some(v), steps)
    }

    #[test]
    fn split_takes_the_path_before_the_first_bar() {
        assert_eq!(split("a.log | tail:2 | lower"), ("a.log", Some(" tail:2 | lower")));
        assert_eq!(split(" a.log "), ("a.log", None));
    }

    #[test]
    fn tail_and_head_cut_lines_and_lists() {
        assert_eq!(run(json!("1\n2\n3\n4"), "tail:2"), Some(json!("3\n4")));
        assert_eq!(run(json!("1\n2\n3\n4"), "head:1"), Some(json!("1")));
        assert_eq!(run(json!("1\n2"), "tail:10"), Some(json!("1\n2")));
        assert_eq!(run(json!("1\n2"), "tail:0"), Some(json!("")));
        assert_eq!(run(json!([1, 2, 3]), "tail:2"), Some(json!([2, 3])));
        assert_eq!(run(json!([1, 2, 3]), "head:2"), Some(json!([1, 2])));
    }

    #[test]
    fn json_walks_text_or_an_object_and_missing_is_nothing() {
        assert_eq!(run(json!(r#"{"a":{"b":[10,{"c":"x"}]}}"#), "json:a.b.1.c"), Some(json!("x")));
        assert_eq!(run(json!({ "a": { "b": 5 } }), "json:a.b"), Some(json!(5)));
        assert_eq!(run(json!(r#"{"a":1}"#), "json:b"), None);
        assert_eq!(run(json!("not json"), "json:a"), None);
        assert_eq!(run(json!([1]), "json:5"), None);
        assert_eq!(run(json!(r#"{"a":1}"#), "json:a | default:none"), Some(json!(1)));
    }

    #[test]
    fn lower_and_trunc_work_on_text_and_count_characters() {
        assert_eq!(run(json!("ÀBC"), "lower"), Some(json!("àbc")));
        assert_eq!(run(json!("héllo wörld"), "trunc:5"), Some(json!("héllo")));
        assert_eq!(run(json!(1234), "trunc:2"), Some(json!("12")));
    }

    #[test]
    fn default_fills_what_is_missing_null_or_blank_and_chains() {
        assert_eq!(apply(None, "default:none"), Some(json!("none")));
        assert_eq!(run(Value::Null, "default:'n/a'"), Some(json!("n/a")));
        assert_eq!(run(json!("  "), "default:x"), Some(json!("x")));
        assert_eq!(run(json!("kept"), "default:x"), Some(json!("kept")));
        assert_eq!(run(json!(0), "default:x"), Some(json!(0)));
        assert_eq!(apply(None, "lower"), None);
        assert_eq!(apply(None, "json:a | default:z"), Some(json!("z")), "a step over nothing is nothing, and default fills it");
        assert_eq!(run(json!(r#"{"a":1}"#), "json:b | default:z"), Some(json!("z")));
        assert_eq!(apply(None, "default:Hello | lower"), Some(json!("hello")));
    }

    #[test]
    fn an_unknown_step_or_unreadable_argument_leaves_the_token_unresolved() {
        assert_eq!(run(json!("x"), "shout"), None);
        assert_eq!(run(json!("x"), "tail"), None);
        assert_eq!(run(json!("x"), "tail:abc"), None);
        assert_eq!(run(json!("x"), "tail:-1"), None);
        assert_eq!(run(json!("x"), "tail:100001"), None);
        assert_eq!(run(json!("x"), "lower:1"), None);
        assert_eq!(run(json!("x"), ""), None);
    }

    #[test]
    fn bounds_hold_for_big_text_and_deep_paths() {
        assert_eq!(run(json!("x".repeat(MAX_JSON + 1)), "json:a"), None);
        let deep = vec!["a"; MAX_PATH + 1].join(".");
        assert_eq!(run(json!({}), &format!("json:{deep}")), None);
    }
}

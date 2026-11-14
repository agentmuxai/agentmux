// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Condition block — boolean expression evaluator.
//!
//! Phase 1 supports a deliberately tiny grammar to avoid a JS sandbox:
//!
//!   `<lhs> <op> <rhs>` where op ∈ {`==`, `!=`, `<`, `<=`, `>`, `>=`}
//!
//! Both sides are `{{}}`-resolved first. If both sides parse as numbers
//! the comparison is numeric; otherwise it's string. The block's
//! output is the boolean result, exposed as `{{<this_id>.result}}`.
//!
//! An expression that uses `&&`, `||`, `!`, grouping, a list, or the words `contains`, `matches`, `in_set` or
//! `exists` runs on the Transform expression language instead (`transform/condition.rs`): each `{{reference}}`
//! is bound to the value it holds, typed (a number, a list, an object), and the expression is read as written.
//! Nothing a reference holds is read as syntax. The simple form above keeps its own reading, bare words and all.

use serde_json::{json, Value};

use crate::drone::data_flow::ExecutionScope;
use crate::drone::transform::evaluate_condition;
use crate::drone::types::FlowNode;

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let expr = node
        .data
        .get("expr")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Condition block missing `expr`".to_string())?;
    // The operator is the author's: split the expression as written, then fill
    // in each side. Splitting what was filled in would let a value that holds
    // `==` (an agent's reply, an API field) change what is compared.
    let raw = expr.trim();
    let scanned = scan(raw)?;
    if scanned.rich {
        let mut bound = serde_json::Map::new();
        for (name, path) in &scanned.refs {
            bound.insert(name.clone(), scope.lookup_ref(path).unwrap_or(Value::Null));
        }
        let result = evaluate_condition(&scanned.rewritten, &bound).map_err(|e| format!("condition: {e}"))?;
        return Ok(json!({ "result": result }));
    }
    let result = match split_binop(raw) {
        Some((lhs, op, rhs)) => compare(&scope.resolve(lhs), op, &scope.resolve(rhs)),
        None => bare_boolean(&scope.resolve(raw))?,
    };
    Ok(json!({ "result": result }))
}

/// The authored expression, read once: whether it needs the expression language, and for that language each
/// `{{reference}}` replaced by the name (`item.r0`, ...) its value is bound to.
struct Scanned {
    rich: bool,
    rewritten: String,
    refs: Vec<(String, String)>,
}

/// Words the simple form has no meaning for.
const WORD_OPERATORS: [&str; 4] = ["contains", "matches", "in_set", "exists"];

fn scan(raw: &str) -> Result<Scanned, String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = Scanned { rich: false, rewritten: String::new(), refs: Vec::new() };
    let (mut i, mut quote, mut quoted_ref) = (0, None::<char>, false);
    let mut prev = None::<char>; // the last character outside quotes that was not a space
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' && i + 1 < chars.len() {
                out.rewritten.push(c);
                out.rewritten.push(chars[i + 1]);
                i += 2;
                continue;
            }
            quoted_ref |= c == '{' && chars.get(i + 1) == Some(&'{');
            if c == q {
                quote = None;
            }
            out.rewritten.push(c);
            i += 1;
            continue;
        }
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            let close = (i + 2..chars.len().saturating_sub(1)).find(|&k| chars[k] == '}' && chars[k + 1] == '}');
            let close = close.ok_or_else(|| "a `{{` in the condition has no `}}`".to_string())?;
            let path: String = chars[i + 2..close].iter().collect();
            let name = format!("r{}", out.refs.len());
            out.rewritten.push_str(&format!("item.{name}"));
            out.refs.push((name, path.trim().to_string()));
            prev = Some('}');
            i = close + 2;
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | ')' | '[' | ']' => out.rich = true,
            '&' | '|' if chars.get(i + 1) == Some(&c) => out.rich = true,
            '!' if chars.get(i + 1) != Some(&'=') && matches!(prev, None | Some('(' | '&' | '|' | '!')) => out.rich = true,
            c if c.is_alphabetic() || c == '_' => {
                let end = (i..chars.len()).find(|&k| !(chars[k].is_alphanumeric() || chars[k] == '_')).unwrap_or(chars.len());
                let word: String = chars[i..end].iter().collect();
                out.rich |= WORD_OPERATORS.contains(&word.as_str());
                out.rewritten.push_str(&word);
                prev = Some('a');
                i = end;
                continue;
            }
            _ => {}
        }
        out.rewritten.push(c);
        if !c.is_whitespace() {
            prev = Some(c);
        }
        i += 1;
    }
    if out.rich && quoted_ref {
        return Err("a {{reference}} cannot sit inside quotes in a condition with && || ! ( ) [ ] or contains, matches, in_set, exists: put it outside the quotes".to_string());
    }
    Ok(out)
}

fn bare_boolean(text: &str) -> Result<bool, String> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        Ok(true)
    } else if trimmed.eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        Err(format!("could not parse condition: `{trimmed}`"))
    }
}

fn compare(lhs: &str, op: &str, rhs: &str) -> bool {
    let lhs = lhs.trim().trim_matches(|c| c == '\'' || c == '"');
    let rhs = rhs.trim().trim_matches(|c| c == '\'' || c == '"');
    if let (Ok(a), Ok(b)) = (lhs.parse::<f64>(), rhs.parse::<f64>()) {
        return match op {
            "==" => (a - b).abs() < f64::EPSILON,
            "!=" => (a - b).abs() >= f64::EPSILON,
            "<" => a < b,
            "<=" => a <= b,
            ">" => a > b,
            _ => a >= b,
        };
    }
    match op {
        "==" => lhs == rhs,
        "!=" => lhs != rhs,
        "<" => lhs < rhs,
        "<=" => lhs <= rhs,
        ">" => lhs > rhs,
        _ => lhs >= rhs,
    }
}

/// Splits at the first occurrence of a recognized comparison operator.
/// Multi-char operators are tried before single-char.
fn split_binop(s: &str) -> Option<(&str, &str, &str)> {
    for op in &["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(idx) = s.find(op) {
            // Reject `<=` matching when the actual op is `<`.
            // Already handled by the order above.
            return Some((&s[..idx], op, &s[idx + op.len()..]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;

    fn mk(expr: &str) -> FlowNode {
        FlowNode {
            id: "c".to_string(),
            position: NodePosition::default(),
            data: json!({ "kind": "condition", "expr": expr }),
            node_type: None,
        }
    }

    #[tokio::test]
    async fn numeric_lt() {
        let n = mk("3 < 5");
        let out = run(&n, &ExecutionScope::new()).await.unwrap();
        assert_eq!(out, json!({ "result": true }));
    }

    #[tokio::test]
    async fn string_eq() {
        let n = mk("'a' == 'a'");
        let out = run(&n, &ExecutionScope::new()).await.unwrap();
        assert_eq!(out, json!({ "result": true }));
    }

    #[tokio::test]
    async fn rejects_unparseable() {
        let n = mk("foo bar baz");
        let r = run(&n, &ExecutionScope::new()).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn bare_true() {
        let n = mk("true");
        let out = run(&n, &ExecutionScope::new()).await.unwrap();
        assert_eq!(out, json!({ "result": true }));
    }

    #[tokio::test]
    async fn resolves_var_in_expr() {
        let n = mk("{{var.count}} >= 10");
        let mut scope = ExecutionScope::new();
        scope.vars.insert("count".to_string(), json!(15));
        let out = run(&n, &scope).await.unwrap();
        assert_eq!(out, json!({ "result": true }));
    }

    #[tokio::test]
    async fn a_value_that_holds_an_operator_does_not_change_what_is_compared() {
        let n = mk("{{a.response}} != OK");
        let mut scope = ExecutionScope::new();
        for reply in ["status == red", "if a == b then c", "x != OK", "5 > 3", "OK"] {
            scope.outputs.insert("a".to_string(), json!({ "response": reply }));
            let out = run(&n, &scope).await.unwrap();
            assert_eq!(out, json!({ "result": reply != "OK" }), "{reply:?}");
        }
    }

    // ── the expression language ────────────────────────────────────────────

    async fn ask(expr: &str, scope: &ExecutionScope) -> Result<bool, String> {
        run(&mk(expr), scope).await.map(|out| out["result"].as_bool().unwrap())
    }

    fn scope_with(outputs: Value) -> ExecutionScope {
        let mut scope = ExecutionScope::new();
        for (id, out) in outputs.as_object().unwrap() {
            scope.outputs.insert(id.clone(), out.clone());
        }
        scope
    }

    #[tokio::test]
    async fn combines_references_with_and_or_not_and_grouping() {
        let scope = scope_with(json!({ "a": { "output": { "confidence": "high", "area": "srv" }, "cost_usd": 0.4 } }));
        assert_eq!(ask("{{a.output.confidence}} != 'low' && {{a.output.area}} == 'srv'", &scope).await, Ok(true));
        assert_eq!(ask("!({{a.output.confidence}} == 'high')", &scope).await, Ok(false));
        assert_eq!(ask("({{a.cost_usd}} > 1 || {{a.output.area}} == 'srv') && !({{a.output.area}} == 'ui')", &scope).await, Ok(true));
        assert_eq!(ask("{{a.cost_usd}} < 1 && ({{a.output.confidence}} == 'low' || {{a.output.area}} == 'ui')", &scope).await, Ok(false));
    }

    #[tokio::test]
    async fn contains_matches_in_set_and_exists_read_typed_values() {
        let scope = scope_with(json!({
            "api": { "body": { "labels": ["bug", "srv"], "title": "Crash on start" }, "log": "step 3 failed: disk full" },
        }));
        assert_eq!(ask("{{api.body.labels}} contains 'bug'", &scope).await, Ok(true));
        assert_eq!(ask("{{api.body.title}} contains 'Crash'", &scope).await, Ok(true));
        assert_eq!(ask("{{api.log}} matches 'step [0-9]+ failed'", &scope).await, Ok(true));
        assert_eq!(ask("{{api.body.labels}} contains 'docs'", &scope).await, Ok(false));
        assert_eq!(ask("exists({{api.body.title}}) && !exists({{api.body.owner}})", &scope).await, Ok(true));
        assert_eq!(ask("{{api.body.title}} in_set ['Crash on start', 'Slow']", &scope).await, Ok(true));
        assert_eq!(ask("exists({{nobody.says}})", &scope).await, Ok(false), "a reference to nothing is nothing");
    }

    #[tokio::test]
    async fn what_a_reference_holds_is_never_read_as_syntax() {
        let mut scope = ExecutionScope::new();
        let hostile = ["x' || true || '", "a && b", "true) || (true", "1 == 1", "x\\' contains 'y"];
        for reply in hostile {
            scope.outputs.insert("a".to_string(), json!({ "response": reply }));
            assert_eq!(ask("{{a.response}} == 'ok' && true", &scope).await, Ok(false), "{reply:?}");
            assert_eq!(ask("!({{a.response}} == 'ok') && {{a.response}} contains ' '", &scope).await.is_ok(), true, "{reply:?}");
        }
        scope.outputs.insert("a".to_string(), json!({ "response": "ok" }));
        assert_eq!(ask("{{a.response}} == 'ok' && true", &scope).await, Ok(true));
    }

    #[tokio::test]
    async fn a_reference_may_carry_filters_in_both_forms() {
        let scope = scope_with(json!({ "a": { "log": "all fine\nERROR disk full", "body": r#"{"n":3,"s":"ABC"}"# } }));
        assert_eq!(ask("{{a.log | tail:1}} contains 'ERROR'", &scope).await, Ok(true));
        assert_eq!(ask("{{a.log | head:1}} contains 'ERROR'", &scope).await, Ok(false));
        assert_eq!(ask("{{a.body | json:n}} > 2 && {{a.body | json:s | lower}} == 'abc'", &scope).await, Ok(true));
        assert_eq!(ask("exists({{a.body | json:gone}}) || {{a.missing | default:none}} == 'none'", &scope).await, Ok(true));
        assert_eq!(ask("{{a.log | trunc:3}} == all", &scope).await, Ok(true), "the simple form reads filters too");
    }

    #[tokio::test]
    async fn the_simple_form_keeps_its_reading_with_bare_words() {
        let mut scope = ExecutionScope::new();
        scope.outputs.insert("a".to_string(), json!({ "response": "OK!" }));
        assert_eq!(ask("{{a.response}} == OK!", &scope).await, Ok(true), "a bang after text is no negation");
        assert_eq!(ask("{{a.response}} != OK", &scope).await, Ok(true));
        assert_eq!(ask("3 != 4", &scope).await, Ok(true));
    }

    #[tokio::test]
    async fn a_reference_inside_quotes_is_refused_in_the_new_form_and_a_bad_expression_says_why() {
        let scope = scope_with(json!({ "a": { "response": "x" } }));
        let err = ask("'{{a.response}}' contains 'x'", &scope).await.unwrap_err();
        assert!(err.contains("cannot sit inside quotes"), "{err}");
        let err = ask("{{a.response} contains 'x'", &scope).await.unwrap_err();
        assert!(err.contains("no `}}`"), "{err}");
        let err = ask("{{a.response}} contains", &scope).await.unwrap_err();
        assert!(err.starts_with("condition: "), "{err}");
        let err = ask("{{a.response}} && true", &scope).await.unwrap_err();
        assert!(err.contains("true or false is expected"), "{err}");
    }
}

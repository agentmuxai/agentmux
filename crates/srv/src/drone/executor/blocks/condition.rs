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
//! Phase 2 adds `&&` / `||` / `!` and grouping; Phase 3 may swap to a
//! real expression parser (`evalexpr`, etc).

use serde_json::{json, Value};

use crate::drone::data_flow::ExecutionScope;
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
    let result = match split_binop(raw) {
        Some((lhs, op, rhs)) => compare(&scope.resolve(lhs), op, &scope.resolve(rhs)),
        None => bare_boolean(&scope.resolve(raw))?,
    };
    Ok(json!({ "result": result }))
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
}

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

use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let expr = node
        .data
        .get("expr")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Condition block missing `expr`".to_string())?;
    let resolved = scope.resolve(expr);
    let result = eval_simple(&resolved)?;
    Ok(json!({ "result": result }))
}

fn eval_simple(expr: &str) -> Result<bool, String> {
    let trimmed = expr.trim();
    // Bare boolean literal.
    if trimmed.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Ok(false);
    }
    let (lhs, op, rhs) = split_binop(trimmed)
        .ok_or_else(|| format!("could not parse condition: `{trimmed}`"))?;
    let lhs = lhs.trim().trim_matches(|c| c == '\'' || c == '"');
    let rhs = rhs.trim().trim_matches(|c| c == '\'' || c == '"');

    if let (Ok(a), Ok(b)) = (lhs.parse::<f64>(), rhs.parse::<f64>()) {
        return Ok(match op {
            "==" => (a - b).abs() < f64::EPSILON,
            "!=" => (a - b).abs() >= f64::EPSILON,
            "<" => a < b,
            "<=" => a <= b,
            ">" => a > b,
            ">=" => a >= b,
            _ => unreachable!(),
        });
    }

    Ok(match op {
        "==" => lhs == rhs,
        "!=" => lhs != rhs,
        "<" => lhs < rhs,
        "<=" => lhs <= rhs,
        ">" => lhs > rhs,
        ">=" => lhs >= rhs,
        _ => unreachable!(),
    })
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
    use crate::workflows::types::NodePosition;

    fn mk(expr: &str) -> FlowNode {
        FlowNode {
            id: "c".to_string(),
            position: NodePosition::default(),
            data: json!({ "kind": "condition", "expr": expr }),
            node_type: String::new(),
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
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Join block: where branches that split come together
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.7).
//!
//! Block config (`node.data`):
//!   * `mode`             `all` (default), `any` or `n_of_m`.
//!   * `n`                with `n_of_m`, how many of the incoming branches must arrive.
//!   * `on_branch_error`  `fail` (default): a branch that failed fails the run, as any
//!                        block that failed does. `skip`: a branch that failed is left out,
//!                        and the mode says whether the join still holds.
//!
//! A branch *arrives* when the edge into the join is taken. One that was not taken
//! because its source did not run (the other side of a Condition) is *pruned*: it does
//! not count for or against the join. One whose source failed is *failed*.
//!
//!   * `all`     holds when no branch failed: every branch that ran arrived.
//!   * `any`     holds when at least one arrived.
//!   * `n_of_m`  holds when at least `n` arrived.
//!
//! The join runs once its branches are decided, so `any` does not stop the slower
//! branches: they have finished by then.

use serde_json::{json, Map, Value};

use crate::backend::storage::drone_types::FlowNode;

/// When a join holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Any,
    NOfM(usize),
}

/// What the block is set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinSpec {
    pub mode: Mode,
    /// A branch that failed is left out instead of failing the run.
    pub skip_failed: bool,
}

/// The block's settings, or what is wrong with them.
pub fn parse(node: &FlowNode) -> Result<JoinSpec, String> {
    let id = &node.id;
    let text = |key: &str| node.data.get(key).and_then(Value::as_str).map(str::trim);
    let mode = match text("mode").unwrap_or("all") {
        "all" => Mode::All,
        "any" => Mode::Any,
        "n_of_m" => {
            let n = node.data.get("n").and_then(Value::as_u64).filter(|n| *n >= 1).ok_or_else(|| format!("{id}: a Join with `n_of_m` needs `n`, how many branches must arrive (1 or more)"))?;
            Mode::NOfM(usize::try_from(n).unwrap_or(usize::MAX))
        }
        other => return Err(format!("{id}: `mode` must be all, any or n_of_m, not {other:?}")),
    };
    if !matches!(mode, Mode::NOfM(_)) && node.data.get("n").is_some_and(|n| !n.is_null()) {
        return Err(format!("{id}: `n` goes with `n_of_m`"));
    }
    let skip_failed = match text("on_branch_error").unwrap_or("fail") {
        "fail" => false,
        "skip" => true,
        other => return Err(format!("{id}: `on_branch_error` must be fail or skip, not {other:?}")),
    };
    Ok(JoinSpec { mode, skip_failed })
}

/// The settings, and that the join can hold with the branches it has.
pub fn check(node: &FlowNode, branches: usize) -> Result<(), String> {
    let spec = parse(node)?;
    if branches < 2 {
        return Err(format!("{}: a Join brings branches together and has {branches} edge{} into it: it needs at least two", node.id, if branches == 1 { "" } else { "s" }));
    }
    if let Mode::NOfM(n) = spec.mode {
        if n > branches {
            return Err(format!("{}: `n` is {n}, but only {branches} branches lead to this Join", node.id));
        }
    }
    Ok(())
}

/// How the branches into a join ended, each by the block it came from.
#[derive(Debug, Default)]
pub struct Branches {
    pub arrived: Vec<(String, Value)>,
    /// A branch whose source failed, with what it said.
    pub failed: Vec<(String, String)>,
    pub pruned: Vec<String>,
}

/// The join's output, or why it does not hold.
pub fn evaluate(id: &str, spec: JoinSpec, branches: Branches) -> Result<Value, String> {
    let Branches { arrived, failed, pruned } = branches;
    let failures = || failed.iter().map(|(b, why)| format!("{b}: {why}")).collect::<Vec<_>>().join("; ");
    if !failed.is_empty() && !spec.skip_failed {
        return Err(format!("{id}: a branch failed ({})", failures()));
    }
    match spec.mode {
        Mode::All if !failed.is_empty() => return Err(format!("{id}: every branch must arrive, and a branch failed ({})", failures())),
        Mode::Any if arrived.is_empty() => return Err(format!("{id}: no branch arrived{}", if failed.is_empty() { String::new() } else { format!(" ({})", failures()) })),
        Mode::NOfM(n) if arrived.len() < n => {
            return Err(format!("{id}: {} of {n} branches arrived{}", arrived.len(), if failed.is_empty() { String::new() } else { format!(" ({})", failures()) }));
        }
        _ => {}
    }
    let outputs: Map<String, Value> = arrived.iter().cloned().collect();
    let errors: Map<String, Value> = failed.iter().map(|(b, why)| (b.clone(), json!(why))).collect();
    Ok(json!({
        "arrived": arrived.len(),
        "outputs": outputs,
        "failed": failed.iter().map(|(b, _)| b.as_str()).collect::<Vec<_>>(),
        "errors": errors,
        "pruned": pruned,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;

    fn join(data: Value) -> FlowNode {
        let mut full = json!({ "kind": "join" });
        full.as_object_mut().unwrap().extend(data.as_object().cloned().unwrap_or_default());
        FlowNode { id: "j".to_string(), position: NodePosition::default(), data: full, node_type: None }
    }

    fn spec(mode: Mode, skip_failed: bool) -> JoinSpec {
        JoinSpec { mode, skip_failed }
    }

    fn branches(arrived: &[&str], failed: &[&str], pruned: &[&str]) -> Branches {
        Branches {
            arrived: arrived.iter().map(|b| (b.to_string(), json!({ "value": b }))).collect(),
            failed: failed.iter().map(|b| (b.to_string(), "boom".to_string())).collect(),
            pruned: pruned.iter().map(|b| b.to_string()).collect(),
        }
    }

    #[test]
    fn a_join_is_all_of_its_branches_and_fails_on_a_failed_one_unless_told_otherwise() {
        assert_eq!(parse(&join(json!({}))).unwrap(), spec(Mode::All, false));
        assert_eq!(parse(&join(json!({ "mode": "any", "on_branch_error": "skip" }))).unwrap(), spec(Mode::Any, true));
        assert_eq!(parse(&join(json!({ "mode": "n_of_m", "n": 2 }))).unwrap(), spec(Mode::NOfM(2), false));
    }

    #[test]
    fn a_join_that_is_set_wrongly_says_what_to_fix() {
        assert!(parse(&join(json!({ "mode": "first" }))).unwrap_err().contains("`mode` must be"));
        assert!(parse(&join(json!({ "mode": "n_of_m" }))).unwrap_err().contains("needs `n`"));
        assert!(parse(&join(json!({ "mode": "n_of_m", "n": 0 }))).unwrap_err().contains("needs `n`"));
        assert!(parse(&join(json!({ "mode": "any", "n": 2 }))).unwrap_err().contains("goes with `n_of_m`"));
        assert!(parse(&join(json!({ "on_branch_error": "ignore" }))).unwrap_err().contains("`on_branch_error` must be"));
        assert!(check(&join(json!({})), 1).unwrap_err().contains("at least two"));
        assert!(check(&join(json!({ "mode": "n_of_m", "n": 3 })), 2).unwrap_err().contains("only 2 branches"));
        assert!(check(&join(json!({ "mode": "n_of_m", "n": 2 })), 3).is_ok());
    }

    #[test]
    fn all_holds_when_no_branch_failed_and_a_pruned_one_does_not_count() {
        let out = evaluate("j", spec(Mode::All, false), branches(&["a", "b"], &[], &["c"])).unwrap();
        assert_eq!(out["arrived"], json!(2));
        assert_eq!(out["outputs"]["a"], json!({ "value": "a" }));
        assert_eq!(out["pruned"], json!(["c"]));
        assert!(evaluate("j", spec(Mode::All, true), branches(&["a"], &["b"], &[])).unwrap_err().contains("every branch must arrive"));
    }

    #[test]
    fn a_failed_branch_fails_the_join_unless_it_is_skipped() {
        assert!(evaluate("j", spec(Mode::Any, false), branches(&["a"], &["b"], &[])).unwrap_err().contains("a branch failed (b: boom)"));
        let out = evaluate("j", spec(Mode::Any, true), branches(&["a"], &["b"], &[])).unwrap();
        assert_eq!(out["failed"], json!(["b"]));
        assert_eq!(out["errors"]["b"], json!("boom"));
    }

    #[test]
    fn any_needs_one_and_n_of_m_needs_n() {
        assert!(evaluate("j", spec(Mode::Any, true), branches(&[], &["a", "b"], &[])).unwrap_err().contains("no branch arrived"));
        assert!(evaluate("j", spec(Mode::NOfM(2), true), branches(&["a"], &["b"], &["c"])).unwrap_err().contains("1 of 2 branches arrived"));
        assert!(evaluate("j", spec(Mode::NOfM(2), true), branches(&["a", "c"], &["b"], &[])).is_ok());
    }
}

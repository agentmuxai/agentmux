// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The rule for blocks that can run at the same time and write the same
//! variable (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.2):
//! **two blocks that can run at the same time may both write a variable only if
//! its reducer is not `replace`**, so there is no silent race. The graph is
//! refused at start, naming both blocks and the variable.
//!
//! Two blocks *can* run at the same time when neither leads to the other and
//! they are not the two sides of one Condition (a block only the `true` branch
//! reaches, and one only the `false` branch reaches, never both run). The
//! scheduler runs one block at a time today, so nothing races yet; this keeps a
//! graph that would race once branches run in parallel from being written.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use super::schedule::{foreach_config, Issue, Plan};
use crate::drone::types::{FlowEdge, FlowNode};

fn is_kind(node: &FlowNode, kind: &str) -> bool {
    node.data.get("kind").and_then(Value::as_str) == Some(kind)
}

/// The variables a Variables block writes, each with the reducer it declares.
fn writes_of(node: &FlowNode) -> Vec<(String, Option<String>)> {
    if !is_kind(node, "variables") {
        return Vec::new();
    }
    let entries = node.data.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries
        .iter()
        .filter_map(|e| {
            let name = e.get("name").and_then(Value::as_str)?.to_string();
            Some((name, e.get("reducer").and_then(Value::as_str).map(str::to_string)))
        })
        .collect()
}

/// For each Condition, which side of it a block is on: `Some(true)` when only
/// the `true` branch leads to it, `Some(false)` for only `false`, else `None`.
/// A block reachable from the start without passing the Condition is on neither.
fn sides(n: usize, out: &[Vec<(usize, Option<&str>)>], roots: &[usize], conditions: &[usize]) -> BTreeMap<usize, Vec<Option<bool>>> {
    let walk = |starts: Vec<usize>, skip: Option<usize>| -> Vec<bool> {
        let mut seen = vec![false; n];
        let mut queue: VecDeque<usize> = starts.into_iter().filter(|s| Some(*s) != skip).collect();
        while let Some(u) = queue.pop_front() {
            if seen[u] {
                continue;
            }
            seen[u] = true;
            queue.extend(out[u].iter().map(|(v, _)| *v).filter(|v| Some(*v) != skip && !seen[*v]));
        }
        seen
    };
    let mut result = BTreeMap::new();
    for &c in conditions {
        let target = |handle: &str| out[c].iter().filter(|(_, h)| *h == Some(handle)).map(|(v, _)| *v).collect::<Vec<_>>();
        let (on_true, on_false) = (walk(target("true"), Some(c)), walk(target("false"), Some(c)));
        let without_c = walk(roots.to_vec(), Some(c));
        result.insert(
            c,
            (0..n)
                .map(|x| match (on_true[x], on_false[x], without_c[x]) {
                    (true, false, false) => Some(true),
                    (false, true, false) => Some(false),
                    _ => None,
                })
                .collect(),
        );
    }
    result
}

pub(super) fn check(sorted: &[&FlowNode], edges: &[(usize, usize, &FlowEdge)], reach: &[Vec<bool>], plan: &Plan, issues: &mut Vec<Issue>) {
    let n = sorted.len();
    // A for-each body writes when its for-each does.
    let owner_of = |idx: usize| -> usize {
        sorted
            .iter()
            .position(|f| is_kind(f, "foreach") && foreach_config(f).is_ok_and(|c| c.body == sorted[idx].id) && plan.owned.contains(&sorted[idx].id))
            .unwrap_or(idx)
    };
    let mut writers: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    for (idx, node) in sorted.iter().enumerate() {
        for (name, reducer) in writes_of(node) {
            if let Some(r) = reducer {
                declared.insert(name.clone(), r);
            }
            writers.entry(name).or_default().insert(owner_of(idx));
        }
    }
    let mut out: Vec<Vec<(usize, Option<&str>)>> = vec![Vec::new(); n];
    let mut has_incoming = vec![false; n];
    for &(s, t, e) in edges {
        out[s].push((t, e.source_handle.as_deref()));
        has_incoming[t] = true;
    }
    let roots: Vec<usize> = (0..n).filter(|i| !has_incoming[*i]).collect();
    let conditions: Vec<usize> = (0..n).filter(|i| is_kind(sorted[*i], "condition")).collect();
    let side = sides(n, &out, &roots, &conditions);
    let exclusive = |a: usize, b: usize| side.values().any(|s| matches!((s[a], s[b]), (Some(x), Some(y)) if x != y));

    for (name, who) in &writers {
        if declared.get(name).is_some_and(|r| r != "replace") {
            continue;
        }
        let who: Vec<usize> = who.iter().copied().collect();
        for (i, &a) in who.iter().enumerate() {
            for &b in &who[i + 1..] {
                if !reach[a][b] && !reach[b][a] && !exclusive(a, b) {
                    issues.push(Issue {
                        node: Some(sorted[a].id.clone()),
                        message: format!(
                            "{} and {} can run at the same time and both write {name:?}, which replaces: give {name:?} a reducer (append, merge or add) or write it from one block",
                            sorted[a].id, sorted[b].id
                        ),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::super::schedule::analyze;
    use crate::drone::types::{DroneGraph, FlowEdge, FlowNode, NodePosition};

    fn node(id: &str, data: Value) -> FlowNode {
        FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
    }

    fn writer(id: &str, var: &str, reducer: Option<&str>) -> FlowNode {
        let mut entry = json!({ "name": var, "value": id });
        if let Some(r) = reducer {
            entry["reducer"] = json!(r);
        }
        node(id, json!({ "kind": "variables", "entries": [entry] }))
    }

    fn edge(id: &str, source: &str, target: &str, handle: Option<&str>) -> FlowEdge {
        FlowEdge {
            id: id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            source_handle: handle.map(str::to_string),
            target_handle: None,
        }
    }

    /// start fans out to a and b, which both write `x`.
    fn fan_out(a_reducer: Option<&str>, b_reducer: Option<&str>) -> DroneGraph {
        DroneGraph {
            nodes: vec![writer("start", "s", None), writer("a", "x", a_reducer), writer("b", "x", b_reducer)],
            edges: vec![edge("e1", "start", "a", None), edge("e2", "start", "b", None)],
        }
    }

    #[test]
    fn two_branches_that_replace_the_same_variable_are_refused_and_named() {
        let issues = analyze(&fan_out(None, None)).unwrap_err();
        assert!(issues.iter().any(|i| i.message.contains("a and b can run at the same time") && i.message.contains("\"x\"")), "{issues:?}");
    }

    #[test]
    fn the_same_two_branches_are_fine_when_the_variable_has_a_reducer() {
        assert!(analyze(&fan_out(Some("append"), None)).is_ok(), "one declaration is the variable's reducer");
        assert!(analyze(&fan_out(Some("add"), Some("add"))).is_ok());
        assert!(analyze(&fan_out(Some("replace"), None)).is_err(), "replace is no reducer");
    }

    #[test]
    fn blocks_in_a_row_and_the_two_sides_of_a_condition_may_share_a_variable() {
        let row = DroneGraph {
            nodes: vec![writer("a", "x", None), writer("b", "x", None)],
            edges: vec![edge("e1", "a", "b", None)],
        };
        assert!(analyze(&row).is_ok());
        let sides = DroneGraph {
            nodes: vec![
                writer("start", "s", None),
                node("c", json!({ "kind": "condition", "expr": "1 < 2" })),
                writer("yes", "x", None),
                writer("no", "x", None),
            ],
            edges: vec![edge("e1", "start", "c", None), edge("e2", "c", "yes", Some("true")), edge("e3", "c", "no", Some("false"))],
        };
        assert!(analyze(&sides).is_ok(), "only one side runs");
    }

    #[test]
    fn a_branch_that_skips_the_condition_can_still_race_with_a_side() {
        // `yes` also hangs off start directly, so it can run beside `no`... which needs c's false branch.
        let g = DroneGraph {
            nodes: vec![
                writer("start", "s", None),
                node("c", json!({ "kind": "condition", "expr": "1 < 2" })),
                writer("yes", "x", None),
                writer("no", "x", None),
            ],
            edges: vec![
                edge("e1", "start", "c", None),
                edge("e2", "c", "yes", Some("true")),
                edge("e3", "c", "no", Some("false")),
                edge("e4", "start", "yes", None),
            ],
        };
        assert!(analyze(&g).is_err());
    }

    #[test]
    fn a_for_each_body_counts_as_its_for_each() {
        // The body writes `x` once per item; a sibling branch writes it too.
        let g = DroneGraph {
            nodes: vec![
                writer("start", "s", None),
                node("each", json!({ "kind": "foreach", "items": "{{trigger.l}}", "body": "work" })),
                writer("work", "x", None),
                writer("other", "x", None),
            ],
            edges: vec![edge("e1", "start", "each", None), edge("e2", "start", "other", None)],
        };
        let issues = analyze(&g).unwrap_err();
        assert!(issues.iter().any(|i| i.message.contains("each and other")), "{issues:?}");
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the step scheduler needs to know about a graph, and the rules a graph
//! with a loop must meet (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md
//! §3.1). Pure: nothing here runs a block.
//!
//! A graph may hold a cycle, but **every cycle must pass through a Condition**,
//! which is the loop's guard. A guard has `max_iterations` (1 to 1,000, 25 when
//! absent), `on_cap` (`end_run`, the default, or `exit_loop`), and optionally
//! `max_cost_usd` and `max_minutes`. Each time a guard sends the run along an
//! edge that stays inside its cycle that is one more iteration; the scheduler
//! stops the loop at any cap.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use crate::drone::types::{DroneGraph, FlowEdge, FlowNode};

pub const DEFAULT_MAX_ITERATIONS: u32 = 25;
pub const MAX_ITERATIONS_CEILING: u32 = 1_000;

/// What happens when a loop reaches its `max_iterations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnCap {
    /// The run fails, and says why (the default).
    EndRun,
    /// The run leaves the loop by the guard's other edges, with
    /// `{{<guard>.capped}}` set.
    ExitLoop,
}

/// A Condition that sits on a cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct Guard {
    pub max_iterations: u32,
    pub on_cap: OnCap,
    /// What the loop's blocks may spend in all, in dollars; none is no limit.
    pub max_cost_usd: Option<f64>,
    /// How long the loop may run, in minutes; none is no limit.
    pub max_minutes: Option<f64>,
    /// The ids of the edges out of the guard that stay inside its cycle.
    pub loop_edges: BTreeSet<String>,
}

/// What the scheduler reads of a graph that passed [`analyze`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// The edges that close a cycle. The scheduler does not wait on them for
    /// their target to be ready, so the others form an acyclic graph.
    pub back_edges: BTreeSet<String>,
    /// Each Condition on a cycle, by node id.
    pub guards: BTreeMap<String, Guard>,
    /// For each node on a cycle, every node of its cycle: the ones that run
    /// again when a loop goes round.
    pub cycle_of: BTreeMap<String, BTreeSet<String>>,
}

/// A reason a graph cannot run, for the author: which block, and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub node: Option<String>,
    pub message: String,
}

impl Issue {
    fn at(node: &str, message: impl Into<String>) -> Self {
        Self { node: Some(node.to_string()), message: message.into() }
    }
}

fn kind_of(node: &FlowNode) -> &str {
    node.data.get("kind").and_then(Value::as_str).unwrap_or("")
}

/// Every pair `(u, v)` with a path of one edge or more from `u` to `v`,
/// following only edges between nodes `keep` allows.
fn closure(out: &[Vec<usize>], keep: &[bool]) -> Vec<Vec<bool>> {
    let n = out.len();
    let mut reach = vec![vec![false; n]; n];
    for start in (0..n).filter(|&u| keep[u]) {
        let mut stack: Vec<usize> = out[start].iter().copied().filter(|&v| keep[v]).collect();
        while let Some(v) = stack.pop() {
            if reach[start][v] {
                continue;
            }
            reach[start][v] = true;
            stack.extend(out[v].iter().copied().filter(|&w| keep[w] && !reach[start][w]));
        }
    }
    reach
}

/// `u` and everything on a cycle with it.
fn cycle_group(reach: &[Vec<bool>], u: usize) -> BTreeSet<usize> {
    (0..reach.len()).filter(|&v| v == u || (reach[u][v] && reach[v][u])).collect()
}

fn names(ids: &[&str], group: &BTreeSet<usize>) -> Vec<String> {
    group.iter().map(|&i| ids[i].to_string()).collect()
}

/// Read a guard's settings. Errors are issues, so a bad value is named, not guessed.
fn read_guard(node: &FlowNode, loop_edges: BTreeSet<String>, issues: &mut Vec<Issue>) -> Guard {
    let mut max_iterations = DEFAULT_MAX_ITERATIONS;
    match node.data.get("max_iterations") {
        None | Some(Value::Null) => {}
        Some(v) => match v.as_u64().filter(|n| (1..=MAX_ITERATIONS_CEILING as u64).contains(n)) {
            Some(n) => max_iterations = n as u32,
            None => issues.push(Issue::at(
                &node.id,
                format!("max_iterations must be a whole number from 1 to {MAX_ITERATIONS_CEILING}"),
            )),
        },
    }
    let on_cap = match node.data.get("on_cap").and_then(Value::as_str) {
        None | Some("end_run") => OnCap::EndRun,
        Some("exit_loop") => OnCap::ExitLoop,
        Some(_) => {
            issues.push(Issue::at(&node.id, "on_cap must be end_run or exit_loop"));
            OnCap::EndRun
        }
    };
    let mut limit = |key: &str| match node.data.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_f64().filter(|n| *n > 0.0) {
            Some(n) => Some(n),
            None => {
                issues.push(Issue::at(&node.id, format!("{key} must be a number above 0")));
                None
            }
        },
    };
    let max_cost_usd = limit("max_cost_usd");
    let max_minutes = limit("max_minutes");
    Guard { max_iterations, on_cap, max_cost_usd, max_minutes, loop_edges }
}

/// The edges that close a cycle, found by a walk from the blocks nothing leads
/// to, and the blocks that walk never reached.
fn walk_back_edges(ids: &[&str], edges: &[(usize, usize, &FlowEdge)]) -> (BTreeSet<String>, Vec<usize>) {
    let n = ids.len();
    let mut next: Vec<Vec<(usize, &FlowEdge)>> = vec![Vec::new(); n];
    let mut has_incoming = vec![false; n];
    for &(s, t, e) in edges {
        next[s].push((t, e));
        has_incoming[t] = true;
    }
    for list in &mut next {
        list.sort_by(|a, b| (ids[a.0], &a.1.id).cmp(&(ids[b.0], &b.1.id)));
    }
    // 0 not seen, 1 on the walk's path, 2 finished.
    let mut colour = vec![0u8; n];
    let mut back = BTreeSet::new();
    for root in (0..n).filter(|&i| !has_incoming[i]) {
        colour[root] = 1;
        let mut path: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(top) = path.len().checked_sub(1) {
            let (u, at) = path[top];
            let Some(&(v, edge)) = next[u].get(at) else {
                colour[u] = 2;
                path.pop();
                continue;
            };
            path[top].1 += 1;
            match colour[v] {
                1 => {
                    back.insert(edge.id.clone());
                }
                0 => {
                    colour[v] = 1;
                    path.push((v, 0));
                }
                _ => {}
            }
        }
    }
    let unreached = (0..n).filter(|&i| colour[i] == 0).collect();
    (back, unreached)
}

/// Check the graph's loops and say how the scheduler should run it. A graph
/// with no cycle gets an empty plan.
pub fn analyze(graph: &DroneGraph) -> Result<Plan, Vec<Issue>> {
    let mut sorted: Vec<&FlowNode> = graph.nodes.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    let mut issues = Vec::new();
    let mut edges: Vec<(usize, usize, &FlowEdge)> = Vec::new();
    for e in &graph.edges {
        match (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            (Some(&s), Some(&t)) => edges.push((s, t, e)),
            _ => issues.push(Issue { node: None, message: format!("edge references unknown node ({} → {})", e.source, e.target) }),
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    let n = ids.len();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(s, t, _) in &edges {
        out[s].push(t);
    }
    let all = closure(&out, &vec![true; n]);
    let is_condition: Vec<bool> = sorted.iter().map(|node| kind_of(node) == "condition").collect();

    // A cycle through no Condition has nothing to stop it.
    let no_guard = closure(&out, &is_condition.iter().map(|c| !c).collect::<Vec<_>>());
    let mut reported = BTreeSet::new();
    for u in (0..n).filter(|&u| no_guard[u][u]) {
        let group = cycle_group(&no_guard, u);
        if reported.insert(group.clone()) {
            issues.push(Issue {
                node: Some(ids[u].to_string()),
                message: format!(
                    "these blocks loop with no Condition among them: {}. Put a Condition in the loop; its max_iterations (25 when not set) is what stops it",
                    names(&ids, &group).join(", ")
                ),
            });
        }
    }

    let mut plan = Plan::default();
    for u in (0..n).filter(|&u| all[u][u]) {
        let group = cycle_group(&all, u);
        plan.cycle_of.insert(ids[u].to_string(), group.iter().map(|&i| ids[i].to_string()).collect());
        if is_condition[u] {
            let loop_edges = edges.iter().filter(|(s, t, _)| *s == u && group.contains(t)).map(|(_, _, e)| e.id.clone()).collect();
            plan.guards.insert(ids[u].to_string(), read_guard(sorted[u], loop_edges, &mut issues));
        }
    }

    let (back_edges, unreached) = walk_back_edges(&ids, &edges);
    plan.back_edges = back_edges;
    if !unreached.is_empty() {
        let list: Vec<&str> = unreached.iter().map(|&i| ids[i]).collect();
        issues.push(Issue {
            node: Some(list[0].to_string()),
            message: format!("{} can only be reached from each other: add a block that leads into the loop", list.join(", ")),
        });
    }
    if issues.is_empty() {
        Ok(plan)
    } else {
        Err(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;
    use serde_json::json;

    fn node(id: &str, kind: &str, extra: Value) -> FlowNode {
        let mut data = json!({ "kind": kind });
        if let (Some(d), Some(x)) = (data.as_object_mut(), extra.as_object()) {
            d.extend(x.clone());
        }
        FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
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

    /// start → check → guard; guard "true" → done; guard "false" → fix → check.
    fn retry_loop(guard: Value) -> DroneGraph {
        DroneGraph {
            nodes: vec![
                node("start", "variables", json!({})),
                node("check", "agent", json!({})),
                node("guard", "condition", guard),
                node("fix", "agent", json!({})),
                node("done", "response", json!({})),
            ],
            edges: vec![
                edge("e0", "start", "check", None),
                edge("e1", "check", "guard", None),
                edge("e2", "guard", "done", Some("true")),
                edge("e3", "guard", "fix", Some("false")),
                edge("e4", "fix", "check", None),
            ],
        }
    }

    #[test]
    fn a_graph_with_no_cycle_has_an_empty_plan() {
        let g = DroneGraph {
            nodes: vec![node("a", "variables", json!({})), node("b", "api", json!({})), node("c", "condition", json!({}))],
            edges: vec![edge("e1", "a", "b", None), edge("e2", "b", "c", None)],
        };
        assert_eq!(analyze(&g), Ok(Plan::default()));
    }

    #[test]
    fn a_cycle_through_a_condition_is_a_guarded_loop() {
        let plan = analyze(&retry_loop(json!({ "expr": "{{check.response}} == OK" }))).unwrap();
        assert_eq!(plan.back_edges, BTreeSet::from(["e4".to_string()]));
        let guard = &plan.guards["guard"];
        assert_eq!((guard.max_iterations, guard.on_cap), (25, OnCap::EndRun), "the defaults");
        assert_eq!(guard.loop_edges, BTreeSet::from(["e3".to_string()]), "only the edge that stays in the loop counts");
        let cycle: BTreeSet<String> = ["check", "fix", "guard"].iter().map(|s| s.to_string()).collect();
        assert_eq!(plan.cycle_of["fix"], cycle);
        assert!(!plan.cycle_of.contains_key("done"), "a block outside the loop is not in it");
    }

    #[test]
    fn the_guard_reads_its_own_limit_and_what_to_do_at_it() {
        let plan = analyze(&retry_loop(json!({ "max_iterations": 3, "on_cap": "exit_loop" }))).unwrap();
        assert_eq!((plan.guards["guard"].max_iterations, plan.guards["guard"].on_cap), (3, OnCap::ExitLoop));
    }

    #[test]
    fn a_loop_may_also_limit_its_cost_and_its_time() {
        let g = analyze(&retry_loop(json!({ "max_cost_usd": 0.5, "max_minutes": 10 }))).unwrap().guards["guard"].clone();
        assert_eq!((g.max_cost_usd, g.max_minutes), (Some(0.5), Some(10.0)));
        let none = analyze(&retry_loop(json!({}))).unwrap().guards["guard"].clone();
        assert_eq!((none.max_cost_usd, none.max_minutes), (None, None));
        for bad in [json!(0), json!(-1), json!("lots")] {
            let issues = analyze(&retry_loop(json!({ "max_cost_usd": bad }))).unwrap_err();
            assert!(issues[0].message.contains("max_cost_usd"), "{issues:?}");
        }
    }

    #[test]
    fn a_limit_outside_one_to_a_thousand_is_refused_and_named() {
        for bad in [json!(0), json!(1001), json!("many"), json!(2.5), json!(-1)] {
            let issues = analyze(&retry_loop(json!({ "max_iterations": bad }))).unwrap_err();
            assert!(issues.iter().any(|i| i.node.as_deref() == Some("guard") && i.message.contains("max_iterations")), "{bad}: {issues:?}");
        }
        assert!(analyze(&retry_loop(json!({ "max_iterations": 1000 }))).is_ok(), "the ceiling itself is allowed");
        let issues = analyze(&retry_loop(json!({ "on_cap": "forever" }))).unwrap_err();
        assert!(issues[0].message.contains("on_cap"), "{issues:?}");
    }

    #[test]
    fn a_cycle_with_no_condition_is_refused_and_says_which_blocks() {
        let g = DroneGraph {
            nodes: vec![node("start", "variables", json!({})), node("a", "api", json!({})), node("b", "agent", json!({}))],
            edges: vec![edge("e1", "start", "a", None), edge("e2", "a", "b", None), edge("e3", "b", "a", None)],
        };
        let issues = analyze(&g).unwrap_err();
        assert_eq!(issues.len(), 1, "one cycle, one issue: {issues:?}");
        assert!(issues[0].message.contains("a, b") && issues[0].message.contains("Condition"), "{issues:?}");
    }

    #[test]
    fn a_second_cycle_that_skips_the_guard_is_still_refused() {
        // The guarded loop check → guard → fix → check, plus fix ⇄ helper with no Condition.
        let mut g = retry_loop(json!({}));
        g.nodes.push(node("helper", "api", json!({})));
        g.edges.push(edge("e5", "fix", "helper", None));
        g.edges.push(edge("e6", "helper", "fix", None));
        let issues = analyze(&g).unwrap_err();
        assert!(issues.iter().any(|i| i.message.contains("fix, helper")), "{issues:?}");
    }

    #[test]
    fn a_block_that_leads_to_itself_needs_to_be_a_condition() {
        let own = |kind: &str| DroneGraph {
            nodes: vec![node("start", "variables", json!({})), node("x", kind, json!({}))],
            edges: vec![edge("e1", "start", "x", None), edge("e2", "x", "x", Some("false"))],
        };
        assert!(analyze(&own("api")).is_err());
        let plan = analyze(&own("condition")).unwrap();
        assert_eq!(plan.back_edges, BTreeSet::from(["e2".to_string()]));
        assert_eq!(plan.guards["x"].loop_edges, BTreeSet::from(["e2".to_string()]));
    }

    #[test]
    fn a_loop_nothing_leads_into_is_refused() {
        let g = DroneGraph {
            nodes: vec![node("a", "condition", json!({})), node("b", "api", json!({}))],
            edges: vec![edge("e1", "a", "b", Some("true")), edge("e2", "b", "a", None)],
        };
        let issues = analyze(&g).unwrap_err();
        assert!(issues[0].message.contains("a, b") && issues[0].message.contains("leads into"), "{issues:?}");
    }

    #[test]
    fn an_edge_to_a_block_that_is_not_there_is_refused() {
        let g = DroneGraph { nodes: vec![node("a", "api", json!({}))], edges: vec![edge("e1", "a", "ghost", None)] };
        assert!(analyze(&g).unwrap_err()[0].message.contains("unknown node"));
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Join block: where branches that split come together
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.7).
//!
//! The scheduler already readies a block once every edge into it is decided, so a
//! join runs when its branches have each arrived, failed or been pruned. It reads
//! how each ended from the edges and from what their source blocks said, and
//! `blocks::join` says whether that holds.

use serde_json::Value;

use super::Scheduler;
use crate::drone::executor::blocks::join::{self as config, Branches};
use crate::drone::types::{BlockKind, FlowNode};

impl<'a> Scheduler<'a> {
    pub(super) fn run_join(&self, node: &'a FlowNode) -> Result<Value, String> {
        let spec = config::parse(node)?;
        config::evaluate(&node.id, spec, self.branches_of(&node.id))
    }

    /// How each branch into the join `id` ended, by the block it came from.
    fn branches_of(&self, id: &str) -> Branches {
        let mut edges = self.incoming.get(id).cloned().unwrap_or_default();
        edges.sort_by(|a, b| a.source.cmp(&b.source));
        let mut branches = Branches::default();
        for edge in edges {
            let source = edge.source.clone();
            let said = self.scope.outputs.get(&source);
            if self.decided.get(edge.id.as_str()) == Some(&true) {
                branches.arrived.push((source, said.cloned().unwrap_or(Value::Null)));
            } else if said.and_then(|o| o.get("failed")).and_then(Value::as_bool) == Some(true) {
                let why = said.and_then(|o| o.get("error")).and_then(Value::as_str).unwrap_or("it failed").to_string();
                branches.failed.push((source, why));
            } else {
                branches.pruned.push(source);
            }
        }
        branches
    }

    /// What a join's output is made of: tainted when a branch that arrived was, and
    /// when one failed (what it said is outside text), derived when one was.
    pub(super) fn join_bits(&self, id: &str) -> (bool, bool) {
        let branches = self.branches_of(id);
        let bit = |(source, _): &(String, Value)| self.scope.bits.get(source).copied().unwrap_or((false, false));
        let bits: Vec<(bool, bool)> = branches.arrived.iter().map(bit).collect();
        let tainted = !branches.failed.is_empty() || bits.iter().any(|(t, _)| *t);
        (tainted, !tainted && bits.iter().any(|(_, d)| *d))
    }

    /// Whether a failure of block `id` is the graph's to handle: it has an error edge,
    /// or every way out of it is into a Join that leaves failed branches out.
    pub(super) fn handles_failure(&self, id: &str) -> bool {
        if self.has_error_edge(id) {
            return true;
        }
        let Some(edges) = self.outgoing.get(id).filter(|e| !e.is_empty()) else { return false };
        edges.iter().all(|e| {
            let join = self.node_of.get(e.target.as_str()).filter(|n| super::kind_of(n).is_ok_and(|k| k == BlockKind::Join));
            join.is_some_and(|n| config::parse(n).is_ok_and(|spec| spec.skip_failed))
        })
    }
}

#[cfg(test)]
#[path = "join_tests.rs"]
mod tests;

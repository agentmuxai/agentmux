// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Drone graph a template carries in `graph.json`
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md, "What is a template?").
//!
//! A graph is checked when the package is read, so a template that names a
//! node this build does not have is `invalid` in the list, with the node named,
//! and never reaches the operator's approval. [`summarize`] is what the
//! approval prompt shows of the graph itself: how many nodes, how many of
//! them start an agent, and every call that writes outside AgentMux.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::drone::types::{BlockKind, DroneGraph};

/// The most nodes and edges a template graph holds.
pub const MAX_NODES: usize = 100;
pub const MAX_EDGES: usize = 300;

/// The node kinds this build has, for the error that names a missing one.
const KINDS: &str = "agent, condition, api, response, variables, message, task, notify, app_query";

/// What the approval prompt shows of a template's graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DroneSummary {
    pub nodes: u32,
    /// Nodes that start an agent, each of which spends money.
    pub agents: u32,
    /// Every call that writes outside AgentMux, as `node id: METHOD url`.
    pub writes_outside: Vec<String>,
}

/// `graph.json` read and checked.
pub fn parse(raw: &[u8]) -> Result<DroneGraph, String> {
    let graph: DroneGraph = serde_json::from_slice(raw).map_err(|e| format!("graph.json: {e}"))?;
    check(&graph)?;
    Ok(graph)
}

/// The kind a node names in `data.kind`.
fn kind_of(node: &crate::drone::types::FlowNode) -> Result<BlockKind, String> {
    let name = node.data.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    BlockKind::parse(name).ok_or_else(|| format!("graph.json: node {:?} is a {name:?} node, and this AgentMux has only {KINDS}", node.id))
}

/// A graph this build can hold: within its limits, every node of a kind it
/// has, every edge between nodes that exist.
pub fn check(graph: &DroneGraph) -> Result<(), String> {
    if graph.nodes.is_empty() {
        return Err("graph.json has no nodes".to_string());
    }
    if graph.nodes.len() > MAX_NODES || graph.edges.len() > MAX_EDGES {
        return Err(format!("graph.json is at most {MAX_NODES} nodes and {MAX_EDGES} edges"));
    }
    let mut ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() || !ids.insert(node.id.as_str()) {
            return Err(format!("graph.json: node id {:?} is empty or used twice", node.id));
        }
        kind_of(node)?;
    }
    for edge in &graph.edges {
        for end in [&edge.source, &edge.target] {
            if !ids.contains(end.as_str()) {
                return Err(format!("graph.json: edge {:?} names node {end:?}, which isn't in the graph", edge.id));
            }
        }
    }
    Ok(())
}

/// What the prompt shows of a checked graph. An `api` node that is not a
/// plain read writes outside; the other nodes this build has stay inside
/// AgentMux (a message goes to an agent on this computer).
pub fn summarize(graph: &DroneGraph) -> DroneSummary {
    let mut out = DroneSummary { nodes: graph.nodes.len() as u32, ..DroneSummary::default() };
    for node in &graph.nodes {
        let text = |key: &str| node.data.get(key).and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
        match kind_of(node) {
            Ok(BlockKind::Agent) => out.agents += 1,
            Ok(BlockKind::Api) => {
                let method = text("method").to_uppercase();
                let method = if method.is_empty() { "GET".to_string() } else { method };
                if method != "GET" && method != "HEAD" {
                    out.writes_outside.push(format!("{}: {method} {}", node.id, text("url")));
                }
            }
            _ => {}
        }
    }
    out
}

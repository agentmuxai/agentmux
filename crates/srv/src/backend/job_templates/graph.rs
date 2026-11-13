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
use serde_json::{Map, Value};

use super::manifest::{self, JobTemplateManifest};
use crate::drone::types::{BlockKind, DroneGraph};

/// The most nodes and edges a template graph holds.
pub const MAX_NODES: usize = 100;
pub const MAX_EDGES: usize = 300;

/// The node kinds this build has, for the error that names a missing one.
const KINDS: &str = "agent, condition, api, response, variables, message, task, notify, app_query, spawn";

/// What the approval prompt shows of a template's graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DroneSummary {
    pub nodes: u32,
    /// Nodes that start an agent (an Agent node, or a Spawn node that opens a worker), each of which spends money.
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
/// plain read writes outside; a Spawn node starts an agent, so it counts with
/// the Agent nodes; the other nodes this build has stay inside AgentMux (a
/// message goes to an agent on this computer).
pub fn summarize(graph: &DroneGraph) -> DroneSummary {
    let mut out = DroneSummary { nodes: graph.nodes.len() as u32, ..DroneSummary::default() };
    for node in &graph.nodes {
        let text = |key: &str| node.data.get(key).and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
        match kind_of(node) {
            Ok(BlockKind::Agent | BlockKind::Spawn) => out.agents += 1,
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

// ── Parameters ──────────────────────────────────────────────────────────────

/// A parameter placeholder in a node's text: `{{param.repo}}`. Any other
/// `{{` belongs to the drone's own templating and is left as it is.
const PARAM_OPEN: &str = "{{param.";

enum Seg<'a> {
    Text(&'a str),
    Param(&'a str),
}

/// `text` split into plain text and the parameter names it uses. A
/// `{{param.` that does not close as a well-formed name is an error, so a
/// typo can't pass as text.
fn split_params(text: &str) -> Result<Vec<Seg<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(PARAM_OPEN) {
        out.push(Seg::Text(&rest[..start]));
        let after = &rest[start + PARAM_OPEN.len()..];
        let name = after.find("}}").map(|end| &after[..end]).filter(|n| manifest::valid_param_name(n));
        let Some(name) = name else {
            return Err(format!("{PARAM_OPEN} must open a placeholder like {PARAM_OPEN}repo}}}}"));
        };
        out.push(Seg::Param(name));
        rest = &after[name.len() + 2..];
    }
    out.push(Seg::Text(rest));
    Ok(out)
}

/// Every string inside the node's `data`, to read or rewrite.
fn each_string(value: &mut Value, f: &mut dyn FnMut(&mut String) -> Result<(), String>) -> Result<(), String> {
    match value {
        Value::String(s) => f(s),
        Value::Array(items) => items.iter_mut().try_for_each(|v| each_string(v, f)),
        Value::Object(map) => map.values_mut().try_for_each(|v| each_string(v, f)),
        _ => Ok(()),
    }
}

/// The parameter names the graph uses, each of which the manifest must declare.
pub fn param_names(graph: &DroneGraph) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for node in &graph.nodes {
        let mut data = node.data.clone();
        each_string(&mut data, &mut |s| {
            for part in split_params(s).map_err(|e| format!("graph.json: node {:?}: {e}", node.id))? {
                if let Seg::Param(name) = part {
                    names.push(name.to_string());
                }
            }
            Ok(())
        })?;
    }
    Ok(names)
}

/// The graph with each `{{param.NAME}}` replaced by its parameter's value
/// (the given one, else the default). The values go in once and are never read
/// for placeholders. A `{{` that a value contains, or that a value forms with the
/// text next to it, is refused: the drone's own templating would read it at run
/// time.
pub fn render(graph: &DroneGraph, m: &JobTemplateManifest, given: &Map<String, Value>) -> Result<DroneGraph, String> {
    let params = manifest::resolve_params(m, given)?;
    if let Some((name, _)) = params.iter().find(|(_, v)| manifest::as_text(v).contains("{{")) {
        return Err(format!("parameter {name:?} can't contain {{{{: a drone would read it as a reference"));
    }
    let mut out = graph.clone();
    for node in &mut out.nodes {
        let id = node.id.clone();
        each_string(&mut node.data, &mut |s| {
            let mut text = String::with_capacity(s.len());
            let mut inserted = Vec::new();
            for part in split_params(s)? {
                match part {
                    Seg::Text(plain) => text.push_str(plain),
                    Seg::Param(name) => {
                        let start = text.len();
                        text.push_str(&params.get(name).map(manifest::as_text).unwrap_or_default());
                        inserted.push(start..text.len());
                    }
                }
            }
            if opens_reference_in(&text, &inserted) {
                return Err("a parameter value forms {{ with the text next to it: a drone would read it as a reference".to_string());
            }
            *s = text;
            Ok(())
        })
        .map_err(|e| format!("node {id:?}: {e}"))?;
    }
    Ok(out)
}

/// Whether a `{{` in `text` takes in a byte of a value that was put in, which
/// the template text alone would not have written.
fn opens_reference_in(text: &str, inserted: &[std::ops::Range<usize>]) -> bool {
    text.as_bytes()
        .windows(2)
        .enumerate()
        .any(|(at, pair)| pair == b"{{" && inserted.iter().any(|r| at < r.end && at + 2 > r.start))
}

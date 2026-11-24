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
use crate::backend::storage::drone_types::{BlockKind, DroneGraph};

/// The most nodes and edges a template graph holds.
pub const MAX_NODES: usize = 100;
pub const MAX_EDGES: usize = 300;

/// The node kinds this build has, for the error that names a missing one.
const KINDS: &str = "agent, condition, api, response, variables, message, task, notify, app_query, spawn, transform, foreach, wait, drone, join, run";

/// What the approval prompt shows of a template's graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DroneSummary {
    pub nodes: u32,
    /// Nodes that start an agent (an Agent node, or a Spawn node that opens a worker), each of which spends money.
    pub agents: u32,
    /// Every call that writes outside AgentMux, as `node id: METHOD url`.
    pub writes_outside: Vec<String>,
    /// Every place the template reads a credential from the Vault (an `auth.vault`), as `where: entry`, found in the
    /// graph and the trigger themselves, not in what the manifest declares. [`summarize`] leaves it empty:
    /// whoever has the trigger fills it in with [`credentials`].
    #[serde(default)]
    pub credentials: Vec<String>,
}

/// `graph.json` read and checked.
pub fn parse(raw: &[u8]) -> Result<DroneGraph, String> {
    let graph: DroneGraph = serde_json::from_slice(raw).map_err(|e| format!("graph.json: {e}"))?;
    check(&graph)?;
    Ok(graph)
}

/// The kind a node names in `data.kind`.
fn kind_of(node: &crate::backend::storage::drone_types::FlowNode) -> Result<BlockKind, String> {
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
        // What another drone does is on the machine it is installed on, not in the template, so the
        // prompt could not list it: a template does not call one.
        let kind = kind_of(node)?;
        if kind == BlockKind::Drone {
            return Err(format!("graph.json: node {:?} calls another drone, and a template cannot: what that drone does could not be shown when the template is approved", node.id));
        }
        // A command is fixed content too: what it runs is not in the graph, and nothing here pins it.
        if kind == BlockKind::Run {
            return Err(format!("graph.json: node {:?} runs a command, and a template cannot: what the command does could not be shown when the template is approved", node.id));
        }
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

/// Whether the graph has a node that writes outside AgentMux (an API call that is not a read) and an Approval block
/// stands on every path to each of them, so no run reaches a write without a person's yes. False when it has no such
/// write, or one of them can be reached without passing an Approval.
pub fn approval_guards_every_write(graph: &DroneGraph) -> bool {
    let kind = |n: &crate::backend::storage::drone_types::FlowNode| n.data.get("kind").and_then(|k| k.as_str()).and_then(BlockKind::parse);
    let writes: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| kind(n) == Some(BlockKind::Api))
        .filter(|n| {
            let method = n.data.get("method").and_then(|m| m.as_str()).unwrap_or("GET").to_uppercase();
            !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS")
        })
        .map(|n| n.id.as_str())
        .collect();
    if writes.is_empty() {
        return false;
    }
    let approvals: HashSet<&str> = graph.nodes.iter().filter(|n| kind(n) == Some(BlockKind::Approval)).map(|n| n.id.as_str()).collect();
    let targets: HashSet<&str> = graph.edges.iter().map(|e| e.target.as_str()).collect();
    // Walk from every node nothing leads to, and do not go past an Approval.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).filter(|id| !targets.contains(id)).collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        // Past an Approval only by its `error` handle (no, or silence) runs without a yes: those targets stay reachable.
        let guarded = approvals.contains(id);
        stack.extend(
            graph
                .edges
                .iter()
                .filter(|e| e.source == id && (!guarded || e.source_handle.as_deref() == Some("error")))
                .map(|e| e.target.as_str()),
        );
    }
    !approvals.is_empty() && writes.iter().all(|w| !seen.contains(w))
}

/// Every Vault entry the template reads a credential from, as `where: entry`: an `auth.vault` on a node or
/// anywhere in the trigger (its poll's filter), whether the entry is a literal id or a `{{param.x}}`. This is
/// what the template does, whatever its manifest's `secrets` says.
pub fn credentials(graph: &DroneGraph, trigger: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = trigger {
        vault_refs(t, "trigger", &mut out);
    }
    for node in &graph.nodes {
        vault_refs(&node.data, &format!("node {}", node.id), &mut out);
    }
    out
}

fn vault_refs(v: &Value, at: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if let Some(entry) = map.get("auth").and_then(|a| a.get("vault")) {
                out.push(format!("{at}: {}", entry.as_str().map(str::to_string).unwrap_or_else(|| entry.to_string())));
            }
            map.values().for_each(|child| vault_refs(child, at, out));
        }
        Value::Array(items) => items.iter().for_each(|i| vault_refs(i, at, out)),
        _ => {}
    }
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
    refuse_reference_in_values(&params)?;
    let mut out = graph.clone();
    for node in &mut out.nodes {
        let id = node.id.clone();
        // A value put into a condition's expression is read as part of it: no quote or backslash, so it cannot close a
        // literal and add to the expression.
        let in_expression = node.data.get("kind").and_then(Value::as_str) == Some("condition");
        each_string(&mut node.data, &mut |s| {
            *s = fill(s, &params, in_expression)?;
            Ok(())
        })
        .map_err(|e| format!("node {id:?}: {e}"))?;
    }
    Ok(out)
}

/// A parameter value that holds `{{` would be read as a reference by the drone's own templating.
fn refuse_reference_in_values(params: &Map<String, Value>) -> Result<(), String> {
    match params.iter().find(|(_, v)| manifest::as_text(v).contains("{{")) {
        Some((name, _)) => Err(format!("parameter {name:?} can't contain {{{{: a drone would read it as a reference")),
        None => Ok(()),
    }
}

/// A parameter put into an address (after `scheme://`) is a name or `owner/name`: letters, digits, `.`, `_` and `-`, at
/// most one `/` between two such parts, and no `.` or `..` part. Anything else (`?`, `#`, `%`, `@`, `:`, a space, a second
/// slash) could turn the call into another one under the same Vault token.
fn address_part(name: &str, value: &str) -> Result<(), String> {
    let part_ok = |p: &str| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let mut parts = value.split('/');
    let (a, b, c) = (parts.next().unwrap_or(""), parts.next(), parts.next());
    if part_ok(a) && b.is_none_or(part_ok) && c.is_none() {
        return Ok(());
    }
    Err(format!("parameter {name:?} is put into an address, so it is a name or owner/name (letters, digits, '.', '_' and '-', one '/' at most), not {value:?}"))
}

/// A parameter put into a condition's expression holds no quote mark and no backslash: a value with one could close the
/// literal it sits in and change what the condition says.
fn expression_part(name: &str, value: &str) -> Result<(), String> {
    if value.chars().any(|c| matches!(c, '\'' | '"' | '`' | '\\')) {
        return Err(format!("parameter {name:?} is put into a condition, so it holds no quote mark or backslash: {value:?}"));
    }
    Ok(())
}

/// `s` with each `{{param.NAME}}` replaced by the parameter's text. The values go in once and are never read for
/// placeholders.
fn fill(s: &str, params: &Map<String, Value>, in_expression: bool) -> Result<String, String> {
    let mut text = String::with_capacity(s.len());
    let mut inserted = Vec::new();
    for part in split_params(s)? {
        match part {
            Seg::Text(plain) => text.push_str(plain),
            Seg::Param(name) => {
                let value = params.get(name).map(manifest::as_text).unwrap_or_default();
                // Where a value lands decides what it may hold: in an address a Vault token is sent to, or in an expression.
                if text.contains("://") {
                    address_part(name, &value)?;
                }
                if in_expression {
                    expression_part(name, &value)?;
                }
                let start = text.len();
                text.push_str(&value);
                inserted.push(start..text.len());
            }
        }
    }
    if opens_reference_in(&text, &inserted) {
        return Err("a parameter value forms {{ with the text next to it: a drone would read it as a reference".to_string());
    }
    Ok(text)
}

/// The parameter names a JSON value's strings use (a template's trigger default), each of which the manifest must declare.
pub fn param_names_in(value: &Value) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    each_string(&mut value.clone(), &mut |s| {
        for part in split_params(s)? {
            if let Seg::Param(name) = part {
                names.push(name.to_string());
            }
        }
        Ok(())
    })?;
    Ok(names)
}

/// `value` with each `{{param.NAME}}` in its strings replaced, for a template's trigger default: the repository a poll
/// asks about is the one the drone's calls name. `params` are the resolved ones.
pub fn render_value(value: &mut Value, params: &Map<String, Value>) -> Result<(), String> {
    refuse_reference_in_values(params)?;
    each_string(value, &mut |s| {
        *s = fill(s, params, false)?;
        Ok(())
    })
}

/// Whether a `{{` in `text` takes in a byte of a value that was put in, which
/// the template text alone would not have written.
fn opens_reference_in(text: &str, inserted: &[std::ops::Range<usize>]) -> bool {
    text.as_bytes()
        .windows(2)
        .enumerate()
        .any(|(at, pair)| pair == b"{{" && inserted.iter().any(|r| at < r.end && at + 2 > r.start))
}

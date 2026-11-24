// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Rule of Two, made checkable (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §6.2 and §6.3).
//!
//! A drone that reads outside text (a pull request title, a CI log, a webhook body, an API response, an agent's reply)
//! must not let it reach a node that changes the world without a person in between. This reads a graph, before it is
//! turned on, and says which nodes break that. Two rules:
//!
//! * **Writes outside.** An API call that is not a read, a command, an Agent block that may run commands, and a call to
//!   another drone are nodes that change things outside AgentMux. One of them may not read a value that is outside
//!   text unless an Approval block stands on every path from where that text came in to the node.
//! * **Where outside text may not go.** Never into a header name, the address (scheme and host) of a request that
//!   carries a credential or changes something, a Vault credential, the target of a Message or a Task, or the place a command runs, however it got there. It may go
//!   into the text of a Message, a Task or a Notify: those say it is data.
//!
//! What counts as outside text is the engine's own rule (`scheduler::mark_bits`), read from the graph instead of a
//! run: the trigger's fields that its schema does not call clean (`payload_schema`), and the output of an API call, an
//! Agent, a Message reply, an app query, a command, a callee and an event wait, and whatever a Response, Variables,
//! Transform, For-each or Join makes from them. A Condition, a Once, an Approval, a Task, a Notify and a Spawn give
//! nothing outside. An Approval is the one thing that clears it: a person read the card.
//!
//! It is conservative. A reference it cannot place on a path counts as unapproved, and nothing here promises a graph
//! that passes is safe, only that it does not skip the person where the spec says the person must be.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::drone::payload_schema::{is_tainted, PayloadField};
use crate::drone::types::{BlockKind, DroneGraph, FlowNode};

/// One node that breaks a rule, and why, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub node: String,
    pub why: String,
}

/// What the check knows about a graph.
struct Flow<'a> {
    nodes: Vec<(&'a FlowNode, Option<BlockKind>)>,
    index: HashMap<&'a str, usize>,
    out: HashMap<&'a str, Vec<&'a str>>,
    /// Where an Approval's `error` edges go. A no and a silence fail the block, and only an `error` edge takes them, so these
    /// blocks run exactly when the person did not approve: an Approval does not clear them.
    past_no: HashMap<&'a str, Vec<&'a str>>,
    incoming: HashMap<&'a str, Vec<&'a str>>,
    roots: Vec<&'a str>,
    /// A for-each's body block, to the for-each.
    foreach_of: HashMap<&'a str, &'a FlowNode>,
    fields: &'a [PayloadField],
    /// The blocks whose output is outside text.
    outside: HashSet<&'a str>,
}

/// Every `{{reference}}` in `text`, as the path it names (a filter after `|` is not part of it).
fn refs_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(len) = rest[open + 2..].find("}}") else { break };
        let token = &rest[open + 2..open + 2 + len];
        found.push(token.split('|').next().unwrap_or("").trim().to_string());
        rest = &rest[open + 2 + len + 2..];
    }
    found
}

/// Every text a node holds, with the name of the field it is in.
fn texts_of<'a>(node: &'a FlowNode) -> Vec<(&'a str, &'a str)> {
    fn walk<'a>(v: &'a Value, field: &'a str, out: &mut Vec<(&'a str, &'a str)>) {
        match v {
            Value::String(s) => out.push((field, s.as_str())),
            Value::Array(a) => a.iter().for_each(|e| walk(e, field, out)),
            Value::Object(m) => m.values().for_each(|e| walk(e, field, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(map) = node.data.as_object() {
        for (field, v) in map {
            if field != "kind" {
                walk(v, field.as_str(), &mut out);
            }
        }
    }
    out
}

fn kind_of(node: &FlowNode) -> Option<BlockKind> {
    node.data.get("kind").and_then(Value::as_str).and_then(BlockKind::parse)
}

/// Whether `node` changes things outside AgentMux. A Drone block is counted: what the callee does is not known here.
fn writes_outside(node: &FlowNode, kind: BlockKind) -> bool {
    match kind {
        BlockKind::Api => {
            let method = node.data.get("method").and_then(Value::as_str).unwrap_or("GET").to_uppercase();
            !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS")
        }
        BlockKind::Run | BlockKind::Drone => true,
        // `allow_commands` is `true`, or the list of command lines the agent may run: either one lets it run commands.
        BlockKind::Agent => match node.data.get("allow_commands") {
            Some(Value::Bool(b)) => *b,
            Some(Value::Array(list)) => !list.is_empty(),
            Some(Value::String(s)) => !s.trim().is_empty(),
            _ => false,
        },
        _ => false,
    }
}

/// The part of a request address that names where it goes: the scheme and the host.
fn address_of(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(0, |i| i + 3);
    let end = url[after_scheme..].find(['/', '?', '#']).map_or(url.len(), |i| after_scheme + i);
    &url[..end]
}

impl<'a> Flow<'a> {
    fn new(graph: &'a DroneGraph, fields: &'a [PayloadField]) -> Self {
        let nodes: Vec<(&FlowNode, Option<BlockKind>)> = graph.nodes.iter().map(|n| (n, kind_of(n))).collect();
        let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, (n, _))| (n.id.as_str(), i)).collect();
        let mut out: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut past_no: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut foreach_of = HashMap::new();
        for e in &graph.edges {
            let from_approval = index.get(e.source.as_str()).is_some_and(|i| nodes[*i].1 == Some(BlockKind::Approval));
            if from_approval && e.source_handle.as_deref() == Some("error") {
                past_no.entry(e.source.as_str()).or_default().push(e.target.as_str());
            } else {
                out.entry(e.source.as_str()).or_default().push(e.target.as_str());
            }
            incoming.entry(e.target.as_str()).or_default().push(e.source.as_str());
        }
        // A for-each's body has no edges of its own: it runs where the for-each does.
        for &(n, kind) in &nodes {
            if kind != Some(BlockKind::ForEach) {
                continue;
            }
            if let Some(body) = n.data.get("body").and_then(Value::as_str) {
                if let Some(&i) = index.get(body.trim()) {
                    let b: &'a FlowNode = nodes[i].0;
                    out.entry(n.id.as_str()).or_default().push(b.id.as_str());
                    incoming.entry(b.id.as_str()).or_default().push(n.id.as_str());
                    foreach_of.insert(b.id.as_str(), n);
                }
            }
        }
        let roots = nodes.iter().map(|(n, _)| n.id.as_str()).filter(|id| !incoming.contains_key(id)).collect();
        let mut flow = Self { nodes, index, out, past_no, incoming, roots, foreach_of, fields, outside: HashSet::new() };
        flow.settle();
        flow
    }

    /// The blocks reached from `start` (themselves included), without passing through an Approval when `stop`.
    fn reach(&self, start: &[&'a str], stop: bool) -> HashSet<&'a str> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = start.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id) {
                continue;
            }
            let approval = self.index.get(id).is_some_and(|i| self.nodes[*i].1 == Some(BlockKind::Approval));
            if approval {
                queue.extend(self.past_no.get(id).into_iter().flatten().copied());
                if stop {
                    continue;
                }
            }
            queue.extend(self.out.get(id).into_iter().flatten().copied());
        }
        seen
    }

    /// Whether text that came in at `source` (the trigger when none) can reach `at` with no Approval on some path. A node
    /// the text cannot be placed on a path to counts as reached.
    fn reaches_unapproved(&self, source: Option<&'a str>, at: &str) -> bool {
        let start: Vec<&'a str> = match source {
            None => self.roots.clone(),
            Some(id) => self.out.get(id).cloned().unwrap_or_default(),
        };
        let without_approval = self.reach(&start, true);
        without_approval.contains(at) || !self.reach(&start, false).contains(at)
    }

    /// Whether `{{path}}`, read by the block `at`, is outside text no Approval has cleared.
    fn ref_is_outside(&self, path: &str, at: &str) -> bool {
        let (head, rest) = path.split_once('.').unwrap_or((path, ""));
        match head {
            "trigger" => is_tainted(self.fields, rest) && self.reaches_unapproved(None, at),
            "env" => false,
            "item" | "index" => self.foreach_of.get(at).is_some_and(|fe| self.items_outside(fe)),
            "var" | "vars" => self
                .nodes
                .iter()
                .filter(|(n, k)| matches!(k, Some(BlockKind::Variables | BlockKind::Transform)) && self.outside.contains(n.id.as_str()))
                .any(|(n, _)| self.reaches_unapproved(Some(n.id.as_str()), at)),
            id => self.outside.contains(id) && self.reaches_unapproved(self.index.get(id).map(|i| self.nodes[*i].0.id.as_str()), at),
        }
    }

    fn items_outside(&self, foreach: &FlowNode) -> bool {
        let items = foreach.data.get("items").and_then(Value::as_str).unwrap_or("");
        !self.outside_refs(items, foreach.id.as_str()).is_empty()
    }

    /// The references in `text`, read by `at`, that are outside text.
    fn outside_refs(&self, text: &str, at: &str) -> Vec<String> {
        refs_in(text).into_iter().filter(|r| self.ref_is_outside(r, at)).collect()
    }

    /// The references anywhere in `node` that are outside text.
    fn outside_refs_of(&self, node: &FlowNode) -> Vec<String> {
        let mut found: Vec<String> = texts_of(node).iter().flat_map(|(_, t)| self.outside_refs(t, node.id.as_str())).collect();
        found.sort();
        found.dedup();
        found
    }

    /// Whether the output of `node` is outside text, given what is known so far.
    fn makes_outside_text(&self, node: &FlowNode, kind: BlockKind) -> bool {
        match kind {
            BlockKind::Agent | BlockKind::Api | BlockKind::Message | BlockKind::AppQuery | BlockKind::Run | BlockKind::Drone => true,
            BlockKind::Wait => node.data.get("on").is_some(),
            BlockKind::Condition | BlockKind::Once | BlockKind::Approval | BlockKind::Task | BlockKind::Notify | BlockKind::Spawn => false,
            BlockKind::Response | BlockKind::Variables | BlockKind::Transform => !self.outside_refs_of(node).is_empty(),
            BlockKind::ForEach => {
                let body = node.data.get("body").and_then(Value::as_str).unwrap_or("").trim();
                self.items_outside(node) || self.outside.contains(body)
            }
            BlockKind::Join => self.incoming.get(node.id.as_str()).is_some_and(|from| from.iter().any(|f| self.outside.contains(f))),
        }
    }

    /// Work out which blocks give outside text: a block can make it from another's, so go round until nothing changes.
    fn settle(&mut self) {
        for _ in 0..=self.nodes.len() {
            let found: Vec<&'a str> = self
                .nodes
                .iter()
                .filter_map(|&(n, kind)| kind.filter(|k| self.makes_outside_text(n, *k)).map(|_| n.id.as_str()))
                .collect();
            if found.len() == self.outside.len() {
                return;
            }
            self.outside = found.into_iter().collect();
        }
    }
}

fn join(refs: &[String]) -> String {
    refs.iter().map(|r| format!("{{{{{r}}}}}")).collect::<Vec<_>>().join(", ")
}

/// Whether a request carries a credential: its `auth`, a header that names one (`Authorization`, a key, a token, a cookie) or
/// whose value reads the environment or the Vault, or any text in it that reads them.
fn sends_credential(node: &FlowNode) -> bool {
    const NAMES: [&str; 6] = ["auth", "token", "key", "secret", "cookie", "password"];
    let reads_secret = |t: &str| t.contains("{{env.") || t.contains("{{vault");
    if node.data.get("auth").is_some() {
        return true;
    }
    let header = node.data.get("headers").and_then(Value::as_object).is_some_and(|m| {
        m.iter().any(|(name, value)| {
            let name = name.to_lowercase();
            NAMES.iter().any(|n| name.contains(n)) || reads_secret(&value.to_string())
        })
    });
    header || texts_of(node).iter().any(|(_, t)| reads_secret(t))
}

/// The places in `node` that outside text may not choose, each with what it is.
fn forbidden_places<'a>(node: &'a FlowNode, kind: BlockKind) -> Vec<(&'static str, String)> {
    let text = |key: &str| node.data.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let keys = |key: &str| node.data.get(key).and_then(Value::as_object).map(|m| m.keys().cloned().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    match kind {
        BlockKind::Api => {
            let mut places = vec![("the name of a header", keys("headers")), ("a credential", node.data.get("auth").map(Value::to_string).unwrap_or_default())];
            // Where a request goes matters when it carries a credential or changes something. A plain read with no
            // credential may follow an address an earlier answer gave (a redirect to a log, a next page): what it can
            // reach is limited by the engine's own refusal of private addresses, and it sends nothing secret.
            if writes_outside(node, kind) || sends_credential(node) {
                places.push(("the address a request goes to", address_of(&text("url")).to_string()));
            }
            places
        }
        BlockKind::Message => vec![("who a message goes to", text("to"))],
        BlockKind::Task => vec![("who a task is for", text("target"))],
        BlockKind::Run => vec![
            ("where a command runs", format!("{} {} {}", text("where"), text("cwd"), text("image"))),
            ("the name of an environment variable", keys("env")),
        ],
        _ => Vec::new(),
    }
}

/// Every way `graph` breaks the Rule of Two when its trigger has `fields`.
pub fn check(graph: &DroneGraph, fields: &[PayloadField]) -> Vec<Finding> {
    let flow = Flow::new(graph, fields);
    let mut findings = Vec::new();
    for (node, kind) in &flow.nodes {
        let Some(kind) = *kind else { continue };
        let id = node.id.clone();
        for (place, text) in forbidden_places(node, kind) {
            let refs = flow.outside_refs(&text, &id);
            if !refs.is_empty() {
                findings.push(Finding {
                    node: id.clone(),
                    why: format!("{id}: {} is text from outside and may not choose {place}; only the graph's own words may. An Approval on the path before it clears it", join(&refs)),
                });
            }
        }
        if writes_outside(node, kind) {
            let refs = flow.outside_refs_of(node);
            if !refs.is_empty() {
                findings.push(Finding {
                    node: id.clone(),
                    why: format!(
                        "{id}: changes things outside AgentMux and reads text from outside ({}) with no Approval on the way. Put an Approval block before it, or give it only values the trigger declares clean",
                        join(&refs)
                    ),
                });
            }
        }
    }
    findings
}

/// The findings as one message for a refusal, or `Ok` when there are none.
pub fn refuse(graph: &DroneGraph, fields: &[PayloadField]) -> Result<(), String> {
    let findings = check(graph, fields);
    if findings.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = findings.iter().map(|f| f.why.as_str()).collect();
    Err(format!("this drone cannot be turned on: {}", list.join("; ")))
}

#[cfg(test)]
#[path = "rule_of_two_tests.rs"]
mod tests;

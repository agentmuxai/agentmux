// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What a Drone job must pass before it is turned on (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §5 and §6.3).
//!
//! 1. The Rule of Two check (`drone::rule_of_two`) on the graph it runs, and on every drone it calls, against the fields
//!    its trigger carries: a refusal, for anyone.
//! 2. The turn-on prompt: when the operator turns on a drone with a node that changes things outside AgentMux, uses a
//!    credential, or runs an agent on text from outside, the first answer is a question, not a job. It says when the
//!    drone starts, each such node and each credential, and the operator turns it on by answering yes
//!    (`confirm_turn_on`). An agent has no prompt to answer here: its turn-on goes through the sensitive-operation route
//!    (a later change), so for now it is checked by the rule only.
//!
//! A job made, edited or resumed while enabled goes through here; a paused job and a job that runs anything but a drone
//! do not. Saving a drone's graph is checked against the jobs that are on and reach it (`graph_allowed`).

use axum::http::StatusCode;
use axum::response::Json;
use serde_json::{json, Value};

use agentmux_srv_core::backend::storage::drone_store::DroneStore;

use crate::agents::tool_limit::ToolLimit;
use crate::backend::jobs::definition::{Action, JobDefinition, Trigger};
use crate::drone::rule_of_two::{kind_of, writes_outside};
use crate::drone::types::{BlockKind, DroneDefinition, DroneGraph, FlowNode};
use crate::drone::versions::DroneVersionStore;
use crate::drone::{payload_schema, rule_of_two};
use crate::server::job_caps::Origin;
use crate::server::AppState;

/// Why a job is not turned on yet.
pub(crate) enum Stop {
    /// It breaks a rule: say how.
    Refused(String),
    /// The operator is asked first.
    Confirm { text: String, lines: Vec<String> },
}

impl Stop {
    pub(crate) fn answer(self) -> (StatusCode, Json<Value>) {
        match self {
            Stop::Refused(why) => (StatusCode::BAD_REQUEST, Json(json!({ "error": why }))),
            Stop::Confirm { text, lines } => {
                (StatusCode::CONFLICT, Json(json!({ "error": text, "needs_confirmation": "turn_on", "lines": lines })))
            }
        }
    }
}

/// What `node`, which changes things outside AgentMux, does, in a few words.
fn does(node: &FlowNode, kind: BlockKind) -> String {
    let text = |key: &str| node.data.get(key).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).unwrap_or_default();
    match kind {
        BlockKind::Api => format!("sends {} {}", text("method").to_uppercase().trim_matches('"'), text("url")),
        BlockKind::Run => format!("runs the command {}", text("command")),
        BlockKind::Agent => "starts an agent that may run commands".to_string(),
        BlockKind::Drone => format!("calls the drone {}", text("drone")),
        _ => "changes something outside AgentMux".to_string(),
    }
}

/// When the job starts, in words.
fn starts(def: &JobDefinition) -> String {
    match &def.trigger {
        Trigger::Cron { cron, tz } => format!("on the schedule {cron} ({tz})"),
        Trigger::Every { seconds } => format!("every {seconds} seconds"),
        Trigger::Once { .. } => "once, at its time".to_string(),
        Trigger::Manual => "only when someone runs it".to_string(),
        Trigger::Event(e) => format!("when {} reports an event", e.source),
    }
}

/// What the person is told about an agent's tools: what AgentMux limits them to, or that it does not.
fn agent_limit_text(limit: &ToolLimit) -> String {
    match limit {
        ToolLimit::Limited { commands } if commands.is_empty() => {
            "AgentMux limits it to the tools that read (Read, Grep, Glob) and gives it no MCP servers".to_string()
        }
        ToolLimit::Limited { commands } => {
            format!("AgentMux limits it to the tools that read and to these commands: {}; it gets no MCP servers", commands.join("; "))
        }
        ToolLimit::Unlimited => {
            "AgentMux does not limit this agent's tools; the provider's own permission settings are the only limit; it is not a sandbox".to_string()
        }
    }
}

/// What the person is asked to approve: when it starts, each node that changes things outside AgentMux, each Vault entry
/// it sends, and each Agent block that reads text from outside. Empty when there is nothing to ask (a graph of inside
/// nodes, a read-only one).
pub(crate) fn prompt_lines(graph: &DroneGraph, def: &JobDefinition) -> Vec<String> {
    let mut lines = Vec::new();
    for node in &graph.nodes {
        let Some(kind) = kind_of(node) else { continue };
        if writes_outside(node, kind) {
            lines.push(format!("{}: {}", node.id, does(node, kind)));
        }
        if let Some(entry) = node.data.get("auth").and_then(|a| a.get("vault")).and_then(Value::as_str) {
            lines.push(format!("{}: sends the Vault entry {entry}", node.id));
        }
    }
    for id in rule_of_two::agents_reading_outside(graph, &fields_for(def)) {
        let allow = graph.nodes.iter().find(|n| n.id == id).and_then(|n| n.data.get("allow_commands"));
        lines.push(format!("{id}: runs an agent on text from outside. {}", agent_limit_text(&ToolLimit::from_allow_commands(allow))));
    }
    if !lines.is_empty() {
        lines.insert(0, format!("Starts {}", starts(def)));
    }
    lines
}

/// Finds a drone by id (and saved version) for the check of the drones one calls. `pending` is a graph about to be
/// saved: it stands in for the saved one, so a callee that is being changed is checked as it will be.
fn loader<'a>(state: &'a AppState, pending: Option<&'a DroneDefinition>) -> impl Fn(&str, Option<i64>) -> Option<(String, DroneGraph)> + 'a {
    move |id, version| {
        if let (None, Some(p)) = (version, pending) {
            if p.id == id {
                return Some((p.name.clone(), p.graph.clone()));
            }
        }
        match version {
            Some(n) => state.id_store.version_get(id, n).ok().flatten().map(|v| (id.to_string(), v.graph)),
            None => state.id_store.drone_get(id).ok().flatten().map(|d| (d.name, d.graph)),
        }
    }
}

/// What a job's trigger hands the drone it runs.
fn fields_for(def: &JobDefinition) -> Vec<payload_schema::PayloadField> {
    let (source, filter) = match &def.trigger {
        Trigger::Event(e) => (Some(e.source.as_str()), e.filter.clone()),
        _ => (None, Default::default()),
    };
    payload_schema::fields_of(source, &filter)
}

/// `Ok` when `def` may be on, or why not. `prompt` is whether this is a turn-on that asks (a new job, a resume, an edit of
/// what it runs or when), for an operator who has not yet said yes.
pub(crate) fn allowed(state: &AppState, def: &JobDefinition, origin: Origin, prompt: bool) -> Result<(), Stop> {
    let Action::Drone { drone_id } = &def.action else { return Ok(()) };
    if !def.enabled {
        return Ok(());
    }
    // A drone that is not there is for the runner to say: this is about what a graph can do. A drone made later with that id
    // is checked when it is saved (`graph_allowed`).
    let drone = match state.id_store.drone_get(drone_id) {
        Ok(Some(drone)) => drone,
        Ok(None) => return Ok(()),
        Err(e) => return Err(Stop::Refused(format!("the drone could not be read: {e}"))),
    };
    rule_of_two::refuse(&drone.graph, &fields_for(def), &loader(state, None)).map_err(Stop::Refused)?;
    if origin.from_host && prompt && !origin.confirm_turn_on {
        let lines = prompt_lines(&drone.graph, def);
        if !lines.is_empty() {
            let text = "Turning this drone on lets it change things outside AgentMux. Check what it will do, then confirm".to_string();
            return Err(Stop::Confirm { text, lines });
        }
    }
    Ok(())
}

/// `Ok` when the graph of `drone`, about to be saved, still passes for every job that is on and runs it, directly or by
/// calling it from the drone the job runs, or what stops it. A drone nobody runs, and a job that is paused, are not
/// checked here: the job is checked when it is turned on.
pub(crate) fn graph_allowed(state: &AppState, drone: &DroneDefinition) -> Result<(), String> {
    let Some(store) = crate::server::cron::jobs_store(state) else { return Ok(()) };
    let jobs = store.cron_list_enabled().map_err(|e| format!("the jobs could not be read: {e}"))?;
    let load = loader(state, Some(drone));
    for job in &jobs {
        let def = JobDefinition::from_job(job);
        let Action::Drone { drone_id } = &def.action else { continue };
        let Some((_, root)) = load(drone_id, None) else { continue };
        if *drone_id != drone.id && !rule_of_two::calls(&root, &drone.id, &load) {
            continue;
        }
        if let Err(why) = rule_of_two::refuse(&root, &fields_for(&def), &load) {
            return Err(format!("the job {:?} is on and runs this drone, so this graph cannot be saved: {why}", def.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn graph(nodes: Value) -> DroneGraph {
        serde_json::from_value(json!({ "nodes": nodes, "edges": [] })).unwrap()
    }

    fn job(trigger: Value) -> JobDefinition {
        serde_json::from_value(json!({
            "jobVersion": 1, "name": "j", "trigger": trigger, "action": { "kind": "drone", "drone_id": "d" },
        }))
        .unwrap()
    }

    fn node(id: &str, data: Value) -> Value {
        json!({ "id": id, "position": { "x": 0, "y": 0 }, "data": data })
    }

    #[test]
    fn the_prompt_says_when_it_starts_and_each_write_and_credential() {
        let g = graph(json!([
            node("fetch", json!({ "kind": "api", "method": "GET", "url": "https://a.example.com" })),
            node("merge", json!({ "kind": "api", "method": "post", "url": "https://api.example.com/merge", "auth": { "vault": "github-token", "as": "bearer" } })),
            node("sh", json!({ "kind": "run", "command": ["./check.sh", "v1"] })),
        ]));
        let lines = prompt_lines(&g, &job(json!({ "kind": "manual" })));
        assert_eq!(lines[0], "Starts only when someone runs it");
        assert!(lines.iter().any(|l| l == "merge: sends POST https://api.example.com/merge"), "{lines:?}");
        assert!(lines.iter().any(|l| l == "merge: sends the Vault entry github-token"), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("sh: runs the command ")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.starts_with("fetch")), "a read is not asked about");
    }

    #[test]
    fn a_graph_that_only_reads_or_works_inside_asks_nothing() {
        let g = graph(json!([
            node("fetch", json!({ "kind": "api", "method": "GET", "url": "https://a.example.com" })),
            node("t", json!({ "kind": "task", "title": "x", "target": "opaz" })),
        ]));
        assert!(prompt_lines(&g, &job(json!({ "kind": "manual" }))).is_empty());
    }

    #[test]
    fn an_agent_that_reads_text_from_outside_is_named_with_what_limits_its_tools() {
        let g = graph(json!([
            node("fetch", json!({ "kind": "api", "method": "GET", "url": "https://ci.example.com/log" })),
            node("triage", json!({ "kind": "agent", "task": "Say what failed in {{fetch.body}}" })),
            node("checker", json!({ "kind": "agent", "task": "Check {{fetch.body}}", "allow_commands": ["./check.sh v1"] })),
            node("free", json!({ "kind": "agent", "task": "Do what {{fetch.body}} says", "allow_commands": true })),
            node("plain", json!({ "kind": "agent", "task": "Say good morning" })),
        ]));
        let lines = prompt_lines(&g, &job(json!({ "kind": "manual" })));
        let said = |id: &str| lines.iter().find(|l| l.starts_with(&format!("{id}: runs an agent on text from outside"))).unwrap_or_else(|| panic!("{lines:?}")).clone();
        let (triage, checker, free) = (said("triage"), said("checker"), said("free"));
        assert!(triage.contains("AgentMux limits it to the tools that read") && triage.contains("no MCP servers"), "{triage}");
        assert!(checker.contains("these commands: ./check.sh v1"), "{checker}");
        assert!(free.contains("AgentMux does not limit this agent's tools") && free.contains("not a sandbox"), "only a block with no limit says so: {free}");
        assert!(!lines.iter().any(|l| l.to_lowercase().contains("read-only")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.starts_with("plain:")), "an agent that reads nothing from outside is not asked about");
    }
}

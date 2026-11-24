// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Drone job must pass before it is turned on (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §5 and §6.3).
//!
//! 1. The Rule of Two check (`drone::rule_of_two`) on the graph it runs, against the fields its trigger carries: a
//!    refusal, for anyone.
//! 2. The turn-on prompt: when the operator turns on a drone with a node that changes things outside AgentMux or uses a
//!    credential, the first answer is a question, not a job. It says when the drone starts, each such node and each
//!    credential, and the operator turns it on by answering yes (`confirm_turn_on`). An agent has no prompt to answer
//!    here: its turn-on goes through the sensitive-operation route (a later change), so for now it is checked by the
//!    rule only.
//!
//! A job made, edited or resumed while enabled goes through here; a paused job and a job that runs anything but a drone
//! do not.

use axum::http::StatusCode;
use axum::response::Json;
use serde_json::{json, Value};

use agentmux_srv_core::backend::storage::drone_store::DroneStore;

use crate::backend::jobs::definition::{Action, JobDefinition, Trigger};
use crate::drone::rule_of_two::{kind_of, writes_outside};
use crate::drone::types::{BlockKind, DroneGraph, FlowNode};
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

/// What the person is asked to approve: when it starts, each node that changes things outside AgentMux, each Vault entry
/// it sends. Empty when there is nothing to ask (a graph of inside nodes, a read-only one).
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
    if !lines.is_empty() {
        lines.insert(0, format!("Starts {}", starts(def)));
    }
    lines
}

/// `Ok` when `def` may be on, or why not. `prompt` is whether this is a turn-on that asks (a new job, a resume, an edit of
/// what it runs or when), for an operator who has not yet said yes.
pub(crate) fn allowed(state: &AppState, def: &JobDefinition, origin: Origin, prompt: bool) -> Result<(), Stop> {
    let Action::Drone { drone_id } = &def.action else { return Ok(()) };
    if !def.enabled {
        return Ok(());
    }
    // A drone that is not there is for the runner to say: this is about what a graph can do.
    let drone = match state.id_store.drone_get(drone_id) {
        Ok(Some(drone)) => drone,
        Ok(None) => return Ok(()),
        Err(e) => return Err(Stop::Refused(format!("the drone could not be read: {e}"))),
    };
    let (source, filter) = match &def.trigger {
        Trigger::Event(e) => (Some(e.source.as_str()), e.filter.clone()),
        _ => (None, Default::default()),
    };
    rule_of_two::refuse(&drone.graph, &payload_schema::fields_of(source, &filter)).map_err(Stop::Refused)?;
    if origin.from_host && prompt && !origin.confirm_turn_on {
        let lines = prompt_lines(&drone.graph, def);
        if !lines.is_empty() {
            let text = "Turning this drone on lets it change things outside AgentMux. Check what it will do, then confirm".to_string();
            return Err(Stop::Confirm { text, lines });
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
}

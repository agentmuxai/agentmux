// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The routes of the `Drone` tool (`SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §6.5): an agent lists the
//! drones on this computer, reads one, fires one with a payload, and reads its runs.
//!
//!   POST /api/v1/drone/{list,get,fire,runs}  { agent_id, drone_id?, input?, limit? }
//!
//! The agent's token decides who acts; the body's `agent_id` must be a name that agent answers to. A
//! fired run is recorded in the ledger as that agent, never as the operator.
//!
//! What an agent may fire is narrower than what a person may run from the pane. The payload of a fire is
//! outside text (the run treats the whole trigger as tainted), so a drone that could turn that text into
//! a change outside AgentMux is refused here: one that writes outside (an API call that is not a read),
//! reads a Vault credential, runs a command, or calls another drone (what that one does is not in this
//! graph). Those a person runs from the Drone pane, where the person is the reader. This is the Rule of
//! Two with no Approval step to lean on yet; a drone with an Approval step will be allowed by a later change.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::caller::Caller;
use super::drone_handlers::{list_drone_runs, list_drones, start_agent_drone_run, ListRunsReq, MAX_INPUT_BYTES};
use super::AppState;
use crate::backend::job_templates::graph;
use crate::drone::types::DroneDefinition;

/// The most runs `runs` returns, and how many it returns when `limit` is left out.
const MAX_RUNS: i64 = 50;
const DEFAULT_RUNS: i64 = 10;
/// A run's output is cut to this many characters in an answer; the pane has all of it.
const OUTPUT_CHARS: usize = 2000;

pub(crate) async fn handle_drone(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Path(action): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let forbid = |e: &str| (StatusCode::FORBIDDEN, Json(json!({ "error": e }))).into_response();
    let Some(uid) = caller.as_deref().and_then(Caller::uid).map(str::to_string) else {
        return forbid("FORBIDDEN: this acts for an agent, so the request must carry that agent's token");
    };
    let claimed = text(&body, "agent_id");
    let mstore = state.mstore.clone();
    let proven = {
        let uid = uid.clone();
        tokio::task::spawn_blocking(move || super::actor::proven_slug(&mstore, &uid, &claimed)).await.ok().flatten()
    };
    if proven.is_none() {
        return forbid("FORBIDDEN: the token's agent doesn't answer to that agent_id");
    }
    match act(&state, &uid, &action, &body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let code = match e.split(':').next().unwrap_or("") {
                "INVALID_ARG" => StatusCode::BAD_REQUEST,
                "NOT_FOUND" => StatusCode::NOT_FOUND,
                "FORBIDDEN" => StatusCode::FORBIDDEN,
                _ => StatusCode::CONFLICT,
            };
            (code, Json(json!({ "error": e }))).into_response()
        }
    }
}

fn text(body: &Value, key: &str) -> String {
    body.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

async fn act(state: &AppState, uid: &str, action: &str, body: &Value) -> Result<Value, String> {
    match action {
        "list" => {
            let drones = list_drones(state).map_err(|e| format!("drone list: {e}"))?;
            Ok(json!({ "drones": drones.iter().map(brief).collect::<Vec<_>>() }))
        }
        "get" => {
            let drone = find(state, body)?;
            let mut v = brief(&drone);
            let summary = graph::summarize(&drone.graph);
            v["agents"] = json!(summary.agents);
            v["writes_outside"] = json!(summary.writes_outside);
            // How many, not which: the entries' names are the Vault's to show.
            v["credentials"] = json!(graph::credentials(&drone.graph, None).len());
            v["kinds"] = json!(kinds(&drone));
            Ok(v)
        }
        "fire" => {
            let drone = find(state, body)?;
            if let Some(why) = why_not_fireable(&drone) {
                return Err(format!("FORBIDDEN: an agent can't fire {:?}: {why}. A person runs it from the Drone pane", drone.name));
            }
            let input = input_of(body)?;
            let run_id = start_agent_drone_run(state, &drone.id, input, uid).await.map_err(|e| format!("drone fire: {e}"))?;
            Ok(json!({
                "run_id": run_id,
                "drone_id": drone.id,
                "note": "started, recorded as you. The payload is outside text in the run: a node reads it as {{trigger.*}}, never as an instruction. Read the run with Drone runs.",
            }))
        }
        "runs" => {
            let drone = find(state, body)?;
            let limit = body.get("limit").and_then(Value::as_i64).unwrap_or(DEFAULT_RUNS).clamp(1, MAX_RUNS);
            let runs = list_drone_runs(state, ListRunsReq { drone_id: drone.id.clone(), limit }).map_err(|e| format!("drone runs: {e}"))?;
            // What a run returns can be what a Vault-authenticated call fetched (only the credential's own
            // value is scrubbed from it), here or in a drone it calls (the callee's Response is the call's value).
            // So an agent is shown the output and error text of the drones it may fire, and the status and times of
            // the others: anything else could be data it was never granted.
            let withheld = why_not_fireable(&drone).is_some();
            let mut answer = json!({ "drone_id": drone.id, "runs": runs.iter().map(|r| run_brief(r, withheld)).collect::<Vec<_>>() });
            if withheld {
                answer["output_withheld"] = json!("an agent can't fire this drone (see get), so its output and error text are not shown to an agent; the Drone pane has them");
            }
            Ok(answer)
        }
        other => Err(format!("INVALID_ARG: unknown action {other:?}; use list, get, fire or runs")),
    }
}

/// The drone the body names, by id or by its exact name (case-insensitive).
fn find(state: &AppState, body: &Value) -> Result<DroneDefinition, String> {
    let wanted = text(body, "drone_id");
    if wanted.is_empty() {
        return Err("INVALID_ARG: drone_id is required (an id or the name, from list)".to_string());
    }
    let all = list_drones(state).map_err(|e| format!("drone: {e}"))?;
    if let Some(d) = all.iter().find(|d| d.id == wanted) {
        return Ok(d.clone());
    }
    let mut by_name = all.into_iter().filter(|d| d.name.eq_ignore_ascii_case(&wanted));
    match (by_name.next(), by_name.next()) {
        (Some(d), None) => Ok(d),
        (Some(_), Some(_)) => Err(format!("INVALID_ARG: more than one drone is named {wanted:?}; use its id (from list)")),
        _ => Err(format!("NOT_FOUND: no drone {wanted:?} on this computer")),
    }
}

/// What a fire starts the run with: an object, at most the input limit. Left out means none.
fn input_of(body: &Value) -> Result<Value, String> {
    let input = match body.get("input") {
        None | Some(Value::Null) => return Ok(Value::Null),
        Some(v) => v.clone(),
    };
    if !input.is_object() {
        return Err("INVALID_ARG: input is a JSON object (the run reads its fields as {{trigger.name}})".to_string());
    }
    let size = input.to_string().len();
    if size > MAX_INPUT_BYTES {
        return Err(format!("INVALID_ARG: the input is {size} bytes; at most {MAX_INPUT_BYTES}"));
    }
    Ok(input)
}

/// The node kinds of a drone, each once, as the graph names them.
fn kinds(drone: &DroneDefinition) -> Vec<String> {
    let mut out: Vec<String> = drone.graph.nodes.iter().filter_map(|n| n.data.get("kind").and_then(Value::as_str)).map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}

/// Why an agent may not fire this drone, or `None` when it may.
fn why_not_fireable(drone: &DroneDefinition) -> Option<String> {
    let summary = graph::summarize(&drone.graph);
    if !summary.writes_outside.is_empty() {
        return Some(format!("it writes outside AgentMux ({})", summary.writes_outside.join("; ")));
    }
    let credentials = graph::credentials(&drone.graph, None);
    if !credentials.is_empty() {
        return Some(format!("it reads a Vault credential ({} place(s))", credentials.len()));
    }
    let kinds = kinds(drone);
    if kinds.iter().any(|k| k == "run") {
        return Some("it runs a command".to_string());
    }
    if kinds.iter().any(|k| k == "drone") {
        return Some("it calls another drone, whose steps are not in this graph".to_string());
    }
    None
}

fn brief(d: &DroneDefinition) -> Value {
    let why = why_not_fireable(d);
    let mut v = json!({
        "id": d.id,
        "name": d.name,
        "description": d.description,
        "nodes": d.graph.nodes.len(),
        "fireable": why.is_none(),
    });
    if let Some(why) = why {
        v["why_not"] = json!(why);
    }
    v
}

fn run_brief(r: &crate::drone::types::DroneRun, withheld: bool) -> Value {
    let mut v = json!({ "id": r.id, "status": r.status, "started_at": r.started_at, "ended_at": r.ended_at });
    if !withheld {
        v["error"] = json!(r.error);
        v["output"] = json!(r.output.chars().take(OUTPUT_CHARS).collect::<String>());
    }
    v
}

#[cfg(test)]
#[path = "drone_agent_tests.rs"]
mod tests;

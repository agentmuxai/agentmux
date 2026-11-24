// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent fires a drone it did not make and was not given: the `drone_fire` sensitive operation
//! (`docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §6.5). The request says who asked, which drone, what its
//! graph can do and what payload goes in; the super agent reviews it and the person can stop it in the countdown.
//! When the countdown ends unstopped the run starts, recorded as the agent that asked, on the graph the person was
//! shown: a drone edited since is not run.
//!
//! The payload is not a fact (a banner line is short), so it waits here by the request's id, with the graph's hash.
//! It is kept in memory, as the requests are: after a restart the request is gone too.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::backend::job_templates::graph;
use crate::backend::sensitive_ops::VALUE_MAX;
use crate::backend::sensitive_requests::{registry, Request};
use crate::drone::storage::DroneStore;
use crate::drone::types::DroneDefinition;
use crate::drone::versions::graph_hash;
use crate::server::AppState;

/// How long a payload waits for its request: longer than the longest countdown and its review.
const HOLD: Duration = Duration::from_secs(30 * 60);
/// The most payloads held at once. A request that is never decided ends (`prune` in the registry); this is the
/// same bound, so a flood of fires cannot fill memory with payloads of 64 KB.
const MAX_HELD: usize = 64;

struct Held {
    drone_id: String,
    graph_hash: String,
    input: Value,
    at: std::time::Instant,
}

static HELD: LazyLock<Mutex<HashMap<String, Held>>> = LazyLock::new(Mutex::default);

fn held() -> std::sync::MutexGuard<'static, HashMap<String, Held>> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the banner says the drone can do: its node kinds and how many of its nodes start an agent (each one spends).
pub(crate) fn can_do(drone: &DroneDefinition) -> String {
    let summary = graph::summarize(&drone.graph);
    format!("{} nodes ({}); {} start an agent, which spends", summary.nodes, crate::server::drone_agent::kinds(drone).join(", "), summary.agents)
}

fn payload_fact(input: &Value) -> String {
    if input.is_null() {
        return "none".to_string();
    }
    let text = input.to_string();
    let digest = Sha256::digest(text.as_bytes());
    format!("{} bytes, sha256 {}", text.len(), digest.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

/// File the request for `slug` (whose uid is `uid`) to fire `drone` with `input`, and keep the payload for it.
pub(crate) async fn ask(state: &AppState, (uid, slug): (&str, &str), drone: &DroneDefinition, input: Value) -> Result<Value, String> {
    let facts = BTreeMap::from([
        ("drone".to_string(), drone_fact(drone)),
        ("can_do".to_string(), fit(&can_do(drone))),
        ("payload".to_string(), payload_fact(&input)),
    ]);
    let filed = crate::server::sensitive_handlers::file_request(state, uid, slug, "drone_fire", &facts, &format!("fire the drone {}", drone.name)).await?;
    let id = filed.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    if !id.is_empty() && filed.get("joined") != Some(&json!(true)) {
        let entry = Held { drone_id: drone.id.clone(), graph_hash: graph_hash(&drone.graph), input, at: std::time::Instant::now() };
        if !hold(&id, entry) {
            // Without its payload the request could only fail at the end of the countdown: withdraw it now and say why.
            let _ = registry().cancel(&id, uid, agentmux_common::time::now_ms());
            return Err("LIMIT: too many drone fires are waiting for an answer; try again when one has been decided".to_string());
        }
    }
    Ok(json!({
        "asked": true,
        "request_id": id,
        "drone_id": drone.id,
        "note": "You don't own this drone and weren't given it, so the person is asked first: the super agent reviews it and the person has a countdown to stop it. It runs when the countdown ends unless it is stopped, recorded as you; you are told. Ask the person to give you the drone (Drone pane) to fire it at once."
    }))
}

/// Keep the payload for `request_id`, after letting go of the payloads of requests that ended without running (cancelled,
/// denied, vetoed or expired) or waited too long. False when the room is full.
fn hold(request_id: &str, entry: Held) -> bool {
    let mut all = held();
    all.retain(|id, h| h.at.elapsed() < HOLD && registry().get(id).is_some_and(|r| r.status.is_open()));
    if all.len() >= MAX_HELD {
        return false;
    }
    all.insert(request_id.to_string(), entry);
    true
}

/// The drone as a fact: its name and its whole id. A long name is cut, never the id, so two drones are never one request.
fn drone_fact(drone: &DroneDefinition) -> String {
    let id = format!(" ({})", drone.id);
    let room = VALUE_MAX.saturating_sub(id.chars().count());
    let name = if drone.name.chars().count() <= room {
        drone.name.clone()
    } else {
        let mut cut: String = drone.name.chars().take(room.saturating_sub(1)).collect();
        cut.push('…');
        cut
    };
    format!("{name}{id}")
}

/// `text` cut to what a fact of a request may hold.
fn fit(text: &str) -> String {
    if text.chars().count() <= VALUE_MAX {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(VALUE_MAX - 1).collect();
    cut.push('…');
    cut
}

/// The runner: the countdown ended unvetoed.
pub(crate) async fn run_after_countdown(state: AppState, request: Request) -> Result<(), String> {
    let entry = held().remove(&request.id).ok_or("the payload for this request is gone (AgentMux restarted, or it waited too long); ask again")?;
    let drone = state
        .id_store
        .drone_get(&entry.drone_id)
        .map_err(|e| format!("drone: {e}"))?
        .ok_or("the drone was deleted while the request waited")?;
    if graph_hash(&drone.graph) != entry.graph_hash {
        return Err(format!("{:?} was edited while the request waited, so it is not what the person saw; ask again", drone.name));
    }
    if let Some(why) = crate::server::drone_agent::why_not_fireable(&drone) {
        return Err(format!("an agent can't fire {:?}: {why}", drone.name));
    }
    let run_id = crate::server::drone_handlers::start_agent_drone_run(&state, &drone, entry.input, &request.requester_uid).await?;
    crate::sagas::sensitive_window::tell_askers(&request, &format!("The person didn't stop it: {:?} is running as your fire (run {run_id}). Read it with Drone runs.", drone.name));
    Ok(())
}

#[cfg(test)]
#[path = "drone_fire_tests.rs"]
mod tests;

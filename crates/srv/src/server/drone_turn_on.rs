// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Drone job must pass before it is turned on (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §6.3):
//! the Rule of Two check (`drone::rule_of_two`) on the graph it runs, against the fields its trigger carries.
//! A job made, edited or resumed while enabled goes through here; a paused job and a job that runs anything but a
//! drone do not.

use agentmux_srv_core::backend::storage::drone_store::DroneStore;

use crate::drone::versions::DroneVersionStore;

use crate::backend::jobs::definition::{Action, JobDefinition, Trigger};
use crate::drone::types::{DroneDefinition, DroneGraph};
use crate::drone::{payload_schema, rule_of_two};
use crate::server::AppState;

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

/// `Ok` when `def` may be on, or what in the drone's graph (or in a drone it calls) stops it.
pub(crate) fn allowed(state: &AppState, def: &JobDefinition) -> Result<(), String> {
    let Action::Drone { drone_id } = &def.action else { return Ok(()) };
    if !def.enabled {
        return Ok(());
    }
    // A drone that is not there is for the runner to say: this is about what a graph can do. A drone made later with that id
    // is checked when it is saved (`graph_allowed`).
    let Some(drone) = state.id_store.drone_get(drone_id).map_err(|e| format!("the drone could not be read: {e}"))? else { return Ok(()) };
    rule_of_two::refuse(&drone.graph, &fields_for(def), &loader(state, None))
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

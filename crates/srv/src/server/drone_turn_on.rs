// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Drone job must pass before it is turned on (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §6.3):
//! the Rule of Two check (`drone::rule_of_two`) on the graph it runs, against the fields its trigger carries.
//! A job made, edited or resumed while enabled goes through here; a paused job and a job that runs anything but a
//! drone do not.

use agentmux_srv_core::backend::storage::drone_store::DroneStore;

use crate::backend::jobs::definition::{Action, JobDefinition, Trigger};
use crate::drone::types::DroneDefinition;
use crate::drone::{payload_schema, rule_of_two};
use crate::server::AppState;

/// `Ok` when `def` may be on, or what in the drone's graph stops it.
pub(crate) fn allowed(state: &AppState, def: &JobDefinition) -> Result<(), String> {
    let Action::Drone { drone_id } = &def.action else { return Ok(()) };
    if !def.enabled {
        return Ok(());
    }
    // A drone that is not there is for the runner to say: this is about what a graph can do.
    let Some(drone) = state.id_store.drone_get(drone_id).map_err(|e| format!("the drone could not be read: {e}"))? else { return Ok(()) };
    let (source, filter) = match &def.trigger {
        Trigger::Event(e) => (Some(e.source.as_str()), e.filter.clone()),
        _ => (None, Default::default()),
    };
    rule_of_two::refuse(&drone.graph, &payload_schema::fields_of(source, &filter))
}

/// `Ok` when the graph of `drone`, about to be saved, still passes for every job that is on and runs it, or what stops it. A
/// drone nobody runs, and a job that is paused, are not checked here: the job is checked when it is turned on.
pub(crate) fn graph_allowed(state: &AppState, drone: &DroneDefinition) -> Result<(), String> {
    let Some(store) = crate::server::cron::jobs_store(state) else { return Ok(()) };
    let jobs = store.cron_list_enabled().map_err(|e| format!("the jobs could not be read: {e}"))?;
    for job in &jobs {
        let def = JobDefinition::from_job(job);
        if !matches!(&def.action, Action::Drone { drone_id } if *drone_id == drone.id) {
            continue;
        }
        let (source, filter) = match &def.trigger {
            Trigger::Event(e) => (Some(e.source.as_str()), e.filter.clone()),
            _ => (None, Default::default()),
        };
        if let Err(why) = rule_of_two::refuse(&drone.graph, &payload_schema::fields_of(source, &filter)) {
            return Err(format!("the job {:?} runs this drone and is on, so this graph cannot be saved: {why}", def.name));
        }
    }
    Ok(())
}

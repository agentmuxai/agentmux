// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Drone job must pass before it is turned on (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §6.3):
//! the Rule of Two check (`drone::rule_of_two`) on the graph it runs, against the fields its trigger carries.
//! A job made, edited or resumed while enabled goes through here; a paused job and a job that runs anything but a
//! drone do not.

use agentmux_srv_core::backend::storage::drone_store::DroneStore;

use crate::backend::jobs::definition::{Action, JobDefinition, Trigger};
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

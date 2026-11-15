// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Taking Drone runs up again when AgentMux starts
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "Resume").
//!
//! A run left `running` by the old process is no longer marked interrupted and
//! forgotten when it has checkpoints: its newest one is loaded, the graph it was
//! written on is found (the saved version, or the current definition when that is
//! still the same graph), and the engine carries on from there. A run with no
//! checkpoint, or none it can go on from, is left for the restart to interrupt,
//! as before; the reason is in the log.

use std::collections::HashSet;
use std::sync::Arc;

use crate::backend::jobs::RunLedger;
use crate::backend::storage::drone_store::DroneStore;
use crate::drone::checkpoint::{Checkpoint, GraphRef};
use crate::drone::executor::resume_drone;
use crate::drone::recorder::{load, Persist};
use crate::drone::types::{DroneGraph, DroneRun};
use crate::drone::versions::{graph_hash, DroneVersionStore};
use crate::server::AppState;

/// Take up again every run the old process left running that can be. Returns
/// their ids: the restart leaves those rows open (`recover_after_restart_except`).
/// Call before the ledger recovers, from the state that has the stores. `ledger`
/// closes the run's job row when it ends; none for a state without one.
pub(crate) async fn resume_runs(state: &AppState, ledger: Option<&Arc<RunLedger>>) -> HashSet<String> {
    let mut resumed = HashSet::new();
    let running = match state.mstore.drone_runs_running() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "drone: couldn't read the runs left running");
            return resumed;
        }
    };
    for run in running {
        match checkpoint_and_graph(state, &run) {
            Ok(Some((checkpoint, graph))) => {
                let persist = Persist { sink: state.mstore.clone(), graph: checkpoint.graph.clone() };
                let step = checkpoint.step;
                match resume_drone(graph, checkpoint, persist).await {
                    Ok(handle) => {
                        let ledger_run = ledger.and_then(|l| l.open_run_of_drone(&run.id).map(|id| (l.clone(), id)));
                        tracing::info!(run_id = %run.id, drone_id = %run.drone_id, step, "drone: taking a run up again after the restart");
                        crate::server::drone_handlers::spawn_run_drain(state, handle, run.drone_id.clone(), run.started_at, ledger_run);
                        resumed.insert(run.id);
                    }
                    Err(e) => tracing::warn!(run_id = %run.id, error = %e, "drone: a run could not be taken up again"),
                }
            }
            Ok(None) => {}
            Err(why) => tracing::warn!(run_id = %run.id, drone_id = %run.drone_id, "drone: a run is not taken up again: {why}"),
        }
    }
    resumed
}

/// The newest checkpoint of `run` and the graph it was written on; `None` when
/// the run kept no checkpoint.
fn checkpoint_and_graph(state: &AppState, run: &DroneRun) -> Result<Option<(Checkpoint, DroneGraph)>, String> {
    let rows = state.mstore.drone_checkpoints_for(&run.id).map_err(|e| format!("its checkpoints cannot be read: {e}"))?;
    let Some(checkpoint) = load(&rows).map_err(|e| format!("its checkpoints are unusable: {e}"))? else { return Ok(None) };
    let graph = graph_of(state, &run.drone_id, &checkpoint.graph)?;
    Ok(Some((checkpoint, graph)))
}

/// The graph a run started on: the saved version it names when that is the same
/// graph (by hash), else the drone's current graph when that is. A graph that was
/// edited since, with no saved version to go back to, is no graph the run's
/// checkpoints fit.
fn graph_of(state: &AppState, drone_id: &str, graph: &GraphRef) -> Result<DroneGraph, String> {
    if graph.version > 0 {
        if let Ok(Some(v)) = state.id_store.version_get(drone_id, graph.version) {
            if v.hash == graph.hash {
                return Ok(v.graph);
            }
        }
    }
    match state.id_store.drone_get(drone_id) {
        Ok(Some(d)) if graph_hash(&d.graph) == graph.hash => Ok(d.graph),
        Ok(Some(_)) => Err("the drone was edited since, and the graph the run started on was not kept".to_string()),
        Ok(None) => Err("the drone no longer exists".to_string()),
        Err(e) => Err(format!("the drone cannot be read: {e}")),
    }
}

#[cfg(test)]
#[path = "drone_resume_tests.rs"]
mod tests;

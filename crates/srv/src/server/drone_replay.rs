// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `replaydronerun`: start a new run from a step of a finished one
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "Replay and time travel").
//!
//! The new run has its own id and its own checkpoints, starts from the state the earlier run had
//! at that step, and is on the graph version the earlier run started on. A block that would send,
//! file, start or write says what it said in the earlier run instead of doing it again, unless the
//! caller asks for it `do_it_for_real`. A replay that is taken up again after a restart (`resume_runs`)
//! is still a replay: its checkpoints name the run it came from.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::storage::drone_store::DroneStore;
use crate::backend::storage::drone_types::{DroneRun, RunStatus};
use crate::drone::checkpoint::{Checkpoint, Fork};
use crate::drone::executor::replay_drone;
use crate::drone::recorder::{load, Persist};
use crate::drone::replay::{fork_checkpoint, Replay};
use crate::server::drone_handlers::spawn_run_drain;
use crate::server::drone_resume::graph_of;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// What `replaydronerun` is asked to do.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReplayRunReq {
    /// The run to replay: one that has ended.
    pub run_id: String,
    /// The checkpoint to start from: 0 is before its first block, then one more after each block.
    pub from_step: u32,
    /// Run the blocks that have effects as they ran before (send again, file again). The default
    /// is that they say what they said before and do nothing.
    #[serde(default)]
    pub do_it_for_real: bool,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReplayRunResp {
    /// The new run.
    pub run_id: String,
    pub forked_from: String,
    pub from_step: u32,
    pub real: bool,
}

/// What a replaying run needs besides its checkpoint: the run it came from said what, for the blocks it
/// stubs. `None` for a checkpoint of a run that is not a replay. A replay whose source run's checkpoints are
/// gone cannot be taken up again (what its stubs said is not kept anywhere else).
pub(crate) fn replay_for(state: &AppState, c: &Checkpoint) -> Result<Option<Arc<Replay>>, String> {
    let Some(fork) = c.fork.clone() else { return Ok(None) };
    let original = if fork.real {
        Default::default()
    } else {
        let rows = state.mstore.drone_checkpoints_for(&fork.run_id).map_err(|e| format!("the run it replays cannot be read: {e}"))?;
        let newest = load(&rows).map_err(|e| format!("the run it replays is unusable: {e}"))?;
        newest.ok_or_else(|| format!("the run it replays ({}) kept no checkpoints any more", fork.run_id))?.outputs
    };
    Ok(Some(Arc::new(Replay { fork, original })))
}

/// `replaydronerun`.
pub(crate) async fn replay_drone_run(state: &AppState, cmd: ReplayRunReq) -> Result<ReplayRunResp, String> {
    let why = |e: String| format!("replaydronerun: {e}");
    let rows = state.mstore.drone_checkpoints_for(&cmd.run_id).map_err(|e| why(format!("its checkpoints cannot be read: {e}")))?;
    let newest = load(&rows).map_err(|e| why(format!("its checkpoints are unusable: {e}")))?.ok_or_else(|| why("it kept no checkpoints, so there is nothing to start from".to_string()))?;
    if cmd.from_step > newest.step {
        return Err(why(format!("it has checkpoints 0 to {}, not {}", newest.step, cmd.from_step)));
    }
    let kept: Vec<_> = rows.into_iter().filter(|r| r.step <= cmd.from_step).collect();
    let at = load(&kept).map_err(|e| why(format!("checkpoint {} is unusable: {e}", cmd.from_step)))?.ok_or_else(|| why("it kept no checkpoints".to_string()))?;
    let drone_id = at.drone_id.clone();
    let row = state
        .mstore
        .drone_runs_for(&drone_id, 1_000)
        .map_err(|e| why(e.to_string()))?
        .into_iter()
        .find(|r| r.id == cmd.run_id)
        .ok_or_else(|| why(format!("there is no run {} of drone {drone_id}", cmd.run_id)))?;
    ended(&row).map_err(why)?;
    let graph = graph_of(state, &drone_id, &at.graph).map_err(|e| why(format!("the graph it ran on is not available: {e}")))?;
    let wf = state.id_store.drone_get(&drone_id).map_err(|e| why(e.to_string()))?.ok_or_else(|| why(format!("drone {drone_id} no longer exists")))?;

    let run_id = uuid::Uuid::new_v4().to_string();
    let fork = Fork { run_id: cmd.run_id.clone(), step: cmd.from_step, real: cmd.do_it_for_real };
    let first = fork_checkpoint(&at, &run_id, fork.clone());
    let original = if cmd.do_it_for_real { Default::default() } else { newest.outputs };
    let persist = Persist { sink: state.mstore.clone(), graph: at.graph.clone() };
    let trigger = first.trigger.clone();
    let handle = replay_drone(graph, first, persist, Arc::new(Replay { fork, original })).await.map_err(why)?;
    let started_at = now_ms();
    crate::server::drone_handlers::insert_run_placeholder(state, &run_id, &drone_id, started_at).map_err(why)?;
    let ledger = state.cron_scheduler.runs().cloned();
    let ledger_run = ledger.and_then(|l| l.drone_started(&run_id, &drone_id, &wf.name, started_at, &trigger).map(|id| (l, id)));
    spawn_run_drain(state, handle, drone_id, started_at, ledger_run);
    Ok(ReplayRunResp { run_id, forked_from: cmd.run_id, from_step: cmd.from_step, real: cmd.do_it_for_real })
}

/// A run can be replayed once it is over: not while it runs, waits or needs an answer.
fn ended(row: &DroneRun) -> Result<(), String> {
    let live = [RunStatus::Running, RunStatus::Waiting, RunStatus::NeedsAttention].iter().any(|s| s.as_str() == row.status);
    if live {
        return Err(format!("run {} is still {}: replay one that has ended", row.id, row.status));
    }
    Ok(())
}

#[cfg(test)]
#[path = "drone_replay_tests.rs"]
mod tests;

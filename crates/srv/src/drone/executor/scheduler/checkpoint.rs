// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The scheduler's side of checkpoints: what it writes after a block
//! (`snapshot`, `checkpoint`) and how it takes a run up again from one
//! (`restore`)
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3).
//!
//! A checkpoint is written when the scheduler is between blocks, so nothing is
//! half done: the blocks that are ready, the edges decided, the outputs read
//! so far, the variables with their taint bits, and what has been spent. A
//! block's history (`{{block.history}}`) is not stored twice: the output
//! already carries the earlier ones, so a restore rebuilds it from there.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Scheduler, HISTORY_LEN};
use crate::drone::checkpoint::{self, Checkpoint, CheckpointError, Frontier, GraphRef, Position};
use crate::drone::executor::engine::{mark_state, RunEvent};
use crate::drone::recorder::RecordError;
use crate::drone::state::RunState;
use crate::drone::types::BlockState;

/// The fields a loop's exit adds to a block's output after its history was kept.
const ADDED_AFTER_HISTORY: [&str; 2] = ["capped", "capped_reason"];

impl<'a> Scheduler<'a> {
    /// Keep the run as it stands as its next checkpoint, `after` the block that
    /// just finished (none for the first, before any block). A run that cannot
    /// keep a checkpoint goes on, and the next one is a diff from the last kept;
    /// a run that has used up its steps (§3.3: at most 1,000) ends, saying so.
    pub(super) fn checkpoint(&mut self, after: Option<&str>) -> Result<(), String> {
        let Some(step) = self.recorder.as_ref().map(|(r, _)| r.next_step()) else { return Ok(()) };
        let c = self.snapshot(step);
        let Some((recorder, _)) = self.recorder.as_mut() else { return Ok(()) };
        match recorder.write(c) {
            Ok(()) => {
                let _ = self.tx.send(RunEvent::StepCheckpointed { run_id: self.run_id.to_string(), step, block_id: after.map(str::to_string) });
                Ok(())
            }
            Err(RecordError::Checkpoint(CheckpointError::TooManySteps { max, .. })) => {
                let why = format!("stopped: a run keeps at most {max} steps and this one used them all");
                self.cap_reached(after.unwrap_or_default(), &why);
                Err(why)
            }
            Err(e) => {
                tracing::warn!(run_id = %self.run_id, error = %e, "drone: a checkpoint was not kept");
                Ok(())
            }
        }
    }

    /// The run as it stands, as checkpoint `step`.
    fn snapshot(&self, step: u32) -> Checkpoint {
        let ms = |since: &Instant| u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut finished: Vec<String> = self.finished.iter().map(|id| id.to_string()).collect();
        finished.sort_unstable();
        Checkpoint {
            schema_version: checkpoint::SCHEMA_VERSION,
            run_id: self.run_id.to_string(),
            step,
            state: self.scope.state.snapshot(),
            frontier: Frontier {
                ready: self.ready.iter().map(|(id, _)| id.to_string()).collect(),
                forced: self.ready.iter().filter(|(_, forced)| *forced).map(|(id, _)| id.to_string()).collect(),
                running: Vec::new(),
                waiting: Vec::new(),
            },
            loop_counters: self.rounds.iter().map(|(id, n)| (id.to_string(), *n)).collect(),
            cost_usd: self.spent,
            elapsed_ms: self.elapsed_before_ms + ms(&self.started),
            deadlines: BTreeMap::new(),
            resume_on: BTreeMap::new(),
            graph: self.recorder.as_ref().map(|(_, g)| g.clone()).unwrap_or(GraphRef { version: 0, hash: String::new() }),
            drone_id: self.scope.drone_id.clone(),
            trigger: self.scope.trigger.clone(),
            run_cap_usd: self.run_cap_usd,
            outputs: self.scope.outputs.iter().map(|(id, v)| (id.clone(), v.clone())).collect(),
            bits: self.scope.bits.iter().map(|(id, b)| (id.clone(), *b)).collect(),
            position: Position {
                decided: self.decided.iter().map(|(id, carries)| (id.to_string(), *carries)).collect(),
                finished,
                held: self.held.iter().map(|e| e.id.clone()).collect(),
                spent_by: self.spent_by.iter().map(|(id, usd)| (id.to_string(), *usd)).collect(),
                loop_elapsed_ms: self.loop_started.iter().map(|(id, since)| (id.clone(), ms(since))).collect(),
                response: self.response.clone(),
            },
        }
    }

    /// Set this scheduler (new, for the graph the checkpoint was written on) to
    /// where `c` left the run. A checkpoint that names a block or an edge the graph
    /// does not have was written on another graph, and is refused.
    pub(super) fn restore(&mut self, c: &Checkpoint) -> Result<(), String> {
        let edges: HashMap<&'a str, &'a crate::drone::types::FlowEdge> = self.outgoing.values().flatten().map(|e| (e.id.as_str(), *e)).collect();
        let node = |id: &str| -> Result<&'a str, String> {
            self.node_of.get_key_value(id).map(|(k, _)| *k).ok_or_else(|| format!("the saved run does not fit its graph: it has no block {id}"))
        };
        let edge = |id: &str| -> Result<&'a crate::drone::types::FlowEdge, String> {
            edges.get(id).copied().ok_or_else(|| format!("the saved run does not fit its graph: it has no edge {id}"))
        };

        let mut ready = VecDeque::new();
        let mut queued = HashSet::new();
        for id in &c.frontier.ready {
            let id = node(id)?;
            ready.push_back((id, c.frontier.forced.iter().any(|f| f == id)));
            queued.insert(id);
        }
        let mut decided = HashMap::new();
        for (id, carries) in &c.position.decided {
            decided.insert(edge(id)?.id.as_str(), *carries);
        }
        let finished = c.position.finished.iter().map(|id| node(id)).collect::<Result<HashSet<_>, _>>()?;
        let held = c.position.held.iter().map(|id| edge(id)).collect::<Result<Vec<_>, _>>()?;
        let mut rounds = HashMap::new();
        for (id, n) in &c.loop_counters {
            rounds.insert(node(id)?, *n);
        }
        let mut spent_by = HashMap::new();
        for (id, usd) in &c.position.spent_by {
            spent_by.insert(node(id)?, *usd);
        }
        let mut history = HashMap::new();
        for (id, output) in &c.outputs {
            if let Ok(id) = node(id) {
                history.insert(id, history_of(output));
            }
        }
        let now = Instant::now();
        let loop_started = c
            .position
            .loop_elapsed_ms
            .iter()
            .map(|(id, ms)| (id.clone(), now.checked_sub(Duration::from_millis(*ms)).unwrap_or(now)))
            .collect();

        self.ready = ready;
        self.queued = queued;
        self.decided = decided;
        self.finished = finished;
        self.held = held;
        self.rounds = rounds;
        self.spent_by = spent_by;
        self.history = history;
        self.loop_started = loop_started;
        self.spent = c.cost_usd;
        self.response = c.position.response.clone();
        self.run_cap_usd = c.run_cap_usd;
        self.elapsed_before_ms = c.elapsed_ms;
        self.scope.outputs = c.outputs.iter().map(|(id, v)| (id.clone(), v.clone())).collect();
        self.scope.bits = c.bits.iter().map(|(id, b)| (id.clone(), *b)).collect();
        self.scope.vars = c.state.vars.iter().map(|(name, tv)| (name.clone(), tv.v.clone())).collect();
        self.scope.state = RunState::from_snapshot(c.state.clone());
        if let Some((recorder, _)) = self.recorder.as_mut() {
            recorder.resume_from(c.clone());
        }
        Ok(())
    }

    /// Show what the restored blocks did, as the run's own block states, so what
    /// a person sees of a resumed run is not blank where blocks already ran.
    pub(super) async fn show_restored(&self) {
        for id in &self.finished {
            let output = self.scope.outputs.get(*id).cloned();
            let status = if output.is_some() { "done" } else { "skipped" };
            mark_state(self.states, id, BlockState { status: status.to_string(), output, error: None, started_at: None, completed_at: Some(agentmux_common::time::now_ms()) }).await;
        }
    }
}

/// A block's earlier outputs, newest last, from the output it was saved with:
/// the ones its `history` lists, then its own without what was added around
/// it. A handled failure (`{error, failed}`) is not one of them: the block's
/// history never had it.
fn history_of(output: &Value) -> VecDeque<Value> {
    let mut past: VecDeque<Value> = output.get("history").and_then(Value::as_array).map(|a| a.iter().cloned().collect()).unwrap_or_default();
    let mut plain = output.clone();
    if let Some(map) = plain.as_object_mut() {
        map.remove("history");
        for key in ADDED_AFTER_HISTORY {
            map.remove(key);
        }
        if is_handled_failure(map) {
            return past;
        }
    }
    past.push_back(plain);
    while past.len() > HISTORY_LEN {
        past.pop_front();
    }
    past
}

fn is_handled_failure(map: &serde_json::Map<String, Value>) -> bool {
    map.len() == 2 && map.get("failed") == Some(&Value::Bool(true)) && map.contains_key("error")
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;

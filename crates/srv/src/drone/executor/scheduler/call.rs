// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Drone block: a step that is another drone
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.5).
//!
//! The callee runs inside the caller's run, on the caller's remaining budget and
//! under its cancel, so it cannot spend more than the run that called it and
//! stopping the run stops it. It is a child run with an id of its own, announced
//! by `ChildStarted` and `ChildEnded`; it keeps no checkpoints of its own, because
//! the block is a step with effects (the callee may do anything): a restart that
//! finds it running asks what to do with it, as for any such step. What the callee's
//! Response says is the block's `value`, and it is outside text (tainted): the
//! boundary does not yet promise any value is clean.
//!
//! A drone cannot call itself, directly or through the drones it calls, and calls
//! nest at most [`MAX_DEPTH`] deep.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{Limits, Scheduler};
use crate::drone::executor::blocks::call as config;
use crate::drone::executor::engine::{Cancel, RunEnd, RunEvent};
use crate::drone::types::FlowNode;

/// The deepest a chain of calls may go, the first drone counted.
pub(crate) const MAX_DEPTH: usize = 4;

/// What the callee may be given, as JSON: the same limit as a run's own input.
const MAX_INPUT_BYTES: usize = 64 * 1024;

impl<'a> Scheduler<'a> {
    /// Call the drone `node` names and give what it came to. It is boxed, because a
    /// callee can call too: a future that runs a run that can run it cannot say it is
    /// `Send` without help.
    pub(super) fn run_call<'s>(&'s mut self, node: &'a FlowNode) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send + 's>>
    where
        'a: 's,
    {
        Box::pin(self.call(node))
    }

    async fn call(&mut self, node: &'a FlowNode) -> Result<Value, String> {
        let id = node.id.as_str();
        let spec = config::parse(node)?;
        let lookup = self.lookup.clone().ok_or_else(|| format!("{id}: there are no drones to call from here"))?;

        // The drones above this one, then this one: none may be called again.
        let mut chain = self.chain.clone();
        chain.push(self.scope.drone_id.clone());
        if chain.contains(&spec.drone) {
            return Err(format!("{id}: drone {} cannot be called from {}: it is already running above it ({})", spec.drone, self.scope.drone_id, chain.join(" > ")));
        }
        if chain.len() >= MAX_DEPTH {
            return Err(format!("{id}: drones call at most {MAX_DEPTH} deep, and this is the {} ({})", chain.len(), chain.join(" > ")));
        }
        let (graph, version) = lookup.graph(&spec.drone, spec.version)?;

        let inputs: serde_json::Map<String, Value> = spec.inputs.iter().map(|(name, template)| (name.clone(), self.scope.value_of(template))).collect();
        let inputs = Value::Object(inputs);
        let size = inputs.to_string().len();
        if size > MAX_INPUT_BYTES {
            return Err(format!("{id}: the inputs are {size} bytes; at most {MAX_INPUT_BYTES}"));
        }

        // The callee has what the caller has left of its cap. Its cancel is its own, so that it can be
        // stopped when the call is dropped (the block ran out of its own time), and it follows the caller.
        let remaining = remaining_cap(self.run_cap_usd, self.spent);
        let child_cancel = Cancel::default();
        let _stop = StopWith::new(self.cancel.clone(), child_cancel.clone());
        let limits = Limits { run_cap_usd: remaining, cancel: child_cancel, persist: None, chain, lookup: Some(Arc::clone(&lookup)) };
        let child_id = uuid::Uuid::new_v4().to_string();
        let _ = self.tx.send(RunEvent::ChildStarted {
            run_id: self.run_id.to_string(),
            block_id: id.to_string(),
            child_run_id: child_id.clone(),
            drone_id: spec.drone.clone(),
            version,
        });

        // The callee runs as a task of its own: its own events and block states are not the
        // caller's and go nowhere, and a stop (the shared cancel) reaches it wherever it is.
        // It announces its own end, so that is said even when the caller is stopped first.
        let (parent_tx, run_id, block_id) = (self.tx.clone(), self.run_id.to_string(), id.to_string());
        let (drone, callee_run) = (spec.drone.clone(), child_id.clone());
        let late = Arc::clone(&self.late_spend);
        let task = tokio::spawn(async move {
            let (tx, rx) = mpsc::unbounded_channel();
            drop(rx);
            let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let (ended, spent) = super::execute_costed(&callee_run, (&graph, inputs, limits, &drone), &tx, &states).await;
            let status = match &ended {
                Ok(_) => "done",
                Err(RunEnd::Failed(_)) => "failed",
                Err(RunEnd::Cancelled(_)) => "cancelled",
            };
            // What it spent is the caller, and if the caller has gone on without waiting (the call ran out
            // of its time) it is counted from here, at its next block.
            late.lock().unwrap_or_else(|e| e.into_inner()).insert(callee_run.clone(), (block_id.clone(), spent));
            let _ = parent_tx.send(RunEvent::ChildEnded { run_id, block_id, child_run_id: callee_run, status: status.to_string(), cost_usd: spent });
            (ended, spent)
        });
        let (ended, spent) = task.await.map_err(|e| format!("{id}: the called drone stopped unexpectedly: {e}"))?;
        // Counted here, so it is not counted again from the late list.
        self.late_spend.lock().unwrap_or_else(|e| e.into_inner()).remove(&child_id);
        match ended {
            // The cost of a block that finished is counted from its output, as for any block.
            Ok(value) => Ok(json!({ "value": value, "run_id": child_id, "version": version, "cost_usd": spent })),
            Err(RunEnd::Failed(why)) => {
                // What it spent before it failed was still spent.
                self.spent += spent;
                *self.spent_by.entry(id).or_default() += spent;
                Err(format!("drone {} failed: {why}", spec.drone))
            }
            Err(RunEnd::Cancelled(why)) => {
                self.spent += spent;
                *self.spent_by.entry(id).or_default() += spent;
                self.cancelled = self.cancel.is_cancelled();
                Err(format!("stopped: {why}"))
            }
        }
    }
}

impl Scheduler<'_> {
    /// Count what callees spent after the call that started them was given up on.
    pub(super) fn count_late_spend(&mut self) {
        let late: Vec<(String, f64)> = self.late_spend.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, v)| v).collect();
        for (block, spent) in late {
            self.spent += spent;
            if let Some((id, _)) = self.node_of.get_key_value(block.as_str()) {
                *self.spent_by.entry(id).or_default() += spent;
            }
        }
    }
}

/// Stops the callee when the call is dropped, and when the caller is stopped.
struct StopWith {
    callee: Cancel,
    follow: tokio::task::AbortHandle,
}

impl StopWith {
    fn new(caller: Cancel, callee: Cancel) -> Self {
        let (from, to) = (caller, callee.clone());
        let follow = tokio::spawn(async move {
            from.cancelled().await;
            to.cancel(&from.reason());
        })
        .abort_handle();
        Self { callee, follow }
    }
}

impl Drop for StopWith {
    fn drop(&mut self) {
        self.follow.abort();
        self.callee.cancel("the call was stopped before it finished");
    }
}

/// What a callee may spend: what its caller has left of its cap, none when there is no cap.
fn remaining_cap(cap: Option<f64>, spent: f64) -> Option<f64> {
    cap.map(|cap| (cap - spent).max(0.0))
}

#[cfg(test)]
#[path = "call_tests.rs"]
mod tests;

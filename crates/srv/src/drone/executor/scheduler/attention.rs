// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A run that needs attention: a step with effects was running when AgentMux
//! stopped, so it may already have done its work
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3).
//!
//! The run does not repeat the step on its own. It waits, `needs_attention`, for
//! a person or an agent to say: *run it again*, *skip it* (saying what it is taken
//! to have said, which is outside text, so tainted) or *cancel the run*. A run
//! that is stopped meanwhile (its job's timeout, the next run of its job) ends as
//! any run does.

use serde_json::Value;

use super::Scheduler;
use crate::backend::drone_decision::{self, Decision};
use crate::drone::executor::engine::{mark_state, RunEvent};
use crate::drone::types::BlockState;
use agentmux_common::time::now_ms;

impl<'a> Scheduler<'a> {
    /// Wait for the answer about the in-flight block `id`, then do it: the block
    /// is ready to run again, or it is taken as finished with the output given.
    pub(super) async fn settle_in_flight(&mut self, id: &'a str, forced: bool) -> Result<(), String> {
        let run_id = self.run_id.to_string();
        let answer = drone_decision::wait_for_decision(&run_id);
        let reason = format!("block {id} was running when the run stopped and it has effects, so it may already have done its work");
        let _ = self.tx.send(RunEvent::NeedsAttention { run_id: run_id.clone(), block_id: id.to_string(), reason });
        let cancel = self.cancel.clone();
        let decision = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                drone_decision::stop_waiting(&run_id);
                self.cancelled = true;
                return Err(format!("stopped: {}", cancel.reason()));
            }
            decision = answer => decision,
        };
        let Ok(decision) = decision else { return Err("the run stopped waiting for an answer".to_string()) };
        let choice = match &decision {
            Decision::RunAgain => "run_again",
            Decision::Skip { .. } => "skip",
        };
        let _ = self.tx.send(RunEvent::AttentionAnswered { run_id, block_id: id.to_string(), choice: choice.to_string() });
        match decision {
            Decision::RunAgain => {
                // It runs as the next block, marked running again before it starts.
                self.in_flight = None;
                self.enqueue(id, forced);
                Ok(())
            }
            Decision::Skip { output } => self.skip_in_flight(id, output).await,
        }
    }

    /// The in-flight block is taken as finished with `output`, which came from
    /// outside the graph and so is tainted, and the run goes on from there.
    async fn skip_in_flight(&mut self, id: &'a str, mut output: Value) -> Result<(), String> {
        let node = self.node_of[id];
        self.in_flight = None;
        self.finished.insert(id);
        self.keep_history(id, &mut output);
        self.scope.outputs.insert(id.to_string(), output.clone());
        self.scope.bits.insert(id.to_string(), (true, false));
        let _ = self.tx.send(RunEvent::BlockDone { run_id: self.run_id.to_string(), block_id: id.to_string(), output: output.clone() });
        mark_state(self.states, id, BlockState { status: "done".to_string(), output: Some(output.clone()), error: None, started_at: None, completed_at: Some(now_ms()) }).await;
        self.route(node, &output, false)?;
        self.checkpoint(Some(id))
    }
}

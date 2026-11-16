// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Replay from a step (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "Replay and time travel").
//!
//! A replay is a **new run** that starts from checkpoint N of an earlier run, on the same graph
//! version, with the same state. It does not repeat the world: a block that sends, files, starts or
//! writes is **stubbed**, and says what it said in the run it replays, unless the person asks for it
//! "for real". Blocks that only read run as they do.
//!
//! This module is the part that needs no engine: the new run's first checkpoint, and what a stub says.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::drone::checkpoint::{Checkpoint, Fork};

/// What a replaying run is told: where it came from, and what its stubbed blocks say.
#[derive(Debug, Clone, PartialEq)]
pub struct Replay {
    pub fork: Fork,
    /// What each block said in the run replayed, by block id (its newest output).
    pub original: BTreeMap<String, Value>,
    /// The taint bits each block's output had there, which a stub keeps: what the block said is as
    /// clean or as tainted as it was.
    pub bits: BTreeMap<String, (bool, bool)>,
}

impl Replay {
    /// What a stubbed block comes to: what it said there, or, when it failed there and its graph
    /// handled the failure, that failure again, so the replay takes the same road out of it.
    pub fn said(&self, block_id: &str) -> Result<Value, String> {
        if let Some(Value::Object(said)) = self.original.get(block_id) {
            if said.get("failed") == Some(&json!(true)) {
                let error = said.get("error").and_then(Value::as_str).unwrap_or("it failed");
                // The block's end wraps an error as `block <id> failed: ...`; that wrapper is put back there.
                let wrapper = format!("block {block_id} failed: ");
                return Err(error.strip_prefix(wrapper.as_str()).unwrap_or(error).to_string());
            }
        }
        Ok(self.stub(block_id))
    }

    /// What a stubbed block says: what it said in the run replayed, marked `stubbed`, without what
    /// it cost there (nothing ran, so the replay spends nothing); a block that said nothing there
    /// (it had not run, or failed) says only that it was stubbed.
    pub fn stub(&self, block_id: &str) -> Value {
        match self.original.get(block_id) {
            Some(Value::Object(said)) => {
                let mut said = said.clone();
                said.remove("history");
                said.remove("cost_usd");
                said.insert("stubbed".to_string(), json!(true));
                Value::Object(said)
            }
            Some(other) => json!({ "stubbed": true, "value": other }),
            None => json!({ "stubbed": true }),
        }
    }
}

/// The first checkpoint of the run that replays from `at`, the checkpoint of an earlier run:
/// the same state, position and graph, under a new run id, with nothing spent, no time gone, and
/// no wait still pending. A block that was running or waiting at `at` is ready again: the new
/// run starts it from its start.
pub fn fork_checkpoint(at: &Checkpoint, run_id: &str, fork: Fork) -> Checkpoint {
    let mut c = at.clone();
    c.run_id = run_id.to_string();
    c.step = 0;
    c.fork = Some(fork);
    let running = std::mem::take(&mut c.frontier.running);
    let waiting = std::mem::take(&mut c.frontier.waiting);
    let ready = std::mem::take(&mut c.frontier.ready);
    let mut all = Vec::new();
    for id in running.into_iter().chain(waiting).chain(ready) {
        if !all.contains(&id) {
            all.push(id);
        }
    }
    c.frontier.ready = all;
    c.deadlines.clear();
    c.resume_on.clear();
    c.wait_timeouts.clear();
    c.cost_usd = 0.0;
    c.elapsed_ms = 0;
    c.position.spent_by.clear();
    c.position.loop_elapsed_ms.clear();
    c
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The part of a replay that the scheduler decides: which blocks are stubbed
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, `drone::replay`).

use serde_json::Value;

use super::Scheduler;
use crate::drone::executor::effect::Effect;
use crate::drone::types::{BlockKind, FlowNode};

impl Scheduler<'_> {
    /// What `node` says instead of running, when this run replays an earlier one and the block
    /// would do something to the world (it sends, files, starts, writes or calls a drone that may).
    /// A block that only reads runs as it always does, and so does every block of a run that
    /// was asked to do it for real.
    pub(super) fn stub_of(&self, node: &FlowNode, kind: BlockKind) -> Option<Result<Value, String>> {
        let replay = self.replay.as_ref().filter(|r| !r.fork.real)?;
        (self.real_effect_of_node(node, kind) != Effect::ReadOnly).then(|| replay.said(&node.id))
    }

    /// A stubbed block's output is as tainted as it was in the run replayed.
    pub(super) fn keep_stub_bits(&mut self, id: &str) {
        if let Some(bits) = self.replay.as_ref().and_then(|r| r.bits.get(id).copied()) {
            self.scope.bits.insert(id.to_string(), bits);
        }
    }
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;

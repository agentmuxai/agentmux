// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A checkpoint: what the engine writes after every node so a run can resume
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3).
//!
//! **Module B of the engine**, with `state.rs`. The types and the limits are
//! fixed here; [`encode`], [`decode`], [`diff`] and [`apply`] are the module's
//! job. Its acceptance tests are `state_acceptance_tests.rs`.
//!
//! A checkpoint is stored as a **diff from the one before it** (§3.3, the
//! retention paragraph); step 0 has nothing before it and is stored whole. The
//! ceiling is [`MAX_STEPS`] checkpoints per run.
//!
//! **The taint bits are part of the state, and a restore never clears one**:
//! decoding then re-encoding gives back the same bits, and a value whose bits
//! are missing in the bytes is an error (see `value::Tv`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::state::StateSnapshot;

/// The format of an encoded checkpoint. A reader refuses a version it does not
/// know (`CheckpointError::UnsupportedVersion`), so an upgrade never resumes a
/// run from bytes it would misread.
pub const SCHEMA_VERSION: u32 = 1;

/// A run has at most this many checkpoints (§3.3).
pub const MAX_STEPS: u32 = 1_000;

/// The nodes of the graph by where they stand.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frontier {
    pub ready: Vec<String>,
    pub running: Vec<String>,
    /// A waiting node and what it waits for is in `Checkpoint::deadlines` and `resume_on`.
    pub waiting: Vec<String>,
}

/// The graph version a run started on: its number and the hash of its
/// graph (`versions::graph_hash`). A run keeps this however the drone is edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphRef {
    pub version: i64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub schema_version: u32,
    pub run_id: String,
    /// 0 for the first checkpoint, then one more each time.
    pub step: u32,
    pub state: StateSnapshot,
    pub frontier: Frontier,
    /// Iterations used per loop guard, by node id.
    pub loop_counters: BTreeMap<String, u32>,
    pub cost_usd: f64,
    pub elapsed_ms: u64,
    /// Wake-up times (ms epoch) of waiting nodes, by node id.
    pub deadlines: BTreeMap<String, i64>,
    /// The event key each waiting node resumes on, by node id.
    pub resume_on: BTreeMap<String, String>,
    pub graph: GraphRef,
}

/// What changed from one checkpoint to the next. Fields that did not change
/// are absent. Variables removed are listed in `removed_vars`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckpointDiff {
    pub schema_version: u32,
    pub run_id: String,
    pub from_step: u32,
    pub to_step: u32,
    #[serde(default, skip_serializing_if = "StateSnapshot::is_empty")]
    pub changed_state: StateSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_vars: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier: Option<Frontier>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub loop_counters: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadlines: Option<BTreeMap<String, i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_on: Option<BTreeMap<String, String>>,
}

impl StateSnapshot {
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty() && self.reducers.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// The bytes were written by a build that knows a newer format. `supported` is [`SCHEMA_VERSION`].
    #[error("checkpoint: format {found} is newer than this build reads ({supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    /// The bytes are not a checkpoint (not JSON, a field missing, a taint bit missing).
    #[error("checkpoint: unreadable: {0}")]
    Unreadable(String),
    /// A run has at most [`MAX_STEPS`] checkpoints: step `step` was refused.
    #[error("checkpoint: step {step} is over the limit of {max} steps")]
    TooManySteps { step: u32, max: u32 },
    /// A diff applied to a checkpoint it does not follow (another run, or `from_step` is not its step).
    #[error("checkpoint: the diff does not follow this checkpoint ({0})")]
    DoesNotFollow(String),
}

/// A checkpoint as bytes (UTF-8 JSON, so a person can read one in a bug report).
/// Refuses `step >= MAX_STEPS` with [`CheckpointError::TooManySteps`].
///
/// **Module B.** Not written yet.
pub fn encode(c: &Checkpoint) -> Result<Vec<u8>, CheckpointError> {
    let _ = c;
    todo!("module B: checkpoint::encode")
}

/// The checkpoint in `bytes`. A newer `schema_version` is
/// [`CheckpointError::UnsupportedVersion`] (checked before the rest is read);
/// anything else wrong is [`CheckpointError::Unreadable`]. A restored value
/// has exactly the bits it was saved with.
///
/// **Module B.** Not written yet.
pub fn decode(bytes: &[u8]) -> Result<Checkpoint, CheckpointError> {
    let _ = bytes;
    todo!("module B: checkpoint::decode")
}

/// What changed from `prev` to `next` (`next.step == prev.step + 1`, same run).
///
/// **Module B.** Not written yet.
pub fn diff(prev: &Checkpoint, next: &Checkpoint) -> Result<CheckpointDiff, CheckpointError> {
    let _ = (prev, next);
    todo!("module B: checkpoint::diff")
}

/// `prev` with `d` applied: equal to the `next` the diff was made from.
///
/// **Module B.** Not written yet.
pub fn apply(prev: &Checkpoint, d: &CheckpointDiff) -> Result<Checkpoint, CheckpointError> {
    let _ = (prev, d);
    todo!("module B: checkpoint::apply")
}

/// A small valid checkpoint, for tests and docs.
#[cfg(test)]
pub(crate) fn sample(step: u32) -> Checkpoint {
    use crate::drone::state::Reducer;
    use crate::drone::value::Tv;
    use serde_json::json;
    let mut state = StateSnapshot::default();
    state.vars.insert("title".into(), Tv::tainted(json!("fix the build")));
    state.vars.insert("attempts".into(), Tv::derived(json!(2)));
    state.vars.insert("owner".into(), Tv::clean(json!("agenty")));
    state.reducers.insert("attempts".into(), Reducer::Add);
    Checkpoint {
        schema_version: SCHEMA_VERSION,
        run_id: "run-1".into(),
        step,
        state,
        frontier: Frontier { ready: vec!["n3".into()], running: vec![], waiting: vec!["w1".into()] },
        loop_counters: BTreeMap::from([("guard".to_string(), 2)]),
        cost_usd: 0.25,
        elapsed_ms: 1_500,
        deadlines: BTreeMap::from([("w1".to_string(), 1_800_000_000_000)]),
        resume_on: BTreeMap::new(),
        graph: GraphRef { version: 3, hash: "abc".into() },
    }
}

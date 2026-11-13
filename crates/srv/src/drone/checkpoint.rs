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
pub fn encode(c: &Checkpoint) -> Result<Vec<u8>, CheckpointError> {
    if c.step >= MAX_STEPS {
        return Err(CheckpointError::TooManySteps { step: c.step, max: MAX_STEPS });
    }
    serde_json::to_vec(c).map_err(|error| CheckpointError::Unreadable(error.to_string()))
}

/// The checkpoint in `bytes`. A newer `schema_version` is
/// [`CheckpointError::UnsupportedVersion`] (checked before the rest is read);
/// anything else wrong is [`CheckpointError::Unreadable`]. A restored value
/// has exactly the bits it was saved with.
pub fn decode(bytes: &[u8]) -> Result<Checkpoint, CheckpointError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| CheckpointError::Unreadable(error.to_string()))?;
    // The version decides before any other field is looked at: bytes from a
    // newer build may be missing fields this build requires or hold ones it
    // does not know, and must not be misread as this format.
    let found = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CheckpointError::Unreadable("no numeric schema_version".to_string()))?;
    if found > u64::from(SCHEMA_VERSION) {
        return Err(CheckpointError::UnsupportedVersion {
            found: u32::try_from(found).unwrap_or(u32::MAX),
            supported: SCHEMA_VERSION,
        });
    }
    if found < u64::from(SCHEMA_VERSION) {
        return Err(CheckpointError::Unreadable(format!(
            "format {found} is older than this build reads ({SCHEMA_VERSION})"
        )));
    }
    // `Tv` requires both bits, so a value saved without one fails here instead
    // of coming back clean.
    serde_json::from_value(value).map_err(|error| CheckpointError::Unreadable(error.to_string()))
}

/// What changed from `prev` to `next` (`next.step == prev.step + 1`, same run).
pub fn diff(prev: &Checkpoint, next: &Checkpoint) -> Result<CheckpointDiff, CheckpointError> {
    if prev.run_id != next.run_id {
        return Err(CheckpointError::DoesNotFollow(format!(
            "run {:?} is not run {:?}",
            next.run_id, prev.run_id
        )));
    }
    if next.step != prev.step.checked_add(1).unwrap_or(u32::MAX) {
        return Err(CheckpointError::DoesNotFollow(format!(
            "step {} does not follow step {}",
            next.step, prev.step
        )));
    }
    if prev.graph != next.graph {
        // The graph is pinned when the run starts; a run never moves to another
        // graph, so a pair that does is not one step of one run.
        return Err(CheckpointError::DoesNotFollow("the graph ref changed inside the run".to_string()));
    }
    let mut d = CheckpointDiff {
        schema_version: next.schema_version,
        run_id: next.run_id.clone(),
        from_step: prev.step,
        to_step: next.step,
        ..CheckpointDiff::default()
    };
    d.changed_state = changed_state(&prev.state, &next.state);
    d.removed_vars = prev
        .state
        .vars
        .keys()
        .filter(|name| !next.state.vars.contains_key(*name))
        .cloned()
        .collect();
    if prev.frontier != next.frontier {
        d.frontier = Some(next.frontier.clone());
    }
    d.loop_counters = changed_counters(&prev.loop_counters, &next.loop_counters);
    if prev.cost_usd != next.cost_usd {
        d.cost_usd = Some(next.cost_usd);
    }
    if prev.elapsed_ms != next.elapsed_ms {
        d.elapsed_ms = Some(next.elapsed_ms);
    }
    if prev.deadlines != next.deadlines {
        d.deadlines = Some(next.deadlines.clone());
    }
    if prev.resume_on != next.resume_on {
        d.resume_on = Some(next.resume_on.clone());
    }
    Ok(d)
}

/// `prev` with `d` applied: equal to the `next` the diff was made from.
pub fn apply(prev: &Checkpoint, d: &CheckpointDiff) -> Result<Checkpoint, CheckpointError> {
    if d.run_id != prev.run_id {
        return Err(CheckpointError::DoesNotFollow(format!(
            "diff for run {:?} is not run {:?}",
            d.run_id, prev.run_id
        )));
    }
    if d.from_step != prev.step {
        return Err(CheckpointError::DoesNotFollow(format!(
            "diff from step {} is not step {}",
            d.from_step, prev.step
        )));
    }
    if d.to_step != d.from_step.checked_add(1).unwrap_or(u32::MAX) {
        return Err(CheckpointError::DoesNotFollow(format!(
            "diff goes from step {} to step {}",
            d.from_step, d.to_step
        )));
    }
    let mut out = prev.clone();
    out.schema_version = d.schema_version;
    out.step = d.to_step;
    for (name, value) in &d.changed_state.vars {
        out.state.vars.insert(name.clone(), value.clone());
    }
    for (name, reducer) in &d.changed_state.reducers {
        out.state.reducers.insert(name.clone(), *reducer);
    }
    // A variable's declaration goes with the variable: `removed_vars` names the
    // variable only, and a restore drops its declaration too. A declaration
    // made before the first write (a name with no value yet) is left alone.
    for name in &d.removed_vars {
        out.state.vars.remove(name);
        out.state.reducers.remove(name);
    }
    if let Some(frontier) = &d.frontier {
        out.frontier = frontier.clone();
    }
    out.loop_counters.extend(d.loop_counters.iter().map(|(node, count)| (node.clone(), *count)));
    if let Some(cost_usd) = d.cost_usd {
        out.cost_usd = cost_usd;
    }
    if let Some(elapsed_ms) = d.elapsed_ms {
        out.elapsed_ms = elapsed_ms;
    }
    if let Some(deadlines) = &d.deadlines {
        out.deadlines = deadlines.clone();
    }
    if let Some(resume_on) = &d.resume_on {
        out.resume_on = resume_on.clone();
    }
    Ok(out)
}

/// The variables and reducers that are new or different in `next`.
fn changed_state(prev: &StateSnapshot, next: &StateSnapshot) -> StateSnapshot {
    let mut changed = StateSnapshot::default();
    for (name, value) in &next.vars {
        if prev.vars.get(name) != Some(value) {
            changed.vars.insert(name.clone(), value.clone());
        }
    }
    for (name, reducer) in &next.reducers {
        if prev.reducers.get(name) != Some(reducer) {
            changed.reducers.insert(name.clone(), *reducer);
        }
    }
    changed
}

/// The loop counters that changed. A counter for a loop guard never disappears
/// while the run lasts (its graph is pinned), so an entry can only be added or
/// raised, and the changed entries alone are enough to restore the map.
fn changed_counters(prev: &BTreeMap<String, u32>, next: &BTreeMap<String, u32>) -> BTreeMap<String, u32> {
    next.iter()
        .filter(|(node, count)| prev.get(*node) != Some(*count))
        .map(|(node, count)| (node.clone(), *count))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::state::Reducer;
    use crate::drone::value::Tv;
    use serde_json::json;

    /// A checkpoint varied in the fields a diff carries, seeded so each seed
    /// differs in values, bits, reducers and the maps that can grow and shrink.
    fn varied(step: u32, seed: u32) -> Checkpoint {
        let mut c = sample(step);
        c.run_id = format!("run-{seed}");
        c.graph = GraphRef { version: i64::from(seed) + 1, hash: format!("hash-{seed}") };
        c.cost_usd = 0.25 + f64::from(seed) * 0.5;
        c.elapsed_ms = 1_500 + u64::from(seed) * 7;
        c.frontier.ready = (0..seed % 4).map(|i| format!("n{i}")).collect();
        c.frontier.waiting = if seed % 2 == 0 { vec![] } else { vec!["w1".into()] };
        c.loop_counters = (0..seed % 3).map(|i| (format!("g{i}"), i + seed)).collect();
        c.deadlines = (0..seed % 2).map(|i| (format!("d{i}"), i64::from(i))).collect();
        c.resume_on = (0..seed % 2).map(|i| (format!("r{i}"), format!("key{i}"))).collect();
        c.state.vars.clear();
        c.state.reducers.clear();
        for i in 0..=seed % 5 {
            let value = match i % 3 {
                0 => Tv::clean(json!({ "i": i })),
                1 => Tv::tainted(json!(["x", i])),
                _ => Tv::derived(json!(i)),
            };
            c.state.vars.insert(format!("v{i}"), value);
            if i % 2 == 0 {
                c.state.reducers.insert(format!("v{i}"), Reducer::Add);
            }
        }
        c
    }

    /// `prev` one step later: a value removed, one added, one changed and
    /// tainted, and each map grown.
    fn grown(prev: &Checkpoint, step: u32, seed: u32) -> Checkpoint {
        let mut next = prev.clone();
        next.step = step;
        next.cost_usd += 0.01;
        next.elapsed_ms += 5;
        next.frontier.ready.push(format!("n{step}"));
        if let Some(name) = prev.state.vars.keys().next().cloned() {
            next.state.vars.remove(&name);
            next.state.reducers.remove(&name);
        }
        next.state.vars.insert(format!("new{seed}"), Tv::derived(json!(seed)));
        if let Some(value) = next.state.vars.values_mut().next() {
            value.v = json!("changed");
            value.tainted = true;
            value.derived = false;
        }
        next.loop_counters.insert("g0".into(), step + seed);
        next.deadlines.insert(format!("d{step}"), i64::from(step));
        next.resume_on.insert(format!("r{seed}"), format!("k{step}"));
        next
    }

    #[test]
    fn a_round_trip_and_a_diff_keep_every_bit_and_reducer() {
        for seed in 0..12 {
            let prev = varied(1, seed);
            let next = grown(&prev, 2, seed);
            let bytes = encode(&prev).unwrap();
            let back = decode(&bytes).unwrap();
            assert_eq!(back, prev, "seed {seed}");
            for (name, value) in &prev.state.vars {
                assert_eq!((back.state.vars[name].tainted, back.state.vars[name].derived), (value.tainted, value.derived));
            }
            let d = diff(&prev, &next).unwrap();
            let restored = apply(&prev, &d).unwrap();
            assert_eq!(restored, next, "seed {seed}");
            for (name, value) in &next.state.vars {
                assert_eq!(restored.state.vars[name], *value, "the bits of {name} survive a diff");
            }
            // A decoded diff is the same diff: both ends survive a store.
            let bytes = serde_json::to_vec(&d).unwrap();
            let read: CheckpointDiff = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(apply(&prev, &read).unwrap(), next, "seed {seed}");
        }
    }

    #[test]
    fn a_removed_variable_takes_its_reducer_with_it() {
        let prev = varied(1, 3);
        let name = prev.state.vars.keys().next().unwrap().clone();
        let mut next = prev.clone();
        next.step = 2;
        next.state.vars.remove(&name);
        next.state.reducers.remove(&name);
        let d = diff(&prev, &next).unwrap();
        assert_eq!(d.removed_vars, vec![name.clone()]);
        assert!(!d.changed_state.reducers.contains_key(&name));
        let restored = apply(&prev, &d).unwrap();
        assert_eq!(restored.state.reducers.get(&name), None);
        assert_eq!(restored, next);
    }
}

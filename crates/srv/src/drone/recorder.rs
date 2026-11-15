// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Writing a run's checkpoints and reading them back
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3).
//!
//! The first checkpoint of a run is stored whole and each later one as the diff
//! from the one before it (`checkpoint::diff`), so a run of a thousand steps does
//! not store a thousand copies of its state. [`load`] folds the rows of a run
//! back into its newest checkpoint.

use std::sync::Arc;

use super::checkpoint::{self, Checkpoint, CheckpointDiff, CheckpointError, GraphRef};
use crate::backend::storage::drone_store::{CheckpointRow, DroneStore};
use crate::backend::storage::store::Store;

pub const KIND_FULL: &str = "full";
pub const KIND_DIFF: &str = "diff";

/// Where a run's checkpoints are kept: the store beside its run row, or memory in a test.
pub trait CheckpointSink: Send + Sync {
    fn put(&self, run_id: &str, step: u32, kind: &str, body: &[u8]) -> Result<(), String>;
}

impl CheckpointSink for Store {
    fn put(&self, run_id: &str, step: u32, kind: &str, body: &[u8]) -> Result<(), String> {
        self.drone_checkpoint_put(run_id, step, kind, body, agentmux_common::time::now_ms()).map_err(|e| e.to_string())
    }
}

/// What a run is given to keep its checkpoints: where, and the graph version
/// it started on.
#[derive(Clone)]
pub struct Persist {
    pub sink: Arc<dyn CheckpointSink>,
    pub graph: GraphRef,
}

/// Writes one run's checkpoints, a step at a time.
pub struct Recorder {
    sink: Arc<dyn CheckpointSink>,
    run_id: String,
    /// The newest checkpoint that is stored, which the next is a diff from.
    last: Option<Checkpoint>,
}

impl Recorder {
    pub fn new(sink: Arc<dyn CheckpointSink>, run_id: &str) -> Self {
        Self { sink, run_id: run_id.to_string(), last: None }
    }

    /// A recorder that goes on from `last`, the newest checkpoint stored for a run that is resuming.
    pub fn resuming(sink: Arc<dyn CheckpointSink>, last: Checkpoint) -> Self {
        Self { sink, run_id: last.run_id.clone(), last: Some(last) }
    }

    /// Go on from `last`, the newest checkpoint stored for the run, which is what a run taken up again starts from.
    pub fn resume_from(&mut self, last: Checkpoint) {
        self.last = Some(last);
    }

    /// The step the next checkpoint has.
    pub fn next_step(&self) -> u32 {
        self.last.as_ref().map_or(0, |c| c.step + 1)
    }

    /// Store `c`, whose `step` is [`next_step`](Self::next_step). When the store
    /// fails nothing is recorded as written, so the next try is a diff from the
    /// last checkpoint that did get stored.
    pub fn write(&mut self, c: Checkpoint) -> Result<(), RecordError> {
        let (kind, body) = match &self.last {
            None => (KIND_FULL, checkpoint::encode(&c)?),
            Some(prev) => {
                let d = checkpoint::diff(prev, &c)?;
                (KIND_DIFF, serde_json::to_vec(&d).map_err(|e| CheckpointError::Unreadable(e.to_string()))?)
            }
        };
        self.sink.put(&self.run_id, c.step, kind, &body).map_err(RecordError::Store)?;
        self.last = Some(c);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// The checkpoint could not be made (the ceiling of steps, or a pair that is not one step).
    #[error("{0}")]
    Checkpoint(#[from] CheckpointError),
    /// The store refused the write.
    #[error("the checkpoint could not be stored: {0}")]
    Store(String),
}

/// The newest checkpoint in `rows` (a run's rows, oldest step first): the first
/// is whole and each next one a diff from it. An empty list is `Ok(None)`; a
/// list that does not start whole, skips a step, or has a row that is unreadable
/// is an error, never a guess.
pub fn load(rows: &[CheckpointRow]) -> Result<Option<Checkpoint>, CheckpointError> {
    let mut at: Option<Checkpoint> = None;
    for row in rows {
        at = Some(match (&at, row.kind.as_str()) {
            (None, KIND_FULL) => checkpoint::decode(&row.body)?,
            (Some(prev), KIND_DIFF) => {
                let d: CheckpointDiff = serde_json::from_slice(&row.body).map_err(|e| CheckpointError::Unreadable(e.to_string()))?;
                checkpoint::apply(prev, &d)?
            }
            (None, other) => return Err(CheckpointError::Unreadable(format!("the first checkpoint is {other:?}, not whole"))),
            (Some(_), other) => return Err(CheckpointError::Unreadable(format!("step {} is {other:?}, not a diff", row.step))),
        });
    }
    Ok(at)
}

/// A sink in memory, for tests of what is written.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemorySink {
    pub(crate) rows: std::sync::Mutex<Vec<(String, CheckpointRow)>>,
    /// Writes to refuse, in order: `true` refuses that call.
    pub(crate) refuse: std::sync::Mutex<std::collections::VecDeque<bool>>,
}

#[cfg(test)]
impl MemorySink {
    pub(crate) fn rows_of(&self, run_id: &str) -> Vec<CheckpointRow> {
        self.rows.lock().unwrap().iter().filter(|(r, _)| r == run_id).map(|(_, row)| row.clone()).collect()
    }
}

#[cfg(test)]
impl CheckpointSink for MemorySink {
    fn put(&self, run_id: &str, step: u32, kind: &str, body: &[u8]) -> Result<(), String> {
        if self.refuse.lock().unwrap().pop_front().unwrap_or(false) {
            return Err("the disk is full".to_string());
        }
        self.rows.lock().unwrap().push((run_id.to_string(), CheckpointRow { step, kind: kind.to_string(), body: body.to_vec() }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::checkpoint::{sample, MAX_STEPS};

    fn step(n: u32) -> Checkpoint {
        let mut c = sample(n);
        c.cost_usd = f64::from(n);
        c.frontier.ready = vec![format!("n{n}")];
        c
    }

    #[test]
    fn the_first_checkpoint_is_whole_and_the_rest_are_diffs_that_load_back() {
        let sink = Arc::new(MemorySink::default());
        let mut r = Recorder::new(sink.clone(), "run-1");
        assert_eq!(r.next_step(), 0);
        for n in 0..4 {
            r.write(step(n)).unwrap();
        }
        let rows = sink.rows_of("run-1");
        assert_eq!(rows.iter().map(|r| (r.step, r.kind.as_str())).collect::<Vec<_>>(), [(0, "full"), (1, "diff"), (2, "diff"), (3, "diff")]);
        assert!(rows[1].body.len() < rows[0].body.len(), "a diff is smaller than the whole");
        assert_eq!(load(&rows).unwrap(), Some(step(3)));
        assert_eq!(load(&rows[..2]).unwrap(), Some(step(1)), "a run can be read as of an earlier step");
        assert_eq!(r.next_step(), 4);
    }

    #[test]
    fn a_refused_write_leaves_the_next_one_a_diff_from_what_was_stored() {
        let sink = Arc::new(MemorySink::default());
        let mut r = Recorder::new(sink.clone(), "run-1");
        r.write(step(0)).unwrap();
        sink.refuse.lock().unwrap().push_back(true);
        assert!(matches!(r.write(step(1)), Err(RecordError::Store(_))));
        assert_eq!(r.next_step(), 1, "the step number is not spent");
        r.write(step(1)).unwrap();
        assert_eq!(load(&sink.rows_of("run-1")).unwrap(), Some(step(1)));
    }

    #[test]
    fn a_run_stops_at_the_step_ceiling() {
        let sink = Arc::new(MemorySink::default());
        let mut r = Recorder::new(sink, "run-1");
        r.write(step(0)).unwrap();
        let mut last = step(0);
        for n in 1..MAX_STEPS {
            last = step(n);
            r.write(last.clone()).unwrap();
        }
        assert_eq!(last.step, MAX_STEPS - 1);
        assert!(matches!(
            r.write(step(MAX_STEPS)),
            Err(RecordError::Checkpoint(CheckpointError::TooManySteps { .. }))
        ));
    }

    #[test]
    fn rows_that_do_not_chain_are_an_error_not_a_guess() {
        let sink = Arc::new(MemorySink::default());
        let mut r = Recorder::new(sink.clone(), "run-1");
        for n in 0..3 {
            r.write(step(n)).unwrap();
        }
        let rows = sink.rows_of("run-1");
        assert_eq!(load(&[]).unwrap(), None);
        assert!(load(&rows[1..]).is_err(), "it must start whole");
        assert!(load(&[rows[0].clone(), rows[2].clone()]).is_err(), "a step may not be skipped");
        let mut torn = rows.clone();
        torn[1].body.truncate(5);
        assert!(load(&torn).is_err());
    }

    #[test]
    fn a_resuming_recorder_goes_on_from_the_last_checkpoint() {
        let sink = Arc::new(MemorySink::default());
        let mut first = Recorder::new(sink.clone(), "run-1");
        for n in 0..3 {
            first.write(step(n)).unwrap();
        }
        let stored = load(&sink.rows_of("run-1")).unwrap().unwrap();
        let mut again = Recorder::resuming(sink.clone(), stored);
        assert_eq!(again.next_step(), 3);
        again.write(step(3)).unwrap();
        assert_eq!(load(&sink.rows_of("run-1")).unwrap(), Some(step(3)));
    }
}

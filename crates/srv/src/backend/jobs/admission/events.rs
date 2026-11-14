// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Letting a run start for an event (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2, §3.7): the same
//! overlap, group, global-limit and spending-cap decisions a timer's slot gets, but a fire with no slot. What differs:
//!
//! - **At least once, deduplicated.** The job's `(job_id, event_id)` is recorded first (`db_job_trigger_events`); an event it
//!   already heard of starts nothing, here or in another instance sharing the ledger.
//! - **A fire that policy or a cap refuses is a `skipped` row**, folded into the job's newest skip of the same reason
//!   like a timer's, not a refusal: nobody pressed Run now, so there is nobody to tell.
//! - **Pause all** skips it the same way (`paused_all`).
//!
//! The event's data is not in the ledger. It waits in memory for the run that starts ([`RunLedger::put_event_input`]):
//! the Drone run keeps it as its input, and the ledger row keeps only what every run's row does.

use agentmux_common::time::now_ms;
use serde_json::Value;

use super::Admission;
use crate::backend::jobs::policy::Decision;
use crate::backend::jobs::{Begin, RunLedger, BY_EVENT, SOURCE_CRON, TRIGGER_EVENT};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{reason, state, JobRun};

/// The most event inputs held for runs that have not started. A run that never starts (its job was deleted while it
/// waited) leaves its input here until the process ends; the bound keeps a flood from growing it.
const MAX_HELD_INPUTS: usize = 1000;

impl RunLedger {
    /// Hold `input` (what the graph reads as `{{trigger.*}}`) for run `run_id`, until it starts.
    pub fn put_event_input(&self, run_id: &str, input: Value) {
        let mut held = self.event_inputs.lock().unwrap_or_else(|e| e.into_inner());
        if held.len() >= MAX_HELD_INPUTS {
            if let Some(oldest) = held.keys().next().cloned() {
                held.remove(&oldest);
            }
        }
        held.insert(run_id.to_string(), input);
    }

    /// The input held for run `run_id`, taken: a run starts once.
    pub fn take_event_input(&self, run_id: &str) -> Option<Value> {
        self.event_inputs.lock().unwrap_or_else(|e| e.into_inner()).remove(run_id)
    }

    /// Record the event `event_id` for `job` and decide whether its run starts. `Taken` is an event the job already
    /// heard of; `Skipped` is a fire that Pause all, overlap policy or a cap refused (it has its row).
    pub fn admit_event(&self, job: &CronJob, event_id: &str, source: &str) -> Admission {
        let _one_at_a_time = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        let id = uuid::Uuid::new_v4().to_string();
        match self.store.job_event_claim(&job.id, event_id, &id, now) {
            Ok(true) => {}
            Ok(false) => return Admission::Taken,
            Err(e) => {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record an event; firing it unrecorded");
                return Admission::Unrecorded;
            }
        }
        if self.paused_all().is_some() {
            return self.skip_event(job, id, source, reason::PAUSED_ALL);
        }
        let decision = self.decide(job, false);
        if decision == Decision::SkipOverlap {
            return self.skip_event(job, id, source, reason::OVERLAP);
        }
        let mut account = String::new();
        if matches!(decision, Decision::Start | Decision::CancelThenStart) {
            let gate = self.cap_gate(job);
            if let Some(refusal) = gate.refusal {
                if let Begin::Run(run_id) = self.begin_event(job, &id, source, now) {
                    self.skip_run_for_cap(&run_id, job, refusal);
                }
                return Admission::Skipped;
            }
            account = gate.account;
        }
        let run_id = match self.begin_event(job, &id, source, now) {
            Begin::Run(run_id) => run_id,
            Begin::SlotTaken => return Admission::Taken,
            Begin::Unrecorded => return Admission::Unrecorded,
        };
        self.note_account(&run_id, &account);
        match decision {
            Decision::Wait(why) => {
                if let Err(e) = self.store.job_run_set_wait(&run_id, why) {
                    tracing::warn!(run_id = %run_id, error = %e, "jobs: couldn't make an event's run wait");
                }
                self.changed(&job.id);
                self.ask_remote_stops(job, decision);
                Admission::Waiting(run_id, why)
            }
            Decision::CancelThenStart => {
                self.cancel_started(job);
                Admission::Run(run_id)
            }
            Decision::Start | Decision::SkipOverlap => Admission::Run(run_id),
        }
    }

    /// The `scheduled` row of an event's run, with the id it was recorded under.
    fn begin_event(&self, job: &CronJob, run_id: &str, source: &str, now: i64) -> Begin {
        let mut run = JobRun::new(run_id, &job.id, SOURCE_CRON, &job.name, TRIGGER_EVENT, state::SCHEDULED, now);
        run.recorded_by = self.recorded_by.clone();
        run.triggered_by = format!("{BY_EVENT}:{source}");
        run.target_agent_uid = job.target_uid.clone();
        match self.store.job_run_insert(&run) {
            Ok(true) => Begin::Run(run.id),
            Ok(false) => Begin::SlotTaken,
            Err(e) => {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record an event's run");
                Begin::Unrecorded
            }
        }
    }

    /// An event that is skipped for `why`: a `skipped` row, folded into the job's newest skip of the same reason.
    fn skip_event(&self, job: &CronJob, run_id: String, source: &str, why: &str) -> Admission {
        if let Begin::Run(id) = self.begin_event(job, &run_id, source, now_ms()) {
            if let Err(e) = self.store.job_run_skip(&id, why, 1, now_ms()) {
                tracing::warn!(run_id = %id, error = %e, "jobs: couldn't skip an event's run");
            }
        }
        self.changed(&job.id);
        Admission::Skipped
    }
}

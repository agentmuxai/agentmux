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
//! - **A trigger's `delay`** makes the run a `scheduled` row (`delay_wait`) due at event time + delay, picked up by the
//!   same wake that starts waiting runs; policy and the caps are asked when it is due, not when the event came.
//!
//! The event's data is not in the ledger row. It is kept beside it (`db_job_event_inputs`) for the run that starts: the
//! Drone run reads it as its input, a retry of the run takes it over, and a restart in between does not lose it.

use agentmux_common::time::now_ms;
use serde_json::Value;

use super::Admission;
use crate::backend::jobs::definition::MAX_EVENT_DELAY_MINUTES;
use crate::backend::jobs::policy::Decision;
use crate::backend::jobs::{Begin, RunLedger, BY_EVENT, SOURCE_CRON, TRIGGER_EVENT};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_events::{INPUT_KEEP_MS, KEEP_MS};
use crate::backend::storage::job_runs::{reason, state, JobRun};

impl RunLedger {
    /// The input kept for run `run_id` (what the graph reads as `{{trigger.*}}`). It stays while the run lives, so a
    /// run that waited through a delay, its turn or a restart, and a retry of it, read the same event.
    pub fn event_input(&self, run_id: &str) -> Option<Value> {
        self.store.job_event_input_get(run_id).unwrap_or_else(|e| {
            tracing::warn!(run_id, error = %e, "jobs: couldn't read an event's input");
            None
        })
    }

    /// The run is over for good (it was not retried): its input is no longer needed.
    pub(in crate::backend::jobs) fn drop_event_input(&self, run_id: &str) {
        if let Err(e) = self.store.job_event_input_drop(run_id) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't let go of an event's input");
        }
    }

    /// A retry `to` takes over what `from` held, so the next attempt reads the same event.
    pub(in crate::backend::jobs) fn carry_event_input(&self, from: &str, to: &str) {
        if let Err(e) = self.store.job_event_input_carry(from, to) {
            tracing::warn!(run_id = from, error = %e, "jobs: couldn't hand an event's input to its retry");
        }
    }

    /// Record the event `event_id` for `job` and decide whether its run starts. `Taken` is an event the job already
    /// heard of; `Skipped` is a fire that Pause all, overlap policy or a cap refused (it has its row).
    pub fn admit_event(&self, job: &CronJob, event_id: &str, source: &str, input: &Value) -> Admission {
        let _one_at_a_time = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        let id = uuid::Uuid::new_v4().to_string();
        match self.store.job_event_claim(&job.id, event_id, &id, now) {
            Ok(true) => {
                if let Err(e) = self.store.job_events_forget_before(now - KEEP_MS).and(self.store.job_event_inputs_forget_before(now - INPUT_KEEP_MS)) {
                    tracing::warn!(error = %e, "jobs: couldn't forget old event ids");
                }
            }
            Ok(false) => return Admission::Taken,
            Err(e) => {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record an event; firing it unrecorded");
                return Admission::Unrecorded;
            }
        }
        if self.paused_all().is_some() {
            return self.skip_event(job, id, source, reason::PAUSED_ALL);
        }
        let delay_ms = event_delay_ms(job);
        if delay_ms > 0 {
            return self.hold_for_delay(job, id, source, input, now, delay_ms);
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
        self.keep_event_input(&run_id, &job.id, input, now);
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

    /// The row of an event's run, `scheduled`, under the id the event was recorded with.
    fn event_row(&self, job: &CronJob, run_id: &str, source: &str, now: i64) -> JobRun {
        let mut run = JobRun::new(run_id, &job.id, SOURCE_CRON, &job.name, TRIGGER_EVENT, state::SCHEDULED, now);
        run.recorded_by = self.recorded_by.clone();
        run.triggered_by = format!("{BY_EVENT}:{source}");
        run.target_agent_uid = job.target_uid.clone();
        run
    }

    /// The `scheduled` row of an event's run, with the id it was recorded under.
    fn begin_event(&self, job: &CronJob, run_id: &str, source: &str, now: i64) -> Begin {
        let run = self.event_row(job, run_id, source, now);
        match self.store.job_run_insert(&run) {
            Ok(true) => Begin::Run(run.id),
            Ok(false) => Begin::SlotTaken,
            Err(e) => {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record an event's run");
                Begin::Unrecorded
            }
        }
    }

    /// An event whose trigger has a `delay`: its run waits as a `scheduled` row until `now + delay_ms`.
    fn hold_for_delay(&self, job: &CronJob, run_id: String, source: &str, input: &Value, now: i64, delay_ms: i64) -> Admission {
        let mut run = self.event_row(job, &run_id, source, now);
        run.reason = reason::DELAY_WAIT.to_string();
        run.links = serde_json::json!({ "due_at_ms": now + delay_ms });
        match self.store.job_run_insert(&run) {
            Ok(true) => {}
            Ok(false) => return Admission::Taken,
            Err(e) => {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record a delayed event's run");
                return Admission::Unrecorded;
            }
        }
        self.keep_event_input(&run_id, &job.id, input, now);
        self.changed(&job.id);
        Admission::Waiting(run_id, reason::DELAY_WAIT)
    }

    /// Keep the event's data for its run. A run that finds none when it starts fails saying so.
    fn keep_event_input(&self, run_id: &str, job_id: &str, input: &Value, now: i64) {
        if let Err(e) = self.store.job_event_input_put(run_id, job_id, input, now) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't keep an event's input");
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

/// The trigger's `delay` in ms: 0 for none.
fn event_delay_ms(job: &CronJob) -> i64 {
    let minutes = job.trigger_event.get("delay").and_then(Value::as_u64).unwrap_or(0);
    i64::try_from(minutes.min(u64::from(MAX_EVENT_DELAY_MINUTES))).unwrap_or(0) * 60_000
}

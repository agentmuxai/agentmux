// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Letting a run start under its job's policy
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1 "Policy"): overlap,
//! concurrency groups and the global limit when it wants to start, the
//! timeout while it runs, and the retry of an infrastructure failure when it
//! ends. The decisions are `policy.rs`; this reads who is running from the
//! ledger and writes the rows.
//!
//! One lock covers deciding and writing a run's row, so two fires in this
//! process can't both take the last place. Two instances sharing the ledger
//! can still both see room for one more for an instant; the limits are for
//! the providers' rate limits and for tidiness, not safety.

use std::sync::Arc;

use agentmux_common::time::now_ms;
use serde_json::json;

use super::policy::{self, Decision, Occupancy};
use super::{Begin, RunLedger, BY_SCHEDULER, SOURCE_CRON, TRIGGER_RETRY};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{infra, reason, state, JobRun};
use crate::backend::storage::store::Store;

/// Whether a disabled job is done rather than paused: a one-time job after its
/// time, or one that reached its fire count or age. Its last run may still be
/// waiting for a place, and must get one.
fn schedule_finished(job: &CronJob, now_ms: i64) -> bool {
    job.trigger_kind == "once"
        || job.max_fires.is_some_and(|max| job.fire_count >= max)
        || crate::backend::cron::is_expired_by_age(job.created_at, job.max_age_secs, now_ms / 1000)
}

/// What admitting a fire did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Start it: this is its run id.
    Run(String),
    /// Its row waits, for the reason.
    Waiting(String, &'static str),
    /// The slot is skipped (`overlap`) and has its row.
    Skipped,
    /// Run now, refused: the previous run is still going (policy `overlap`).
    Refused,
    /// Another scheduler has the slot.
    Taken,
    /// The row couldn't be written; fire anyway, unrecorded.
    Unrecorded,
}

/// A run another instance's scheduler might also be starting is only
/// started by whoever clears its waiting reason.
pub type Started = (JobRun, CronJob);

impl RunLedger {
    /// Where jobs are, for the policy of other jobs' runs.
    pub fn install_jobs(&self, jobs: Arc<Store>) {
        let _ = self.jobs.set(jobs);
    }

    /// Tests: how to interrupt the turn in a pane (a timeout, `cancel_other`).
    /// Otherwise the pane's controller is asked, as the Stop button does.
    #[cfg(test)]
    pub fn install_interrupt(&self, interrupt: super::BlockHook) {
        let _ = self.interrupt.set(interrupt);
    }

    /// How to stop a worker a `spawn` job started, once its run ended.
    pub fn install_worker_stop(&self, stop: super::BlockHook) {
        let _ = self.worker_stop.set(stop);
    }

    pub(super) fn job(&self, id: &str) -> Option<CronJob> {
        self.jobs.get()?.cron_get(id).ok().flatten()
    }

    fn interrupt_block(&self, block_id: &str) {
        if block_id.is_empty() {
            return;
        }
        match self.interrupt.get() {
            Some(f) => f(block_id),
            None => {
                let sigint = crate::backend::blockcontroller::BlockInputUnion::signal("SIGINT");
                if let Some(ctrl) = crate::backend::blockcontroller::get_controller(block_id) {
                    if let Err(e) = ctrl.send_input(sigint, None) {
                        tracing::warn!(block_id, error = %e, "jobs: couldn't interrupt a run's turn");
                    }
                }
            }
        }
    }

    /// Who is running, for the next run of `job`.
    fn occupancy(&self, job: &CronJob) -> Occupancy {
        let mut occ = Occupancy::default();
        let group = policy::group_of(job);
        let mut known: std::collections::HashMap<String, Option<CronJob>> = std::collections::HashMap::new();
        for r in self.store.job_runs_occupying().unwrap_or_default() {
            let other = known.entry(r.job_id.clone()).or_insert_with(|| self.job(&r.job_id)).clone();
            if r.job_id == job.id {
                occ.job_running += 1;
                occ.job_started += usize::from(r.state == state::RUNNING && !self.run_followed_elsewhere(&r));
            }
            if group.is_some() && other.as_ref().and_then(policy::group_of) == group {
                occ.group_running += 1;
            }
            if other.as_ref().is_none_or(policy::starts_an_agent) {
                occ.agents_running += 1;
            }
        }
        let waiting = self.store.job_runs_waiting().unwrap_or_default();
        let own = || waiting.iter().filter(|r| r.job_id == job.id);
        occ.job_buffered = own().filter(|r| r.reason == reason::OVERLAP).count();
        occ.job_blocked = own().filter(|r| r.reason == reason::GROUP_BUSY || r.reason == reason::GLOBAL_LIMIT).count();
        occ
    }

    /// Insert the row of a cron fire and decide whether it starts (see the
    /// module doc). A scheduled slot that policy skips is a collapsed
    /// `skipped (overlap)` row; Run now is refused.
    pub fn admit_cron(&self, job: &CronJob, trigger: &str, scheduled_for_ms: Option<i64>, triggered_by: &str) -> Admission {
        let _one_at_a_time = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let decision = policy::decide(job, &self.occupancy(job), false);
        if decision == Decision::SkipOverlap {
            return match scheduled_for_ms {
                Some(slot) => {
                    self.record_skip_slot(job, slot, reason::OVERLAP);
                    Admission::Skipped
                }
                None => Admission::Refused,
            };
        }
        let id = match self.begin_cron(job, trigger, scheduled_for_ms, triggered_by) {
            Begin::Run(id) => id,
            Begin::SlotTaken => return Admission::Taken,
            Begin::Unrecorded => return Admission::Unrecorded,
        };
        match decision {
            Decision::Wait(why) => {
                if let Err(e) = self.store.job_run_set_wait(&id, why) {
                    tracing::warn!(run_id = %id, error = %e, "jobs: couldn't make a run wait");
                }
                self.changed(&job.id);
                Admission::Waiting(id, why)
            }
            Decision::CancelThenStart => {
                self.cancel_started(job);
                Admission::Run(id)
            }
            Decision::Start | Decision::SkipOverlap => Admission::Run(id),
        }
    }

    /// `cancel_other`: stop the job's own running runs, interrupting their
    /// turns.
    fn cancel_started(&self, job: &CronJob) {
        for r in self.store.job_runs_occupying().unwrap_or_default() {
            if r.job_id == job.id && r.state == state::RUNNING {
                self.stop_run(&r, infra::INTERRUPTED, reason::OVERLAP, "stopped: the job's next run started (overlap: cancel_other)");
            }
        }
    }

    /// Close a running run that was stopped on purpose, and interrupt its
    /// turn. `false` when it had ended already.
    pub(crate) fn stop_run(&self, run: &JobRun, infra_status: &str, why: &str, summary: &str) -> bool {
        if self.run_followed_elsewhere(run) {
            return false;
        }
        match self.store.job_run_stop(&run.id, infra_status, why, summary, now_ms()) {
            Ok(true) => {
                self.interrupt_block(&run.block_id);
                self.reap_worker(&run.id);
                self.run_changed(&run.id);
                super::pending_wake().notify_one();
                true
            }
            Ok(false) => false,
            Err(e) => {
                tracing::warn!(run_id = %run.id, error = %e, "jobs: couldn't stop a run");
                false
            }
        }
    }

    /// Stop this channel's runs that ran past their job's `timeout_minutes`
    /// (infra `timeout`). Returns how many.
    pub fn enforce_timeouts(&self) -> usize {
        let now = now_ms();
        let mut stopped = 0;
        for r in self.store.job_runs_occupying().unwrap_or_default() {
            if r.state != state::RUNNING || !self.recorded_here(&r) {
                continue;
            }
            let Some(minutes) = self.job(&r.job_id).and_then(|j| j.policy.timeout_minutes) else { continue };
            if now - r.started_at_ms >= i64::from(minutes) * 60_000
                && self.stop_run(&r, infra::TIMEOUT, reason::TIMEOUT, &format!("stopped after {minutes} minutes (timeout_minutes)"))
            {
                stopped += 1;
            }
        }
        stopped
    }

    /// Whether another channel's instance follows `run`: its agent is there,
    /// so this process can't interrupt it. Closing the row from here would free
    /// its place while the agent goes on working, so such a run is left to end
    /// with its own turn, and counts as one `cancel_other` can't replace.
    fn run_followed_elsewhere(&self, run: &JobRun) -> bool {
        super::remote::followed_by(run).is_some_and(|f| f.channel != self.channel)
    }

    fn recorded_here(&self, run: &JobRun) -> bool {
        run.recorded_by == self.recorded_by || run.recorded_by.starts_with(&format!("{}@", self.channel))
    }

    /// Start the waiting runs that policy now lets start, oldest first, each
    /// against what is running after the ones before it. Runs of a job that
    /// is gone or paused end; a retry waits out its backoff first. The
    /// caller starts the returned runs.
    pub fn start_waiting(&self) -> Vec<Started> {
        let _one_at_a_time = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        if self.paused_all().is_some() {
            return Vec::new();
        }
        let now = now_ms();
        let mut out = Vec::new();
        for run in self.store.job_runs_waiting().unwrap_or_default() {
            // The ledger is shared by every instance, the jobs store is not: a
            // run another instance recorded is its to start or end.
            if !self.recorded_here(&run) {
                continue;
            }
            let Some(job) = self.job(&run.job_id) else {
                let _ = self.store.job_run_skip(&run.id, reason::TARGET_MISSING, 1, now);
                self.run_changed(&run.id);
                continue;
            };
            // A paused job's scheduled runs end; a Run now, asked for by a person, and
            // the last run of a job whose schedule is done still get their place.
            if !job.enabled && run.trigger != super::TRIGGER_MANUAL && !schedule_finished(&job, now) {
                let _ = self.store.job_run_skip(&run.id, reason::PAUSED, 1, now);
                self.changed(&job.id);
                continue;
            }
            if run.reason == reason::RETRY_WAIT && run.links["retry_at_ms"].as_i64().unwrap_or(0) > now {
                continue;
            }
            match policy::decide(&job, &self.occupancy(&job), true) {
                Decision::Start | Decision::CancelThenStart => {
                    if self.store.job_run_admit(&run.id, &run.reason, now).unwrap_or(false) {
                        self.cancel_started(&job);
                        out.push((run, job));
                    }
                }
                Decision::Wait(why) if why != run.reason => {
                    let _ = self.store.job_run_set_wait(&run.id, why);
                }
                Decision::Wait(_) | Decision::SkipOverlap => {}
            }
        }
        out
    }

    /// Whether a pending catch-up delivery of `job` may start now: the same
    /// answer a waiting run gets (overlap, group, the global limit). A job that
    /// would cancel its own running run does so here, as a new slot would.
    pub fn pending_may_start(&self, job: &CronJob) -> bool {
        let _one_at_a_time = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        match policy::decide(job, &self.occupancy(job), true) {
            Decision::Start => true,
            Decision::CancelThenStart => {
                self.cancel_started(job);
                true
            }
            Decision::Wait(_) | Decision::SkipOverlap => false,
        }
    }

    /// A run ended: if it failed in the infrastructure and its job has
    /// retries left, file the next attempt, waiting out the backoff (1, 5,
    /// then 15 minutes). The failed run is marked `retried_by`, so it isn't
    /// an incident of its own; the last attempt decides.
    pub(super) fn maybe_retry(&self, run_id: &str) {
        let Some(run) = self.store.job_run_get(run_id).ok().flatten() else { return };
        if run.job_source != SOURCE_CRON || !policy::infra_retryable(&run) || !run.links["retried_by"].is_null() {
            return;
        }
        let Some(job) = self.job(&run.job_id) else { return };
        if run.attempt > i64::from(job.policy.retries) {
            return;
        }
        let now = now_ms();
        let mut next = self.row(&job.id, SOURCE_CRON, &job.name, TRIGGER_RETRY, now);
        next.attempt = run.attempt + 1;
        next.state = state::SCHEDULED.to_string();
        next.reason = reason::RETRY_WAIT.to_string();
        next.triggered_by = BY_SCHEDULER.to_string();
        next.target_agent_uid = run.target_agent_uid.clone();
        next.links = json!({ "retry_of": run.id, "retry_at_ms": now + policy::retry_delay_ms(run.attempt) });
        match self.store.job_run_insert(&next) {
            Ok(true) => {
                let _ = self.store.job_run_add_links(&run.id, &json!({ "retried_by": next.id }));
                tracing::info!(job_id = %job.id, attempt = next.attempt, "jobs: an infrastructure failure will be retried");
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(run_id, error = %e, "jobs: couldn't file a retry"),
        }
    }

    /// A `task` run filed its task: the run is done, outcome `ok`, linked to
    /// the task (`links.kanban_task`, with its queue id).
    pub fn task_filed(&self, run_id: &str, job_name: &str, work_id: &str, num: i64) {
        let _ = self.store.job_run_add_links(run_id, &json!({ "kanban_task": num, "work_id": work_id }));
        let summary = format!("filed task #{num} for \"{job_name}\"");
        if let Err(e) = self.store.job_run_conclude(run_id, "ok", &summary, now_ms()) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't close a task run");
        }
        self.run_changed(run_id);
    }

    /// A `spawn` run ended: stop its worker, unless the job keeps it.
    pub(crate) fn reap_worker(&self, run_id: &str) {
        let Some(run) = self.store.job_run_get(run_id).ok().flatten() else { return };
        let worker = &run.links["worker"];
        let block_id = worker["block_id"].as_str().unwrap_or("");
        if block_id.is_empty() || worker["keep"].as_bool().unwrap_or(false) {
            return;
        }
        if let Some(stop) = self.worker_stop.get() {
            stop(block_id);
        }
    }
}

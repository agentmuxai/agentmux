// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The watchdog for cloud jobs (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.4).
//!
//! A scheduler can't report its own outage, so this compares what should have run with what the ledger has:
//!
//! - **A slot with no report.** A cloud job's expected slot plus its grace has passed and the ledger has no run for
//!   it: a `missed` row (`no_report`) takes the slot. A late event for that slot takes the row over and it reads
//!   "late" (`storage::job_cloud`), so a slot is one row whichever way it ends.
//! - **A run that stopped reporting.** A `started` run with no `finished` event, and no heartbeat, within the job's
//!   `maxDurationMinutes` becomes `lost`.
//!
//! Both are failures the incident keeper judges like any other (§3.4): several misses of one job are one incident,
//! one task and one alert. A pass runs every five minutes, and once after start-up, a little late so that what the
//! relay held while this computer was off has been handed over first.

use std::collections::BTreeSet;
use std::sync::Arc;

use agentmux_common::time::now_ms;

use super::{RunLedger, SOURCE_CLOUD, TRIGGER_SCHEDULE};
use crate::backend::cron::schedule::JobSchedule;
use crate::backend::job_templates::cloud::CloudExpect;
use crate::backend::job_templates::JobTemplates;
use crate::backend::storage::job_runs::{reason, state, JobRun};

const DAY_MS: i64 = 24 * 3600 * 1000;
/// Between two passes.
const PASS_MS: u64 = 5 * 60 * 1000;
/// Before the first pass.
const SETTLE_MS: u64 = 2 * 60 * 1000;
/// How far back a pass looks for slots without a report: an event older than this is refused anyway (§4.2), so a
/// slot given up on earlier could not turn late.
const LOOKBACK_MS: i64 = 7 * DAY_MS;
/// Slots read per job in a pass, newest first.
const MAX_SLOTS_LOOKED_AT: usize = 2000;
/// Misses written per job in a pass: the newest. A job that reports every minute and was cut off for a day is one
/// incident whatever the count, and its ledger is not filled with a day of rows.
const MAX_MISSES_PER_PASS: usize = 50;
/// A run that started this long before its slot, or a clock this far off, still belongs to the slot.
const SLOT_SLACK_MS: i64 = 60 * 1000;
/// How long a run of a job nobody approves any more may go without a report before it is `lost`.
const UNKNOWN_JOB_MAX_RUN_MS: i64 = DAY_MS;
/// Who gave up on a slot.
const BY_WATCHDOG: &str = "watchdog";

/// The schedule a cloud job reports on, and how long after a slot its report may be.
pub struct WatchedSchedule {
    pub schedule: JobSchedule,
    pub grace_ms: i64,
}

/// A cloud job as the watchdog needs it.
pub struct WatchedJob {
    pub id: String,
    pub name: String,
    /// When it was approved: no slot before this is a miss.
    pub since_ms: i64,
    /// `None` for a job that events start.
    pub schedule: Option<WatchedSchedule>,
    /// How long a run may go without a report before it is `lost`.
    pub max_run_ms: i64,
}

/// Where the cloud jobs to watch come from.
pub trait WatchedJobs {
    fn watched_jobs(&self) -> Vec<WatchedJob>;
}

/// The cloud templates the operator approved.
impl WatchedJobs for Arc<JobTemplates> {
    fn watched_jobs(&self) -> Vec<WatchedJob> {
        self.approved_clouds()
            .into_iter()
            .filter_map(|t| {
                let m = &t.manifest;
                let schedule = match m.expect.as_ref()? {
                    CloudExpect::Never(_) => None,
                    CloudExpect::Schedule(s) => {
                        Some(WatchedSchedule { schedule: JobSchedule::cron(&s.cron, &s.tz).ok()?, grace_ms: i64::from(s.grace_minutes) * 60_000 })
                    }
                };
                Some(WatchedJob {
                    id: t.id.clone(),
                    name: m.name.clone(),
                    since_ms: t.approved_at_ms,
                    schedule,
                    max_run_ms: i64::from(m.max_duration_minutes?) * 60_000,
                })
            })
            .collect()
    }
}

/// What a pass gave up on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Watched {
    pub missed: usize,
    pub lost: usize,
}

impl RunLedger {
    /// One pass of the watchdog over `jobs` at `now_ms`. Blocking. A job that gave up a slot or a run is told to the
    /// Jobs tab and the incident keeper. Running it again changes nothing.
    pub fn watch_cloud(&self, jobs: &[WatchedJob], now_ms: i64) -> Watched {
        let mut out = Watched::default();
        let mut touched: BTreeSet<&str> = BTreeSet::new();
        for job in jobs {
            let missed = self.give_up_slots(job, now_ms);
            out.missed += missed;
            if missed > 0 {
                touched.insert(&job.id);
            }
        }
        let running = match self.store().job_cloud_running() {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(target: "jobs", error = %e, "jobs: the watchdog couldn't read the running cloud runs");
                Vec::new()
            }
        };
        let mut lost_jobs = BTreeSet::new();
        for run in running {
            let max_run_ms = jobs.iter().find(|j| j.id == run.job_id).map_or(UNKNOWN_JOB_MAX_RUN_MS, |j| j.max_run_ms);
            if now_ms <= last_heard_from(&run) + max_run_ms {
                continue;
            }
            match self.store().job_cloud_lose(&run.id, now_ms, max_run_ms) {
                Ok(true) => {
                    out.lost += 1;
                    lost_jobs.insert(run.job_id);
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(target: "jobs", error = %e, run = %run.id, "jobs: the watchdog couldn't give up on a run"),
            }
        }
        for job_id in touched.into_iter().map(str::to_string).chain(lost_jobs) {
            self.changed(&job_id);
        }
        out
    }

    /// A `missed` row for each slot of `job` whose grace has passed with no run for it; how many it wrote.
    fn give_up_slots(&self, job: &WatchedJob, now_ms: i64) -> usize {
        let Some(watched) = &job.schedule else { return 0 };
        // Only slots after the job's newest row: what the ledger already shows is judged, and a backlog cut to the
        // newest misses is not filled in behind them by later passes.
        let newest_row = match self.store().job_run_last(&job.id) {
            Ok(last) => last.map_or(0, |r| r.started_at_ms),
            Err(e) => {
                tracing::warn!(target: "jobs", error = %e, job = %job.id, "jobs: the watchdog couldn't read a cloud job's runs");
                return 0;
            }
        };
        let after = job.since_ms.max(now_ms - LOOKBACK_MS).max(newest_row);
        // Newest first, so a long silence is cut to its newest misses and a job that reports every minute costs at most
        // `MAX_SLOTS_LOOKED_AT` reads however long it was silent.
        let mut uncovered = Vec::new();
        let mut before = now_ms - watched.grace_ms + 1;
        for _ in 0..MAX_SLOTS_LOOKED_AT {
            let Some(slot) = watched.schedule.prev_before(before) else { break };
            if slot <= after {
                break;
            }
            before = slot;
            // The slack is at most half the period, so two slots of a job that reports every minute don't share runs.
            let next = watched.schedule.next_after(slot).unwrap_or(slot + DAY_MS);
            let slack = SLOT_SLACK_MS.min((next - slot) / 2);
            let (from, until) = (slot - slack, next - slack);
            match self.store().job_cloud_slot_covered(&job.id, slot, from, until) {
                Ok(true) => {}
                Ok(false) => uncovered.push((slot, from, until)),
                Err(e) => {
                    tracing::warn!(target: "jobs", error = %e, job = %job.id, "jobs: the watchdog couldn't read a cloud job's runs");
                    return 0;
                }
            }
            if uncovered.len() == MAX_MISSES_PER_PASS {
                break;
            }
        }
        let mut written = 0;
        for (slot, from, until) in uncovered.into_iter().rev() {
            let mut run = JobRun::new(&format!("watchdog:{}:{slot}", job.id), &job.id, SOURCE_CLOUD, &job.name, TRIGGER_SCHEDULE, state::MISSED, slot);
            run.scheduled_for_ms = Some(slot);
            run.triggered_by = BY_WATCHDOG.to_string();
            run.reason = reason::NO_REPORT.to_string();
            run.ended_at_ms = Some(now_ms);
            run.recorded_by = self.recorded_by.clone();
            match self.store().job_cloud_give_up_slot(&run, from, until) {
                Ok(true) => written += 1,
                // A report came in since the slot was read, or another pass or channel gave up on it first.
                Ok(false) => {}
                Err(e) => tracing::warn!(target: "jobs", error = %e, job = %job.id, "jobs: the watchdog couldn't record a missed slot"),
            }
        }
        written
    }
}

/// When the run last reported: its start, or its latest heartbeat.
fn last_heard_from(run: &JobRun) -> i64 {
    run.links.get("heartbeat_ms").and_then(|v| v.as_i64()).unwrap_or(0).max(run.started_at_ms)
}

/// Start the watchdog: a first pass shortly after start-up, then one every five minutes. Needs a tokio runtime;
/// without one it does nothing.
pub fn start(ledger: Arc<RunLedger>, jobs: Arc<dyn WatchedJobs + Send + Sync>) {
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    rt.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(SETTLE_MS)).await;
        loop {
            let (ledger, jobs) = (ledger.clone(), jobs.clone());
            let _ = tokio::task::spawn_blocking(move || {
                let got = ledger.watch_cloud(&jobs.watched_jobs(), now_ms());
                if got != Watched::default() {
                    tracing::info!(target: "jobs", missed = got.missed, lost = got.lost, "jobs: the watchdog gave up on cloud runs");
                }
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(PASS_MS)).await;
        }
    });
}

#[cfg(test)]
#[path = "watchdog_tests.rs"]
mod tests;

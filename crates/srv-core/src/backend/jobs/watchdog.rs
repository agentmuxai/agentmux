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
//! - **A local slot nothing handled** (the second half of this file). A local job's slot passed while AgentMux was
//!   running and the ledger has no row for it, not even a skip: the job's scheduler task is gone or stalled. A
//!   `missed` row (`not_started`) takes the slot.
//!
//! Both are failures the incident keeper judges like any other (§3.4): several misses of one job are one incident,
//! one task and one alert. A pass runs every five minutes, and once after start-up, a little late so that what the
//! relay held while this computer was off has been handed over first.

use std::collections::BTreeSet;
use std::sync::Arc;

use agentmux_common::time::now_ms;

use super::{RunLedger, SOURCE_CLOUD, SOURCE_CRON, TRIGGER_SCHEDULE};
use crate::backend::cron::schedule::{JobSchedule, JITTER_MAX_MS};
use crate::backend::job_templates::cloud::CloudExpect;
use crate::backend::job_templates::JobTemplates;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{reason, state, JobRun};
use crate::backend::storage::store::Store;

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
            // A slot owns the runs from just before it to just before the next slot. Each boundary is worked out from
            // the two slots it separates (at most `SLOT_SLACK_MS` before the later, never past the middle), so one run
            // can't belong to two slots whatever the gaps between them are.
            let prev = watched.schedule.prev_before(slot).unwrap_or(slot - DAY_MS);
            let next = watched.schedule.next_after(slot).unwrap_or(slot + DAY_MS);
            let (from, until) = (slot - SLOT_SLACK_MS.min((slot - prev) / 2), next - SLOT_SLACK_MS.min((next - slot) / 2));
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
            // The window of start times that were this slot's: a late report that names no slot is matched by it.
            run.links = serde_json::json!({ "window_from": from, "window_until": until });
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

// ── Local jobs ──────────────────────────────────────────────────────────────────────────────────────────────

/// How long after its slot a local job may take to write its row: the scheduler fires up to `JITTER_MAX_MS` after a
/// slot and writes the row as it does, so a slot is only judged once this has passed.
const LOCAL_GRACE_MS: i64 = JITTER_MAX_MS + 3 * 60 * 1000;
/// A pass this much after the last means the computer was asleep or stopped: its scheduler tasks woke late and fire
/// what they can, so nothing before that pass is judged.
const SUSPEND_GAP_MS: i64 = 2 * PASS_MS as i64 + SETTLE_MS as i64;

/// What the watchdog remembers between passes about the local jobs it watches. Only what happened while it ran
/// counts: a slot from before it started belongs to the start-up catch-up (`app_not_running`), and one from before a
/// job was edited or turned on again belongs to the old schedule.
pub struct LocalWatch {
    started_ms: i64,
    last_pass_ms: Option<i64>,
    /// Per job: its schedule when last seen, and the time after which its slots are judged.
    seen: std::collections::HashMap<String, (String, i64)>,
}

impl LocalWatch {
    pub fn new(started_ms: i64) -> Self {
        Self { started_ms, last_pass_ms: None, seen: std::collections::HashMap::new() }
    }
}

/// Everything about a job that decides its slots.
fn schedule_signature(job: &CronJob) -> String {
    format!("{}|{}|{}|{:?}|{:?}|{}", job.trigger_kind, job.expression, job.tz, job.every_secs, job.once_at_ms, job.created_at)
}

impl RunLedger {
    /// One pass over the enabled local jobs `jobs` at `now_ms`: a `missed` row (`not_started`) for each slot past its
    /// grace with no row of any kind. Returns how many it wrote. Blocking; running it again changes nothing.
    pub fn watch_local(&self, watch: &mut LocalWatch, jobs: &[CronJob], now_ms: i64) -> usize {
        let first_pass = watch.last_pass_ms.is_none();
        let resumed = watch.last_pass_ms.is_some_and(|last| now_ms - last > SUSPEND_GAP_MS);
        watch.last_pass_ms = Some(now_ms);
        let live: BTreeSet<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
        // A job that is paused, deleted or done is forgotten, so its slots after it comes back are judged from then.
        watch.seen.retain(|id, _| live.contains(id.as_str()));
        let mut written = 0;
        for job in jobs {
            let Ok(schedule) = JobSchedule::of(job) else { continue };
            if !matches!(schedule, JobSchedule::Cron { .. } | JobSchedule::Every { .. }) {
                continue;
            }
            let sig = schedule_signature(job);
            let baseline = match watch.seen.get_mut(&job.id) {
                Some(seen) => {
                    if seen.0 != sig || resumed {
                        // Edited, or the computer slept: judge only what comes after this pass.
                        *seen = (sig, now_ms);
                    }
                    seen.1
                }
                None => {
                    // At start-up a job's slots count from the start; a job that appears later, or comes back, from now.
                    let from = if first_pass { watch.started_ms } else { now_ms };
                    watch.seen.insert(job.id.clone(), (sig, from));
                    from
                }
            };
            let missed = self.give_up_local_slots(job, &schedule, baseline, now_ms);
            if missed > 0 {
                written += missed;
                self.changed(&job.id);
            }
            if let Some(seen) = watch.seen.get_mut(&job.id) {
                // What was judged is not looked at again.
                seen.1 = seen.1.max(now_ms - LOCAL_GRACE_MS);
            }
        }
        written
    }

    /// A `missed` row for each slot of `job` in `(baseline_ms, now_ms - grace]` with no row; how many were written.
    fn give_up_local_slots(&self, job: &CronJob, schedule: &JobSchedule, baseline_ms: i64, now_ms: i64) -> usize {
        let until = now_ms - LOCAL_GRACE_MS;
        let mut uncovered = Vec::new();
        for slot in schedule.after(baseline_ms).take(MAX_SLOTS_LOOKED_AT) {
            if slot > until {
                break;
            }
            match self.store().job_run_slot_taken(&job.id, slot) {
                Ok(true) => {}
                Ok(false) => uncovered.push(slot),
                Err(e) => {
                    tracing::warn!(target: "jobs", error = %e, job = %job.id, "jobs: the watchdog couldn't read a local job's runs");
                    return 0;
                }
            }
        }
        // The newest misses, as for a cloud job: a job that missed a day of minutes is one incident either way.
        let skip = uncovered.len().saturating_sub(MAX_MISSES_PER_PASS);
        let mut written = 0;
        for slot in uncovered.into_iter().skip(skip) {
            let mut run = JobRun::new(&format!("watchdog:{}:{slot}", job.id), &job.id, SOURCE_CRON, &job.name, TRIGGER_SCHEDULE, state::MISSED, slot);
            run.scheduled_for_ms = Some(slot);
            run.triggered_by = BY_WATCHDOG.to_string();
            run.reason = reason::NOT_STARTED.to_string();
            run.target_agent_uid = job.target_uid.clone();
            run.ended_at_ms = Some(now_ms);
            run.recorded_by = self.recorded_by.clone();
            // The slot's unique index is the lease: a row the scheduler wrote since the read above wins.
            match self.store().job_run_insert(&run) {
                Ok(true) => written += 1,
                Ok(false) => {}
                Err(e) => tracing::warn!(target: "jobs", error = %e, job = %job.id, "jobs: the watchdog couldn't record a missed local slot"),
            }
        }
        written
    }
}

/// Start the local half of the watchdog over the cron jobs in `shared`: a first pass shortly after start-up, then one
/// every five minutes. Needs a tokio runtime; without one it does nothing.
pub fn start_local(ledger: Arc<RunLedger>, shared: Arc<Store>) {
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    rt.spawn(async move {
        let mut watch = LocalWatch::new(now_ms());
        tokio::time::sleep(std::time::Duration::from_millis(SETTLE_MS)).await;
        loop {
            let (ledger, shared) = (ledger.clone(), shared.clone());
            let moved = tokio::task::spawn_blocking(move || {
                let missed = match shared.cron_list_enabled() {
                    Ok(jobs) => ledger.watch_local(&mut watch, &jobs, now_ms()),
                    Err(e) => {
                        tracing::warn!(target: "jobs", error = %e, "jobs: the watchdog couldn't list the cron jobs");
                        0
                    }
                };
                (watch, missed)
            })
            .await;
            match moved {
                Ok((back, missed)) => {
                    watch = back;
                    if missed > 0 {
                        tracing::info!(target: "jobs", missed, "jobs: the watchdog gave up on local slots nothing started");
                    }
                }
                Err(_) => return,
            }
            tokio::time::sleep(std::time::Duration::from_millis(PASS_MS)).await;
        }
    });
}

#[cfg(test)]
#[path = "watchdog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "watchdog_local_tests.rs"]
mod local_tests;

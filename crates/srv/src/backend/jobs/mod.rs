// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Jobs: things that run by themselves, and the one run ledger under them
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3).
//!
//! [`RunLedger`] writes `db_job_runs` rows (`storage::job_runs`) for cron
//! fires and Drone runs, follows a delivered fire's turn to its end
//! ([`follower`]), and on startup closes the runs a restart interrupted.
//! Cron fires are never held for a stopped agent: an undelivered fire is
//! `missed`, or, under catch-up `latest`, the job's one `pending` fire (§3.2,
//! §5.2), which the scheduler delivers when the agent is back.

pub mod follower;

use std::sync::Arc;

use agentmux_common::time::now_ms;
use serde_json::json;

use crate::backend::cron::{FireOutcome, MissReason};
use crate::backend::mps::Broker;
use crate::backend::storage::job_runs::{infra, reason, state, JobRun, RunCost};
use crate::backend::storage::store::Store;
use crate::drone::storage::DroneStore;

/// Job kinds in `job_source` (§3.1).
pub const SOURCE_CRON: &str = "cron";
pub const SOURCE_DRONE: &str = "drone";

/// Run triggers in `trigger` (§3.1).
pub const TRIGGER_SCHEDULE: &str = "schedule";
pub const TRIGGER_CATCH_UP: &str = "catch_up";
pub const TRIGGER_MANUAL: &str = "manual";

/// `triggered_by` for runs the scheduler starts, and for the operator.
pub const BY_SCHEDULER: &str = "scheduler";
pub const BY_OPERATOR: &str = "operator";

/// A cron job's catch-up policy (§5.2): what an undelivered fire leaves.
pub const CATCH_UP_SKIP: &str = "skip";
pub const CATCH_UP_LATEST: &str = "latest";

/// The status a Drone run left `running` by a restart gets in
/// `db_drone_runs` (§3.2). An open string there, like every other status.
pub const DRONE_STATUS_INTERRUPTED: &str = "interrupted";

/// The channel and version that write rows here, `<channel>@<version>`.
/// Startup cleanup and the pending-fire delivery only touch rows their own
/// channel wrote: the ledger is shared by every instance on the computer.
pub fn recorded_by() -> String {
    format!("{}@{}", crate::backend::reactive::registry::local_channel_id(), env!("CARGO_PKG_VERSION"))
}

/// Woken whenever an agent registers (`reactive::registered`), so a pending
/// fire reaches its agent as soon as it is back.
pub fn pending_wake() -> &'static tokio::sync::Notify {
    static WAKE: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);
    &WAKE
}

/// The line a fire's prompt ends with: the run id, and how to report (§3.2).
/// A late fire (a catch-up, a pending fire delivered when its agent came
/// back) also says which slot it was for and when it started, so the agent
/// can tell its work is late. Plain text: srv rewrites `[AgentMux]` markers
/// in anything forwarded.
pub fn brief_line(run_id: &str, late_for_ms: Option<i64>, now_ms: i64) -> String {
    let late = late_for_ms
        .and_then(|slot| {
            let slot = chrono::DateTime::from_timestamp_millis(slot)?;
            let now = chrono::DateTime::from_timestamp_millis(now_ms)?;
            Some(format!(
                " It was due {} and started {} (late).",
                slot.format("%Y-%m-%d %H:%M UTC"),
                now.format("%Y-%m-%d %H:%M UTC")
            ))
        })
        .unwrap_or_default();
    format!(
        "(Scheduled run {run_id}.{late} When it is done, say how it went: Schedule action report with run_id \
         \"{run_id}\", outcome ok, findings or failed, and a one-line summary.)"
    )
}

/// What inserting a cron slot's row answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begin {
    /// The slot is ours; this is its run id.
    Run(String),
    /// Another scheduler sharing the job has this slot: don't fire.
    SlotTaken,
    /// The row couldn't be written (logged): fire anyway, unrecorded.
    Unrecorded,
}

/// The run ledger, shared by the cron scheduler, the Drone handlers and the
/// Schedule routes.
pub struct RunLedger {
    store: Arc<Store>,
    /// The channel's object store: a pane's meta (its account) and agent
    /// definitions. `None` in tests that don't need them.
    mstore: Option<Arc<Store>>,
    recorded_by: String,
    follower: follower::Follower,
}

impl RunLedger {
    /// A ledger over `store` (the identity store), following turns on
    /// `broker`. Needs a tokio runtime (the follower's task).
    pub fn new(store: Arc<Store>, mstore: Option<Arc<Store>>, broker: Arc<Broker>) -> Arc<Self> {
        if let Err(e) = store.job_runs_ensure_schema() {
            tracing::warn!(error = %e, "jobs: couldn't create the run ledger table");
        }
        Arc::new_cyclic(|weak: &std::sync::Weak<RunLedger>| RunLedger {
            store: store.clone(),
            mstore,
            recorded_by: recorded_by(),
            follower: follower::Follower::start(weak.clone(), broker),
        })
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    fn row(&self, job_id: &str, job_source: &str, job_name: &str, trigger: &str, now: i64) -> JobRun {
        let mut run =
            JobRun::new(&uuid::Uuid::new_v4().to_string(), job_id, job_source, job_name, trigger, state::SCHEDULED, now);
        run.recorded_by = self.recorded_by.clone();
        run
    }

    /// Insert the `scheduled` row of a cron fire before firing it. For a
    /// slot, that insert is the lease (§3.1): a second scheduler sharing
    /// the job finds the slot taken.
    pub fn begin_cron(
        &self,
        job: &crate::backend::storage::cron::CronJob,
        trigger: &str,
        scheduled_for_ms: Option<i64>,
        triggered_by: &str,
    ) -> Begin {
        let mut run = self.row(&job.id, SOURCE_CRON, &job.name, trigger, now_ms());
        run.scheduled_for_ms = scheduled_for_ms;
        run.triggered_by = triggered_by.to_string();
        run.target_agent_uid = job.target_uid.clone();
        match self.store.job_run_insert(&run) {
            Ok(true) => Begin::Run(run.id),
            Ok(false) => Begin::SlotTaken,
            Err(e) => {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record a cron run");
                Begin::Unrecorded
            }
        }
    }

    /// Record what a cron fire's delivery decided for run `run_id` of
    /// `job`. A scheduled fire also ends the job's older pending fire: it
    /// is superseded when this one is pending in its place, else expired
    /// (its next fire time came).
    pub fn record_delivery(&self, run_id: &str, job: &crate::backend::storage::cron::CronJob, outcome: &FireOutcome, scheduled: bool) {
        let now = now_ms();
        let latest = job.catch_up == CATCH_UP_LATEST;
        let result = match outcome {
            FireOutcome::Delivered { block_id } => {
                self.mark_running(run_id, &job.target_uid, block_id.as_deref().unwrap_or(""), now);
                Ok(())
            }
            FireOutcome::Missed { reason: MissReason::TargetStopped } => {
                let st = if latest && scheduled { state::PENDING } else { state::MISSED };
                self.store.job_run_set_undelivered(run_id, st, reason::TARGET_STOPPED, now).map(drop)
            }
            FireOutcome::Missed { reason: MissReason::TargetMissing } => {
                self.store.job_run_skip(run_id, reason::TARGET_MISSING, 1, now).map(drop)
            }
            FireOutcome::Failed { error } => self.store.job_run_finish(run_id, infra::ERROR, None, error, now).map(drop),
        };
        if let Err(e) = result {
            tracing::warn!(run_id, error = %e, "jobs: couldn't record a fire's delivery");
        }
        if scheduled {
            let now_pending = self.store.job_run_get(run_id).ok().flatten().is_some_and(|r| r.state == state::PENDING && r.ended_at_ms.is_none());
            let why = if now_pending { reason::SUPERSEDED } else { reason::EXPIRED };
            if let Err(e) = self.store.job_runs_end_pending(&job.id, run_id, why, now) {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't end an older pending fire");
            }
        }
    }

    /// A pending fire was delivered, or failed for good, when its agent came
    /// back (`CronScheduler::deliver_pending`). Still missing keeps it
    /// pending.
    pub fn record_pending_delivery(&self, run: &JobRun, outcome: &FireOutcome) {
        let now = now_ms();
        let result = match outcome {
            FireOutcome::Delivered { block_id } => {
                self.mark_running(&run.id, &run.target_agent_uid, block_id.as_deref().unwrap_or(""), now);
                Ok(())
            }
            FireOutcome::Missed { .. } => Ok(()),
            FireOutcome::Failed { error } => self.store.job_run_finish(&run.id, infra::ERROR, None, error, now).map(drop),
        };
        if let Err(e) = result {
            tracing::warn!(run_id = %run.id, error = %e, "jobs: couldn't record a pending fire's delivery");
        }
    }

    /// End a pending fire whose next fire time passed, or whose job is gone.
    pub fn expire_pending(&self, run_id: &str) {
        if let Err(e) = self.store.job_run_end_one_pending(run_id, reason::EXPIRED, now_ms()) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't expire a pending fire");
        }
    }

    /// This channel's pending fires still waiting for their agent.
    pub fn waiting_here(&self) -> Vec<JobRun> {
        let channel = crate::backend::reactive::registry::local_channel_id();
        self.store
            .job_runs_open_recorded_by(&channel, SOURCE_CRON, &[state::PENDING])
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.ended_at_ms.is_none())
            .collect()
    }

    /// A run reached its agent: running from now, with the pane's account,
    /// and its turn followed to the end.
    fn mark_running(&self, run_id: &str, target_uid: &str, block_id: &str, now: i64) {
        let (account, provider) = self.pane_account(block_id);
        if let Err(e) = self.store.job_run_mark_running(run_id, target_uid, block_id, &account, &provider, now) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't mark a run running");
            return;
        }
        if block_id.is_empty() {
            return;
        }
        let _ = self.store.job_run_add_links(run_id, &json!({ "transcript": { "block_id": block_id } }));
        self.follower.arm(block_id, run_id, now);
    }

    /// The account a pane runs on (`agent:run_account`) and its provider.
    fn pane_account(&self, block_id: &str) -> (String, String) {
        let Some(mstore) = &self.mstore else { return Default::default() };
        if block_id.is_empty() {
            return Default::default();
        }
        let Ok(Some(block)) = mstore.get::<crate::backend::obj::Block>(block_id) else {
            return Default::default();
        };
        let account = crate::backend::obj::meta_get_string(
            &block.meta,
            crate::server::app_api::agent_spawn::RUN_ACCOUNT_META_KEY,
            "",
        );
        let agent = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
        let provider = match mstore.agent_def_get(&agent) {
            Ok(Some(def)) if !agent.is_empty() => crate::backend::providers::resolve_provider_alias(&def.provider).to_string(),
            _ => String::new(),
        };
        (account, provider)
    }

    /// Write the collapsed skip for slots missed while AgentMux was off
    /// (`app_not_running`, §3.2): `count` slots from `first_slot_ms`.
    pub fn record_missed_while_off(&self, job: &crate::backend::storage::cron::CronJob, first_slot_ms: i64, count: i64) {
        if count <= 0 {
            return;
        }
        let Begin::Run(id) = self.begin_cron(job, TRIGGER_SCHEDULE, Some(first_slot_ms), BY_SCHEDULER) else {
            return;
        };
        if let Err(e) = self.store.job_run_skip(&id, reason::APP_NOT_RUNNING, count, now_ms()) {
            tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record missed slots");
        }
    }

    /// The followed turn ended (or never showed up).
    pub(crate) fn turn_finished(&self, run_id: &str, infra_status: &str, cost: Option<RunCost>, summary: &str) {
        if let Err(e) = self.store.job_run_finish(run_id, infra_status, cost, summary, now_ms()) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't close a run");
        }
    }

    pub(crate) fn turn_bound(&self, run_id: &str, turn_id: i64, partial: bool) {
        if let Err(e) = self.store.job_run_bind_turn(run_id, turn_id, partial) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't record a run's turn");
        }
    }

    /// A Drone run started: its `manual` row, running, linked to the run.
    pub fn drone_started(&self, drone_run_id: &str, drone_id: &str, drone_name: &str, started_at_ms: i64) -> Option<String> {
        let mut run = self.row(drone_id, SOURCE_DRONE, drone_name, TRIGGER_MANUAL, started_at_ms);
        run.state = state::RUNNING.to_string();
        run.triggered_by = BY_OPERATOR.to_string();
        run.links = json!({
            "drone_run_id": drone_run_id,
            "channel": crate::backend::reactive::registry::local_channel_id(),
        });
        match self.store.job_run_insert(&run) {
            Ok(true) => Some(run.id),
            Ok(false) => None,
            Err(e) => {
                tracing::warn!(drone_id, error = %e, "jobs: couldn't record a Drone run");
                None
            }
        }
    }

    /// A Drone run ended: `ok` and the summed cost of its Agent blocks, or
    /// `error` with its message.
    pub fn drone_finished(
        &self,
        run_id: &str,
        ok: bool,
        block_states: &std::collections::HashMap<String, crate::drone::types::BlockState>,
        error: &str,
    ) {
        let infra_status = if ok { infra::OK } else { infra::ERROR };
        self.turn_finished(run_id, infra_status, Some(drone_cost(block_states)), error);
    }

    /// After a restart: close what this channel left open (§3.2). Cron runs
    /// still `scheduled` or `running`, and Drone runs still `running`, were
    /// interrupted, in the ledger and in `db_drone_runs`. Pending fires stay:
    /// they wait for their agent across a restart. Call before the scheduler
    /// starts.
    pub fn recover_after_restart(&self) -> usize {
        let channel = crate::backend::reactive::registry::local_channel_id();
        let now = now_ms();
        let open = |source: &str, states: &[&str]| {
            self.store.job_runs_open_recorded_by(&channel, source, states).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "jobs: couldn't read open runs");
                Vec::new()
            })
        };
        let mut closed = 0;
        let stale = open(SOURCE_CRON, &[state::SCHEDULED, state::RUNNING]).into_iter().chain(open(SOURCE_DRONE, &[state::RUNNING]));
        for run in stale {
            if self.store.job_run_finish(&run.id, infra::INTERRUPTED, None, "AgentMux restarted during this run", now).unwrap_or(false) {
                closed += 1;
            }
        }
        if let Some(mstore) = &self.mstore {
            match mstore.drone_runs_mark_interrupted(DRONE_STATUS_INTERRUPTED, now) {
                Ok(n) if n > 0 => tracing::info!(count = n, "jobs: Drone runs interrupted by the restart marked"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "jobs: couldn't mark interrupted Drone runs"),
            }
        }
        if closed > 0 {
            tracing::info!(count = closed, "jobs: closed runs left open by the restart");
        }
        closed
    }
}

/// The summed `cost_usd` of a Drone run's blocks: only Agent blocks report
/// one, in their output (`drone/executor/blocks/agent.rs`).
pub fn drone_cost(block_states: &std::collections::HashMap<String, crate::drone::types::BlockState>) -> RunCost {
    let mut cost = RunCost { cost_known: true, ..Default::default() };
    for b in block_states.values() {
        let Some(out) = &b.output else { continue };
        if let Some(c) = out.get("cost_usd").and_then(|v| v.as_f64()) {
            cost.cost_usd += c;
        }
        if let Some(t) = out.pointer("/tokens/output").and_then(|v| v.as_i64()) {
            cost.output_tokens += t;
        }
    }
    cost
}

/// Missed cron slots after `base_ms` and before `now_ms`: the newest, and
/// the earlier ones as `(first, count)`. Counting stops at `cap` slots so a
/// one-minute job off for a year costs bounded work.
pub fn missed_slots(expression: &str, base_ms: i64, now_ms: i64, cap: usize) -> Option<(i64, Option<(i64, i64)>)> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(&format!("0 {expression}")).ok()?;
    let base = chrono::DateTime::from_timestamp_millis(base_ms)?;
    let mut first = None;
    let mut newest = None;
    let mut count: i64 = 0;
    for t in schedule.after(&base).take(cap) {
        let ms = t.timestamp_millis();
        if ms >= now_ms {
            break;
        }
        first.get_or_insert(ms);
        newest = Some(ms);
        count += 1;
    }
    let newest = newest?;
    let earlier = (count > 1).then(|| (first.unwrap_or(newest), count - 1));
    Some((newest, earlier))
}

/// The job's next fire time after `slot_ms`: when a pending fire for that
/// slot expires (§5.2).
pub fn next_slot_after(expression: &str, slot_ms: i64) -> Option<i64> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(&format!("0 {expression}")).ok()?;
    let base = chrono::DateTime::from_timestamp_millis(slot_ms)?;
    schedule.after(&base).next().map(|t| t.timestamp_millis())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

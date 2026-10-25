// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Jobs: things that run by themselves, and the one run ledger under them
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3).
//!
//! [`RunLedger`] writes `db_job_runs` rows (`storage::job_runs`) for cron
//! fires and Drone runs, follows a delivered fire's turn to its end
//! ([`follower`]), and on startup closes the runs a restart interrupted.
//! After each change it nudges [`incidents`], which opens one incident per
//! failing job (§3.4).
//! Cron fires are never held for a stopped agent: an undelivered fire is
//! `missed`, or, under catch-up `latest`, the job's one `pending` fire (§3.2,
//! §5.2), which the scheduler delivers when the agent is back.

pub mod admission;
pub mod definition;
pub mod follower;
pub mod incidents;
pub mod paused;
pub mod policy;
pub mod remote;
pub mod verdict;

use std::sync::Arc;

use agentmux_common::time::now_ms;
use serde_json::json;

use crate::backend::cron::{FireOutcome, MissReason};
use crate::backend::mps::Broker;
use crate::backend::storage::job_runs::{infra, reason, state, JobFlag, JobRun, RunCost};
use crate::backend::storage::store::Store;
use crate::drone::storage::DroneStore;

/// Job kinds in `job_source` (§3.1).
pub const SOURCE_CRON: &str = "cron";
pub const SOURCE_DRONE: &str = "drone";

/// Run triggers in `trigger` (§3.1).
pub const TRIGGER_SCHEDULE: &str = "schedule";
pub const TRIGGER_CATCH_UP: &str = "catch_up";
pub const TRIGGER_MANUAL: &str = "manual";
pub const TRIGGER_RETRY: &str = "retry";

/// `triggered_by` for runs the scheduler starts, and for the operator.
pub const BY_SCHEDULER: &str = "scheduler";
pub const BY_OPERATOR: &str = "operator";

/// A cron job's catch-up policy (§5.2): what an undelivered fire leaves.
pub const CATCH_UP_SKIP: &str = "skip";
pub const CATCH_UP_LATEST: &str = "latest";

/// The identity-store switch Pause all sets (§3.3): while it is on, every
/// scheduled fire is a collapsed `skipped (paused_all)` row and Run now is
/// refused. Only the operator turns it on or off.
pub const FLAG_PAUSED_ALL: &str = "jobs:paused_all";

/// What a Run now gets while the job's previous run is still going and its
/// policy `overlap` is `skip`.
pub const OVERLAP_REFUSAL: &str = "this job's previous run is still running (policy overlap: skip)";

/// What a Run now gets while Pause all is on.
pub const PAUSED_ALL_REFUSAL: &str =
    "the operator has paused every job (Pause all); Run now is off until they resume them";

/// The status a Drone run left `running` by a restart gets in
/// `db_drone_runs` (§3.2). An open string there, like every other status.
pub const DRONE_STATUS_INTERRUPTED: &str = "interrupted";

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

/// Something done to a pane, by its block id: interrupting its turn, stopping a worker.
pub type BlockHook = Arc<dyn Fn(&str) + Send + Sync>;

/// What [`RunLedger::pane`] reads from a pane's meta.
#[derive(Debug, Default)]
struct Pane {
    /// The block is in this channel's store.
    found: bool,
    agent_uid: String,
    account: String,
    provider: String,
}

/// Whether an agent is registered in `block_id` on this instance.
fn registered_here(block_id: &str) -> bool {
    crate::backend::reactive::get_global_handler().list_agents().iter().any(|r| r.block_id == block_id)
}

/// The UID the agent registered in `block_id` gave, if any.
fn registered_uid(block_id: &str) -> Option<String> {
    if block_id.is_empty() {
        return None;
    }
    crate::backend::reactive::get_global_handler()
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == block_id)
        .and_then(|r| r.uid)
        .filter(|u| !u.is_empty())
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
    /// This instance's channel (`registry::local_channel_id`).
    channel: String,
    /// The channel and version that write rows here, `<channel>@<version>`.
    /// Startup cleanup and the pending-fire delivery only touch rows their
    /// own channel wrote: the ledger is shared by every instance on the
    /// computer.
    recorded_by: String,
    follower: follower::Follower,
    /// Where `JobsChanged` goes after each write, so the Jobs tab refreshes.
    broker: Arc<Broker>,
    /// Failure incidents (§3.4), once [`Self::install_incidents`] ran.
    incidents: std::sync::OnceLock<Arc<incidents::Incidents>>,
    /// Where jobs are, for policy (`admission.rs`).
    jobs: std::sync::OnceLock<Arc<Store>>,
    /// Held while a run's place is decided and its row written.
    admission: std::sync::Mutex<()>,
    /// Pending fires that were told to start and are being delivered: run id to
    /// job id. A pending row doesn't occupy a place in the ledger, so until the
    /// delivery lands these count as running.
    pending_claims: std::sync::Mutex<std::collections::HashMap<String, String>>,
    interrupt: std::sync::OnceLock<BlockHook>,
    worker_stop: std::sync::OnceLock<BlockHook>,
}

impl RunLedger {
    /// A ledger over `store` (the identity store), following turns on
    /// `broker`. Needs a tokio runtime (the follower's task).
    pub fn new(store: Arc<Store>, mstore: Option<Arc<Store>>, broker: Arc<Broker>) -> Arc<Self> {
        Self::new_in(store, mstore, broker, &crate::backend::reactive::registry::local_channel_id())
    }

    /// [`Self::new`] as the instance of `channel`: tests run two channels'
    /// ledgers in one process.
    pub(crate) fn new_in(store: Arc<Store>, mstore: Option<Arc<Store>>, broker: Arc<Broker>, channel: &str) -> Arc<Self> {
        if let Err(e) = store.job_runs_ensure_schema() {
            tracing::warn!(error = %e, "jobs: couldn't create the run ledger table");
        }
        Arc::new_cyclic(|weak: &std::sync::Weak<RunLedger>| RunLedger {
            store: store.clone(),
            mstore,
            channel: channel.to_string(),
            recorded_by: format!("{channel}@{}", env!("CARGO_PKG_VERSION")),
            follower: follower::Follower::start(weak.clone(), broker.clone()),
            broker,
            incidents: std::sync::OnceLock::new(),
            jobs: std::sync::OnceLock::new(),
            admission: std::sync::Mutex::new(()),
            pending_claims: std::sync::Mutex::default(),
            interrupt: std::sync::OnceLock::new(),
            worker_stop: std::sync::OnceLock::new(),
        })
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Open an incident when a job fails, from now on (§3.4). `jobs` is
    /// where cron jobs are, for a job's owner.
    pub fn install_incidents(&self, jobs: Option<Arc<Store>>) {
        if let Some(jobs) = &jobs {
            self.install_jobs(jobs.clone());
        }
        let keeper = incidents::Incidents::new(self.store.clone(), self.mstore.clone(), jobs, self.broker.clone());
        if self.incidents.set(keeper.clone()).is_ok() {
            keeper.start();
        }
    }

    /// Tell the Jobs tab job `job_id`'s runs changed (`JobsChanged`); `""`
    /// for a change to every job. A job's change is also looked at for an
    /// incident.
    pub fn changed(&self, job_id: &str) {
        let event = crate::backend::mps_events::JobsChangedEvent { job_id: job_id.to_string() };
        self.broker.publish_event::<crate::backend::mps_events::JobsChanged>(&event, vec![], 0);
        if !job_id.is_empty() {
            if let Some(keeper) = self.incidents.get() {
                keeper.nudge(job_id);
            }
        }
    }

    /// [`Self::changed`] for the job run `run_id` belongs to.
    pub(crate) fn run_changed(&self, run_id: &str) {
        let job_id = self.store.job_run_get(run_id).ok().flatten().map(|r| r.job_id).unwrap_or_default();
        self.changed(&job_id);
    }

    /// Pause all, when the operator has it on: since when, and by whom.
    pub fn paused_all(&self) -> Option<JobFlag> {
        self.store.job_flag_get(FLAG_PAUSED_ALL).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "jobs: couldn't read Pause all");
            None
        })
    }

    /// Turn Pause all on or off. The caller has checked that the operator
    /// asked: agents can't (§7).
    pub fn set_paused_all(&self, on: bool, by: &str) -> Result<(), crate::backend::storage::StoreError> {
        if on {
            self.store.job_flag_set(FLAG_PAUSED_ALL, "1", by, now_ms())?;
        } else {
            self.store.job_flag_clear(FLAG_PAUSED_ALL)?;
        }
        self.changed("");
        Ok(())
    }

    /// A slot of `job` that didn't fire because the job, or every job, was
    /// paused (`reason`): counted into the job's newest row when that is the
    /// same skip, else a new `skipped` row; either way the slot keeps its own
    /// leased row, so no scheduler fires it later (§3.1, §3.3).
    pub fn record_skip_slot(&self, job: &crate::backend::storage::cron::CronJob, slot_ms: i64, why: &str) {
        let now = now_ms();
        let mut run = self.row(&job.id, SOURCE_CRON, &job.name, TRIGGER_SCHEDULE, now);
        run.state = state::SKIPPED.to_string();
        run.reason = why.to_string();
        run.scheduled_for_ms = Some(slot_ms);
        run.triggered_by = BY_SCHEDULER.to_string();
        run.target_agent_uid = job.target_uid.clone();
        run.skip_count = 1;
        run.last_skip_at_ms = Some(slot_ms);
        run.ended_at_ms = Some(now);
        match self.store.job_run_record_skip_slot(&run) {
            Ok(()) => self.changed(&job.id),
            Err(e) => tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't record a paused slot"),
        }
    }

    pub(crate) fn row(&self, job_id: &str, job_source: &str, job_name: &str, trigger: &str, now: i64) -> JobRun {
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
    /// `job`. `delivery_started_ms` is when the delivery began: delivery can
    /// wait for the turn it starts, so that turn may begin, or even end,
    /// before the answer. A scheduled fire also ends the job's older pending
    /// fire: it is superseded when this one is pending in its place, else
    /// expired (its next fire time came).
    pub fn record_delivery(
        &self,
        run_id: &str,
        job: &crate::backend::storage::cron::CronJob,
        outcome: &FireOutcome,
        scheduled: bool,
        delivery_started_ms: i64,
    ) {
        let now = now_ms();
        let latest = job.catch_up == CATCH_UP_LATEST;
        let result = match outcome {
            FireOutcome::Delivered { block_id } => {
                self.mark_running(run_id, &job.target_uid, block_id.as_deref().unwrap_or(""), delivery_started_ms);
                Ok(())
            }
            FireOutcome::Missed { reason: MissReason::TargetStopped } => {
                let st = if latest && scheduled { state::PENDING } else { state::MISSED };
                self.store.job_run_set_undelivered(run_id, st, reason::TARGET_STOPPED, now).map(drop)
            }
            FireOutcome::Missed { reason: MissReason::TargetMissing } => {
                self.store.job_run_skip(run_id, reason::TARGET_MISSING, 1, now).map(drop)
            }
            FireOutcome::Failed { error } => {
                let finished = self.store.job_run_finish(run_id, infra::ERROR, None, error, now);
                self.maybe_retry(run_id);
                self.reap_worker(run_id);
                finished.map(drop)
            }
        };
        if let Err(e) = result {
            tracing::warn!(run_id, error = %e, "jobs: couldn't record a fire's delivery");
        }
        self.changed(&job.id);
        if scheduled {
            let now_pending = self.store.job_run_get(run_id).ok().flatten().is_some_and(|r| r.state == state::PENDING && r.ended_at_ms.is_none());
            let why = if now_pending { reason::SUPERSEDED } else { reason::EXPIRED };
            if let Err(e) = self.store.job_runs_end_pending(&job.id, run_id, why, now) {
                tracing::warn!(job_id = %job.id, error = %e, "jobs: couldn't end an older pending fire");
            }
        }
    }

    /// A pending fire was delivered, or failed for good, when its agent came
    /// back (`CronScheduler::deliver_pending`, which expires it first once
    /// its next fire time has passed). Still missing, or a failure the
    /// held-jekt replay also retries (the agent still starting, a rate
    /// limit, a full queue, not signed in yet), keeps it pending for the
    /// next wake or sweep. `delivery_started_ms` as for [`Self::record_delivery`].
    pub fn record_pending_delivery(&self, run: &JobRun, outcome: &FireOutcome, delivery_started_ms: i64) {
        let now = now_ms();
        let result = match outcome {
            FireOutcome::Delivered { block_id } => {
                self.mark_running(&run.id, &run.target_agent_uid, block_id.as_deref().unwrap_or(""), delivery_started_ms);
                Ok(())
            }
            FireOutcome::Missed { .. } => Ok(()),
            FireOutcome::Failed { error } if retry_later(error) => Ok(()),
            FireOutcome::Failed { error } => self.store.job_run_finish(&run.id, infra::ERROR, None, error, now).map(drop),
        };
        if let Err(e) = result {
            tracing::warn!(run_id = %run.id, error = %e, "jobs: couldn't record a pending fire's delivery");
        }
        self.changed(&run.job_id);
    }

    /// End a pending fire whose next fire time passed, or whose job is gone.
    pub fn expire_pending(&self, run_id: &str) {
        if let Err(e) = self.store.job_run_end_one_pending(run_id, reason::EXPIRED, now_ms()) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't expire a pending fire");
        }
        self.run_changed(run_id);
    }

    /// This channel's pending fires still waiting for their agent.
    pub fn waiting_here(&self) -> Vec<JobRun> {
        self.store
            .job_runs_open_recorded_by(&self.channel, SOURCE_CRON, &[state::PENDING])
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.ended_at_ms.is_none())
            .collect()
    }

    /// A run reached its agent: running from now, with the pane's account,
    /// and its turn followed to the end. `target_agent_uid` becomes the UID
    /// of the agent whose pane took it (a job created by name records none),
    /// so that agent can report the run.
    ///
    /// A pane on another instance (another channel took it) sends its turn
    /// events to that instance's broker, so that instance follows the run
    /// ([`Self::follow_forwarded`], #1988). This one only watches the row:
    /// a receiver too old to follow is closed after the no-turn timeout.
    pub(crate) fn mark_running(&self, run_id: &str, target_uid: &str, block_id: &str, now: i64) {
        let pane = self.pane(block_id);
        let remote = !block_id.is_empty() && self.mstore.is_some() && !pane.found && !registered_here(block_id);
        if remote && self.followed_elsewhere(run_id) {
            // The receiver marked it running with its pane's account.
            self.follower.watch(run_id, now);
            return;
        }
        let receiver = [pane.agent_uid.as_str(), registered_uid(block_id).as_deref().unwrap_or(""), target_uid]
            .into_iter()
            .find(|u| !u.is_empty())
            .unwrap_or("")
            .to_string();
        let (account, provider) = (pane.account, pane.provider);
        if let Err(e) = self.store.job_run_mark_running(run_id, &receiver, block_id, &account, &provider, now) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't mark a run running");
            return;
        }
        if block_id.is_empty() {
            // Delivered with no pane to follow (a relay answer names none):
            // nothing would ever close it, and a restart would call it
            // interrupted. It ran as far as AgentMux can tell (#5390).
            self.turn_finished(run_id, infra::OK, None, "delivered; AgentMux can't follow this run's turn");
            return;
        }
        if remote {
            // No transcript link: it wouldn't resolve here.
            self.follower.watch(run_id, now);
            return;
        }
        let _ = self.store.job_run_add_links(run_id, &json!({ "transcript": { "block_id": block_id } }));
        self.follower.arm(block_id, run_id, now);
    }

    /// Who a pane belongs to (its `agentId`, the agent's UID), the account
    /// it runs on (`agent:run_account`) and its provider.
    fn pane(&self, block_id: &str) -> Pane {
        let Some(mstore) = &self.mstore else { return Pane::default() };
        if block_id.is_empty() {
            return Pane::default();
        }
        let Ok(Some(block)) = mstore.get::<crate::backend::obj::Block>(block_id) else {
            return Pane::default();
        };
        let account = crate::backend::obj::meta_get_string(
            &block.meta,
            crate::server::app_api::agent_spawn::RUN_ACCOUNT_META_KEY,
            "",
        );
        let agent_uid = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
        let provider = match mstore.agent_def_get(&agent_uid) {
            Ok(Some(def)) if !agent_uid.is_empty() => crate::backend::providers::resolve_provider_alias(&def.provider).to_string(),
            _ => String::new(),
        };
        Pane { found: true, agent_uid, account, provider }
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
        self.changed(&job.id);
    }

    /// The followed turn ended (or never showed up).
    pub(crate) fn turn_finished(&self, run_id: &str, infra_status: &str, cost: Option<RunCost>, summary: &str) {
        // Only the call that closes the row retries and reaps: a run stopped on
        // purpose (overlap, timeout) is already closed, and its turn ending later
        // must not file a retry of a run nobody wants again.
        match self.store.job_run_finish(run_id, infra_status, cost, summary, now_ms()) {
            Ok(true) => {
                self.maybe_retry(run_id);
                self.reap_worker(run_id);
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(run_id, error = %e, "jobs: couldn't close a run"),
        }
        self.run_changed(run_id);
        pending_wake().notify_one();
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
            Ok(true) => {
                self.changed(drone_id);
                Some(run.id)
            }
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
    /// interrupted, in the ledger and in `db_drone_runs`; so were the runs
    /// it followed for another channel (#1988). A run another channel
    /// follows goes on there, and is watched again from here. Pending fires
    /// stay: they wait for their agent across a restart. Call before the
    /// scheduler starts.
    pub fn recover_after_restart(&self) -> usize {
        let now = now_ms();
        let open = |source: &str, states: &[&str]| {
            self.store.job_runs_open_recorded_by(&self.channel, source, states).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "jobs: couldn't read open runs");
                Vec::new()
            })
        };
        let mut closed = 0;
        let followed_here = self.store.job_runs_open_followed_by(&self.channel).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "jobs: couldn't read followed runs");
            Vec::new()
        });
        // A run waiting for a place (overlap, group, the global limit, a retry's
        // backoff) never started: it stays open for the scheduler to start.
        let (ours, followed_elsewhere): (Vec<JobRun>, Vec<JobRun>) = open(SOURCE_CRON, &[state::SCHEDULED, state::RUNNING])
            .into_iter()
            .filter(|r| !(r.state == state::SCHEDULED && reason::WAITING.contains(&r.reason.as_str())))
            .partition(|r| remote::followed_by(r).is_none_or(|f| f.channel == self.channel));
        for run in &followed_elsewhere {
            self.follower.watch(&run.id, now);
        }
        let stale = ours.into_iter().chain(open(SOURCE_DRONE, &[state::RUNNING])).chain(followed_here);
        let mut jobs = std::collections::BTreeSet::new();
        for run in stale {
            if self.store.job_run_finish(&run.id, infra::INTERRUPTED, None, "AgentMux restarted during this run", now).unwrap_or(false) {
                closed += 1;
                jobs.insert(run.job_id);
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
            self.changed("");
            for job_id in jobs {
                self.changed(&job_id);
            }
        }
        closed
    }
}

/// Whether a pending fire's failed delivery is retried at the next wake or
/// sweep rather than closing the run: the same classification as the
/// held-jekt replay (`server::jekt_held`), plus an account out of quota
/// (`HoldReason::Limited`), which clears at its reset.
fn retry_later(error: &str) -> bool {
    crate::backend::reactive::is_transient_delivery_error(error)
        || crate::backend::reactive::is_spawn_gate_refusal(error)
        || crate::backend::reactive::is_unavailable_refusal(error)
        || error.contains("out of quota")
        || error.contains("usage limit")
}

/// The summed `cost_usd` of a Drone run's blocks: only Agent blocks report
/// one, in their output (`drone/executor/blocks/agent.rs`). Known only when
/// at least one block reported a cost.
pub fn drone_cost(block_states: &std::collections::HashMap<String, crate::drone::types::BlockState>) -> RunCost {
    let mut cost = RunCost::default();
    for b in block_states.values() {
        let Some(out) = &b.output else { continue };
        if let Some(c) = out.get("cost_usd").and_then(|v| v.as_f64()) {
            cost.cost_usd += c;
            cost.cost_known = true;
        }
        if let Some(t) = out.pointer("/tokens/output").and_then(|v| v.as_i64()) {
            cost.output_tokens += t;
        }
    }
    cost
}

/// Missed slots of a job's `schedule` after `base_ms` and before `now_ms`:
/// the newest, and the earlier ones as `(first, count)`. The newest is the
/// schedule's last slot before now, found directly, so it is right after any
/// downtime; only the count of earlier ones stops at `cap`, so a one-minute
/// job off for a year costs bounded work.
pub fn missed_slots(
    schedule: &crate::backend::cron::schedule::JobSchedule,
    base_ms: i64,
    now_ms: i64,
    cap: usize,
) -> Option<(i64, Option<(i64, i64)>)> {
    let newest = schedule.prev_before(now_ms)?;
    if newest <= base_ms {
        return None;
    }
    let mut first = None;
    let mut count: i64 = 0;
    for ms in schedule.after(base_ms).take(cap) {
        if ms >= newest {
            break;
        }
        first.get_or_insert(ms);
        count += 1;
    }
    Some((newest, first.map(|f| (f, count))))
}

/// Whether the agent with `uid` is registered by another channel's
/// AgentMux on this computer and that process is alive: the cross-channel
/// tier the inject path forwards to (Tier 2b, the host-global shared
/// registry). A pending fire whose agent lives there is delivered through
/// it, rather than expiring here unseen.
pub fn registered_in_another_channel(shared_dir: &std::path::Path, uid: &str, channel: &str) -> bool {
    !uid.is_empty()
        && crate::backend::reactive::registry::list_all_shared(shared_dir)
            .iter()
            .any(|e| e.uid == uid && e.channel != channel && crate::backend::reactive::registry::pid_alive(e.pid))
}

/// The job's next fire time after `slot_ms`: when a pending fire for that
/// slot expires (§5.2). `None` for a job whose trigger has no later slot or
/// doesn't read.
pub fn next_slot_after(job: &crate::backend::storage::cron::CronJob, slot_ms: i64) -> Option<i64> {
    crate::backend::cron::schedule::JobSchedule::of(job).ok()?.next_after(slot_ms)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

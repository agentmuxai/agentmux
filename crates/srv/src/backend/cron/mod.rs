// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persistent cron scheduler for AgentMux.
//!
//! Backs each enabled `CronJob` with a tokio task that sleeps until the
//! next scheduled fire time (its trigger's next slot, [`schedule`]: a cron
//! expression in the job's time zone, an interval or one time), plus the
//! job's fixed jitter,
//! delivers the prompt as `/agentmux/reactive/inject` would — in process
//! since identity M4c-3 (see [`FireDelivery`]) — and records the fire in the
//! DB.
//!
//! Every fire is a run in the run ledger (`backend::jobs`): its row is
//! inserted before it fires, which for a slot is the lease two schedulers
//! sharing a job need, and only a delivered fire counts toward `fire_count`
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.2). Cron fires are
//! never held for a stopped agent; what an undelivered fire leaves is the
//! job's catch-up policy (§5.2): `skip`, a missed run, or `latest`, one
//! pending fire this scheduler delivers when the agent is back.
//!
//! On startup, a `latest` job whose next scheduled time after `last_fired`
//! is already in the past fires once as catch-up (FIRE_ONCE_NOW — never
//! replay all missed fires, never cause a cron storm); the slots missed
//! while AgentMux was off are one collapsed `skipped` row.
//!
//! See `docs/specs/SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md §3.2`.

pub mod expr;
pub mod schedule;
mod start;

pub use start::{ActionOutcome, ActionRunner};

use agentmux_common::AUTH_KEY_HEADER;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentmux_common::api_types::InjectRequest;
use chrono::{DateTime, Utc};

use crate::backend::jobs::admission::Admission;
use crate::backend::jobs::{self, RunLedger};
use crate::backend::storage::store::Store;
use crate::backend::storage::cron::CronJob;
use crate::backend::mps::Broker;
use crate::backend::reactive::types::InjectionRequest;

/// Abort handles for every currently scheduled cron task.
type HandleMap = Mutex<HashMap<String, tokio::task::AbortHandle>>;

/// Missed slots counted at startup, at most (`jobs::missed_slots`).
const MISSED_SLOTS_CAP: usize = 100_000;

/// How often pending fires are checked when no agent registers meanwhile.
const PENDING_SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// In-process delivery of a fire: the server's shared inject path
/// (`server::reactive::deliver`, on the instance-key tier), given the job's
/// request and its creator's UID. Identity M4c-3
/// (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9): the
/// scheduler is built before `AppState`, so the server installs this once
/// `AppState` exists ([`CronScheduler::install_delivery`]), as it installs
/// agent-turn delivery. Until then a fire POSTs to the route, as before.
pub type FireDelivery = Arc<
    dyn Fn(
            InjectionRequest,
            String,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = serde_json::Value> + Send>>
        + Send
        + Sync,
>;

/// Whether an agent UID is registered (running) on this instance, so a
/// pending fire is only attempted once its agent is back.
pub type AgentPresence = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Why a fire reached no one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissReason {
    /// No live agent answered to the job's target UID or name. Cron fires
    /// are never held, so inject can't tell a stopped agent from a deleted
    /// one: this covers both.
    TargetStopped,
    /// The job names no target at all ([`CronScheduler::deliver`]).
    TargetMissing,
}

/// What one fire's delivery decided, computed once in [`CronScheduler::fire`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FireOutcome {
    Delivered { block_id: Option<String> },
    Missed { reason: MissReason },
    Failed { error: String },
}

impl FireOutcome {
    pub(crate) fn delivered(&self) -> bool {
        matches!(self, FireOutcome::Delivered { .. })
    }

    /// The job's `last_miss_reason` for an undelivered fire; `None` when
    /// delivered.
    pub(crate) fn miss_reason(&self) -> Option<String> {
        match self {
            FireOutcome::Delivered { .. } => None,
            FireOutcome::Missed { reason: MissReason::TargetStopped } => Some("target_stopped".into()),
            FireOutcome::Missed { reason: MissReason::TargetMissing } => Some("target_missing".into()),
            FireOutcome::Failed { error } => Some(format!("failed: {error}")),
        }
    }
}

/// Read a fire's inject answer (`None`: the request itself failed). Cron
/// fires are never held (`hold_applies`), so a stopped agent answers "agent
/// not found" like a missing one.
pub(crate) fn fire_outcome(body: Option<&serde_json::Value>) -> FireOutcome {
    let Some(body) = body else {
        return FireOutcome::Failed { error: "inject request failed".to_string() };
    };
    if body.get("success").and_then(|v| v.as_bool()) == Some(true) {
        let block_id = body.get("block_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
        return FireOutcome::Delivered { block_id };
    }
    let error = body.get("error").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if error.starts_with("agent not found") {
        return FireOutcome::Missed { reason: MissReason::TargetStopped };
    }
    FireOutcome::Failed { error }
}

/// The text a fire delivers: the prompt, then the run line. A slash command
/// (`/compact`, `/model x`) gets no run line: it would become the command's
/// arguments. That run is still recorded and followed by its `request_id`;
/// only its agent can't report an outcome, so it stays `unknown`.
pub(crate) fn fire_message(prompt: &str, run_id: Option<&str>, late_for: Option<i64>) -> String {
    match run_id {
        Some(run_id) if !prompt.trim_start().starts_with('/') => {
            format!("{prompt}\n\n{}", jobs::brief_line(run_id, late_for, agentmux_common::time::now_ms()))
        }
        _ => prompt.to_string(),
    }
}

/// Which slot a fire is for (§3.1 `trigger`, `scheduled_for_ms`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Slot {
    /// A scheduled slot, ms epoch.
    Scheduled(i64),
    /// The newest slot missed while AgentMux was off, fired at startup.
    CatchUp(i64),
    /// Run now, by the operator or an agent uid. Doesn't count toward
    /// `fire_count` or `last_fired`: those are the schedule's.
    Manual { triggered_by: String },
}

/// How a fire ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fired {
    /// The run's id in the ledger, when it was recorded.
    pub run_id: Option<String>,
    /// `None` when the fire didn't happen: another scheduler had the slot,
    /// or there was no way to deliver.
    pub outcome: Option<FireOutcome>,
    /// Why the run waits to start (policy), when it does: its row is
    /// `scheduled` with this reason.
    pub waiting: Option<&'static str>,
}

/// What `start` found for a job: the newest missed slot, and the earlier
/// ones as `(first, count)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Missed {
    newest_ms: i64,
    earlier: Option<(i64, i64)>,
}

pub struct CronScheduler {
    handles: HandleMap,
    /// Where jobs are: `db_jobs` in the shared store.
    jobs_store: Option<Arc<Store>>,
    http_client: reqwest::Client,
    local_url: String,
    auth_key: String,
    broker: Arc<Broker>,
    delivery: std::sync::OnceLock<FireDelivery>,
    presence: std::sync::OnceLock<AgentPresence>,
    runs: std::sync::OnceLock<Arc<RunLedger>>,
    /// Runs the `task` and `spawn` actions ([`ActionRunner`]).
    actions: std::sync::OnceLock<ActionRunner>,
    /// Jobs this scheduler disabled for their age bound (`max_age_secs`),
    /// not paused: their pending fire may still be delivered within that
    /// bound (`deliver_pending`). Cleared when the job is paused, deleted or
    /// scheduled again (`cancel_job`). In memory: after a restart such a
    /// job's pending fire expires like a paused one's.
    aged_out: Mutex<std::collections::HashSet<String>>,
}

impl CronScheduler {
    pub fn new(
        jobs_store: Option<Arc<Store>>,
        http_client: reqwest::Client,
        local_url: String,
        auth_key: String,
        broker: Arc<Broker>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handles: Mutex::new(HashMap::new()),
            jobs_store,
            http_client,
            local_url,
            auth_key,
            broker,
            delivery: std::sync::OnceLock::new(),
            presence: std::sync::OnceLock::new(),
            runs: std::sync::OnceLock::new(),
            actions: std::sync::OnceLock::new(),
            aged_out: Mutex::default(),
        })
    }

    /// Deliver fires in process from now on (see [`FireDelivery`]). Once;
    /// a second install is ignored.
    pub fn install_delivery(&self, delivery: FireDelivery) {
        let _ = self.delivery.set(delivery);
    }

    /// How to tell an agent is back, for pending fires. Once.
    pub fn install_presence(&self, presence: AgentPresence) {
        let _ = self.presence.set(presence);
    }

    /// Record fires in the run ledger from now on. Once; a second install
    /// is ignored.
    pub fn install_runs(&self, ledger: Arc<RunLedger>) {
        let _ = self.runs.set(ledger);
    }

    /// The run ledger, once installed.
    pub fn runs(&self) -> Option<&Arc<RunLedger>> {
        self.runs.get()
    }

    fn publish_changed(&self) {
        self.broker.publish_event::<crate::backend::mps_events::CronChanged>(&(), vec![], 0);
    }

    /// Load all enabled jobs from the DB and schedule them. Call once at startup.
    pub async fn start(self: &Arc<Self>) {
        self.start_at(Utc::now()).await;
    }

    /// [`Self::start`] counting the slots missed while off up to `now_dt`, so a
    /// test fixes the instant its expected counts are computed from.
    pub(crate) async fn start_at(self: &Arc<Self>, now_dt: DateTime<Utc>) {
        let store = match &self.jobs_store {
            Some(s) => s.clone(),
            None => {
                tracing::warn!("cron: no job store — cron scheduler disabled");
                return;
            }
        };

        let jobs = match store.cron_list_enabled() {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "cron: failed to load jobs on startup");
                return;
            }
        };

        let count = jobs.len();

        for job in jobs {
            let missed = self.missed_while_off(&job, now_dt);
            self.schedule_job_inner(&job, missed);
        }

        self.spawn_pending_delivery();
        tracing::info!(count, "cron: scheduled {} job(s) from DB", count);
    }

    /// The slots a job missed while AgentMux was off, if any. The base is the
    /// later of `last_fired` and the job's newest ledger row, so slots the
    /// ledger already accounts for (a miss while AgentMux ran) aren't counted
    /// as missed while it was off. Only for a job that has fired before
    /// (`should_catchup`).
    fn missed_while_off(&self, job: &CronJob, now_dt: DateTime<Utc>) -> Option<Missed> {
        if !should_catchup(job, now_dt) {
            return None;
        }
        let mut base_ms = catch_up_base_secs(job).saturating_mul(1000);
        if let Some(last) = self.runs().and_then(|r| r.store().job_run_last(&job.id).ok().flatten()) {
            let seen = last.last_skip_at_ms.or(last.scheduled_for_ms).unwrap_or(last.started_at_ms);
            base_ms = base_ms.max(seen);
        }
        let schedule = schedule::JobSchedule::of(job).ok()?;
        let (newest_ms, earlier) = jobs::missed_slots(&schedule, base_ms, now_dt.timestamp_millis(), MISSED_SLOTS_CAP)?;
        Some(Missed { newest_ms, earlier })
    }

    /// Schedule a single job (or reschedule it after a DB change). Replaces
    /// any existing task for the same `job.id`.
    pub fn schedule_job(self: &Arc<Self>, job: &CronJob) {
        self.schedule_job_inner(job, None);
    }

    fn schedule_job_inner(self: &Arc<Self>, job: &CronJob, missed: Option<Missed>) {
        self.cancel_job(&job.id);

        let schedule = match schedule::JobSchedule::of(job) {
            Ok(schedule::JobSchedule::Manual) => return,
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(id = %job.id, expr = %job.expression, error = %e, "cron: invalid trigger — skipping job");
                return;
            }
        };
        // A one-time job fires at its time; the offset spreads recurring ones.
        let jitter = if matches!(schedule, schedule::JobSchedule::Once { .. }) { 0 } else { schedule::jitter_ms(&job.id) };

        let sched = self.clone();
        let job = job.clone();
        let job_id = job.id.clone();

        let handle = tokio::spawn(async move {
            // Seeded from the persisted fire_count so max_fires is enforced
            // across restarts, not per process run.
            let mut fires: i64 = job.fire_count;
            // The last slot this loop handled: the next is the one after it,
            // so the jitter (a fire up to 2 minutes after its slot) never
            // skips a slot.
            let mut last_slot: Option<i64> = None;
            if let Some(m) = missed {
                // `latest` catches up the newest missed slot; `skip` leaves
                // every missed slot in the collapsed skip row.
                let latest = job.catch_up == jobs::CATCH_UP_LATEST;
                let skipped = match (latest, m.earlier) {
                    (true, earlier) => earlier,
                    (false, Some((first, n))) => Some((first, n + 1)),
                    (false, None) => Some((m.newest_ms, 1)),
                };
                if let (Some(runs), Some((first, n))) = (sched.runs(), skipped) {
                    let (runs, job) = (runs.clone(), job.clone());
                    let _ = tokio::task::spawn_blocking(move || runs.record_missed_while_off(&job, first, n)).await;
                }
                // FIRE_ONCE_NOW, before the first sleep, so the catch-up
                // counts toward max_fires only when it was delivered.
                if latest && sched.fire(&job, Slot::CatchUp(m.newest_ms)).await.outcome.is_some_and(|o| o.delivered()) {
                    fires += 1;
                }
            }
            loop {
                // Guard at top of loop: if fires was seeded at or above max_fires
                // (e.g. a catch-up fire brought the persisted count to the cap),
                // don't fire again before sleeping — this prevents one extra fire
                // on restart when the catch-up itself hits the limit. Age is
                // checked the same way (Phase 0, SPEC_AGENT_POLLING_AND_WAKEUP_
                // HARDENING_2026_08_04.md) — a job created before max_age_secs
                // existed has max_age_secs = None and is never expired by this.
                if let Some(max) = job.max_fires {
                    if fires >= max {
                        sched.disable_job(&job.id, "max_fires reached");
                        break;
                    }
                }
                if is_expired_by_age(job.created_at, job.max_age_secs, Utc::now().timestamp()) {
                    sched.disable_job(&job.id, "max_age_secs reached");
                    break;
                }
                let now_ms = agentmux_common::time::now_ms();
                let from = last_slot.map_or(now_ms, |l| l.max(now_ms - jitter));
                let Some(next_ms) = schedule.next_after(from) else {
                    // A one-time job has had its time.
                    if matches!(schedule, schedule::JobSchedule::Once { .. }) {
                        sched.disable_job(&job.id, "once fired");
                    }
                    break;
                };
                let Some(next) = DateTime::from_timestamp_millis(next_ms) else { break };
                // Reviewer-caught gap (P1, PR #2418): checking age only in "now"
                // terms at the top of the loop lets one extra fire slip through
                // when the cron interval is longer than the remaining time to
                // expiry — the loop would sleep straight past the expiry bound
                // and fire anyway, only catching it on the *next* iteration.
                // Check whether the fire we're about to sleep for would itself
                // land at/after expiry, and skip it (disable now, don't sleep)
                // if so — the "hard expiry bound, regardless of fire count"
                // guarantee has to hold at fire time, not just at loop-top time.
                if is_expired_by_age(job.created_at, job.max_age_secs, next.timestamp()) {
                    sched.disable_job(&job.id, "max_age_secs would be reached before the next scheduled fire");
                    break;
                }
                let due = next_ms + jitter - agentmux_common::time::now_ms();
                tokio::time::sleep(std::time::Duration::from_millis(due.max(0) as u64)).await;
                last_slot = Some(next_ms);
                // A pending fire may have been delivered meanwhile: count from
                // the DB, so max_fires holds across both.
                if let Some(n) = sched.persisted_fire_count(&job.id) {
                    fires = fires.max(n);
                }
                if job.max_fires.is_some_and(|max| fires >= max) {
                    sched.disable_job(&job.id, "max_fires reached");
                    break;
                }
                if sched.fire(&job, Slot::Scheduled(next_ms)).await.outcome.is_some_and(|o| o.delivered()) {
                    fires += 1;
                }
                // Post-fire check: enforce max_fires. The top-of-loop guard
                // handles the restart/seeded-at-cap case; this handles the
                // normal live-run case.
                if let Some(max) = job.max_fires {
                    if fires >= max {
                        sched.disable_job(&job.id, "max_fires reached");
                        break;
                    }
                }
            }
        });

        self.handles.lock().unwrap().insert(job_id, handle.abort_handle());
    }

    /// Whether a job has a live scheduled task.
    #[cfg(test)]
    pub(crate) fn is_scheduled(&self, id: &str) -> bool {
        self.handles.lock().unwrap().contains_key(id)
    }

    /// Stop (abort) a job's scheduled task without removing it from the DB.
    pub fn cancel_job(&self, id: &str) {
        self.aged_out.lock().unwrap().remove(id);
        if let Some(handle) = self.handles.lock().unwrap().remove(id) {
            handle.abort();
        }
    }

    /// Disable a job in the DB (audit trail preserved, unlike delete), drop
    /// its live task handle, and notify listeners. Shared by every
    /// self-disabling reason a scheduled job's own loop hits (max_fires,
    /// max_age_secs) — extracted so those call sites don't each repeat the
    /// same three-step disable sequence.
    fn disable_job(&self, job_id: &str, reason: &str) {
        tracing::info!(id = job_id, reason, "cron: disabling job");
        if reason.starts_with("max_age_secs") {
            self.aged_out.lock().unwrap().insert(job_id.to_string());
        }
        if let Some(store) = &self.jobs_store {
            let _ = store.cron_set_enabled(job_id, false);
        }
        self.handles.lock().unwrap().remove(job_id);
        self.publish_changed();
    }

    /// Run a job now: a `manual` run through the same path as a scheduled
    /// fire (§3.2), triggered by the operator or an agent uid.
    pub(crate) async fn run_now(&self, job: &CronJob, triggered_by: &str) -> Fired {
        self.fire(job, Slot::Manual { triggered_by: triggered_by.to_string() }).await
    }

    /// Fire a cron job: record the run, deliver the prompt, and record the
    /// fire in DB when it was delivered.
    ///
    /// Identity M3 (spec §5.4): a job that captured its target's UID at
    /// creation fires BY UID — a job firing at 03:00 must never resolve a
    /// name. A job with no UID (created before M3, or by a caller that did
    /// not carry one) fires by name as before, counted.
    ///
    /// Identity M4c-3 (§6.5.9): delivered in process ([`FireDelivery`]), with
    /// the creator's UID as attribution on this instance only — a forwarded
    /// hop never carries it. **The sender name stays `"cron"`**: showing the
    /// creator's name unsigned would make every fire of an agent-created job
    /// look forged (its key is on file under that name) and echo each fire
    /// into the creator's pane. A job with no captured creator UID fires as
    /// before, counted.
    ///
    /// The run's id is the delivery's `request_id`, and the prompt ends with
    /// a line naming it, so the agent can report the run's outcome.
    pub(crate) async fn fire(&self, job: &CronJob, slot: Slot) -> Fired {
        let id = job.id.as_str();
        let (trigger, scheduled_for, by) = match &slot {
            Slot::Scheduled(ms) => (jobs::TRIGGER_SCHEDULE, Some(*ms), jobs::BY_SCHEDULER.to_string()),
            Slot::CatchUp(ms) => (jobs::TRIGGER_CATCH_UP, Some(*ms), jobs::BY_SCHEDULER.to_string()),
            Slot::Manual { triggered_by } => (jobs::TRIGGER_MANUAL, None, triggered_by.clone()),
        };
        if let Some(fired) = self.paused_all_answer(job, scheduled_for).await {
            return fired;
        }
        let admitted = match self.runs() {
            Some(runs) => {
                let (runs, owned, by) = (runs.clone(), job.clone(), by);
                tokio::task::spawn_blocking(move || runs.admit_cron(&owned, trigger, scheduled_for, &by))
                    .await
                    .unwrap_or(Admission::Unrecorded)
            }
            None => Admission::Unrecorded,
        };
        let run_id = match admitted {
            Admission::Run(run_id) => Some(run_id),
            Admission::Unrecorded => None,
            Admission::Taken => {
                tracing::debug!(id, ?slot, "cron: another scheduler has this slot");
                return Fired { run_id: None, outcome: None, waiting: None };
            }
            Admission::Skipped => return Fired { run_id: None, outcome: None, waiting: None },
            Admission::Refused => {
                let error = jobs::OVERLAP_REFUSAL.to_string();
                return Fired { run_id: None, outcome: Some(FireOutcome::Failed { error }), waiting: None };
            }
            Admission::Capped(refusal) => {
                let error = format!("Run now is refused: {}", refusal.words());
                return Fired { run_id: None, outcome: Some(FireOutcome::Failed { error }), waiting: None };
            }
            Admission::Waiting(run_id, why) => {
                self.publish_changed();
                return Fired { run_id: Some(run_id), outcome: None, waiting: Some(why) };
            }
        };
        let late_for = match slot {
            Slot::CatchUp(ms) => Some(ms),
            _ => None,
        };
        let scheduled = !matches!(slot, Slot::Manual { .. });
        self.start_run(job, run_id, scheduled, late_for).await
    }

    /// While the operator has Pause all on (jobs spec §3.3): a slot becomes
    /// a collapsed `skipped (paused_all)` row and isn't fired, and Run now
    /// (no slot) is refused. `None` when Pause all is off.
    async fn paused_all_answer(&self, job: &CronJob, scheduled_for: Option<i64>) -> Option<Fired> {
        let runs = self.runs()?.clone();
        let job = job.clone();
        let answer = tokio::task::spawn_blocking(move || {
            runs.paused_all()?;
            Some(match scheduled_for {
                Some(slot) => {
                    runs.record_skip_slot(&job, slot, crate::backend::storage::job_runs::reason::PAUSED_ALL);
                    Fired { run_id: None, outcome: None, waiting: None }
                }
                None => Fired {
                    run_id: None,
                    outcome: Some(FireOutcome::Failed { error: jobs::PAUSED_ALL_REFUSAL.to_string() }),
                    waiting: None,
                },
            })
        });
        answer.await.ok().flatten()
    }

    /// Deliver `job`'s prompt as run `run_id` (its `request_id`, and the
    /// brief line), and read the answer. `None` when there is no way to
    /// deliver at all.
    async fn deliver(&self, job: &CronJob, run_id: Option<&str>, late_for: Option<i64>) -> Option<FireOutcome> {
        let id = job.id.as_str();
        if job.created_by_uid.is_empty() {
            crate::backend::agent_resolve::record_uid_fallback("m4c.cron_fire_unattributed");
        }
        let target_agent = fire_target(&job.target, &job.target_uid).to_string();
        if target_agent.trim().is_empty() {
            tracing::warn!(id, "cron: the job names no target");
            return Some(FireOutcome::Missed { reason: MissReason::TargetMissing });
        }
        let message = fire_message(&job.prompt, run_id, late_for);
        let request_id = run_id.map(str::to_string);
        let body = match self.delivery.get() {
            Some(deliver) => {
                let req = InjectionRequest {
                    target_agent,
                    message,
                    source_agent: Some("cron".to_string()),
                    request_id,
                    ..Default::default()
                };
                Some(deliver(req, job.created_by_uid.clone()).await)
            }
            None => {
                if self.local_url.is_empty() || self.auth_key.is_empty() {
                    tracing::warn!(id, "cron: no local_url/auth_key — skipping fire");
                    return None;
                }
                crate::backend::agent_resolve::record_uid_fallback("m4c.cron_fire_http");
                self.post_fire(id, target_agent, &message, request_id).await
            }
        };
        let outcome = fire_outcome(body.as_ref());
        // Inject answers `success: false` when the target is not reachable —
        // since M3 that includes a UID nobody is registered under. A fire
        // that reached no one is a failure, and a firing job has nobody to
        // tell but the log (§5.4) and the run ledger.
        if outcome.delivered() {
            tracing::debug!(id, target = %job.target, "cron: fired");
        } else {
            tracing::warn!(id, target = %job.target, target_uid = %job.target_uid, ?outcome, "cron: fire was not delivered");
        }
        Some(outcome)
    }

    /// Whether `job` was disabled by this scheduler for its age bound and is
    /// still inside it (`created_at + max_age_secs`).
    fn within_age_bound(&self, job: &CronJob, now_ms: i64) -> bool {
        self.aged_out.lock().unwrap().contains(&job.id)
            && job.max_age_secs.is_some_and(|age| now_ms < job.created_at.saturating_add(age).saturating_mul(1000))
    }

    /// Disable a job as its own loop does at a limit (`reason`), for tests.
    #[cfg(test)]
    pub(crate) fn disable_for_test(&self, job_id: &str, reason: &str) {
        self.disable_job(job_id, reason);
    }

    /// The job's `fire_count` in the DB.
    fn persisted_fire_count(&self, id: &str) -> Option<i64> {
        self.jobs_store.as_ref()?.cron_get(id).ok().flatten().map(|j| j.fire_count)
    }

    /// After a fire the job's own loop didn't make (a pending fire), stop
    /// that loop if the fire reached `max_fires`; `cron_record_fire` has
    /// already disabled the job in the DB.
    fn stop_at_max_fires(&self, id: &str) {
        let at_max = self
            .jobs_store
            .as_ref()
            .and_then(|s| s.cron_get(id).ok().flatten())
            .is_some_and(|j| j.max_fires.is_some_and(|max| j.fire_count >= max));
        if at_max {
            tracing::info!(id, "cron: a pending fire reached max_fires; stopping the job");
            self.cancel_job(id);
        }
    }

    /// Record a scheduled fire on the job: a delivered one counts
    /// (`last_fired`, `fire_count`, `max_fires`); one that reached no one is
    /// a miss (`missed_count`, `last_missed_at`, `last_miss_reason`), which
    /// Swarm's Cron bucket and `Schedule list` show.
    fn record_result(&self, id: &str, outcome: &FireOutcome) {
        if let Some(store) = &self.jobs_store {
            let now = Utc::now().timestamp();
            let recorded = match outcome.miss_reason() {
                None => store.cron_record_fire(id, now),
                Some(reason) => store.cron_record_miss(id, now, &reason),
            };
            if let Err(e) = recorded {
                tracing::warn!(id, error = %e, "cron: failed to record the fire in DB");
            }
        }
    }

    /// Deliver pending fires (catch-up `latest`) whose agent is back, now and
    /// whenever an agent registers or every 30 s; expire the ones whose next
    /// fire time passed or whose job is gone (§5.2).
    fn spawn_pending_delivery(self: &Arc<Self>) {
        if self.runs().is_none() {
            return;
        }
        let sched = self.clone();
        tokio::spawn(async move {
            loop {
                sched.deliver_pending().await;
                sched.run_policy_pass().await;
                tokio::select! {
                    _ = jobs::pending_wake().notified() => {}
                    _ = tokio::time::sleep(PENDING_SWEEP_EVERY) => {}
                }
            }
        });
    }

    /// One pass over this channel's pending fires. Returns how many were
    /// delivered.
    pub(crate) async fn deliver_pending(&self) -> usize {
        let Some(runs) = self.runs().cloned() else { return 0 };
        let waiting = {
            let runs = runs.clone();
            tokio::task::spawn_blocking(move || runs.waiting_here()).await.unwrap_or_default()
        };
        let now = agentmux_common::time::now_ms();
        let paused_all = {
            let runs = runs.clone();
            tokio::task::spawn_blocking(move || runs.paused_all().is_some()).await.unwrap_or(false)
        };
        let mut delivered = 0;
        for run in waiting {
            let job = self.jobs_store.as_ref().and_then(|s| s.cron_get(&run.job_id).ok().flatten());
            let expired = match (&job, run.scheduled_for_ms) {
                (Some(job), Some(slot)) => {
                    // A pause ends it at once; an age bound only once passed;
                    // max_fires reached means it may not fire at all.
                    let stopped = !job.enabled && !self.within_age_bound(job, now);
                    let at_max = job.max_fires.is_some_and(|max| job.fire_count >= max);
                    stopped || at_max || jobs::next_slot_after(job, slot).is_none_or(|next| next <= now)
                }
                _ => true,
            };
            if expired {
                runs.expire_pending(&run.id);
                continue;
            }
            // Pause all holds it, until it expires as above.
            if paused_all {
                continue;
            }
            let Some(job) = job else { continue };
            let back = match (self.presence.get(), job.target_uid.is_empty()) {
                (Some(present), false) => present(&job.target_uid),
                // A job by name, or no way to tell: try it.
                _ => true,
            };
            if !back {
                continue;
            }
            // It waits for its place like any other run: policy decides, and a
            // pending fire that can't start yet stays pending.
            let may_start = {
                let (runs, job) = (runs.clone(), job.clone());
                let run_id = run.id.clone();
                tokio::task::spawn_blocking(move || runs.pending_may_start(&job, &run_id)).await.unwrap_or(false)
            };
            if !may_start {
                continue;
            }
            let delivery_started = agentmux_common::time::now_ms();
            let Some(outcome) = self.deliver(&job, Some(&run.id), run.scheduled_for_ms).await else {
                runs.pending_delivery_done(&run.id);
                continue;
            };
            runs.record_pending_delivery(&run, &outcome, delivery_started);
            runs.pending_delivery_done(&run.id);
            // Only a delivery is recorded here: the slot already counted as one
            // miss when it first went undelivered, and a pending fire that
            // misses again stays pending (it is retried, not a new miss).
            if outcome.delivered() {
                self.record_result(&job.id, &outcome);
                self.stop_at_max_fires(&job.id);
                delivered += 1;
                self.publish_changed();
            }
        }
        delivered
    }

    /// The pre-M4c-3 fire: POST to the local inject route with the instance
    /// key. Only until [`Self::install_delivery`] runs. `None` when the POST
    /// itself failed (logged here).
    async fn post_fire(&self, id: &str, target_agent: String, prompt: &str, request_id: Option<String>) -> Option<serde_json::Value> {
        let url = format!("{}/agentmux/reactive/inject", self.local_url.trim_end_matches('/'));
        let req = InjectRequest {
            target_agent,
            message: prompt.to_string(),
            source_agent: Some("cron".to_string()),
            request_id,
            ..Default::default()
        };
        match self.http_client.post(&url).header(AUTH_KEY_HEADER, &self.auth_key).json(&req).send().await {
            Ok(r) if r.status().is_success() => Some(r.json().await.unwrap_or_default()),
            Ok(r) => {
                tracing::warn!(id, status = %r.status(), "cron: inject returned non-2xx");
                None
            }
            Err(e) => {
                tracing::warn!(id, error = %e, "cron: inject request failed");
                None
            }
        }
    }
}

/// Determine if a job needs a catch-up fire on startup.
///
/// True when there is a scheduled fire time that falls strictly between
/// `last_fired` and `now` — i.e., the first occurrence of the cron expression
/// after `last_fired` is already in the past. This is correct regardless of
/// schedule granularity: a daily job that ran at 09:00 and is restarted at
/// 09:05 yields a next-after-last of TOMORROW 09:00, which is NOT < now, so
/// no spurious catch-up fires.
///
/// A job that has never fired counts from its creation: a slot after it was
/// created and already past was missed too (jobs spec §5.2).
fn should_catchup(job: &CronJob, now_dt: DateTime<Utc>) -> bool {
    let last = catch_up_base_secs(job);
    let last_dt = match DateTime::from_timestamp(last, 0) {
        Some(dt) => dt,
        None => return false,
    };
    let Ok(schedule) = schedule::JobSchedule::of(job) else {
        return false;
    };
    // First scheduled time after last_fired — if it's already past, a fire was missed.
    match schedule.next_after(last_dt.timestamp_millis()) {
        Some(next_after_last) => next_after_last < now_dt.timestamp_millis(),
        None => false,
    }
}

/// Where missed slots are counted from: the last fire, or, for a job that
/// never fired, its creation.
fn catch_up_base_secs(job: &CronJob) -> i64 {
    job.last_fired.unwrap_or(job.created_at)
}

/// True when a job's hard age-expiry bound (`max_age_secs`, seconds since
/// `created_at`) has been reached. `None` = no bound, never expires by age
/// (the default for every job created before this field existed, and for
/// any job that explicitly opts out). See
/// docs/specs/SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md Phase 0.
pub(crate) fn is_expired_by_age(created_at: i64, max_age_secs: Option<i64>, now: i64) -> bool {
    match max_age_secs {
        Some(max_age) => now - created_at >= max_age,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_expired_by_age_none_never_expires() {
        assert!(!is_expired_by_age(1_000, None, 1_000_000_000));
    }

    #[test]
    fn is_expired_by_age_before_bound_is_not_expired() {
        assert!(!is_expired_by_age(1_000, Some(3_600), 1_000 + 3_599));
    }

    #[test]
    fn is_expired_by_age_at_bound_is_expired() {
        // >= at the boundary, matching the max_fires check's own >= semantics.
        assert!(is_expired_by_age(1_000, Some(3_600), 1_000 + 3_600));
    }

    #[test]
    fn is_expired_by_age_past_bound_is_expired() {
        assert!(is_expired_by_age(1_000, Some(3_600), 1_000 + 10_000));
    }

    /// Regression test for PR #2418's P1 review finding: the scheduler now
    /// calls this same function with the *next scheduled fire's* timestamp,
    /// not just "now", to decide whether to skip a fire that would land past
    /// expiry rather than sleeping through the bound and firing anyway. A
    /// job created 1000s ago with a 3600s bound, checked against a `next`
    /// fire time far past that bound (e.g. a daily cron job's next fire is
    /// hours away), must report expired — this is exactly the "would this
    /// fire itself violate the hard expiry bound" check the loop performs
    /// before deciding whether to sleep at all.
    #[test]
    fn is_expired_by_age_catches_a_next_fire_time_past_the_bound() {
        let created_at = 1_000;
        let max_age_secs = Some(3_600);
        let next_fire_far_in_the_future = created_at + 86_400; // 24h away
        assert!(is_expired_by_age(created_at, max_age_secs, next_fire_far_in_the_future));
    }
}

/// A job for tests: every minute, enabled, never fired, catch-up `skip`.
#[cfg(test)]
pub(crate) fn test_job(id: &str, prompt: &str, target: &str, target_uid: &str, created_by_uid: &str) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: format!("{id} name"),
        expression: "* * * * *".to_string(),
        prompt: prompt.to_string(),
        target: target.to_string(),
        target_uid: target_uid.to_string(),
        created_by: "creator".to_string(),
        created_by_uid: created_by_uid.to_string(),
        enabled: true,
        last_fired: None,
        fire_count: 0,
        max_fires: None,
        created_at: 0,
        max_age_secs: None,
        catch_up: jobs::CATCH_UP_SKIP.to_string(),
        ..Default::default()
    }
}

/// What a cron fire addresses: the UID when one was captured at creation,
/// else the name (counted, spec §9.2 — `cron.fire_by_name` reaching zero is
/// part of M5's exit criterion).
fn fire_target<'a>(target: &'a str, target_uid: &'a str) -> &'a str {
    let uid = target_uid.trim();
    if uid.is_empty() {
        crate::backend::agent_resolve::record_uid_fallback("cron.fire_by_name");
        target
    } else {
        uid
    }
}

#[cfg(test)]
mod identity_m3_tests {
    use super::fire_target;

    #[test]
    fn fires_by_uid_when_captured_and_by_name_otherwise() {
        assert_eq!(fire_target("AgentY", "4f3c-a91"), "4f3c-a91");
        assert_eq!(fire_target("AgentY", ""), "AgentY");
        assert_eq!(fire_target("AgentY", "   "), "AgentY");
    }
}

#[cfg(test)]
mod identity_m4c3_tests {
    use super::*;

    /// Identity M4c-3 (spec §6.5.9): with delivery installed, a fire goes in
    /// process — by target UID, as `"cron"`, with the creator's UID beside
    /// it — and never over HTTP.
    #[tokio::test]
    async fn a_fire_is_delivered_in_process_as_cron_with_the_creators_uid() {
        let sched = CronScheduler::new(
            None,
            reqwest::Client::new(),
            // Unreachable: a fire that fell back to HTTP could not succeed.
            "http://127.0.0.1:1".to_string(),
            "test".to_string(),
            Arc::new(Broker::new()),
        );
        let seen: Arc<Mutex<Vec<(InjectionRequest, String)>>> = Arc::default();
        let sink = seen.clone();
        sched.install_delivery(Arc::new(move |req, creator_uid| {
            sink.lock().unwrap().push((req, creator_uid));
            Box::pin(async { serde_json::json!({"success": true}) })
        }));

        sched.fire(&test_job("job-1", "check in", "AgentY", "uid-target", "uid-creator"), Slot::Scheduled(0)).await;

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (req, creator_uid) = &seen[0];
        assert_eq!(req.target_agent, "uid-target");
        assert_eq!(req.message, "check in", "no ledger installed: no run line");
        assert_eq!(req.source_agent.as_deref(), Some("cron"), "the sender name stays cron");
        assert_eq!(creator_uid, "uid-creator");
        assert_eq!(req.audit_source_uid, "", "the delivery path sets it, on this instance only");
    }
}


#[cfg(test)]
mod missed_fire_tests {
    use super::*;
    use serde_json::json;

    fn sched_with(store: Option<Arc<Store>>, answer: serde_json::Value) -> Arc<CronScheduler> {
        let sched = CronScheduler::new(
            store,
            reqwest::Client::new(),
            "http://127.0.0.1:1".to_string(),
            "test".to_string(),
            Arc::new(Broker::new()),
        );
        sched.install_delivery(Arc::new(move |_, _| {
            let answer = answer.clone();
            Box::pin(async move { answer })
        }));
        sched
    }

    fn store_with(job: &CronJob) -> Arc<Store> {
        let store = Arc::new(Store::open_in_memory().expect("store"));
        store.apply_identity_schema_for_tests().expect("the identity schema, which holds db_cron_jobs");
        store.cron_create(job).expect("create");
        store
    }

    #[test]
    fn reads_the_inject_response() {
        assert_eq!(
            fire_outcome(Some(&json!({"success": true, "block_id": "b-1"}))),
            FireOutcome::Delivered { block_id: Some("b-1".into()) }
        );
        assert_eq!(
            fire_outcome(Some(&json!({"success": false, "error": "agent not found: uid-t"}))),
            FireOutcome::Missed { reason: MissReason::TargetStopped }
        );
        assert!(matches!(
            fire_outcome(Some(&json!({"success": false, "error": "ambiguous agent name: 'AgentY'"}))),
            FireOutcome::Failed { .. }
        ));
        assert!(matches!(fire_outcome(None), FireOutcome::Failed { .. }));
    }

    #[test]
    fn a_miss_says_why_and_a_delivery_has_no_reason() {
        assert_eq!(FireOutcome::Delivered { block_id: None }.miss_reason(), None);
        assert_eq!(
            FireOutcome::Missed { reason: MissReason::TargetStopped }.miss_reason().as_deref(),
            Some("target_stopped")
        );
        assert_eq!(
            FireOutcome::Missed { reason: MissReason::TargetMissing }.miss_reason().as_deref(),
            Some("target_missing")
        );
        assert_eq!(FireOutcome::Failed { error: "x".into() }.miss_reason().as_deref(), Some("failed: x"));
    }

    /// A scheduled fire nobody received is recorded as a miss with its
    /// reason, and `fire_count` (which `max_fires` reads) is left alone.
    #[tokio::test]
    async fn an_undelivered_fire_is_a_miss_not_a_fire() {
        let job = test_job("job-m", "tick", "AgentZ", "uid-z", "uid-y");
        let store = store_with(&job);
        let sched = sched_with(Some(store.clone()), json!({"success": false, "error": "agent not found: uid-z"}));

        let fired = sched.fire(&job, Slot::Scheduled(0)).await;
        assert_eq!(fired.outcome, Some(FireOutcome::Missed { reason: MissReason::TargetStopped }));
        let stored = store.cron_get("job-m").unwrap().unwrap();
        assert_eq!((stored.fire_count, stored.last_fired), (0, None));
        assert_eq!((stored.missed_count, stored.last_miss_reason.as_str()), (1, "target_stopped"));
        assert!(stored.last_missed_at.is_some());
    }

    #[tokio::test]
    async fn a_delivered_fire_counts_and_leaves_the_miss_record() {
        let job = test_job("job-d", "tick", "AgentZ", "uid-z", "uid-y");
        let store = store_with(&job);
        let sched = sched_with(Some(store.clone()), json!({"success": true, "block_id": "b-1"}));

        sched.fire(&job, Slot::Scheduled(0)).await;
        let stored = store.cron_get("job-d").unwrap().unwrap();
        assert_eq!((stored.fire_count, stored.missed_count), (1, 0));
        assert!(stored.last_fired.is_some());
    }

    /// A job with no target misses without trying to deliver.
    #[tokio::test]
    async fn a_job_with_no_target_is_a_miss() {
        let job = test_job("job-n", "tick", "", "", "uid-y");
        let store = store_with(&job);
        let sched = sched_with(Some(store.clone()), json!({"success": true}));

        let fired = sched.fire(&job, Slot::Scheduled(0)).await;
        assert_eq!(fired.outcome, Some(FireOutcome::Missed { reason: MissReason::TargetMissing }));
        assert_eq!(store.cron_get("job-n").unwrap().unwrap().last_miss_reason, "target_missing");
    }

    /// Run now is the ledger's alone: neither a fire nor a miss on the job.
    #[tokio::test]
    async fn run_now_counts_neither_way() {
        let job = test_job("job-r", "tick", "AgentZ", "uid-z", "uid-y");
        let store = store_with(&job);
        let sched = sched_with(Some(store.clone()), json!({"success": false, "error": "agent not found: uid-z"}));

        sched.fire(&job, Slot::Manual { triggered_by: "user".into() }).await;
        let stored = store.cron_get("job-r").unwrap().unwrap();
        assert_eq!((stored.fire_count, stored.missed_count), (0, 0));
    }
}

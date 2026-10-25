// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Spending caps at work (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.3): the reservation before a run starts, the stop when a run reaches
//! its own cap, and the skips a cap makes. The rules are `caps.rs`; this
//! reads the spend from the ledger and writes the rows.
//!
//! **The reservation.** A run starts only if its job's spend today plus its
//! per-run cap fits the job's daily cap, and the same for its account. The
//! spend counts the runs that ended, and, for each run still going, its
//! whole per-run cap: so the day's total can't pass a daily cap even if every
//! run spends all of its own.
//!
//! **The stop.** The turn ledger's cost is the sum of the `result` frames of
//! the passes that ended (`blockcontroller::health`), so the follower sees a
//! run's spend between passes, not inside one: a turn of several passes is
//! stopped at the first pass that brings it to its cap, and a single pass
//! costs what it costs. The reservation is what bounds the day.
//!
//! **No cost reported.** A run on a provider whose turns carry no cost counts
//! $0 toward the dollar caps (`caps::reports_cost`) and is bounded by its
//! timeout.

use std::sync::Arc;

use agentmux_common::time::now_ms;

use super::caps::{self, Limits, Refusal, Spend};
use super::{policy, RunLedger};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{reason, RunCost};

/// The caps Settings → Jobs holds, read at each decision so a change takes
/// effect at once.
pub type LimitsSource = Arc<dyn Fn() -> Limits + Send + Sync>;

/// Where a job's next run would go: the server knows (a message's target
/// pane, the account a spawn picks).
pub type TargetOf = Arc<dyn Fn(&CronJob) -> Target + Send + Sync>;

/// The account and provider a run would use. Empty when not known.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    pub account: String,
    pub provider: String,
}

pub struct CapSources {
    limits: LimitsSource,
    target: TargetOf,
}

/// What the caps say about starting a run now.
pub(super) struct Gate {
    pub refusal: Option<Refusal>,
    /// The account the run would use, to note on its row.
    pub account: String,
}

impl RunLedger {
    /// Weigh runs against Settings' caps from now on, finding each run's
    /// account and provider with `target`. Once. Without it the defaults
    /// apply and only a job's own spend is weighed.
    pub fn install_caps(&self, limits: LimitsSource, target: TargetOf) {
        let _ = self.caps.set(CapSources { limits, target });
    }

    /// Tests: a day that starts as `f` says, from now on (a test moves it to
    /// cross midnight).
    #[cfg(test)]
    pub fn set_day_start(&self, f: Arc<dyn Fn(i64) -> i64 + Send + Sync>) {
        *self.day_start.lock().unwrap() = Some(f);
    }

    /// Tests: the cap alerts raised so far, `(job id, reason)`.
    #[cfg(test)]
    pub fn cap_alerts(&self) -> Vec<(String, String)> {
        self.cap_alerts.lock().unwrap().clone()
    }

    pub fn limits(&self) -> Limits {
        self.caps.get().map(|c| (c.limits)()).unwrap_or_default()
    }

    fn target(&self, job: &CronJob) -> Target {
        self.caps.get().map(|c| (c.target)(job)).unwrap_or_default()
    }

    /// The start of the operator's local day containing `now`.
    pub(super) fn day_start(&self, now: i64) -> i64 {
        #[cfg(test)]
        if let Some(f) = self.day_start.lock().unwrap().clone() {
            return f(now);
        }
        caps::local_day_start_ms(now)
    }

    /// The most one run of `job` may spend.
    pub fn run_cap_usd(&self, job: &CronJob) -> f64 {
        caps::effective(job.policy.caps.as_ref(), &self.limits()).run_usd
    }

    /// The spend at which run `run_id` is stopped, when its provider reports
    /// what a turn costs. `None` for a run that can't be weighed.
    pub(super) fn stop_at_usd(&self, run_id: &str, provider: &str) -> Option<f64> {
        if !caps::reports_cost(provider) {
            return None;
        }
        let run = self.store.job_run_get(run_id).ok().flatten()?;
        let job = self.job(&run.job_id)?;
        policy::starts_an_agent(&job).then(|| self.run_cap_usd(&job))
    }

    /// What `job` has spent since `day_start`, and what its runs still going
    /// may yet spend, for the job and for `account`.
    fn spend(&self, job: &CronJob, account: &str, day_start: i64) -> Spend {
        let limits = self.limits();
        let (mut job_usd, _) = self.store.job_runs_cost_since(&job.id, day_start).unwrap_or_default();
        let mut account_usd = (!account.is_empty()).then(|| self.store.account_runs_cost_since(account, day_start).unwrap_or(0.0));
        // A pending fire being delivered holds its place before its row says so.
        let occupying = self.store.job_runs_occupying().unwrap_or_default();
        let claimed: Vec<String> = {
            let claims = self.pending_claims.lock().unwrap_or_else(|e| e.into_inner());
            claims.keys().filter(|id| !occupying.iter().any(|r| &r.id == *id)).cloned().collect()
        };
        let claimed_rows = claimed.iter().filter_map(|id| self.store.job_run_get(id).ok().flatten());
        for r in occupying.into_iter().chain(claimed_rows) {
            let Some(other) = self.job(&r.job_id).filter(policy::starts_an_agent) else { continue };
            if !caps::reports_cost(&r.provider) {
                continue;
            }
            let held = caps::effective(other.policy.caps.as_ref(), &limits).run_usd;
            if r.job_id == job.id {
                job_usd += held;
            }
            if let Some(a) = account_usd.as_mut().filter(|_| r.account_id == account) {
                *a += held;
            }
        }
        let messages = if caps::sends_to_another(job) {
            self.store.job_runs_delivered_since(&job.id, day_start).unwrap_or(0)
        } else {
            0
        };
        Spend { job_usd, account_usd, messages }
    }

    /// May `job`'s next run start? Filing a task spends nothing, so it always may.
    pub(super) fn cap_gate(&self, job: &CronJob) -> Gate {
        if !policy::starts_an_agent(job) {
            return Gate { refusal: None, account: String::new() };
        }
        let target = self.target(job);
        let limits = self.limits();
        let mut eff = caps::effective(job.policy.caps.as_ref(), &limits);
        if !caps::reports_cost(&target.provider) {
            eff.run_usd = 0.0;
        }
        let spend = self.spend(job, &target.account, self.day_start(now_ms()));
        Gate { refusal: caps::check(&eff, &limits, &spend, caps::sends_to_another(job)), account: target.account }
    }

    /// The cap `job` is over today, for the Jobs tab ("over cap today"): its
    /// own daily cap or messages a day as of now, or an account cap that
    /// skipped one of its runs today.
    pub fn over_cap_today(&self, job: &CronJob) -> Option<&'static str> {
        if !policy::starts_an_agent(job) {
            return None;
        }
        let limits = self.limits();
        let mut eff = caps::effective(job.policy.caps.as_ref(), &limits);
        if !caps::reports_cost(&self.target(job).provider) {
            eff.run_usd = 0.0;
        }
        let day_start = self.day_start(now_ms());
        let spend = self.spend(job, "", day_start);
        if let Some(over) = caps::check(&eff, &limits, &spend, caps::sends_to_another(job)) {
            return Some(over.reason());
        }
        let skipped = self.store.job_cap_skips_since(&job.id, day_start).unwrap_or_default();
        skipped.iter().any(|r| r == reason::ACCOUNT_CAP).then_some(reason::ACCOUNT_CAP)
    }

    /// Whether `job` has no cap skip yet today: the next one raises the alert.
    fn first_cap_skip_today(&self, job: &CronJob) -> bool {
        self.store.job_cap_skips_since(&job.id, self.day_start(now_ms())).map(|v| v.is_empty()).unwrap_or(false)
    }

    /// A scheduled slot of `job` that a cap skips: a collapsed `skipped` row,
    /// and, for the job's first cap skip of the day, the alert.
    pub(super) fn skip_slot_for_cap(&self, job: &CronJob, slot_ms: i64, refusal: Refusal) {
        let first = self.first_cap_skip_today(job);
        self.record_skip_slot(job, slot_ms, refusal.reason());
        if first {
            self.alert_cap(job, refusal);
        }
    }

    /// A run that already has its row (it waited, or was pending) and that a
    /// cap skips.
    pub(super) fn skip_run_for_cap(&self, run_id: &str, job: &CronJob, refusal: Refusal) {
        let first = self.first_cap_skip_today(job);
        if let Err(e) = self.store.job_run_skip(run_id, refusal.reason(), 1, now_ms()) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't skip a run for a cap");
        }
        self.changed(&job.id);
        if first {
            self.alert_cap(job, refusal);
        }
    }

    /// The follower saw run `run_id` reach `cap` dollars: end it `over_budget`,
    /// interrupt its turn, and stop a worker it started (§5.3).
    pub(crate) fn over_budget(&self, run_id: &str, cost: RunCost, cap: f64) {
        let Some(run) = self.store.job_run_get(run_id).ok().flatten() else { return };
        let summary = format!("stopped: the run reached its spending cap (${cap:.2}; spent ${:.2})", cost.cost_usd);
        match self.store.job_run_over_budget(run_id, cost, &summary, now_ms()) {
            Ok(true) => {
                tracing::info!(run_id, job_id = %run.job_id, spent = cost.cost_usd, cap, "jobs: a run reached its spending cap and was stopped");
                self.interrupt_block(&run.block_id);
                self.stop_worker(run_id, true);
                self.run_changed(run_id);
                super::pending_wake().notify_one();
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(run_id, error = %e, "jobs: couldn't stop an over-budget run"),
        }
    }
}

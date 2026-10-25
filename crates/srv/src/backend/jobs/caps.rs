// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Spending caps for job runs (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.3): the defaults Settings → Jobs holds, the figures a job's
//! `policy.caps` may lower, and the rules that decide, from the spend the
//! ledger found, whether a run may start. Pure: `reservation.rs` reads the
//! ledger and acts on the answer.
//!
//! A day is the operator's local calendar day (§10, decision 7).

use chrono::{NaiveTime, TimeZone};

use crate::backend::jobs::definition::Caps;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::reason;
use crate::backend::wconfig::types::SettingsType;

/// The defaults when Settings leaves a cap unset (§5.3, operator decision 2).
pub const DEFAULT_RUN_USD: f64 = 5.0;
pub const DEFAULT_JOB_DAY_USD: f64 = 20.0;
pub const DEFAULT_ACCOUNT_DAY_USD: f64 = 50.0;
pub const DEFAULT_MESSAGES_PER_DAY: u32 = 24;

/// Room left on a cap smaller than this counts as none: dollar sums are
/// floating point.
const EPSILON: f64 = 1e-9;

/// The caps Settings → Jobs holds: what a job without figures of its own
/// gets, and the most an agent can ask for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub run_usd: f64,
    pub job_day_usd: f64,
    pub account_day_usd: f64,
    pub messages_per_day: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            run_usd: DEFAULT_RUN_USD,
            job_day_usd: DEFAULT_JOB_DAY_USD,
            account_day_usd: DEFAULT_ACCOUNT_DAY_USD,
            messages_per_day: DEFAULT_MESSAGES_PER_DAY,
        }
    }
}

impl Limits {
    /// Read `jobs:cap:*`. A figure that isn't a number of 0 or more is the
    /// default: a typo must not turn a cap off.
    pub fn from_settings(s: &SettingsType) -> Limits {
        let usd = |v: Option<f64>, default: f64| v.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(default);
        Limits {
            run_usd: usd(s.jobs_cap_run_usd, DEFAULT_RUN_USD),
            job_day_usd: usd(s.jobs_cap_job_day_usd, DEFAULT_JOB_DAY_USD),
            account_day_usd: usd(s.jobs_cap_account_day_usd, DEFAULT_ACCOUNT_DAY_USD),
            messages_per_day: s.jobs_cap_messages_day.unwrap_or(DEFAULT_MESSAGES_PER_DAY),
        }
    }
}

/// The figures that apply to one job: its own, else the defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effective {
    pub run_usd: f64,
    pub day_usd: f64,
    pub messages_per_day: u32,
}

pub fn effective(caps: Option<&Caps>, limits: &Limits) -> Effective {
    Effective {
        run_usd: caps.and_then(|c| c.run_usd).unwrap_or(limits.run_usd),
        day_usd: caps.and_then(|c| c.day_usd).unwrap_or(limits.job_day_usd),
        messages_per_day: caps.and_then(|c| c.messages_per_day).unwrap_or(limits.messages_per_day),
    }
}

/// An agent's `caps`, held to the defaults: a higher figure becomes the
/// default (only the operator raises a cap, §7). A figure the job already had
/// (`before`, on an edit) is kept as it is, so an agent changing a job's brief
/// doesn't lower a cap the operator raised. Returns the caps to store and, for
/// each figure changed, a sentence for the reply.
pub fn clamp_to_defaults(caps: Option<Caps>, limits: &Limits, before: Option<&Caps>) -> (Option<Caps>, Vec<String>) {
    let Some(mut caps) = caps else { return (None, Vec::new()) };
    let mut notes = Vec::new();
    let mut dollars = |name: &str, field: &mut Option<f64>, was: Option<f64>, default: f64| {
        if let Some(asked) = field.filter(|v| *v > default + EPSILON && was != Some(*v)) {
            notes.push(format!(
                "policy.caps.{name} {asked} is over the default {default}, so {default} applies: only the operator raises a cap"
            ));
            *field = Some(default);
        }
    };
    dollars("run_usd", &mut caps.run_usd, before.and_then(|b| b.run_usd), limits.run_usd);
    dollars("day_usd", &mut caps.day_usd, before.and_then(|b| b.day_usd), limits.job_day_usd);
    let was = before.and_then(|b| b.messages_per_day);
    if let Some(asked) = caps.messages_per_day.filter(|v| *v > limits.messages_per_day && was != Some(*v)) {
        let default = limits.messages_per_day;
        notes.push(format!(
            "policy.caps.messages_per_day {asked} is over the default {default}, so {default} applies: only the operator raises a cap"
        ));
        caps.messages_per_day = Some(default);
    }
    (Some(caps), notes)
}

/// Local midnight at or before `now_ms` in `tz`, ms epoch. A zone whose
/// midnight doesn't exist that day (a daylight-saving gap at 00:00) starts
/// its day at the first hour that does.
pub fn day_start_ms<Tz: TimeZone>(now_ms: i64, tz: &Tz) -> i64 {
    let Some(now) = tz.timestamp_millis_opt(now_ms).single() else { return now_ms - 24 * 3_600_000 };
    for hour in 0..=3 {
        let start = now.date_naive().and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap_or_default());
        if let Some(at) = tz.from_local_datetime(&start).earliest() {
            return at.timestamp_millis();
        }
    }
    now_ms - 24 * 3_600_000
}

/// The operator's local midnight (the machine's time zone).
pub fn local_day_start_ms(now_ms: i64) -> i64 {
    day_start_ms(now_ms, &chrono::Local)
}

/// What a start may not do, in the order checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    JobCap,
    AccountCap,
    MessageCap,
}

impl Refusal {
    /// The run's `reason`.
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::JobCap => reason::JOB_CAP,
            Refusal::AccountCap => reason::ACCOUNT_CAP,
            Refusal::MessageCap => reason::MESSAGE_CAP,
        }
    }

    /// A few words for a notification's title.
    pub fn label(self) -> &'static str {
        match self {
            Refusal::JobCap | Refusal::AccountCap => "daily spending cap",
            Refusal::MessageCap => "daily message limit",
        }
    }

    /// In words, for Run now's refusal.
    pub fn words(self) -> &'static str {
        match self {
            Refusal::JobCap => "this job's spending for today leaves no room for another run (its daily cap)",
            Refusal::AccountCap => "the account's spending on jobs for today leaves no room for another run (the account's daily cap)",
            Refusal::MessageCap => "this job has sent its messages for today to another agent (messages_per_day)",
        }
    }
}

/// The spend a start is weighed against: what was spent today plus what the
/// runs in flight may still spend (each at its whole per-run cap).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spend {
    pub job_usd: f64,
    /// `None` when the run's account isn't known: only the job's cap applies.
    pub account_usd: Option<f64>,
    /// Messages delivered today (a `message` job to another agent).
    pub messages: i64,
}

/// The reservation (§5.3): the run starts only if the job's spend today plus
/// the run's whole per-run cap fits the job's daily cap, and the same for its
/// account. Every run that starts this way spends at most its cap, so the
/// day's total can't pass the daily cap even if each one spends all of it.
pub fn check(eff: &Effective, limits: &Limits, spend: &Spend, sends_to_another: bool) -> Option<Refusal> {
    if spend.job_usd + eff.run_usd > eff.day_usd + EPSILON {
        return Some(Refusal::JobCap);
    }
    if spend.account_usd.is_some_and(|a| a + eff.run_usd > limits.account_day_usd + EPSILON) {
        return Some(Refusal::AccountCap);
    }
    if sends_to_another && spend.messages >= i64::from(eff.messages_per_day) {
        return Some(Refusal::MessageCap);
    }
    None
}

/// Whether a run that has spent `cost_usd` has reached `cap_usd`: it is
/// stopped. A run that spent nothing hasn't, whatever the cap.
pub fn reached(cost_usd: f64, cap_usd: f64) -> bool {
    cost_usd > 0.0 && cost_usd + EPSILON >= cap_usd
}

/// Whether a run on `provider` reports what it cost. Only Claude's `result`
/// frame carries a cost today (`PassStats::from_result_frame`); any other
/// provider's runs have `cost_known = false`, count $0 toward the dollar caps
/// and are bounded by their timeout (§5.3). A provider not known yet is
/// assumed to report one.
pub fn reports_cost(provider: &str) -> bool {
    provider.trim().is_empty() || crate::backend::providers::resolve_provider_alias(provider) == "claude"
}

/// Whether `job` is a `message` job aimed at an agent other than its owner
/// (§5.3). A job with no owner, the operator's own, isn't: the limit is for
/// an agent's job that could flood another agent's conversation.
pub fn sends_to_another(job: &CronJob) -> bool {
    if !matches!(job.action_kind.as_str(), "" | "message") {
        return false;
    }
    let owner = job.owner_uid();
    if owner.is_empty() && job.created_by.trim().is_empty() {
        return false;
    }
    if !owner.is_empty() && !job.target_uid.is_empty() {
        return job.target_uid != owner;
    }
    // A job by name: compare names with its creator's.
    let (target, creator) = (job.target.trim(), job.created_by.trim());
    if creator.eq_ignore_ascii_case("operator") && owner.is_empty() {
        return false;
    }
    !target.eq_ignore_ascii_case(creator)
}

#[cfg(test)]
#[path = "caps_tests.rs"]
mod tests;

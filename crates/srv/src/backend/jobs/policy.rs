// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What a job's policy decides about a run before it starts, and what to do
//! when it failed (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1
//! "Policy"). Pure: the ledger gathers who is running and acts on the answer
//! (`admission.rs`).

use crate::backend::jobs::definition::Overlap;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{infra, JobRun};

/// Unattended agent runs at once, across all jobs (§5.1): the providers'
/// rate limits.
pub const GLOBAL_LIMIT: usize = 3;

/// The built-in concurrency group infrastructure deploys run in, one at a
/// time (§5.1, operator decision 6).
pub const GROUP_DEPLOY: &str = "deploy";

/// The wait before retry number 1, 2 and 3, in minutes (§5.1).
pub const RETRY_BACKOFF_MIN: [i64; 3] = [1, 5, 15];

/// The job's concurrency group: the one it names, or `deploy` when it
/// declares a deploy output (`deploy`, `deploy:prod`…).
pub fn group_of(job: &CronJob) -> Option<String> {
    if let Some(group) = job.policy.group.as_deref().filter(|g| !g.is_empty()) {
        return Some(group.to_string());
    }
    let deploys = job.outputs.iter().any(|o| o.split(':').next() == Some(GROUP_DEPLOY));
    deploys.then(|| GROUP_DEPLOY.to_string())
}

/// Whether a run of the job is an agent working unattended: a message starts
/// a turn, a spawn starts a worker. Filing a task starts nothing.
pub fn starts_an_agent(job: &CronJob) -> bool {
    matches!(job.action_kind.as_str(), "" | "message" | "spawn")
}

/// Who is running, as the ledger found it, for one job's next run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Occupancy {
    /// This job's runs that are running or being delivered.
    pub job_running: usize,
    /// Of those, the ones already running here (a `cancel_other` can stop them).
    pub job_started: usize,
    /// Of those, the ones running in another channel: stopped by asking that
    /// channel, so a `cancel_other` run waits for them to end.
    pub job_remote: usize,
    /// This job's runs waiting on its own previous run (`overlap`).
    pub job_buffered: usize,
    /// This job's runs that wait for a place (`group_busy`, `global_limit`):
    /// the job's next run is already queued, so another slot isn't.
    pub job_blocked: usize,
    /// Runs of the same concurrency group, in any job.
    pub group_running: usize,
    /// Unattended agent runs in all jobs.
    pub agents_running: usize,
}

/// What to do with a run that wants to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Start,
    /// Start after stopping the job's own running runs (`cancel_other`).
    CancelThenStart,
    /// Wait; the reason is what the row shows.
    Wait(&'static str),
    /// Don't run this slot: the previous run is still going.
    SkipOverlap,
}

/// The policy's answer for a run of `job`. `waiting` is a run that already
/// waits (it keeps its place; the buffer's one place isn't counted against
/// it, and it is never skipped).
pub fn decide(job: &CronJob, occ: &Occupancy, waiting: bool) -> Decision {
    use crate::backend::storage::job_runs::reason;
    let mut cancel = false;
    if occ.job_running > 0 {
        match job.policy.overlap {
            Overlap::Skip if waiting => return Decision::Wait(reason::OVERLAP),
            Overlap::Skip => return Decision::SkipOverlap,
            Overlap::BufferOne if waiting => return Decision::Wait(reason::OVERLAP),
            Overlap::BufferOne if occ.job_buffered > 0 => return Decision::SkipOverlap,
            Overlap::BufferOne => return Decision::Wait(reason::OVERLAP),
            // Only a started run can be stopped; one still being delivered
            // is left to finish its start, so the new slot is skipped.
            Overlap::CancelOther if occ.job_running > occ.job_started + occ.job_remote => return Decision::SkipOverlap,
            // A run in another channel is stopped by its follower, which takes a
            // moment: the new run waits for it, as one buffered run does.
            Overlap::CancelOther if occ.job_remote > 0 && (waiting || occ.job_buffered == 0) => return Decision::Wait(reason::OVERLAP),
            Overlap::CancelOther if occ.job_remote > 0 => return Decision::SkipOverlap,
            Overlap::CancelOther => cancel = true,
        }
    }
    // The runs a cancel_other stops don't hold the group or a place in the limit
    // against the run that replaces them.
    let mut occ = occ.clone();
    if cancel {
        occ.group_running = occ.group_running.saturating_sub(occ.job_started);
        if starts_an_agent(job) {
            occ.agents_running = occ.agents_running.saturating_sub(occ.job_started);
        }
    }
    let occ = &occ;
    // A run of this job already waits for a place: a further slot is the same
    // overlap as one behind a running run, whatever the policy keeps.
    if !waiting && occ.job_running == 0 && occ.job_blocked > 0 {
        return Decision::SkipOverlap;
    }
    if group_of(job).is_some() && occ.group_running > 0 {
        return Decision::Wait(reason::GROUP_BUSY);
    }
    if starts_an_agent(job) && occ.agents_running >= GLOBAL_LIMIT {
        return Decision::Wait(reason::GLOBAL_LIMIT);
    }
    if cancel {
        Decision::CancelThenStart
    } else {
        Decision::Start
    }
}

/// Whether a run that ended should be tried again (§5.1 "Retries"): only an
/// infrastructure failure: the agent never started, its process exited, or
/// the provider answered with a transient error. A run whose agent concluded
/// `failed`, one that hit its own timeout, and one that finished are never
/// retried; they alert.
pub fn infra_retryable(run: &JobRun) -> bool {
    if run.outcome == "failed" {
        return false;
    }
    match run.infra.as_str() {
        infra::INTERRUPTED => true,
        infra::ERROR => {
            crate::backend::reactive::is_transient_delivery_error(&run.summary)
                || crate::backend::reactive::is_unavailable_refusal(&run.summary)
                || crate::agents::failure::classify(None, None, &run.summary, None).retryable
        }
        _ => false,
    }
}

/// Attempt `attempt`'s retry delay in ms (attempt 1 failed: wait 1 minute).
pub fn retry_delay_ms(attempt: i64) -> i64 {
    let i = (attempt.max(1) as usize - 1).min(RETRY_BACKOFF_MIN.len() - 1);
    RETRY_BACKOFF_MIN[i] * 60_000
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What a run concluded for the Jobs tab: whether it failed, and whether its
//! job is failing (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.4), and
//! the state a job is in (§3.3).

use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{reason, state, JobRun};

/// Infrastructure statuses that fail a run (§3.4).
const FAILED_INFRA: &[&str] = &["error", "timeout", "over_budget", "interrupted"];

/// Whether `run` failed (§3.4): its infrastructure status is `error`,
/// `timeout`, `over_budget` or `interrupted`, it was `missed` or `lost`, or
/// it concluded `failed`. A pending fire that expired was never delivered:
/// a miss. `findings` is no failure, and neither is a skip.
pub fn run_failed(run: &JobRun) -> bool {
    FAILED_INFRA.contains(&run.infra.as_str())
        || run.state == state::MISSED
        || run.state == "lost"
        || (run.state == state::PENDING && run.reason == reason::EXPIRED)
        || run.outcome == "failed"
}

/// Whether `run` says how the job is doing: a finished run, or a failure.
/// Skips, and runs still scheduled, pending or running, say nothing yet.
fn decisive(run: &JobRun) -> bool {
    run.state == state::FINISHED || run_failed(run)
}

/// Whether a job whose runs are `newest_first` is failing: its newest
/// decisive run failed. The next success ends that (§3.4's incident).
pub fn job_failing(newest_first: &[JobRun]) -> bool {
    newest_first.iter().find(|r| decisive(r)).is_some_and(run_failed)
}

/// A job's state in the tab (§3.3).
pub mod job_state {
    pub const ACTIVE: &str = "active";
    pub const PAUSED: &str = "paused";
    /// Every job is paused by the operator.
    pub const PAUSED_ALL: &str = "paused_all";
    /// Disabled by its own limit (`max_fires`, `max_age_secs`), not paused.
    pub const ENDED: &str = "ended";
}

/// Whether a disabled job was stopped by its own limit rather than paused:
/// it reached `max_fires`, or its age bound passed by `now_secs`.
pub fn ended_by_limit(job: &CronJob, now_secs: i64) -> bool {
    job.max_fires.is_some_and(|max| job.fire_count >= max)
        || job.max_age_secs.is_some_and(|age| now_secs - job.created_at >= age)
}

/// The state of cron job `job` (§3.3).
pub fn cron_job_state(job: &CronJob, paused_all: bool, now_secs: i64) -> &'static str {
    if !job.enabled {
        return if ended_by_limit(job, now_secs) { job_state::ENDED } else { job_state::PAUSED };
    }
    if paused_all {
        job_state::PAUSED_ALL
    } else {
        job_state::ACTIVE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: &str, infra: &str, outcome: &str, reason: &str) -> JobRun {
        JobRun {
            state: state.into(),
            infra: infra.into(),
            outcome: outcome.into(),
            reason: reason.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_run_fails_on_its_infrastructure_a_miss_or_its_own_verdict() {
        for failed in [
            run("finished", "error", "unknown", ""),
            run("finished", "timeout", "unknown", ""),
            run("finished", "over_budget", "unknown", ""),
            run("finished", "interrupted", "ok", ""),
            run("missed", "", "unknown", "target_stopped"),
            run("lost", "", "unknown", ""),
            run("pending", "", "unknown", "expired"),
            run("finished", "ok", "failed", ""),
        ] {
            assert!(run_failed(&failed), "{failed:?}");
        }
        for fine in [
            run("finished", "ok", "ok", ""),
            run("finished", "ok", "findings", ""),
            run("finished", "ok", "unknown", ""),
            run("skipped", "", "unknown", "paused"),
            run("skipped", "", "unknown", "paused_all"),
            run("pending", "", "unknown", "superseded"),
            run("pending", "", "unknown", "target_stopped"),
            run("running", "", "unknown", ""),
        ] {
            assert!(!run_failed(&fine), "{fine:?}");
        }
    }

    #[test]
    fn a_job_is_failing_until_its_next_success_and_skips_dont_change_that() {
        let failed = run("finished", "error", "unknown", "");
        let ok = run("finished", "ok", "ok", "");
        let skip = run("skipped", "", "unknown", "paused");
        let running = run("running", "", "unknown", "");
        assert!(job_failing(&[skip.clone(), running.clone(), failed.clone(), ok.clone()]));
        assert!(!job_failing(&[ok.clone(), failed.clone()]), "the success after it ends it");
        assert!(!job_failing(&[skip, running]), "nothing decisive yet");
        assert!(!job_failing(&[]));
    }

    #[test]
    fn a_disabled_job_is_paused_unless_its_own_limit_stopped_it() {
        let mut job = crate::backend::cron::test_job("j", "p", "t", "", "");
        assert_eq!(cron_job_state(&job, false, 0), job_state::ACTIVE);
        assert_eq!(cron_job_state(&job, true, 0), job_state::PAUSED_ALL);
        job.enabled = false;
        assert_eq!(cron_job_state(&job, true, 0), job_state::PAUSED, "its own pause shows over Pause all");
        job.max_fires = Some(3);
        job.fire_count = 3;
        assert_eq!(cron_job_state(&job, false, 0), job_state::ENDED);
        job.max_fires = None;
        job.max_age_secs = Some(60);
        assert_eq!(cron_job_state(&job, false, 59), job_state::PAUSED);
        assert_eq!(cron_job_state(&job, false, 60), job_state::ENDED);
    }
}

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
/// The store's `job_run_last_decisive` picks the newest such run in SQL.
#[cfg(test)]
fn decisive(run: &JobRun) -> bool {
    (run.state == state::FINISHED || run_failed(run)) && run.links.get("retried_by").is_none()
}

/// Whether a job is failing, from its newest decisive run
/// (`Store::job_run_last_decisive`): that run failed. The next success ends
/// it (§3.4's incident), however many skips come between.
pub fn job_failing(last_decisive: Option<&JobRun>) -> bool {
    last_decisive.is_some_and(run_failed)
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

    /// The store's SQL pick of the newest decisive run agrees with
    /// [`decisive`] on every shape of run.
    #[test]
    fn the_store_and_this_agree_on_which_runs_are_decisive() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        store.job_runs_ensure_schema().unwrap();
        let shapes = [
            run("finished", "ok", "ok", ""),
            run("finished", "ok", "unknown", ""),
            run("finished", "error", "unknown", ""),
            run("missed", "", "unknown", "target_stopped"),
            run("lost", "", "unknown", ""),
            run("pending", "", "unknown", "expired"),
            run("pending", "", "unknown", "superseded"),
            run("pending", "", "unknown", "target_stopped"),
            run("skipped", "", "unknown", "paused"),
            run("scheduled", "", "unknown", ""),
            run("running", "", "unknown", ""),
            run("running", "interrupted", "unknown", ""),
            run("running", "", "failed", ""),
        ];
        for (i, shape) in shapes.iter().enumerate() {
            let mut r = shape.clone();
            r.id = format!("r{i}");
            r.job_id = format!("j{i}");
            r.job_source = "cron".into();
            r.trigger = "schedule".into();
            r.links = serde_json::json!({});
            store.job_run_insert(&r).unwrap();
            let picked = store.job_run_last_decisive(&r.job_id).unwrap().is_some();
            assert_eq!(picked, decisive(&r), "{r:?}");
        }
    }

    #[test]
    fn a_job_is_failing_when_its_newest_decisive_run_failed() {
        assert!(job_failing(Some(&run("finished", "error", "unknown", ""))));
        assert!(!job_failing(Some(&run("finished", "ok", "ok", ""))));
        assert!(!job_failing(None), "nothing decisive yet");
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

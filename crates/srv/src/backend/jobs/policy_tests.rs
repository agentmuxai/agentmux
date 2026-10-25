// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The policy's decisions, without a ledger (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1).

use super::*;
use crate::backend::jobs::definition::Overlap;
use crate::backend::storage::job_runs::reason;

fn job(overlap: Overlap) -> CronJob {
    let mut job = crate::backend::cron::test_job("j", "p", "t", "", "");
    job.policy.overlap = overlap;
    job
}

fn occ(job_running: usize, job_started: usize) -> Occupancy {
    Occupancy { job_running, job_started, ..Default::default() }
}

#[test]
fn an_idle_job_starts() {
    for overlap in [Overlap::Skip, Overlap::BufferOne, Overlap::CancelOther] {
        assert_eq!(decide(&job(overlap), &Occupancy::default(), false), Decision::Start);
    }
}

#[test]
fn overlap_skip_skips_the_slot() {
    assert_eq!(decide(&job(Overlap::Skip), &occ(1, 1), false), Decision::SkipOverlap);
}

#[test]
fn overlap_buffer_one_keeps_one_waiting_slot() {
    let j = job(Overlap::BufferOne);
    assert_eq!(decide(&j, &occ(1, 1), false), Decision::Wait(reason::OVERLAP));
    let buffered = Occupancy { job_buffered: 1, ..occ(1, 1) };
    assert_eq!(decide(&j, &buffered, false), Decision::SkipOverlap, "the buffer has one place");
    assert_eq!(decide(&j, &buffered, true), Decision::Wait(reason::OVERLAP), "the buffered run keeps waiting");
}

#[test]
fn overlap_cancel_other_stops_a_started_run_but_not_one_being_delivered() {
    let j = job(Overlap::CancelOther);
    assert_eq!(decide(&j, &occ(1, 1), false), Decision::CancelThenStart);
    assert_eq!(decide(&j, &occ(2, 1), false), Decision::SkipOverlap);
}

#[test]
fn a_group_runs_one_at_a_time_and_deploy_is_built_in() {
    let mut grouped = job(Overlap::Skip);
    grouped.policy.group = Some("nightly".into());
    assert_eq!(group_of(&grouped).as_deref(), Some("nightly"));
    let busy = Occupancy { group_running: 1, ..Default::default() };
    assert_eq!(decide(&grouped, &busy, false), Decision::Wait(reason::GROUP_BUSY));
    assert_eq!(decide(&job(Overlap::Skip), &busy, false), Decision::Start, "no group, no wait");

    let mut deploys = job(Overlap::Skip);
    deploys.outputs = vec!["report_file".into(), "deploy:prod".into()];
    assert_eq!(group_of(&deploys).as_deref(), Some(GROUP_DEPLOY));
    deploys.outputs = vec!["deployment_notes".into()];
    assert_eq!(group_of(&deploys), None, "a word that merely starts with deploy");
}

#[test]
fn three_unattended_agent_runs_at_once_and_tasks_are_not_agent_runs() {
    let full = Occupancy { agents_running: GLOBAL_LIMIT, ..Default::default() };
    let almost = Occupancy { agents_running: GLOBAL_LIMIT - 1, ..Default::default() };
    assert_eq!(decide(&job(Overlap::Skip), &full, false), Decision::Wait(reason::GLOBAL_LIMIT));
    assert_eq!(decide(&job(Overlap::Skip), &almost, false), Decision::Start);
    let mut task = job(Overlap::Skip);
    task.action_kind = "task".into();
    assert_eq!(decide(&task, &full, false), Decision::Start);
    task.action_kind = "spawn".into();
    assert_eq!(decide(&task, &full, false), Decision::Wait(reason::GLOBAL_LIMIT));
}

fn ended(infra_status: &str, outcome: &str, summary: &str) -> JobRun {
    let mut run = JobRun::new("r", "j", "cron", "n", "schedule", "finished", 0);
    (run.infra, run.outcome, run.summary) = (infra_status.into(), outcome.into(), summary.into());
    run
}

/// Retries are for the infrastructure only: a process that exited, or a
/// transient provider error. Never a run that concluded `failed`, one that
/// timed out, or one that finished.
#[test]
fn only_infrastructure_failures_are_retried() {
    assert!(infra_retryable(&ended("interrupted", "unknown", "")));
    assert!(infra_retryable(&ended("error", "unknown", "Rate limited by the API (429)")));
    assert!(infra_retryable(&ended("error", "unknown", "API temporarily overloaded (HTTP 529)")));
    assert!(!infra_retryable(&ended("interrupted", "failed", "the agent said so")), "its agent concluded failed");
    assert!(!infra_retryable(&ended("error", "unknown", "invalid cron expression")), "not transient");
    assert!(!infra_retryable(&ended("timeout", "unknown", "")), "its own timeout");
    assert!(!infra_retryable(&ended("over_budget", "unknown", "")));
    assert!(!infra_retryable(&ended("ok", "unknown", "")));
    assert!(!infra_retryable(&ended("ok", "failed", "")));
}

#[test]
fn the_backoff_is_one_five_then_fifteen_minutes() {
    assert_eq!([1, 2, 3, 4].map(retry_delay_ms), [60_000, 300_000, 900_000, 900_000]);
}

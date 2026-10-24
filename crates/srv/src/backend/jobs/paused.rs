// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The slots a paused job lets pass: each is counted into one collapsed
//! `skipped (paused)` row (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §3.3, Pause / Resume). A paused job has no scheduler task, so a minute
//! timer here walks the paused jobs' slots since its last look. Pause all's
//! skips are the scheduler's own (`CronScheduler::fire`): those jobs still
//! have their task.

use std::str::FromStr;
use std::sync::Arc;

use agentmux_common::time::now_ms;

use super::verdict::ended_by_limit;
use super::RunLedger;
use crate::backend::storage::job_runs::reason;
use crate::backend::storage::store::Store;

/// The most slots one job counts in one look (a one-minute job, a day).
const SLOTS_CAP: usize = 1440;

/// Look every minute, a second after it turns, so a slot on the minute is
/// already past.
const LOOK_EVERY_MS: i64 = 60_000;
const LOOK_AFTER_MS: i64 = 1_000;

/// Start the minute timer over the cron jobs in `shared`. Counts from now:
/// slots that passed while AgentMux was off aren't a paused job's.
pub fn spawn(shared: Arc<Store>, ledger: Arc<RunLedger>) {
    tokio::spawn(async move {
        let mut since = now_ms();
        loop {
            let now = now_ms();
            let next = (now / LOOK_EVERY_MS + 1) * LOOK_EVERY_MS + LOOK_AFTER_MS;
            tokio::time::sleep(std::time::Duration::from_millis((next - now).max(0) as u64)).await;
            let until = now_ms();
            let (shared, ledger) = (shared.clone(), ledger.clone());
            let _ = tokio::task::spawn_blocking(move || record_paused_slots(&shared, &ledger, since, until)).await;
            since = until;
        }
    });
}

/// Count every paused job's slots after `since_ms` and up to `until_ms`.
/// Jobs their own limit stopped (`max_fires`, `max_age_secs`) aren't paused
/// and count nothing. Returns how many slots were counted.
pub(crate) fn record_paused_slots(shared: &Store, ledger: &RunLedger, since_ms: i64, until_ms: i64) -> usize {
    let jobs = match shared.cron_list() {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::warn!(error = %e, "jobs: couldn't list cron jobs for paused slots");
            return 0;
        }
    };
    let mut counted = 0;
    for job in jobs.iter().filter(|j| !j.enabled && !ended_by_limit(j, until_ms / 1000)) {
        for slot in slots_between(&job.expression, since_ms, until_ms) {
            ledger.record_skip_slot(job, slot, reason::PAUSED);
            counted += 1;
        }
    }
    counted
}

/// The slots of `expression` after `since_ms` and up to `until_ms`, at most
/// [`SLOTS_CAP`].
fn slots_between(expression: &str, since_ms: i64, until_ms: i64) -> Vec<i64> {
    let Ok(schedule) = cron::Schedule::from_str(&format!("0 {expression}")) else { return Vec::new() };
    let Some(since) = chrono::DateTime::from_timestamp_millis(since_ms) else { return Vec::new() };
    schedule
        .after(&since)
        .map(|t| t.timestamp_millis())
        .take_while(|&ms| ms <= until_ms)
        .take(SLOTS_CAP)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mps::Broker;

    fn ledger_over(store: &Arc<Store>) -> Arc<RunLedger> {
        RunLedger::new(store.clone(), None, Arc::new(Broker::new()))
    }

    fn store() -> Arc<Store> {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.apply_identity_schema_for_tests().unwrap();
        store
    }

    #[test]
    fn slots_are_those_after_since_up_to_until() {
        // 00:00:30 to 00:03:00: the slots at 1, 2 and 3 minutes.
        assert_eq!(slots_between("* * * * *", 30_000, 180_000), vec![60_000, 120_000, 180_000]);
        assert!(slots_between("not a cron", 0, 600_000).is_empty());
    }

    #[tokio::test]
    async fn a_paused_job_counts_its_slots_into_one_skip_row() {
        let store = store();
        let ledger = ledger_over(&store);
        let mut job = crate::backend::cron::test_job("paused-j", "p", "t", "uid-t", "uid-c");
        job.enabled = false;
        store.cron_create(&job).unwrap();
        let active = crate::backend::cron::test_job("active-j", "p", "t", "uid-t", "uid-c");
        store.cron_create(&active).unwrap();

        assert_eq!(record_paused_slots(&store, &ledger, 0, 180_000), 3);
        // The next look counts on into the same row, and a slot already
        // counted (another instance's look) isn't counted twice.
        assert_eq!(record_paused_slots(&store, &ledger, 180_000, 300_000), 2);
        ledger.record_skip_slot(&job, 300_000, reason::PAUSED);

        let runs = store.job_runs_for_job("paused-j", 10, None).unwrap();
        assert_eq!(runs.len(), 1, "{runs:?}");
        let r = &runs[0];
        assert_eq!((r.state.as_str(), r.reason.as_str(), r.skip_count), ("skipped", "paused", 5));
        assert_eq!((r.scheduled_for_ms, r.last_skip_at_ms), (Some(60_000), Some(300_000)));
        assert!(store.job_runs_for_job("active-j", 10, None).unwrap().is_empty(), "an active job counts nothing here");
    }

    #[tokio::test]
    async fn a_job_its_own_limit_stopped_counts_nothing() {
        let store = store();
        let ledger = ledger_over(&store);
        let mut job = crate::backend::cron::test_job("done-j", "p", "t", "uid-t", "uid-c");
        job.enabled = false;
        job.max_fires = Some(1);
        job.fire_count = 1;
        store.cron_create(&job).unwrap();
        assert_eq!(record_paused_slots(&store, &ledger, 0, 180_000), 0);
    }

    #[tokio::test]
    async fn another_skip_or_a_run_starts_a_new_row() {
        let store = store();
        let ledger = ledger_over(&store);
        let job = crate::backend::cron::test_job("mix-j", "p", "t", "uid-t", "uid-c");
        ledger.record_skip_slot(&job, 60_000, reason::PAUSED);
        ledger.record_skip_slot(&job, 120_000, reason::PAUSED_ALL);
        ledger.record_skip_slot(&job, 180_000, reason::PAUSED_ALL);
        ledger.record_skip_slot(&job, 240_000, reason::PAUSED);
        let runs = store.job_runs_for_job("mix-j", 10, None).unwrap();
        let shape: Vec<(&str, i64)> = runs.iter().map(|r| (r.reason.as_str(), r.skip_count)).collect();
        assert_eq!(shape, vec![("paused", 1), ("paused_all", 2), ("paused", 1)]);
    }
}

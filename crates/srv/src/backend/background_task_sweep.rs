// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ends the `db_background_tasks` rows that nothing is left to end
//! (`docs/specs/SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md` §5, the liveness sweep;
//! `docs/retro/retro-agent-pane-progress-flicker-and-orphaned-background-tasks-2026-09-30.md` §5, fixes 4 and 5).
//!
//! A row ends from the CLI's own task feed (`background_task_feed`) or from the renderer's `<task-notification>`.
//! Neither speaks when the CLI dies mid-task, or drops a task without ending it from a feed that no longer holds a
//! record of it (a row older than this srv). Two cases remain, and a pass over the still-`running` rows settles both:
//!
//! - A row with a pid whose process is gone. The pid is checked against the row's start, so a pid the OS has since
//!   given to another program does not keep it alive (`pid_alive_since`). A task that really still runs keeps its pid,
//!   so a long `task dev` stays.
//! - A row with no pid that nothing has seen for [`ORPHAN_AFTER_MS`]. It can't be checked; it only has the CLI's word,
//!   and the word stopped. The dock reads such a row as unknown after two hours; this ends it after a day.
//!
//! Both end as `stopped` and only while still `running` (`background_task_end_if_running`), so a real outcome that
//! arrives during the pass stands. Pids are the host's: a container pane's pid is never filed (`container_tool_chunk`).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::background_task_feed::publish_background_task_updated;
use super::mps::Broker;
use super::reactive::registry::pid_alive_since;
use super::storage::background_tasks::{BackgroundTask, BackgroundTaskStatus};
use super::storage::store::Store;

/// How long after srv starts the first pass runs: rows left by the last run are cleared early, with the rest of the
/// start-up work out of the way.
const FIRST_AFTER: Duration = Duration::from_secs(30);
/// Between passes. A dead task's row reading running for a few more minutes costs nothing.
const EVERY: Duration = Duration::from_secs(5 * 60);
/// A `running` row with no pid is ended when it was last seen this long ago.
pub const ORPHAN_AFTER_MS: i64 = 24 * 60 * 60 * 1000;

/// When `task`, still `running`, ended as far as anyone can tell, or `None` if it may still be running.
/// `alive(pid, started_at_ms)` is whether that process, started before the row, is still there.
fn ended_at(task: &BackgroundTask, now_ms: i64, alive: &impl Fn(u32, u64) -> bool) -> Option<i64> {
    match task.pid {
        Some(pid) => {
            // A pid that doesn't fit, or is not one, can't be looked up: leave the row to the other paths.
            let pid = u32::try_from(pid).ok().filter(|p| *p > 0)?;
            (!alive(pid, u64::try_from(task.started_at_ms).unwrap_or(0))).then_some(now_ms)
        }
        None => (now_ms.saturating_sub(task.last_seen_ms) > ORPHAN_AFTER_MS).then_some(task.last_seen_ms),
    }
}

/// One pass: end every `running` row that [`ended_at`] says is over. Returns the blocks whose rows changed.
fn sweep(store: &Store, now_ms: i64, alive: &impl Fn(u32, u64) -> bool) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    let running = match store.background_task_list_running() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(target: "background_tasks", error = %e, "liveness sweep: could not list the running tasks");
            return changed;
        }
    };
    for task in running {
        let Some(at) = ended_at(&task, now_ms, alive) else { continue };
        match store.background_task_end_if_running(&task.id, BackgroundTaskStatus::Stopped, at) {
            Ok(true) => {
                tracing::info!(target: "background_tasks", block_id = %task.block_id, node_id = %task.id, pid = ?task.pid,
                    "liveness sweep: a background task nothing reports on any more is stopped");
                changed.insert(task.block_id);
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(target: "background_tasks", node_id = %task.id, error = %e, "liveness sweep: could not end a task"),
        }
    }
    changed
}

/// Starts the sweep: a first pass soon after srv starts, then every [`EVERY`].
pub(crate) fn start(store: Arc<Store>, broker: Arc<Broker>) {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_AFTER).await;
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            let store = Arc::clone(&store);
            // The store and the process table are blocking reads.
            let changed = tokio::task::spawn_blocking(move || sweep(&store, agentmux_common::time::now_ms(), &pid_alive_since)).await;
            for block_id in changed.unwrap_or_default() {
                publish_background_task_updated(&broker, &block_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10 * ORPHAN_AFTER_MS;

    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn row(store: &Store, id: &str, block: &str, started: i64, seen: i64, pid: Option<i64>) {
        store.background_task_observe(id, block, "task dev", started, seen).unwrap();
        if let Some(pid) = pid {
            store.background_task_set_pid(id, pid).unwrap();
        }
    }

    fn status(store: &Store, id: &str) -> (BackgroundTaskStatus, Option<i64>) {
        let t = store.background_task_get(id).unwrap().unwrap();
        (t.status, t.ended_at_ms)
    }

    #[test]
    fn a_task_whose_process_is_gone_is_stopped_and_one_still_there_stays() {
        let s = store();
        row(&s, "dead", "b1", NOW - 5000, NOW - 1000, Some(111));
        row(&s, "live", "b2", NOW - 5000, NOW - 1000, Some(222));
        let changed = sweep(&s, NOW, &|pid, _| pid == 222);
        assert_eq!(changed.into_iter().collect::<Vec<_>>(), vec!["b1".to_string()]);
        assert_eq!(status(&s, "dead"), (BackgroundTaskStatus::Stopped, Some(NOW)));
        assert_eq!(status(&s, "live"), (BackgroundTaskStatus::Running, None));
    }

    #[test]
    fn the_pid_is_checked_against_when_the_row_started() {
        let s = store();
        row(&s, "t", "b1", NOW - 5000, NOW - 1000, Some(111));
        let seen = std::cell::Cell::new(None);
        sweep(&s, NOW, &|pid, started| {
            seen.set(Some((pid, started)));
            true
        });
        assert_eq!(seen.get(), Some((111, (NOW - 5000) as u64)));
    }

    #[test]
    fn a_row_with_no_pid_is_left_for_a_day_and_then_ended_at_its_last_sighting() {
        let s = store();
        row(&s, "recent", "b1", NOW - 3 * 60 * 60 * 1000, NOW - ORPHAN_AFTER_MS, None);
        row(&s, "orphan", "b2", NOW - 3 * ORPHAN_AFTER_MS, NOW - ORPHAN_AFTER_MS - 1, None);
        let changed = sweep(&s, NOW, &|_, _| panic!("no pid to look up"));
        assert_eq!(changed.into_iter().collect::<Vec<_>>(), vec!["b2".to_string()]);
        assert_eq!(status(&s, "recent").0, BackgroundTaskStatus::Running);
        assert_eq!(status(&s, "orphan"), (BackgroundTaskStatus::Stopped, Some(NOW - ORPHAN_AFTER_MS - 1)));
    }

    #[test]
    fn a_row_that_already_ended_is_not_touched() {
        let s = store();
        row(&s, "done", "b1", NOW - 5000, NOW - 1000, Some(111));
        s.background_task_complete("done", BackgroundTaskStatus::Done, NOW - 500).unwrap();
        assert!(sweep(&s, NOW, &|_, _| false).is_empty());
        assert_eq!(status(&s, "done"), (BackgroundTaskStatus::Done, Some(NOW - 500)));
    }

    #[test]
    fn a_pid_that_cannot_be_looked_up_leaves_the_row_alone() {
        let s = store();
        row(&s, "zero", "b1", NOW - 5000, NOW - 1000, Some(0));
        row(&s, "huge", "b1", NOW - 5000, NOW - 1000, Some(i64::from(u32::MAX) + 1));
        assert!(sweep(&s, NOW, &|_, _| false).is_empty());
    }

    #[test]
    fn a_block_with_several_ended_rows_is_reported_once() {
        let s = store();
        row(&s, "a", "b1", NOW - 5000, NOW - 1000, Some(1));
        row(&s, "b", "b1", NOW - 5000, NOW - 1000, Some(2));
        assert_eq!(sweep(&s, NOW, &|_, _| false).len(), 1);
    }
}

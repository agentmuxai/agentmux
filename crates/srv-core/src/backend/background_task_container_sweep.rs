// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The liveness sweep for a container agent's background tasks (`background_task_sweep` does a host pane's).
//!
//! A container pane's `agentmux-bashwrap` reports the process id it runs under in the container's own pid
//! namespace. It is filed beside the registry row, never in its host `pid` column
//! (`storage::background_container_calls`), so nothing on the host can signal it. This pass checks each such
//! row inside its container, through the Docker exec API, and ends it when the process is gone:
//!
//! - **The process is gone**, or the pid now belongs to a process that started after the task did (the same
//!   start-time guard `pid_alive_since` gives a host pid): the row ends as `stopped`.
//! - **The container is not running** (the exec is refused because it is stopped or gone): every row of the
//!   panes that ran in it ends at once, each at its last sighting, as when a container is started again
//!   (`background_task_exit::end_for_restarted_container`).
//! - Anything else (Docker unreachable, the probe timed out or printed nothing readable) leaves the row as it
//!   is: it is only ever ended on evidence.
//!
//! Only the probe talks to Docker ([`ContainerProbe`]); this pass is plain logic over the store.

use std::collections::BTreeSet;
use std::sync::Arc;

use futures_util::future::BoxFuture;

use super::storage::background_tasks::BackgroundTaskStatus;
use super::storage::store::Store;

/// How far after a task's row began a process may have started and still be that task's: the pid is reported
/// when the command starts, the row is made a moment later, and the container's clock and this host's can
/// differ a little.
const START_SLACK_MS: i64 = 60_000;

/// What the container says of one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The process is there and is the task's.
    Alive,
    /// No such process, or the pid belongs to a newer one.
    Gone,
    /// The container is not running (or no longer exists): nothing in it is.
    ContainerDown,
    /// Nothing can be said now.
    Unknown,
}

/// Checks a process inside a container. The Docker-backed one is `container_task_probe`.
pub trait ContainerProbe: Send + Sync {
    /// The state of process `pid` of `container`, for a task whose row began at `task_started_ms`.
    fn probe<'a>(&'a self, container: &'a str, pid: u32, task_started_ms: i64) -> BoxFuture<'a, Verdict>;
}

/// The verdict a probe's printed answer gives: `gone`, `alive`, or `alive <start of the process, unix seconds>`.
pub fn verdict_of(output: &str, task_started_ms: i64) -> Verdict {
    let Some(line) = output.lines().map(str::trim).rev().find(|l| *l == "gone" || l.starts_with("alive")) else {
        return Verdict::Unknown;
    };
    if line == "gone" {
        return Verdict::Gone;
    }
    match line["alive".len()..].trim().parse::<i64>() {
        Ok(started_s) if started_s.saturating_mul(1000) > task_started_ms.saturating_add(START_SLACK_MS) => Verdict::Gone,
        _ => Verdict::Alive,
    }
}

/// One pass: end the container-held rows that are over. Returns the panes whose rows changed.
pub async fn sweep(store: &Arc<Store>, now_ms: i64, probe: &dyn ContainerProbe) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    let reader = Arc::clone(store);
    let rows = match tokio::task::spawn_blocking(move || reader.background_container_running()).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(e)) => {
            tracing::warn!(target: "background_tasks", error = %e, "container liveness sweep: could not list the running tasks");
            return changed;
        }
        Err(_) => return changed,
    };
    let mut down: BTreeSet<&str> = BTreeSet::new();
    for task in &rows {
        if down.contains(task.container.as_str()) {
            continue;
        }
        let Some(pid) = u32::try_from(task.pid).ok().filter(|p| *p > 0) else { continue };
        match probe.probe(&task.container, pid, task.started_at_ms).await {
            Verdict::Alive | Verdict::Unknown => {}
            Verdict::Gone => {
                if end(store, &task.id, now_ms).await {
                    tracing::info!(target: "background_tasks", block_id = %task.block_id, node_id = %task.id,
                        "container liveness sweep: a background task whose process is gone is stopped");
                    changed.insert(task.block_id.clone());
                }
            }
            Verdict::ContainerDown => {
                down.insert(task.container.as_str());
            }
        }
    }
    for task in rows.iter().filter(|t| down.contains(t.container.as_str())) {
        let (store, block_id) = (Arc::clone(store), task.block_id.clone());
        let ended = tokio::task::spawn_blocking(move || store.background_task_end_unchecked_for_block(&block_id)).await;
        if matches!(ended, Ok(Ok(n)) if n > 0) {
            tracing::info!(target: "background_tasks", block_id = %task.block_id, container = %task.container,
                "container liveness sweep: the container is not running, its background tasks are over");
            changed.insert(task.block_id.clone());
        }
    }
    changed
}

/// End `id` as stopped if it is still running.
async fn end(store: &Arc<Store>, id: &str, now_ms: i64) -> bool {
    let (store, id) = (Arc::clone(store), id.to_string());
    let ended = tokio::task::spawn_blocking(move || store.background_task_end_if_running(&id, BackgroundTaskStatus::Stopped, now_ms)).await;
    matches!(ended, Ok(Ok(true)))
}

#[cfg(test)]
#[path = "background_task_container_sweep_tests.rs"]
mod tests;

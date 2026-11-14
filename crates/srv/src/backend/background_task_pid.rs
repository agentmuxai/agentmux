// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Files a background task's pid in `db_background_tasks` from srv itself
//! (`docs/specs/SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md` §5, first bullet;
//! `docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md`).
//!
//! `agentmux-bashwrap` publishes a `pid` chunk when the command it runs starts. Only an open agent pane used to
//! read it, and relayed it to srv: with the pane closed or parked the row never got its pid, and the liveness sweep
//! (`background_task_sweep`) had nothing to check it by. The chunk is on the broker whether or not a pane is open, so
//! srv reads it there.
//!
//! The pid usually beats its row: bashwrap publishes it at its own start, and the row is made when the CLI's task
//! feed (in the stdout reader, which needs no pane) or the pane's `docknodestatus` first sees the call run in the
//! background. A pid that finds no row waits in the process-wide pen (`PendingBackgroundPids`), and whoever makes the
//! row applies it ([`apply_stashed`]).

use std::sync::Arc;

use agentmux_common::client_events::ToolChunk;
use serde_json::Value;

use super::background_task_feed::publish_background_task_updated;
use super::mps::{Broker, MuxEvent};
use super::mps_events::{Event, ToolChunk as ToolChunkEvent};
use super::pending_background_pids::PendingBackgroundPids;
use super::storage::store::Store;

/// A command's pid: the pane's block, the call's id (the row's id) and the process id.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PidReport {
    pub block_id: String,
    pub tool_id: String,
    pub pid: i64,
}

/// The pid in a broker event, if it is a `pid` chunk published for a pane.
pub(crate) fn pid_in(event: &str, scopes: &[String], data: Option<&Value>) -> Option<PidReport> {
    if event != <ToolChunkEvent as Event>::NAME {
        return None;
    }
    let ToolChunk::Pid { tool_id, pid, .. } = serde_json::from_value(data?.clone()).ok()? else {
        return None;
    };
    let block_id = scopes.iter().find_map(|s| s.strip_prefix("block:"))?.to_string();
    Some(PidReport { block_id, tool_id, pid: i64::from(pid) })
}

/// Record `pid` on the row `tool_id`, or keep it in the pen until that row exists. Tells the pane to re-read when
/// `broker` is given. Blocking: a store write.
pub(crate) fn file_pid(store: &Store, broker: Option<&Broker>, block_id: &str, tool_id: &str, pid: i64, now_ms: i64) {
    let filed = PendingBackgroundPids::global().set_or_stash(tool_id, pid, now_ms, |pid| store.background_task_set_pid(tool_id, pid));
    match filed {
        Ok(()) => {
            if let Some(broker) = broker {
                publish_background_task_updated(broker, block_id);
            }
        }
        Err(e) => tracing::warn!(target: "background_tasks", node_id = %tool_id, error = %e, "failed to record background task pid in the durable registry"),
    }
}

/// The row `tool_id` now exists: give it the pid that came before it, if one did. Blocking: a store write.
pub(crate) fn apply_stashed(store: &Store, tool_id: &str, now_ms: i64) {
    let applied = PendingBackgroundPids::global().observe_and_apply(tool_id, now_ms, |pid| store.background_task_set_pid(tool_id, pid));
    if let Err(e) = applied {
        tracing::warn!(target: "background_tasks", node_id = %tool_id, error = %e, "failed to apply a pid stashed ahead of this task's registry row");
    }
}

/// Start listening. Called once, with the app's broker and store.
pub(crate) fn install(broker: Arc<Broker>, store: Arc<Store>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PidReport>();
    // Observers run inside `Broker::publish`, under its locks: parse and hand off only.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(report) = pid_in(&ev.event, &ev.scopes, ev.data.as_ref()) {
            let _ = tx.send(report);
        }
    }));
    let notify = Arc::clone(&broker);
    tokio::spawn(async move {
        while let Some(report) = rx.recv().await {
            let (store, broker) = (Arc::clone(&store), Arc::clone(&notify));
            let _ = tokio::task::spawn_blocking(move || {
                file_pid(&store, Some(&broker), &report.block_id, &report.tool_id, report.pid, agentmux_common::time::now_ms());
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::background_task_feed::TaskFeed;
    use serde_json::json;

    fn pid_chunk(tool_id: &str, pid: u32) -> Value {
        json!({ "op": "pid", "tool_id": tool_id, "pid": pid, "timestamp": 7 })
    }

    fn scopes() -> Vec<String> {
        vec!["block:blk".to_string()]
    }

    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn a_pid_chunk_of_a_pane_is_a_report_and_nothing_else_is() {
        let got = pid_in("tool_chunk", &scopes(), Some(&pid_chunk("toolu_1", 42)));
        assert_eq!(got, Some(PidReport { block_id: "blk".into(), tool_id: "toolu_1".into(), pid: 42 }));
        let line = json!({ "op": "chunk", "tool_id": "toolu_1", "kind": "stdout", "content": "x\n", "timestamp": 5 });
        let exit = json!({ "op": "terminal", "tool_id": "toolu_1", "exit_code": 0, "timestamp": 6 });
        assert_eq!(pid_in("tool_chunk", &scopes(), Some(&line)), None);
        assert_eq!(pid_in("tool_chunk", &scopes(), Some(&exit)), None);
        assert_eq!(pid_in("tool_chunk", &scopes(), None), None);
        assert_eq!(pid_in("agents:changed", &scopes(), Some(&pid_chunk("toolu_1", 42))), None, "another event");
        assert_eq!(pid_in("tool_chunk", &[], Some(&pid_chunk("toolu_1", 42))), None, "no pane to tell");
    }

    #[test]
    fn a_pid_filed_after_its_row_is_recorded_at_once() {
        let s = store();
        s.background_task_observe("pid_after_row", "blk", "task dev", 1000, 1000).unwrap();
        file_pid(&s, None, "blk", "pid_after_row", 4242, 2000);
        assert_eq!(s.background_task_get("pid_after_row").unwrap().unwrap().pid, Some(4242));
    }

    // The case of a task started while no pane shows the agent: no `docknodestatus` ever comes, so the row is made by
    // the CLI's task feed in the stdout reader, after the pid that bashwrap published when the command started.
    #[test]
    fn a_pid_that_beats_a_row_made_by_the_task_feed_is_applied_when_the_row_appears() {
        let s = store();
        file_pid(&s, None, "blk", "toolu_nopane", 777, 1000);
        assert!(s.background_task_get("toolu_nopane").unwrap().is_none(), "no row yet: the pid waits");

        let started = json!({ "type": "system", "subtype": "task_started", "task_id": "bgtask1", "tool_use_id": "toolu_nopane",
            "task_type": "local_bash", "description": "task dev", "is_backgrounded": true });
        let mut feed = TaskFeed::default();
        assert!(feed.apply(&s, None, "blk", &started, 2000), "the feed makes the row");
        assert_eq!(s.background_task_get("toolu_nopane").unwrap().unwrap().pid, Some(777));
    }

    #[test]
    fn a_stashed_pid_is_applied_once() {
        let s = store();
        file_pid(&s, None, "blk", "once", 5, 1000);
        s.background_task_observe("once", "blk", "build", 1000, 1000).unwrap();
        apply_stashed(&s, "once", 1500);
        assert_eq!(s.background_task_get("once").unwrap().unwrap().pid, Some(5));
        s.background_task_set_pid("once", 9).unwrap();
        apply_stashed(&s, "once", 1600);
        assert_eq!(s.background_task_get("once").unwrap().unwrap().pid, Some(9), "the pen was emptied");
    }
}

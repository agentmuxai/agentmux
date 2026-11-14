// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ends a background task's row when its command exits, from srv itself
//! (`docs/specs/SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md` §5, first bullet).
//!
//! `agentmux-bashwrap` publishes a `terminal` chunk with the real exit code when the command it runs ends. Until now
//! only an open pane read it, and only to print `[exited N]`; the row itself ended from the CLI's task feed or the
//! pane's `<task-notification>`, and a CLI that died mid-task sent neither. The chunk is on the broker whether or not
//! a pane is open, so this listens there. A chunk of a call that never went to the background finds no row and is
//! ignored, and a row the CLI already ended stays as it ended (`background_task_end_if_running`).

use std::sync::Arc;

use agentmux_common::client_events::ToolChunk;
use serde_json::Value;

use super::background_task_feed::publish_background_task_updated;
use super::mps::{Broker, MuxEvent};
use super::mps_events::{Event, ToolChunk as ToolChunkEvent};
use super::storage::background_tasks::BackgroundTaskStatus;
use super::storage::store::Store;

/// A command that ended: the pane's block, the call's id (the row's id), its exit code and when.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Exit {
    pub block_id: String,
    pub tool_id: String,
    pub exit_code: i32,
    pub at_ms: i64,
}

/// The exit in a broker event, if it is a `terminal` chunk published for a pane.
pub(crate) fn exit_in(event: &str, scopes: &[String], data: Option<&Value>) -> Option<Exit> {
    if event != <ToolChunkEvent as Event>::NAME {
        return None;
    }
    let ToolChunk::Terminal { tool_id, exit_code, timestamp } = serde_json::from_value(data?.clone()).ok()? else {
        return None;
    };
    let block_id = scopes.iter().find_map(|s| s.strip_prefix("block:"))?.to_string();
    Some(Exit { block_id, tool_id, exit_code, at_ms: i64::try_from(timestamp).unwrap_or(i64::MAX) })
}

/// How a row ends for `exit_code`: `done` on 0, `stopped` when a signal ended it (a negative code, or 128 plus
/// SIGINT, SIGKILL or SIGTERM, as a shell reports it): the task was ended, it did not fail. Any other code is `error`.
pub(crate) fn status_for(exit_code: i32) -> BackgroundTaskStatus {
    match exit_code {
        0 => BackgroundTaskStatus::Done,
        c if c < 0 || matches!(c, 130 | 137 | 143) => BackgroundTaskStatus::Stopped,
        _ => BackgroundTaskStatus::Error,
    }
}

/// End the row `exit` is about. `true` when a running row ended (the pane is told to re-read).
fn end_row(store: &Store, exit: &Exit) -> bool {
    match store.background_task_end_if_running(&exit.tool_id, status_for(exit.exit_code), exit.at_ms) {
        Ok(ended) => ended,
        Err(e) => {
            tracing::warn!(target: "background_tasks", node_id = %exit.tool_id, error = %e, "could not end a background task from its exit");
            false
        }
    }
}

/// Start listening. Called once, with the app's broker and store.
pub(crate) fn install(broker: Arc<Broker>, store: Arc<Store>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Exit>();
    // Observers run inside `Broker::publish`, under its locks: parse and hand off only. Every line of every Bash call's
    // output passes here, so a chunk that is not an exit costs one string comparison.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(exit) = exit_in(&ev.event, &ev.scopes, ev.data.as_ref()) {
            let _ = tx.send(exit);
        }
    }));
    let notify = Arc::clone(&broker);
    tokio::spawn(async move {
        while let Some(exit) = rx.recv().await {
            let store = Arc::clone(&store);
            let ended = tokio::task::spawn_blocking(move || end_row(&store, &exit).then_some(exit.block_id)).await;
            if let Ok(Some(block_id)) = ended {
                publish_background_task_updated(&notify, &block_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn terminal(code: i32) -> Value {
        json!({ "op": "terminal", "tool_id": "toolu_1", "exit_code": code, "timestamp": 5000 })
    }

    fn scopes() -> Vec<String> {
        vec!["block:b1".to_string()]
    }

    #[test]
    fn a_terminal_chunk_of_a_pane_is_an_exit() {
        let got = exit_in("tool_chunk", &scopes(), Some(&terminal(3)));
        assert_eq!(got, Some(Exit { block_id: "b1".into(), tool_id: "toolu_1".into(), exit_code: 3, at_ms: 5000 }));
    }

    #[test]
    fn nothing_else_is_an_exit() {
        let line = json!({ "op": "chunk", "tool_id": "toolu_1", "kind": "stdout", "content": "x\n", "timestamp": 5 });
        let pid = json!({ "op": "pid", "tool_id": "toolu_1", "pid": 42, "timestamp": 7 });
        assert_eq!(exit_in("tool_chunk", &scopes(), Some(&line)), None);
        assert_eq!(exit_in("tool_chunk", &scopes(), Some(&pid)), None);
        assert_eq!(exit_in("tool_chunk", &scopes(), None), None);
        assert_eq!(exit_in("agents:changed", &scopes(), Some(&terminal(0))), None, "another event");
        assert_eq!(exit_in("tool_chunk", &[], Some(&terminal(0))), None, "no pane to tell");
    }

    #[test]
    fn the_exit_code_decides_how_the_row_ends() {
        assert_eq!(status_for(0), BackgroundTaskStatus::Done);
        assert_eq!(status_for(1), BackgroundTaskStatus::Error);
        assert_eq!(status_for(2), BackgroundTaskStatus::Error);
        for signalled in [-1, 130, 137, 143] {
            assert_eq!(status_for(signalled), BackgroundTaskStatus::Stopped, "{signalled}");
        }
    }

    fn running_row(store: &Store, id: &str) {
        store.background_task_observe(id, "b1", "task dev", 1000, 1000).unwrap();
    }

    #[test]
    fn a_running_row_ends_at_the_exit_with_its_status() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        running_row(&store, "toolu_1");
        let exit = exit_in("tool_chunk", &scopes(), Some(&terminal(1))).unwrap();
        assert!(end_row(&store, &exit));
        let row = store.background_task_get("toolu_1").unwrap().unwrap();
        assert_eq!((row.status, row.ended_at_ms), (BackgroundTaskStatus::Error, Some(5000)));
    }

    #[test]
    fn a_call_with_no_row_and_a_row_the_cli_already_ended_are_left_alone() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let exit = exit_in("tool_chunk", &scopes(), Some(&terminal(0))).unwrap();
        assert!(!end_row(&store, &exit), "a foreground call has no row");

        running_row(&store, "toolu_1");
        store.background_task_complete("toolu_1", BackgroundTaskStatus::Stopped, 4000).unwrap();
        assert!(!end_row(&store, &exit));
        let row = store.background_task_get("toolu_1").unwrap().unwrap();
        assert_eq!((row.status, row.ended_at_ms), (BackgroundTaskStatus::Stopped, Some(4000)));
    }
}

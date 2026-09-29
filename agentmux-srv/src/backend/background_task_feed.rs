// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Claude CLI's own structured task feed → `db_background_tasks`.
//!
//! The CLI reports every task it runs on the agent's stdout as `system` lines:
//!
//! - `task_started` — `task_id`, `tool_use_id`, `description`, `task_type`
//!   (`local_bash`, `local_agent`, `local_workflow`), `is_backgrounded`, and
//!   `owned_by_subagent` when a subagent launched it.
//! - `task_updated` — `task_id` plus a `patch`: `{is_backgrounded: true}` when a
//!   running command is moved to the background, `{status, end_time}` when it
//!   ends (`completed`, `failed`, `killed`).
//! - `task_notification` — `task_id`, `tool_use_id`, final `status`
//!   (`completed`, `failed`, `stopped`) and a `summary`.
//!
//! These arrive on the PARENT agent's stream even for tasks a subagent owns.
//! Before this module nothing read them: the registry learned about a
//! background task only from the renderer spotting the "Command running in
//! background with ID:" result text, and learned it had ended only from a
//! `<task-notification>` user message in the same pane. A subagent's task gets
//! the first but never the second, so its row stayed `running` forever; a
//! top-level task's result now carries empty `stdout` plus a structured
//! `backgroundTaskId`, so the text match missed it entirely. See
//! docs/specs/SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md.
//!
//! Scope: backgrounded `local_bash` tasks only, which is what the registry has
//! always held. Subagents (`local_agent`) are Swarm's subagent watcher's job.
//! The renderer's existing observe/complete pushes still run; every write here
//! is the same idempotent `background_task_observe`/`background_task_complete`
//! call, so whichever arrives first wins and the other is a no-op or a refresh.

use std::collections::HashMap;

use serde_json::Value;

use super::mps::{Broker, MuxEvent};
use super::storage::background_tasks::BackgroundTaskStatus;
use super::storage::store::Store;

/// One task-feed line, reduced to what the registry needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskFeedEvent {
    /// A shell task started. `backgrounded` is false for an ordinary
    /// foreground Bash call, which can still be backgrounded later.
    Started {
        task_id: String,
        tool_use_id: String,
        label: String,
        backgrounded: bool,
    },
    /// A running task was moved to the background.
    Backgrounded { task_id: String },
    /// A task ended. `tool_use_id` is absent on `task_updated` lines, which
    /// carry only the `task_id`.
    Ended {
        task_id: String,
        tool_use_id: Option<String>,
        status: BackgroundTaskStatus,
    },
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Terminal statuses as the CLI spells them. Anything unrecognised ends the
/// task as `stopped` rather than leaving it `running` — the same lenient rule
/// the renderer's `parseTaskNotification` applies.
fn terminal_status(s: &str) -> BackgroundTaskStatus {
    match s {
        "completed" => BackgroundTaskStatus::Done,
        "failed" => BackgroundTaskStatus::Error,
        _ => BackgroundTaskStatus::Stopped,
    }
}

/// Pure parse of one stdout line. `None` for anything that isn't a task-feed
/// line about a shell task.
pub(crate) fn parse(line: &Value) -> Option<TaskFeedEvent> {
    if str_field(line, "type")? != "system" {
        return None;
    }
    let task_id = str_field(line, "task_id")?.to_string();
    match str_field(line, "subtype")? {
        "task_started" => {
            if str_field(line, "task_type")? != "local_bash" {
                return None;
            }
            let tool_use_id = str_field(line, "tool_use_id")?.to_string();
            let label = str_field(line, "description").unwrap_or("Bash").to_string();
            let backgrounded = line
                .get("is_backgrounded")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(TaskFeedEvent::Started {
                task_id,
                tool_use_id,
                label,
                backgrounded,
            })
        }
        "task_updated" => {
            let patch = line.get("patch")?;
            if let Some(status) = str_field(patch, "status") {
                if matches!(status, "completed" | "failed" | "killed" | "stopped") {
                    return Some(TaskFeedEvent::Ended {
                        task_id,
                        tool_use_id: None,
                        status: terminal_status(status),
                    });
                }
                return None;
            }
            if patch.get("is_backgrounded").and_then(Value::as_bool) == Some(true) {
                return Some(TaskFeedEvent::Backgrounded { task_id });
            }
            None
        }
        "task_notification" => {
            let status = terminal_status(str_field(line, "status").unwrap_or(""));
            let tool_use_id = str_field(line, "tool_use_id").map(str::to_string);
            Some(TaskFeedEvent::Ended {
                task_id,
                tool_use_id,
                status,
            })
        }
        _ => None,
    }
}

/// A shell task seen starting on this stream, remembered until it ends so a
/// later `task_updated` (which names only the `task_id`) can be applied.
#[derive(Debug, Clone)]
struct Pending {
    tool_use_id: String,
    label: String,
    started_at_ms: i64,
    backgrounded: bool,
    /// The Agent call whose subagent issued this Bash call; `None` for the
    /// agent's own.
    owner: Option<String>,
}

/// Bound on `TaskFeed::owners`. Entries are normally consumed by the
/// `task_started` that follows within the same message; this only guards
/// against a stream shape where that never happens.
const MAX_UNCLAIMED_OWNERS: usize = 4096;

/// Per-stream state: one per stdout reader. Holds only shell tasks that have
/// started and not yet ended, plus the owners of subagent Bash calls whose
/// `task_started` hasn't arrived yet, so it stays small.
#[derive(Debug, Default)]
pub(crate) struct TaskFeed {
    pending: HashMap<String, Pending>,
    owners: HashMap<String, String>,
}

impl TaskFeed {
    /// Apply one parsed stdout line. Returns true when the registry changed,
    /// in which case `background-task-updated` has been published.
    pub(crate) fn apply(
        &mut self,
        store: &Store,
        broker: Option<&Broker>,
        block_id: &str,
        line: &Value,
        now_ms: i64,
    ) -> bool {
        self.note_owner(line);
        let Some(event) = parse(line) else {
            return false;
        };
        let changed = match event {
            TaskFeedEvent::Started {
                task_id,
                tool_use_id,
                label,
                backgrounded,
            } => {
                let owner = self.owners.remove(&tool_use_id);
                let observed = backgrounded
                    && observe(
                        store,
                        block_id,
                        &tool_use_id,
                        &label,
                        owner.as_deref(),
                        now_ms,
                        now_ms,
                    );
                self.pending.insert(
                    task_id,
                    Pending {
                        tool_use_id,
                        label,
                        started_at_ms: now_ms,
                        backgrounded,
                        owner,
                    },
                );
                observed
            }
            TaskFeedEvent::Backgrounded { task_id } => match self.pending.get_mut(&task_id) {
                Some(p) if !p.backgrounded => {
                    p.backgrounded = true;
                    // Started when the CLI first reported it; seen now.
                    observe(
                        store,
                        block_id,
                        &p.tool_use_id,
                        &p.label,
                        p.owner.as_deref(),
                        p.started_at_ms,
                        now_ms,
                    )
                }
                _ => false,
            },
            TaskFeedEvent::Ended {
                task_id,
                tool_use_id,
                status,
            } => {
                let pending = self.pending.remove(&task_id);
                // Only a task that was ever backgrounded can have a row. A
                // `task_notification` carries its own tool_use_id, which also
                // covers a task that started before this reader did (an
                // srv restart); completing a row that doesn't exist is a no-op.
                let id = tool_use_id
                    .or_else(|| pending.filter(|p| p.backgrounded).map(|p| p.tool_use_id));
                // The CLI usually reports an end twice (`task_updated`, then
                // `task_notification`). Only a still-running row is completed,
                // so the first end time stands and subscribers hear it once.
                let running = |id: &str| matches!(store.background_task_get(id), Ok(Some(row)) if row.status == BackgroundTaskStatus::Running);
                match id {
                    Some(id) if running(&id) => {
                        match store.background_task_complete(&id, status, now_ms) {
                            Ok(rows) => rows,
                            Err(e) => {
                                tracing::warn!(target: "background_tasks", block_id, node_id = %id, error = %e,
                                "failed to complete a background task from the CLI's task feed");
                                false
                            }
                        }
                    }
                    _ => false,
                }
            }
        };
        if changed {
            if let Some(broker) = broker {
                publish_background_task_updated(broker, block_id);
            }
        }
        changed
    }

    /// A subagent's own stream lines carry `parent_tool_use_id` = the Agent
    /// call that spawned it (also that subagent's `meta.json` `toolUseId`).
    /// Remember it for each Bash call such a line issues; the call's
    /// `task_started`, which follows in the same message, claims it.
    fn note_owner(&mut self, line: &Value) {
        if str_field(line, "type") != Some("assistant") {
            return;
        }
        let Some(parent) = str_field(line, "parent_tool_use_id") else {
            return;
        };
        let Some(blocks) = line.pointer("/message/content").and_then(Value::as_array) else {
            return;
        };
        for block in blocks {
            if str_field(block, "type") == Some("tool_use")
                && str_field(block, "name") == Some("Bash")
            {
                if let Some(id) = str_field(block, "id") {
                    if self.owners.len() >= MAX_UNCLAIMED_OWNERS {
                        self.owners.clear();
                    }
                    self.owners.insert(id.to_string(), parent.to_string());
                }
            }
        }
    }
}

/// Create the row if needed, then record the CLI's description as its label
/// (replacing the renderer's generic "Bash" if it got there first) and its
/// owner.
fn observe(
    store: &Store,
    block_id: &str,
    tool_use_id: &str,
    label: &str,
    owner: Option<&str>,
    started_at_ms: i64,
    seen_at_ms: i64,
) -> bool {
    let result = store
        .background_task_observe(tool_use_id, block_id, label, started_at_ms, seen_at_ms)
        .and_then(|()| {
            store
                .background_task_describe(tool_use_id, label, owner)
                .map(|_| ())
        });
    match result {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(target: "background_tasks", block_id, node_id = %tool_use_id, error = %e,
                "failed to record a background task from the CLI's task feed");
            false
        }
    }
}

/// Notify subscribers that `block_id`'s `db_background_tasks` state changed
/// (observed, pid recorded, or completed), so the frontend can re-query
/// `COMMAND_LIST_BACKGROUND_TASKS` instead of polling. Live-only (`persist:
/// 0`) — a late subscriber gets the current state via the mount-time list
/// query, not event replay. Deliberately carries no task data itself (just an
/// invalidation signal): the list query is the single source of truth for the
/// actual rows. See
/// docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.2.
pub(crate) fn publish_background_task_updated(broker: &Broker, block_id: &str) {
    broker.publish(MuxEvent {
        event: "background-task-updated".to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "block_id": block_id })),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    // Shapes copied from a real Claude CLI stream (2026-09-26), trimmed of
    // uuid/session_id.
    fn started(task_id: &str, tool: &str, bg: bool, subagent: bool) -> Value {
        json!({"type":"system","subtype":"task_started","task_id":task_id,"owned_by_subagent":subagent,
               "tool_use_id":tool,"description":"Background wait for agents","is_backgrounded":bg,"task_type":"local_bash"})
    }
    fn notification(task_id: &str, tool: &str, status: &str) -> Value {
        json!({"type":"system","subtype":"task_notification","task_id":task_id,"tool_use_id":tool,"status":status,
               "output_file":"/tmp/x.output","summary":"Background command \"Background wait for agents\" completed (exit code 0)"})
    }
    fn updated(task_id: &str, patch: Value) -> Value {
        json!({"type":"system","subtype":"task_updated","task_id":task_id,"patch":patch})
    }

    #[test]
    fn parses_a_backgrounded_shell_start() {
        assert_eq!(
            parse(&started("b6p74mfn6", "toolu_1", true, true)),
            Some(TaskFeedEvent::Started {
                task_id: "b6p74mfn6".into(),
                tool_use_id: "toolu_1".into(),
                label: "Background wait for agents".into(),
                backgrounded: true,
            })
        );
    }

    #[test]
    fn ignores_subagent_tasks_and_non_task_lines() {
        let agent = json!({"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"toolu_a",
                           "description":"Map code","task_type":"local_agent","is_backgrounded":true});
        assert_eq!(parse(&agent), None);
        assert_eq!(
            parse(&json!({"type":"system","subtype":"init","session_id":"s"})),
            None
        );
        assert_eq!(parse(&json!({"type":"assistant","message":{}})), None);
        assert_eq!(parse(&updated("t", json!({"status":"running"}))), None);
    }

    #[test]
    fn maps_every_terminal_status() {
        for (cli, want) in [
            ("completed", BackgroundTaskStatus::Done),
            ("failed", BackgroundTaskStatus::Error),
            ("stopped", BackgroundTaskStatus::Stopped),
            ("something-new", BackgroundTaskStatus::Stopped),
        ] {
            match parse(&notification("t", "toolu_x", cli)) {
                Some(TaskFeedEvent::Ended { status, .. }) => assert_eq!(status, want, "{cli}"),
                other => panic!("{cli}: {other:?}"),
            }
        }
        assert!(matches!(
            parse(&updated("t", json!({"status":"killed","end_time":1}))),
            Some(TaskFeedEvent::Ended {
                status: BackgroundTaskStatus::Stopped,
                tool_use_id: None,
                ..
            })
        ));
    }

    /// The stuck-row incident: a subagent's background task starts and ends on
    /// the parent stream, and the row must end with it.
    #[test]
    fn a_subagent_owned_background_task_is_recorded_then_completed() {
        let store = store();
        let mut feed = TaskFeed::default();
        assert!(feed.apply(
            &store,
            None,
            "blk",
            &started("b6p74mfn6", "toolu_1", true, true),
            1_000
        ));
        let row = store.background_task_get("toolu_1").unwrap().unwrap();
        assert_eq!(
            (row.status, row.label.as_str(), row.block_id.as_str()),
            (
                BackgroundTaskStatus::Running,
                "Background wait for agents",
                "blk"
            )
        );

        assert!(feed.apply(
            &store,
            None,
            "blk",
            &notification("b6p74mfn6", "toolu_1", "completed"),
            181_000
        ));
        let row = store.background_task_get("toolu_1").unwrap().unwrap();
        assert_eq!(
            (row.status, row.ended_at_ms),
            (BackgroundTaskStatus::Done, Some(181_000))
        );
    }

    #[test]
    fn a_foreground_call_never_gets_a_row() {
        let store = store();
        let mut feed = TaskFeed::default();
        assert!(!feed.apply(
            &store,
            None,
            "blk",
            &started("t1", "toolu_fg", false, false),
            1
        ));
        assert!(!feed.apply(
            &store,
            None,
            "blk",
            &notification("t1", "toolu_fg", "completed"),
            2
        ));
        assert!(store.background_task_get("toolu_fg").unwrap().is_none());
        assert!(feed.pending.is_empty(), "ended tasks are forgotten");
    }

    #[test]
    fn a_command_moved_to_the_background_gets_a_row_and_its_end_by_task_id() {
        let store = store();
        let mut feed = TaskFeed::default();
        feed.apply(
            &store,
            None,
            "blk",
            &started("t2", "toolu_mv", false, false),
            10,
        );
        assert!(feed.apply(
            &store,
            None,
            "blk",
            &updated("t2", json!({"is_backgrounded": true})),
            20
        ));
        let row = store.background_task_get("toolu_mv").unwrap().unwrap();
        // Started when the CLI first reported it, seen when it was backgrounded.
        assert_eq!(
            (row.status, row.started_at_ms, row.last_seen_ms),
            (BackgroundTaskStatus::Running, 10, 20)
        );

        // task_updated names only the task_id; the feed resolves it.
        assert!(feed.apply(
            &store,
            None,
            "blk",
            &updated("t2", json!({"status":"failed","end_time":30})),
            30
        ));
        assert_eq!(
            store
                .background_task_get("toolu_mv")
                .unwrap()
                .unwrap()
                .status,
            BackgroundTaskStatus::Error
        );
    }

    /// A subagent's Bash call, as the parent stream carries it (shape from a
    /// real recording, trimmed).
    fn subagent_bash_call(tool: &str, parent: &str) -> Value {
        json!({"type":"assistant","parent_tool_use_id":parent,
               "message":{"role":"assistant","content":[
                   {"type":"tool_use","id":tool,"name":"Bash",
                    "input":{"command":"sleep 180; echo waited","run_in_background":true}}]}})
    }

    #[test]
    fn a_subagent_task_records_the_agent_call_that_owns_it() {
        let store = store();
        let mut feed = TaskFeed::default();
        feed.apply(
            &store,
            None,
            "blk",
            &subagent_bash_call("toolu_1", "toolu_agent"),
            1,
        );
        feed.apply(
            &store,
            None,
            "blk",
            &started("b1", "toolu_1", true, true),
            2,
        );
        let row = store.background_task_get("toolu_1").unwrap().unwrap();
        assert_eq!(row.owner_tool_use_id.as_deref(), Some("toolu_agent"));
        assert!(
            feed.owners.is_empty(),
            "the owner is claimed by its task_started"
        );
    }

    #[test]
    fn the_agents_own_task_has_no_owner() {
        let store = store();
        let mut feed = TaskFeed::default();
        let own = json!({"type":"assistant","parent_tool_use_id":null,
                         "message":{"content":[{"type":"tool_use","id":"toolu_own","name":"Bash","input":{}}]}});
        feed.apply(&store, None, "blk", &own, 1);
        feed.apply(
            &store,
            None,
            "blk",
            &started("b2", "toolu_own", true, false),
            2,
        );
        assert_eq!(
            store
                .background_task_get("toolu_own")
                .unwrap()
                .unwrap()
                .owner_tool_use_id,
            None
        );
    }

    #[test]
    fn the_owner_survives_a_later_move_to_the_background() {
        let store = store();
        let mut feed = TaskFeed::default();
        feed.apply(
            &store,
            None,
            "blk",
            &subagent_bash_call("toolu_mv", "toolu_agent"),
            1,
        );
        feed.apply(
            &store,
            None,
            "blk",
            &started("b3", "toolu_mv", false, true),
            2,
        );
        feed.apply(
            &store,
            None,
            "blk",
            &updated("b3", json!({"is_backgrounded": true})),
            3,
        );
        assert_eq!(
            store
                .background_task_get("toolu_mv")
                .unwrap()
                .unwrap()
                .owner_tool_use_id
                .as_deref(),
            Some("toolu_agent")
        );
    }

    /// The renderer's own push can create the row first, labelled with the
    /// tool name; the CLI's description replaces it.
    #[test]
    fn the_clis_description_replaces_a_generic_label() {
        let store = store();
        store
            .background_task_observe("toolu_1", "blk", "Bash", 1, 1)
            .unwrap();
        let mut feed = TaskFeed::default();
        feed.apply(
            &store,
            None,
            "blk",
            &started("b1", "toolu_1", true, false),
            2,
        );
        assert_eq!(
            store.background_task_get("toolu_1").unwrap().unwrap().label,
            "Background wait for agents"
        );
    }

    /// A task that started before this reader existed (srv restart) still ends:
    /// `task_notification` carries its own tool_use_id.
    #[test]
    fn a_notification_completes_a_row_this_reader_never_saw_start() {
        let store = store();
        store
            .background_task_observe("toolu_old", "blk", "task dev", 5, 5)
            .unwrap();
        let mut feed = TaskFeed::default();
        assert!(feed.apply(
            &store,
            None,
            "blk",
            &notification("gone", "toolu_old", "stopped"),
            99
        ));
        assert_eq!(
            store
                .background_task_get("toolu_old")
                .unwrap()
                .unwrap()
                .status,
            BackgroundTaskStatus::Stopped
        );
    }
}

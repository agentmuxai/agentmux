// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "A filed task #N for B at T": the `task-flow` event and the short list of recent ones, the task half of the
//! Atlas agent map's flow data (the message half is [`message_flow`](super::message_flow)).
//!
//! Like it, this carries ids, names, hosts and times only: the task's number and board slug, who did it, who it
//! is for and what was done. There is no field for the title, the payload, a result or a note.
//!
//! The srv that owns the board reports what happens to its tasks. A task filed on another computer's board is
//! also reported by the filer's srv, once it has the number back, so a map connected to either srv draws the
//! line; both name the task by the board's host and number, which is how a reader joins them.
//!
//! The recent list lives in memory only (`RECENT_MAX` entries, newest last). Each entry remembers the board it is
//! on, so a reader is given only the boards it may see (`GET /api/v1/task-flow`); the board's id is never sent.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use super::message_flow::MessageFlowEnd;
use super::mps::Broker;
use super::mps_events::TaskFlow;

/// How many recent task events are kept.
pub const RECENT_MAX: usize = 200;

/// What was done to the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum TaskFlowKind {
    /// Put on a board, for an agent or for anyone.
    Filed,
    /// Moved to another agent, or to anyone, by the operator or an agent, or taken back and given to another.
    Reassigned,
    /// Started on an agent by the operator or another agent.
    Started,
    /// Given back by the agent that held it.
    HandedBack,
}

/// A task: its number on a board, the board's slug and the computer whose board it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TaskFlowTask {
    #[ts(type = "number")]
    pub number: i64,
    pub board: String,
    pub host: String,
}

/// The `task-flow` event's payload (`mps_events::TaskFlow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TaskFlowEvent {
    pub task: TaskFlowTask,
    /// When the srv that owns the board handled it, in milliseconds since 1970.
    #[ts(type = "number")]
    pub at_ms: u64,
    /// Who did it: absent when the operator did, or the app itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub by: Option<MessageFlowEnd>,
    /// Who it is for or went to: absent when it is for anyone (`handed_back` has none either).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<MessageFlowEnd>,
    pub kind: TaskFlowKind,
}

/// What a call site knows of a task event: ids and names, never its content.
pub struct Seen {
    pub number: i64,
    pub board: String,
    /// The board's id, kept to say who may read the event; never sent.
    pub board_id: String,
    pub host: String,
    pub by: Option<MessageFlowEnd>,
    pub to: Option<MessageFlowEnd>,
    pub kind: TaskFlowKind,
}

struct Kept {
    event: TaskFlowEvent,
    board_id: String,
}

struct Flow {
    broker: Arc<Broker>,
    recent: Mutex<VecDeque<Kept>>,
}

static FLOW: OnceLock<Flow> = OnceLock::new();

/// Start publishing on `broker` for the rest of the process. The first call wins.
pub fn install(broker: Arc<Broker>) {
    let _ = FLOW.set(Flow { broker, recent: Mutex::new(VecDeque::with_capacity(RECENT_MAX)) });
}

/// The event for `seen`, as handled at `at_ms`.
fn event_of(seen: &Seen, at_ms: u64) -> TaskFlowEvent {
    TaskFlowEvent {
        task: TaskFlowTask { number: seen.number, board: seen.board.clone(), host: seen.host.clone() },
        at_ms,
        by: seen.by.clone(),
        to: seen.to.clone(),
        kind: seen.kind,
    }
}

/// Adds `kept` to `recent`, dropping the oldest once there are `RECENT_MAX`.
fn remember(recent: &mut VecDeque<Kept>, kept: Kept) {
    if recent.len() >= RECENT_MAX {
        recent.pop_front();
    }
    recent.push_back(kept);
}

impl Flow {
    /// Keeps and publishes the event for `seen`, handled at `at_ms`.
    fn record(&self, seen: &Seen, at_ms: u64) {
        let event = event_of(seen, at_ms);
        remember(&mut self.recent.lock().unwrap_or_else(|e| e.into_inner()), Kept { event: event.clone(), board_id: seen.board_id.clone() });
        self.broker.publish_event::<TaskFlow>(&event, vec![], 0);
    }
}

/// Records that `seen` happened: published as the `task-flow` event, and kept in the recent list. Does nothing
/// before [`install`].
pub fn note(seen: &Seen) {
    if let Some(flow) = FLOW.get() {
        flow.record(seen, agentmux_common::time::now_ms() as u64);
    }
}

/// The recent task events on the boards `sees` says yes to (by board id), oldest first.
pub fn recent_where(sees: impl Fn(&str) -> bool) -> Vec<TaskFlowEvent> {
    let Some(flow) = FLOW.get() else { return Vec::new() };
    flow.recent.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|k| sees(&k.board_id)).map(|k| k.event.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(number: i64, board_id: &str, kind: TaskFlowKind) -> Seen {
        Seen {
            number,
            board: "work".into(),
            board_id: board_id.into(),
            host: "charlie".into(),
            by: Some(MessageFlowEnd { agent: "agenty".into(), host: Some("narko".into()) }),
            to: Some(MessageFlowEnd { agent: "maricon-2".into(), host: Some("charlie".into()) }),
            kind,
        }
    }

    fn flow() -> Flow {
        Flow { broker: Arc::new(Broker::new()), recent: Mutex::new(VecDeque::new()) }
    }

    #[test]
    fn an_event_has_the_task_the_two_agents_and_the_kind_and_nothing_else() {
        let e = event_of(&seen(614, "b1", TaskFlowKind::HandedBack), 7);
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({
                "task": { "number": 614, "board": "work", "host": "charlie" },
                "at_ms": 7,
                "by": { "agent": "agenty", "host": "narko" },
                "to": { "agent": "maricon-2", "host": "charlie" },
                "kind": "handed_back",
            })
        );
    }

    #[test]
    fn a_task_for_anyone_filed_by_the_operator_has_neither_end() {
        let mut s = seen(1, "b1", TaskFlowKind::Filed);
        (s.by, s.to) = (None, None);
        let v = serde_json::to_value(event_of(&s, 1)).unwrap();
        assert!(v.get("by").is_none() && v.get("to").is_none(), "{v}");
        assert_eq!(v["kind"], "filed");
    }

    #[test]
    fn every_kind_has_its_own_name() {
        let name = |k| serde_json::to_value(event_of(&seen(1, "b", k), 1)).unwrap()["kind"].as_str().unwrap().to_string();
        assert_eq!(
            [TaskFlowKind::Filed, TaskFlowKind::Reassigned, TaskFlowKind::Started, TaskFlowKind::HandedBack].map(name),
            ["filed", "reassigned", "started", "handed_back"]
        );
    }

    #[test]
    fn an_event_is_published_and_kept_with_its_board_for_a_reader_who_may_see_it() {
        let flow = flow();
        let published = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&published);
        flow.broker.add_observer(Arc::new(move |e: &crate::backend::mps::MuxEvent| sink.lock().unwrap().push((e.event.clone(), e.data.clone()))));
        flow.record(&seen(1, "open-board", TaskFlowKind::Filed), 5);
        flow.record(&seen(2, "private-board", TaskFlowKind::Started), 6);
        let published = published.lock().unwrap();
        assert_eq!(published.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["task-flow", "task-flow"]);
        assert!(!published[1].1.as_ref().unwrap().to_string().contains("private-board"), "the board's id is never sent");
        let recent = flow.recent.lock().unwrap();
        let visible: Vec<i64> = recent.iter().filter(|k| k.board_id == "open-board").map(|k| k.event.task.number).collect();
        assert_eq!(visible, vec![1]);
    }

    #[test]
    fn the_recent_list_keeps_the_newest_and_is_bounded() {
        let mut recent = VecDeque::new();
        for n in 0..RECENT_MAX + 5 {
            remember(&mut recent, Kept { event: event_of(&seen(n as i64, "b", TaskFlowKind::Filed), n as u64), board_id: "b".into() });
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent.front().unwrap().event.task.number, 5);
        assert_eq!(recent.back().unwrap().event.task.number, RECENT_MAX as i64 + 4);
    }
}

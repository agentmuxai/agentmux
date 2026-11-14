// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The events that put a countdown or a question in front of the person (a sensitive request's countdown, an
//! agent's start notice, a pane's question), and the names they go out under. They live in the core so
//! `backend/mps_events.rs` names them without reaching up into `sagas/`; the sagas re-export them under their
//! old paths. The wire shape is pinned by `sagas/prompt_events_tests.rs`.

use serde::Serialize;

pub const EVENT_SENSITIVE_PENDING: &str = "sensitive:pending";
pub const EVENT_SENSITIVE_CLEARED: &str = "sensitive:cleared";
pub const EVENT_AGENT_NOTICE: &str = "agent:notice";
pub const EVENT_AGENT_GRANTS: &str = "agent:grants";
pub const EVENT_PANE_CONFIRM: &str = "pane:confirm";

/// A countdown as the person's banner shows it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SensitiveView {
    pub request_id: String,
    pub op: String,
    /// What will happen, built by AgentMux from the request's facts.
    pub summary: String,
    /// The words of its two buttons: the cancel and the do-it-now (`OpSpec`).
    pub cancel_label: String,
    pub now_label: String,
    /// The agent that asked.
    pub requester: String,
    /// The super agent that approved it; absent when none reviewed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approver: Option<String>,
    #[ts(type = "number")]
    pub deadline_ms: i64,
    /// When the countdown began. A window that connects later is replayed the countdowns still
    /// running; this is what tells those from a new one.
    #[ts(type = "number")]
    pub opened_ms: i64,
    /// `pending` → `vetoed` or `cancelled`, or `proceeding` → `done` / `failed`.
    #[ts(type = r#""pending" | "vetoed" | "cancelled" | "proceeding" | "done" | "failed""#)]
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// `sensitive:cleared`: the countdown ended, and how.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SensitiveClearedEvent {
    pub request_id: String,
    pub outcome: String,
}

/// A notice as the pane draws it: a one-line `line`, a `pending` card with a countdown, or the end
/// of a card (`cleared`, with its `outcome`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NoticeView {
    pub id: String,
    #[ts(type = r#""spawn" | "ssh" | "google" | "memory""#)]
    pub kind: &'static str,
    pub block_id: String,
    pub agent: String,
    pub headline: String,
    pub body: String,
    /// The agent's own words, for the quote.
    pub quote: String,
    /// The Always button's words; empty when there is none.
    pub always_label: String,
    /// When the countdown ends; 0 for a line.
    #[ts(type = "number")]
    pub deadline_ms: i64,
    #[ts(type = "number")]
    pub opened_ms: i64,
    #[ts(type = r#""pending" | "line" | "cleared""#)]
    pub status: &'static str,
    /// For `cleared`: `stopped`, `now`, `always`, `deadline` or `gone`.
    pub outcome: String,
}

/// `agent:grants`: the agents with an "Always" for `topic`, after a change.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GrantsEvent {
    #[ts(type = r#""spawn" | "google" | "memory""#)]
    pub topic: &'static str,
    pub agents: Vec<String>,
}

/// A question as the pane draws it: `pending`, or its end (`cleared`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PaneConfirmView {
    pub id: String,
    pub block_id: String,
    pub title: String,
    pub message: String,
    pub ok_label: String,
    pub cancel_label: String,
    pub destructive: bool,
    /// The tick box's words; empty when there is none.
    pub checkbox: String,
    /// When srv stops waiting for the answer.
    #[ts(type = "number")]
    pub deadline_ms: i64,
    #[ts(type = r#""pending" | "cleared""#)]
    pub status: &'static str,
}

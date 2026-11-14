// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shutdown events an agent pane's close and its pending window publish, and the cleaner for the text
//! they show. They live in the core so `backend/` names them without reaching up into `sagas/`; the sagas
//! re-export them under their old paths. The wire shape is pinned by `sagas/shutdown_events_tests.rs`.

use serde::Serialize;

/// `agent:shutdown` — the shutdown log a closing pane shows
/// (SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5, §12.1).
pub const EVENT_AGENT_SHUTDOWN: &str = "agent:shutdown";

/// One `agent:shutdown` line (`mps_events::AgentShutdown`): its order, step and text, and what
/// the step adds.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ShutdownLineEvent {
    pub block_id: String,
    #[ts(type = "number")]
    pub seq: u64,
    pub step: String,
    pub text: String,
    #[serde(flatten)]
    pub extra: ShutdownExtra,
}

/// What a shutdown line can add beyond its step and text. Each is omitted when absent.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ShutdownExtra {
    /// `kill`: how long the agent had before it was killed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub forced_after_ms: Option<u64>,
    /// `error`: why the pane couldn't close.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// `container`: a Compose container left running, and that it was left.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub outcome: Option<String>,
    /// `process` / `survivor`: one of the agent's processes, and what became of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub process: Option<ShutdownProcess>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ShutdownProcess {
    pub pid: u32,
    pub name: String,
    /// `killed`, `stopped`, `survived` or `escaped`.
    pub outcome: String,
}

pub const EVENT_SHUTDOWN_PENDING: &str = "agent:shutdown-pending";
pub const EVENT_SHUTDOWN_PENDING_CLEARED: &str = "agent:shutdown-pending-cleared";

/// A request as the caller and the frontend see it (§12.2).
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PendingView {
    pub request_id: String,
    pub block_id: String,
    /// Who asked: an agent id, or `cron` etc.
    pub by: String,
    /// The agent being shut down, resolved when asked (it may be gone after).
    pub target: String,
    /// The tool: `QuitSelf`, `ClosePane`, `FleetBulkStop`.
    pub via: String,
    /// Why, in the asker's words, cleaned for display (`clean_reason`): never
    /// the raw text. Shown on the pane banner only; an OS toast leaves it out.
    pub reason: String,
    /// What happens if the window runs out: `stop` (the agent stops, its pane
    /// stays), `close_tab` (it stops and its tab closes) or `close_pane`.
    #[ts(type = r#""stop" | "close_tab" | "close_pane""#)]
    pub effect: &'static str,
    #[ts(type = "number")]
    pub deadline_ms: i64,
    /// `pending` → `kept_by_user`, or `proceeding` → `shut_down` / `failed` /
    /// `superseded` (the user closed it themselves meanwhile).
    #[ts(type = r#""pending" | "kept_by_user" | "proceeding" | "shut_down" | "failed" | "superseded""#)]
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// A request for several agents at once (`Stop agents`): the ones still waiting. Absent for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub blocks: Option<Vec<BatchBlock>>,
    /// What became of each agent of such a request: kept by the user, or how its close ended. Filled as
    /// the user keeps one, and once the request settles.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub outcomes: Option<Vec<BlockOutcome>>,
    /// This caller joined a request already open for the block rather than
    /// opening one. Never stored; set on the view `request` returns.
    #[serde(skip)]
    #[ts(skip)]
    pub joined: bool,
}

/// `agent:shutdown-pending-cleared`: the request ended, and how.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PendingClearedEvent {
    pub block_id: String,
    pub request_id: String,
    pub outcome: String,
}

/// An agent of a batch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BatchBlock {
    pub block_id: String,
    /// Its display name, cleaned for the user.
    pub name: String,
}

/// What became of one agent of a batch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockOutcome {
    pub block_id: String,
    pub name: String,
    /// `kept_by_user`, or how its close ended: `shut_down`, `superseded` (already gone) or `failed`.
    #[ts(type = r#""kept_by_user" | "shut_down" | "superseded" | "failed""#)]
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// The longest reason the banner shows.
pub const REASON_MAX: usize = 120;
/// Shown in place of a reason that looks like it holds a credential or asks
/// for something destructive.
pub const REASON_HIDDEN: &str = "Reason hidden: it looks sensitive.";

/// A caller's reason, cleaned for the user to read (spec §3.11). Replaced
/// wholesale by `REASON_HIDDEN` when it trips the keyword list the jekt tier
/// gate and notification previews use; otherwise its first line, control and
/// bidi-override characters removed, whitespace collapsed, cut at
/// `REASON_MAX`. Markup is not interpreted here: the banner renders it as
/// text and a toast never carries it.
pub fn clean_reason(raw: &str) -> String {
    if crate::backend::reactive::sanitize::is_sensitive_message(raw) {
        return REASON_HIDDEN.to_string();
    }
    let Some(first) = raw.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return String::new();
    };
    clean_line(first, REASON_MAX)
}

/// One line for display: bidi-override characters removed, control characters
/// turned into spaces, whitespace collapsed, cut at `max` characters.
pub fn clean_line(raw: &str, max: usize) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !crate::backend::notify::router::is_bidi_control(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

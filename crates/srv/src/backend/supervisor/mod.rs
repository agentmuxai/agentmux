// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Swarm Supervisor: when a supervised agent ends a turn, a cheap judge
//! model reads its recent conversation and either sends it a continuation or
//! stops and tells the user why.
//!
//! - `policy`: the rules, pure: when a turn is judged, the limits, and what a
//!   verdict leads to.
//! - `judge`: the prompt and the one reader of its reply.
//! - `message`: the continuation the agent receives.
//! - `watcher`: listens for completed turns and carries a decision out.
//!
//! docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md.

pub mod judge;
pub mod message;
pub mod policy;
pub mod watcher;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::backend::obj::{self, MetaMapType};

/// Block meta: the pane's mode, `"steady"` or `"drive"`; absent is Off.
pub const META_MODE: &str = "supervisor:mode";
/// Block meta: the run state and recent decisions (`SupervisorState`), written
/// by srv only, read by Swarm.
pub const META_STATE: &str = "supervisor:state";

/// How far the Supervisor keeps an agent going (spec §2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum SupervisorMode {
    #[default]
    Off,
    /// Continue an agent that stopped only to check in; stop on real questions.
    Steady,
    /// Also continue through open questions the agent can research and decide.
    Drive,
}

impl SupervisorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SupervisorMode::Off => "off",
            SupervisorMode::Steady => "steady",
            SupervisorMode::Drive => "drive",
        }
    }

    /// The mode a wire or meta value names; anything else is Off.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "steady" => SupervisorMode::Steady,
            "drive" => SupervisorMode::Drive,
            _ => SupervisorMode::Off,
        }
    }

    pub fn from_meta(meta: &MetaMapType) -> Self {
        Self::parse(&obj::meta_get_string(meta, META_MODE, ""))
    }

    pub fn limits(self) -> Limits {
        match self {
            SupervisorMode::Off => Limits { continuations: 0, run_ms: None, no_progress: 0 },
            SupervisorMode::Steady => Limits { continuations: 5, run_ms: None, no_progress: 2 },
            SupervisorMode::Drive => {
                Limits { continuations: 15, run_ms: Some(2 * 60 * 60 * 1000), no_progress: 2 }
            }
        }
    }
}

/// When a run stops for the user (spec §3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Continuations in a row since the user last spoke.
    pub continuations: u32,
    /// Time since the run's first continuation.
    pub run_ms: Option<u64>,
    /// Continued turns in a row with no tool use.
    pub no_progress: u32,
}

/// What the judge said about a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "SupervisorVerdict")]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Continue,
    Done,
    NeedsUser,
    Blocked,
}

impl Verdict {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "continue" => Some(Verdict::Continue),
            "done" => Some(Verdict::Done),
            "needs_user" | "needs_you" => Some(Verdict::NeedsUser),
            "blocked" => Some(Verdict::Blocked),
            _ => None,
        }
    }
}

/// Where a supervised pane stands, as Swarm shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum SupervisorStatus {
    /// Supervising; nothing to say yet.
    #[default]
    Watching,
    /// Asking the judge about the turn that just ended.
    Judging,
    /// Background work is running; the turn it wakes is judged instead.
    Waiting,
    /// Sent a continuation.
    Continued,
    /// Stopped: the user is needed (`reason` says why).
    Paused,
    /// The judge found the work done.
    Done,
}

/// What the Supervisor did after a turn, for the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum SupervisorAction {
    Continued,
    Paused,
    Done,
    Waited,
    /// No verdict (the judge failed, timed out or answered out of format):
    /// nothing was sent, and the user got the ordinary "finished" notice.
    NoVerdict,
    /// The mode was set (a new run).
    ModeSet,
}

/// One decision, newest last in `SupervisorState::history`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SupervisorDecision {
    #[ts(type = "number")]
    pub at_ms: u64,
    pub action: SupervisorAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub verdict: Option<Verdict>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub reason: String,
}

/// How many decisions the history keeps.
pub const HISTORY_MAX: usize = 10;

/// A supervised pane's run state, mirrored to block meta `supervisor:state`
/// after every decision (spec §3.6).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SupervisorState {
    pub mode: SupervisorMode,
    pub status: SupervisorStatus,
    /// Why it stopped, or what it last decided, in a sentence.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub reason: String,
    /// Continuations in a row since the user last spoke.
    #[serde(default)]
    pub continuations: u32,
    /// The mode's continuation limit, for "3 of 15".
    #[serde(default)]
    pub limit: u32,
    /// When this run's first continuation was sent; 0 before one.
    #[serde(default)]
    #[ts(type = "number")]
    pub run_started_at_ms: u64,
    /// Continued turns in a row with no tool use.
    #[serde(default)]
    pub no_progress: u32,
    /// A fingerprint of the agent's last final message, to see it repeat.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub last_reply: String,
    #[serde(default)]
    #[ts(type = "number")]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub history: Vec<SupervisorDecision>,
}

impl SupervisorState {
    /// A fresh run in `mode`.
    pub fn new(mode: SupervisorMode, now_ms: u64) -> Self {
        SupervisorState { mode, limit: mode.limits().continuations, updated_at_ms: now_ms, ..Default::default() }
    }

    /// The state stored in `meta`, if any and readable.
    pub fn from_meta(meta: &MetaMapType) -> Option<Self> {
        serde_json::from_value(meta.get(META_STATE)?.clone()).ok()
    }

    /// Start a new run: the user spoke, or the mode changed. History is kept.
    pub fn reset_run(&mut self) {
        self.continuations = 0;
        self.run_started_at_ms = 0;
        self.no_progress = 0;
        self.last_reply.clear();
    }

    pub fn record(&mut self, decision: SupervisorDecision) {
        self.updated_at_ms = decision.at_ms;
        self.history.push(decision);
        if self.history.len() > HISTORY_MAX {
            let excess = self.history.len() - HISTORY_MAX;
            self.history.drain(..excess);
        }
    }
}

/// Whether the Supervisor can work on this pane at all: a Claude agent, whose
/// conversation srv can read and whose CLI runs the judge (spec §2.1).
pub fn supports(meta: &MetaMapType) -> bool {
    obj::meta_get_string(meta, "agentProvider", "") == "claude"
        && crate::ambient::digest::reads_output_format(&obj::meta_get_string(meta, "agentOutputFormat", ""))
}

/// Whether a pane is supervised now: a mode is set and the pane supports it.
/// The notification router skips its generic "finished" notice for these, as
/// the Supervisor sends its own (spec §3.7).
pub fn is_supervised(meta: &MetaMapType) -> bool {
    SupervisorMode::from_meta(meta) != SupervisorMode::Off && supports(meta)
}

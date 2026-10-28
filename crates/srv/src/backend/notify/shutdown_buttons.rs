// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The two buttons of a countdown's toast: the cancel and the do-it-now
//! (`SPEC_COUNTDOWN_BUTTONS_AND_BATCH_AGENT_ACTIONS_2026_10_28.md` §3).

use super::policy::{keep_label, NotifyAction, ShutdownInfo};

/// The id of the cancel button (the first of the two): Keep running, or a sensitive operation's veto.
pub const ACTION_KEEP: &str = "keep";
/// The id of the do-it-now button: Close now, or a sensitive operation's approve-now.
pub const ACTION_NOW: &str = "now";

/// The cancel and do-it-now words of an agent close: "Keep running" or "Keep open", and "Close now"
/// (a close of a tab or a pane) or "Stop now" (a stop alone).
pub fn labels(effect: &str, running: bool) -> (String, String) {
    let now = if effect == "stop" { "Stop now" } else { "Close now" };
    (keep_label(running).to_string(), now.to_string())
}

/// The two buttons of a countdown's toast: the countdown's own words if it has them, else the agent close's.
pub(super) fn actions(info: &ShutdownInfo) -> Vec<NotifyAction> {
    let (cancel, now) = info.labels.clone().unwrap_or_else(|| labels(info.effect, info.running));
    vec![NotifyAction { id: ACTION_KEEP.to_string(), label: cancel }, NotifyAction { id: ACTION_NOW.to_string(), label: now }]
}

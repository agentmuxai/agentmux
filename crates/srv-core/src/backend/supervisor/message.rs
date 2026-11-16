// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The continuation a supervised agent receives (spec §3.5): srv's header, a
//! fixed instruction for the mode, and the judge's reason and next step,
//! quoted so model text can neither add a line nor open a marker.

use crate::backend::reactive::sanitize::{quote_in_system_note, supervisor_turn_message};

use super::judge::TEXT_MAX_CHARS;
use super::SupervisorMode;

const STEADY: &str = "Continue the task you were already doing. Ask the user only if something truly needs them.";

const DRIVE: &str = "Keep going without waiting for confirmation. Research open questions thoroughly, choose the \
best option on your own judgement, and say what you assumed. Stop and ask only for something irreversible or \
outward-facing, for something only the user can provide, or if you are blocked.";

const CLOSING: &str = "If the work is done, say so and show the evidence.";

/// The full text for continuation `n` of `of`.
pub fn continuation(mode: SupervisorMode, n: u32, of: u32, reason: &str, next: &str) -> String {
    let mut body = String::from(if mode == SupervisorMode::Drive { DRIVE } else { STEADY });
    body.push(' ');
    body.push_str(CLOSING);
    let reason = quote(reason);
    if !reason.is_empty() {
        body.push_str("\nWhy: ");
        body.push_str(&reason);
    }
    let next = quote(next);
    if mode == SupervisorMode::Drive && !next.is_empty() {
        body.push_str("\nSuggested next step: ");
        body.push_str(&next);
    }
    supervisor_turn_message(mode.as_str(), n, of, &body)
}

/// Model text, flattened to one line, markers quoted, capped.
fn quote(text: &str) -> String {
    let quoted = quote_in_system_note(text);
    if quoted.chars().count() <= TEXT_MAX_CHARS {
        return quoted;
    }
    let mut out: String = quoted.chars().take(TEXT_MAX_CHARS - 1).collect();
    out.push('…');
    out
}

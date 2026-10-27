// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! An agent accepting or sending back a Kanban task in Review. Any agent may;
//! the history records the agent that did it, never the operator
//! (`server/work_delegate.rs`).

/// How long the operator can Reopen a task an agent accepted.
pub const REOPEN_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Why a decision was refused. `code` is stable (the audit and the logs carry
/// it); `words` is for the agent, who can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub words: String,
}

fn refuse(code: &'static str, words: impl Into<String>) -> Refusal {
    Refusal { code, words: words.into() }
}

pub fn not_in_review(num: i64, state: &str) -> Refusal {
    refuse("not_in_review", format!("#{num} is {}, not in Review.", state.replace('_', " ")))
}

pub fn sensitive_quote() -> Refusal {
    refuse("sensitive_text", "The text looks like it holds a credential; it isn't stored.")
}

/// What the Kanban's Start message says when the task's text is another
/// agent's, quoted (`app_api/kanban.rs` `start_text`).
pub const RELAYED_START_MARK: &str = "wrote it down; here it is as they wrote it:";

/// What a send-back message says when the reason is an agent's, quoted
/// (`app_api/kanban.rs` `send_back_task`).
pub const RELAYED_REASON_MARK: &str = "here is their reason as they wrote it:";

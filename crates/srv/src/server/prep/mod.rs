// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Prepare, then proceed (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §3): when a user
//! action is about to restart an agent's session, the agent gets a short window to reach a safe
//! point and leave a handoff, then the action runs.
//!
//! An action calls [`decide`]. `Skip` means nothing is lost or the agent cannot take a turn: the
//! action runs at once, as it always did. `Ask` means [`open`]: one `[AgentMux]` note starts a turn
//! on the agent, the agent answers with `Agents ready` (a handoff, or once for more time), and the
//! action's closure runs when the agent has answered and its turn has ended, when the person presses
//! "Switch now", or when the window runs out. A window, never a gate: nothing waits for an agent that
//! cannot answer, and the closure runs exactly once.

mod decide;
mod handlers;
mod ledger;
mod meta;
mod note;
mod window;

pub use decide::{decide, Decision};
pub use handlers::{handle_ready_self, register};
pub use ledger::{cancel, is_open, open};
pub(crate) use meta::set_block_meta;

use std::time::Duration;

/// How long the agent has, from the person's action.
#[cfg(not(test))]
pub const WINDOW: Duration = Duration::from_secs(30);
/// Short in tests, which wait for real.
#[cfg(test)]
pub const WINDOW: Duration = Duration::from_millis(1_000);
/// The one extension an agent may ask for (a push or a deploy in progress), from the same moment.
#[cfg(not(test))]
pub const EXTENDED_WINDOW: Duration = Duration::from_secs(120);
#[cfg(test)]
pub const EXTENDED_WINDOW: Duration = Duration::from_millis(3_000);
/// The pane meta the composer's chip reads: `{id, until_ms, extended, why, action}`.
pub const META_PREP: &str = "agent:prep";
/// The audit actions: one when the agent is asked, one when it has answered or the window ended.
pub const AUDIT_REQUESTED: &str = "prep.requested";
pub const AUDIT_ANSWERED: &str = "prep.answered";

/// What a user action asks of the agent before it restarts the agent's session.
#[derive(Debug, Clone)]
pub struct Prep {
    pub def_id: String,
    /// The agent as its panes show it, for the note and the audit rows.
    pub agent_name: String,
    /// The agent's open panes.
    pub blocks: Vec<String>,
    /// A short name for the note, the audit and the chip (`account_switch`).
    pub action: &'static str,
    /// What the action moves to; asking for the same thing again is a no-op, for another replaces it.
    pub target: String,
    /// One line for the note: what will happen.
    pub what: String,
    /// Who asked: `operator`, or an agent's slug.
    pub actor: String,
    /// Whether the action ends a turn still running when the window ends (`when: now`), as the note says.
    pub cuts_turn: bool,
}

/// How the window ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// The agent answered and its turn has ended.
    Ready,
    /// The window ran out first (or the agent never answered).
    TimedOut,
    /// The person pressed "Switch now".
    PersonSaidNow,
    /// The note could not be delivered, so nothing was asked.
    NotAsked,
}

impl How {
    /// The audit reason of the `prep.answered` row.
    pub fn reason(self) -> &'static str {
        match self {
            How::Ready => "ready",
            How::TimedOut => "timed_out",
            How::PersonSaidNow => "person_said_now",
            How::NotAsked => "not_asked:note_not_delivered",
        }
    }
}

/// What the action does once the window ends. Runs exactly once, on a blocking thread.
pub type Proceed = Box<dyn FnOnce(How) + Send + 'static>;

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) use ledger::{answer, Answer};

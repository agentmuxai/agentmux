// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One status for an agent and one decision function for everything that wants to reach it
//! (`docs/specs/SPEC_AGENT_AVAILABILITY_EVERY_PATH_ONE_DECISION_2026_10_31.md` §4). The status adds
//! three facts to [`Availability`]: whether the agent has a pane, whether it is wanted running, and
//! whether its model API answers. [`decide`] turns a status and an [`Intent`] into one [`Decision`].
//!
//! A message is still ruled by [`verdict`] (the contract is that `Intent::Message` answers exactly as
//! it does). The other intents say, in one table, what the path that asks them does now; a path moves
//! onto this function in its own change, and the two behaviour changes the spec lists (held input for
//! a limited or signed-out parked agent, a turn-starting message while the API is offline) are marked
//! where they will land. Nothing here reads a store, a controller or the clock.

use super::agent_availability::{verdict, Availability, Verdict, WaitReason};

/// Whether the agent has a pane to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// Its block is in a tab.
    Shown,
    /// Its block is in no tab (`agent:parked`): the process runs, no pane shows it.
    Parked,
    /// No block at all: defined, never opened.
    None,
}

/// What the agent's own block says it should be doing (`agent:desired`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    Running,
    Stopped,
    /// The block records nothing (or nothing we know).
    Unset,
}

/// Whether the agent's model API answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Online,
    Degraded,
    Offline,
    /// Tracked and not checked yet, for `for_ms` so far.
    Unchecked { for_ms: u64 },
    /// Not a model API srv tracks: it never holds anything.
    NotTracked,
}

/// How long a model API may stay "not checked yet" before it no longer holds input: the pane's rule.
pub const UNCHECKED_HOLD_MS: u64 = 20_000;

impl Reach {
    /// Whether text the user typed may go to the agent now: not while `Offline`, and not while "not
    /// checked yet" for less than [`UNCHECKED_HOLD_MS`].
    pub fn lets_input_go(self) -> bool {
        match self {
            Reach::Online | Reach::Degraded | Reach::NotTracked => true,
            Reach::Offline => false,
            Reach::Unchecked { for_ms } => for_ms > UNCHECKED_HOLD_MS,
        }
    }
}

/// Everything a path needs to know about an agent before it sends something to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStatus {
    pub availability: Availability,
    pub pane: Pane,
    pub wanted: Wanted,
    pub api: Reach,
    /// A controller holds the agent's process (or is about to start it): something can be sent to it.
    pub controller: bool,
}

/// What a path wants to do with the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// A jekt. `urgent`, `routine` and `cold_after_ms` are [`verdict`]'s arguments.
    Message { urgent: bool, routine: bool, cold_after_ms: Option<u64> },
    /// An `[AgentMux]` note or a bulletin: it never starts a turn.
    Note,
    /// The text the user typed for a pane with no view, which srv sends.
    HeldInput,
    /// A message or cron that may start a stopped agent; `allowed` is the setting and the sender's right.
    Start { allowed: bool },
    /// Whether a holder's claim on a task stays with it. `aimed_at_agent`: the task names an agent.
    Lease { aimed_at_agent: bool, now_ms: u64 },
}

/// What to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Deliver,
    /// Keep it, or refuse it where it can't be kept (the caller's rule), until `until_ms` when known.
    Wait { reason: WaitReason, until_ms: Option<u64> },
    /// Keep it until the model API answers.
    WaitApi,
    /// It rides the next message and never starts a turn.
    Attach,
    /// Start the agent, then deliver.
    Start,
    /// Nobody to give it to.
    Drop,
    /// The holder keeps its claim.
    Renew,
    /// The claim lapses so someone else can take the task.
    Lapse,
}

/// How long a holder must have been out of quota or signed out before its lease on a task open to
/// anyone stops being renewed (R5 of `SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md`).
pub const UNAVAILABLE_LEASE_AFTER_MS: u64 = 30 * 60 * 1000;
/// And how far off its reset must be (or unknown): a limit that lifts within the hour is waited out.
pub const RESET_FAR_MS: u64 = 60 * 60 * 1000;

/// The decision for `intent` to the agent in `status`. `hands_over_work` is asked only for a routine
/// message to a cold session ([`verdict`]'s argument); other intents never call it.
pub fn decide(status: &AgentStatus, intent: Intent, hands_over_work: impl FnOnce() -> bool) -> Decision {
    match intent {
        Intent::Message { urgent, routine, cold_after_ms } => match verdict(&status.availability, urgent, routine, cold_after_ms, hands_over_work) {
            Verdict::Deliver => Decision::Deliver,
            Verdict::Wait { reason, until_ms } => Decision::Wait { reason, until_ms },
        },
        // Right for every agent that has a pane to ride on; an agent with no process takes it only when it
        // is wanted running (the pane that starts shows it).
        Intent::Note => match (status.availability, status.wanted) {
            (Availability::NotRunning { .. }, Wanted::Running) => Decision::Attach,
            (Availability::NotRunning { .. }, _) => Decision::Drop,
            _ => Decision::Attach,
        },
        // What `held_input` does now: it waits for a process and for the API, and does not read a limit or a
        // sign-out (spec D1: those will wait too). That the agent is between turns is the caller's check.
        Intent::HeldInput => match status.availability {
            Availability::NotRunning { .. } => Decision::Wait { reason: WaitReason::NotRunning, until_ms: None },
            _ if !status.api.lets_input_go() => Decision::WaitApi,
            _ => Decision::Deliver,
        },
        Intent::Start { allowed } => match status.availability {
            Availability::NotRunning { .. } if allowed => Decision::Start,
            Availability::NotRunning { .. } => Decision::Wait { reason: WaitReason::NotRunning, until_ms: None },
            _ => Decision::Deliver,
        },
        Intent::Lease { aimed_at_agent, now_ms } => {
            let (since_ms, resets_at_ms) = match status.availability {
                Availability::Limited { resets_at_ms, since_ms } => (since_ms, resets_at_ms),
                Availability::SignedOut { since_ms } => (since_ms, None),
                _ => return Decision::Renew,
            };
            let for_long = now_ms.saturating_sub(since_ms) >= UNAVAILABLE_LEASE_AFTER_MS;
            let reset_far = resets_at_ms.is_none_or(|r| r > now_ms + RESET_FAR_MS);
            if !aimed_at_agent && for_long && reset_far {
                Decision::Lapse
            } else {
                Decision::Renew
            }
        }
    }
}

#[cfg(test)]
#[path = "agent_status_tests.rs"]
mod tests;

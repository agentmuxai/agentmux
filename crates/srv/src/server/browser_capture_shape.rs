// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How many shaped captures an agent may make (#47629).
//!
//! `Browser` `capture` can be told what the value should look like (`agentmux_common::capture_shape`: a
//! short prefix, one of a closed list of named shapes, a length range), and nothing is saved if it is not.
//! The answer to that, saved or not, is a yes or no about a value the agent cannot read, so it is budgeted
//! like any other question about one:
//!
//! - an agent may make [`SHAPED`] captures with a shape in an hour, whether they match or not (a match is
//!   as much an answer as a miss), all its panes together: opening another pane starts nothing new;
//! - at most [`MISSES`] of them may fail; the next one that does ends shapes for the agent for an hour, and
//!   the answer says so. A capture that fails because the host found no value to test counts as a miss too:
//!   that is the same answer.
//!
//! A capture with no shape is not budgeted: it never answers about the value.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Shaped captures an agent may make, and how many may miss, in [`WINDOW`].
const SHAPED: usize = 10;
const MISSES: usize = 3;
const WINDOW: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
struct Agent {
    shaped: Vec<Instant>,
    misses: Vec<Instant>,
    /// Shapes are refused until then.
    locked_until: Option<Instant>,
}

/// What each agent has used, by agent.
#[derive(Default)]
pub(super) struct Ledger(HashMap<String, Agent>);

impl Ledger {
    /// Take one shaped capture from `agent`'s budget at `now`, or say why it may not make one.
    fn start(&mut self, agent: &str, now: Instant) -> Result<(), String> {
        let a = self.0.entry(agent.to_string()).or_default();
        a.shaped.retain(|t| now.duration_since(*t) < WINDOW);
        a.misses.retain(|t| now.duration_since(*t) < WINDOW);
        if let Some(until) = a.locked_until.filter(|u| *u > now) {
            return Err(format!("{}: shapes are refused for you for another {} minutes. Capture without a shape, or ask the person", LOCKED, (until - now).as_secs() / 60 + 1));
        }
        a.locked_until = None;
        if a.shaped.len() >= SHAPED {
            return Err(format!("{SHAPED} captures with a shape in the last hour: a shape is for choosing between values, not for testing what one says. Capture without a shape, or ask the person"));
        }
        a.shaped.push(now);
        Ok(())
    }

    /// Note a shaped capture that did not match; true when it was one too many, and shapes are refused for an hour.
    fn miss(&mut self, agent: &str, now: Instant) -> bool {
        let a = self.0.entry(agent.to_string()).or_default();
        a.misses.retain(|t| now.duration_since(*t) < WINDOW);
        a.misses.push(now);
        if a.misses.len() > MISSES {
            a.locked_until = Some(now + WINDOW);
            return true;
        }
        false
    }
}

const LOCKED: &str = "too many captures with a shape did not match";

fn ledger() -> &'static Mutex<Ledger> {
    static LEDGER: OnceLock<Mutex<Ledger>> = OnceLock::new();
    LEDGER.get_or_init(Mutex::default)
}

/// Take one shaped capture from the agent's budget.
pub(super) fn start(agent: &str) -> Result<(), String> {
    ledger().lock().unwrap_or_else(|p| p.into_inner()).start(&agent.to_lowercase(), Instant::now())
}

/// Note that a shaped capture did not match (or found nothing to test); the words to add to the answer when
/// shapes are now refused for an hour.
pub(super) fn miss(agent: &str) -> &'static str {
    if ledger().lock().unwrap_or_else(|p| p.into_inner()).miss(&agent.to_lowercase(), Instant::now()) {
        " That was one miss too many: shapes are refused for you for an hour."
    } else {
        ""
    }
}

#[cfg(test)]
#[path = "browser_capture_shape_tests.rs"]
mod tests;

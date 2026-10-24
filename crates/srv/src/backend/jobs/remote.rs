// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A run delivered to an agent in another channel (#1988, spec §3.2).
//!
//! Each AgentMux publishes turn events to its own broker, so the instance
//! that fired a run can't see the turn of a pane another channel holds. The
//! ledger is shared, though, and the run id is the delivery's `request_id`:
//! the receiving instance finds the open row by it, marks it running with
//! its own pane's account, notes itself in `links.followed_by_channel` and
//! follows the turn on its own broker ([`RunLedger::follow_forwarded`]).
//!
//! The firing instance only watches the row ([`RunLedger::check_remote`]):
//! a run nobody followed (a receiver on an older build) is closed after the
//! no-turn timeout, cost unknown, as before; a run whose follower's process
//! is gone is interrupted. Every close is the conditional `job_run_finish`,
//! so whichever side writes first wins and the other changes nothing.

use agentmux_common::time::now_ms;
use serde_json::json;

use super::follower::NO_TURN_AFTER_MS;
use super::{registered_uid, RunLedger, SOURCE_CRON};
use crate::backend::storage::job_runs::{infra, state, JobRun};

/// The `links` keys the following instance writes: its channel and its
/// process id.
pub const FOLLOWED_BY_CHANNEL: &str = "followed_by_channel";
pub const FOLLOWED_BY_PID: &str = "followed_by_pid";

/// The summary of a run no instance followed.
pub const NOT_FOLLOWED: &str = "delivered to an agent in another channel; AgentMux can't follow its turn from here";
/// The summary of a run whose following instance stopped during it.
pub const FOLLOWER_GONE: &str = "the AgentMux that followed this run stopped during it";

/// The channel that follows `run`, and its process id (0 when not written).
pub fn followed_by(run: &JobRun) -> Option<(String, u32)> {
    let channel = run.links.get(FOLLOWED_BY_CHANNEL)?.as_str().filter(|c| !c.is_empty())?;
    let pid = run.links.get(FOLLOWED_BY_PID).and_then(|v| v.as_u64()).unwrap_or(0);
    Some((channel.to_string(), u32::try_from(pid).unwrap_or(0)))
}

/// Whether `recorded_by` (`<channel>@<version>`) names `channel`.
fn recorded_in(recorded_by: &str, channel: &str) -> bool {
    recorded_by == channel || recorded_by.strip_prefix(channel).is_some_and(|rest| rest.starts_with('@'))
}

/// What the firing instance does with a watched cross-channel run.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Closed already, or gone: stop watching.
    Done,
    /// Still the receiver's, or still inside the no-turn timeout.
    Wait,
    /// Close it with this status and summary, cost unknown.
    Close { infra: &'static str, summary: &'static str },
}

/// Judge a watched run, armed at `armed_at_ms`. Pure, with the process
/// check passed in, so the rules are testable.
pub fn verdict(run: Option<&JobRun>, armed_at_ms: i64, now: i64, pid_alive: impl Fn(u32) -> bool) -> Verdict {
    let Some(run) = run.filter(|r| r.ended_at_ms.is_none()) else {
        return Verdict::Done;
    };
    match followed_by(run) {
        Some((_, pid)) if pid != 0 && !pid_alive(pid) => Verdict::Close { infra: infra::INTERRUPTED, summary: FOLLOWER_GONE },
        Some(_) => Verdict::Wait,
        None if now - armed_at_ms > NO_TURN_AFTER_MS => Verdict::Close { infra: infra::OK, summary: NOT_FOLLOWED },
        None => Verdict::Wait,
    }
}

impl RunLedger {
    /// Whether another channel's instance follows run `run_id`.
    pub(super) fn followed_elsewhere(&self, run_id: &str) -> bool {
        let run = self.store.job_run_get(run_id).ok().flatten();
        run.as_ref().and_then(followed_by).is_some_and(|(channel, _)| channel != self.channel)
    }

    /// A delivery this instance just placed in `block_id` was forwarded here
    /// with `request_id` `run_id`. When that names a cron run another
    /// channel fired and nobody follows yet, take the follow-up: mark it
    /// running with this pane's account, note this channel in its links and
    /// follow its turn here. Called before the delivery's answer goes back,
    /// so the firing instance sees the run is followed. Never inserts a row,
    /// so the slot lease stays the firing scheduler's.
    pub fn follow_forwarded(&self, run_id: &str, block_id: &str) -> bool {
        if run_id.is_empty() || block_id.is_empty() {
            return false;
        }
        let Ok(Some(run)) = self.store.job_run_get(run_id) else { return false };
        let open = run.ended_at_ms.is_none() && (run.state == state::SCHEDULED || run.state == state::RUNNING);
        if run.job_source != SOURCE_CRON || !open || recorded_in(&run.recorded_by, &self.channel) || followed_by(&run).is_some() {
            return false;
        }
        let now = now_ms();
        let pane = self.pane(block_id);
        let receiver = [pane.agent_uid.as_str(), registered_uid(block_id).as_deref().unwrap_or("")]
            .into_iter()
            .find(|u| !u.is_empty())
            .unwrap_or("")
            .to_string();
        match self.store.job_run_mark_running(run_id, &receiver, block_id, &pane.account, &pane.provider, now) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                tracing::warn!(run_id, error = %e, "jobs: couldn't take a forwarded run");
                return false;
            }
        }
        let links = json!({
            FOLLOWED_BY_CHANNEL: self.channel,
            FOLLOWED_BY_PID: std::process::id(),
            "transcript": { "block_id": block_id, "channel": self.channel },
        });
        if let Err(e) = self.store.job_run_add_links(run_id, &links) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't note who follows a forwarded run");
        }
        self.follower.arm(block_id, run_id, now);
        tracing::info!(run_id, block_id, recorded_by = %run.recorded_by, "jobs: following a run another channel fired");
        true
    }

    /// Judge the watched cross-channel run `run_id` at `now` and write a
    /// close it calls for. `true` when it needs no more watching.
    pub fn check_remote(&self, run_id: &str, armed_at_ms: i64, now: i64) -> bool {
        let run = self.store.job_run_get(run_id).ok().flatten();
        match verdict(run.as_ref(), armed_at_ms, now, crate::backend::reactive::registry::pid_alive) {
            Verdict::Done => true,
            Verdict::Wait => false,
            Verdict::Close { infra, summary } => {
                self.turn_finished(run_id, infra, None, summary);
                true
            }
        }
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;

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
//! is gone is interrupted. That process counts only while it is still the
//! srv that wrote the claim (`process_identity::srv_alive`): a pid another
//! program reused reads as gone. Every close is the conditional `job_run_finish`,
//! so whichever side writes first wins and the other changes nothing.

use serde_json::json;

use super::follower::NO_TURN_AFTER_MS;
use super::{registered_uid, RunLedger, SOURCE_CRON};
use crate::backend::storage::job_runs::{infra, JobRun};

/// The `links` keys the following instance writes: its channel, its
/// process id, and when it took the run.
pub const FOLLOWED_BY_CHANNEL: &str = "followed_by_channel";
pub const FOLLOWED_BY_PID: &str = "followed_by_pid";
pub const FOLLOWED_AT_MS: &str = "followed_at_ms";

/// The summary of a run no instance followed.
pub const NOT_FOLLOWED: &str = "delivered to an agent in another channel; AgentMux can't follow its turn from here";
/// The summary of a run whose following instance stopped during it.
pub const FOLLOWER_GONE: &str = "the AgentMux that followed this run stopped during it";

/// Who follows a run, from its `links`.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowedBy {
    pub channel: String,
    /// 0 when not written.
    pub pid: u32,
    pub at_ms: Option<i64>,
}

/// Who follows `run`, if anyone.
pub fn followed_by(run: &JobRun) -> Option<FollowedBy> {
    let channel = run.links.get(FOLLOWED_BY_CHANNEL)?.as_str().filter(|c| !c.is_empty())?;
    let pid = run.links.get(FOLLOWED_BY_PID).and_then(|v| v.as_u64()).unwrap_or(0);
    let at_ms = run.links.get(FOLLOWED_AT_MS).and_then(|v| v.as_i64());
    Some(FollowedBy { channel: channel.to_string(), pid: u32::try_from(pid).unwrap_or(0), at_ms })
}

/// Whether the follower `pid`, which took the run at `at_ms`, is still that
/// srv: an `agentmux-srv` process started no later than then, give or take
/// `process_identity::START_SLACK_SECS`.
fn follower_alive(pid: u32, at_ms: Option<i64>) -> bool {
    let at = at_ms.and_then(|ms| u64::try_from(ms).ok()).map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms));
    crate::backend::process_identity::srv_alive(pid, at)
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

/// Judge a watched run, armed at `armed_at_ms`. Pure, with the follower
/// check (`pid`, `followed_at_ms`) passed in, so the rules are testable.
pub fn verdict(run: Option<&JobRun>, armed_at_ms: i64, now: i64, alive: impl Fn(u32, Option<i64>) -> bool) -> Verdict {
    let Some(run) = run.filter(|r| r.ended_at_ms.is_none()) else {
        return Verdict::Done;
    };
    match followed_by(run) {
        Some(f) if f.pid != 0 && !alive(f.pid, f.at_ms) => Verdict::Close { infra: infra::INTERRUPTED, summary: FOLLOWER_GONE },
        Some(_) => Verdict::Wait,
        None if now - armed_at_ms > NO_TURN_AFTER_MS => Verdict::Close { infra: infra::OK, summary: NOT_FOLLOWED },
        None => Verdict::Wait,
    }
}

impl RunLedger {
    /// Whether another channel's instance follows run `run_id`.
    pub(super) fn followed_elsewhere(&self, run_id: &str) -> bool {
        let run = self.store.job_run_get(run_id).ok().flatten();
        run.as_ref().and_then(followed_by).is_some_and(|f| f.channel != self.channel)
    }

    /// A delivery this instance just placed in `block_id` was forwarded here
    /// with `request_id` `run_id`. When that names a cron run another
    /// channel fired and nobody follows yet, take the follow-up: mark it
    /// running with this pane's account, note this channel in its links and
    /// follow its turn here, from `delivery_started_ms`, taken before the
    /// injection: a turn the delivery started, even one already over, is
    /// this run's whole turn, as on the local path (`record_delivery`).
    /// Called before the delivery's answer goes back,
    /// so the firing instance sees the run is followed. Never inserts a row,
    /// so the slot lease stays the firing scheduler's.
    pub fn follow_forwarded(&self, run_id: &str, block_id: &str, delivery_started_ms: i64) -> bool {
        if run_id.is_empty() || block_id.is_empty() {
            return false;
        }
        let Ok(Some(run)) = self.store.job_run_get(run_id) else { return false };
        // Any open run, a catch-up `latest` pending fire included: the
        // receiver sees it before the sender marks it running.
        if run.job_source != SOURCE_CRON || !run.is_open() || recorded_in(&run.recorded_by, &self.channel) || followed_by(&run).is_some() {
            return false;
        }
        let now = delivery_started_ms;
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
            FOLLOWED_AT_MS: now,
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
        match verdict(run.as_ref(), armed_at_ms, now, follower_alive) {
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

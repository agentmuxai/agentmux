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
//!
//! **Stopping such a run.** A timeout, or a `cancel_other` overlap, can't
//! interrupt the pane from the firing instance, and closing the row there
//! would free its place while the agent goes on working. So the firing
//! instance asks the following one ([`RunLedger::request_remote_stop`], a
//! [`RemoteStopHook`] the server installs), notes the ask in
//! `links.stop_requested`, and leaves the row open. The following instance
//! ([`RunLedger::stop_followed_here`]) closes the row with the status asked
//! for and interrupts the pane, as a local stop does. When the ask can't be
//! delivered the reason goes in `links.stop_error` and it is tried again.
//!
//! **Whose stop it is.** The ask over HTTP names only the run, the pane and the
//! asking channel. What the follower acts on is the note in the shared ledger,
//! which only the instance that fired the run writes, so another channel that
//! guesses a run id, or names the right channel, stops nothing: the row has to
//! carry a stop its own origin asked for.

use std::sync::Arc;

use agentmux_common::time::now_ms;
use serde_json::json;

use super::follower::NO_TURN_AFTER_MS;
use super::{registered_uid, RunLedger, SOURCE_CRON};
use crate::backend::storage::job_runs::{infra, reason, JobRun};

/// The `links` keys the following instance writes: its channel, its
/// process id, and when it took the run.
pub const FOLLOWED_BY_CHANNEL: &str = "followed_by_channel";
pub const FOLLOWED_BY_PID: &str = "followed_by_pid";
pub const FOLLOWED_AT_MS: &str = "followed_at_ms";

/// The summary of a run no instance followed.
pub const NOT_FOLLOWED: &str = "delivered to an agent in another channel; AgentMux can't follow its turn from here";
/// The summary of a run whose following instance stopped during it.
pub const FOLLOWER_GONE: &str = "the AgentMux that followed this run stopped during it";

/// The `links` keys of a stop the firing instance asked for, and of why the
/// ask could not be delivered.
pub const STOP_REQUESTED: &str = "stop_requested";
pub const STOP_ERROR: &str = "stop_error";
/// An ask is repeated no sooner than this, and no more than `STOP_ATTEMPTS`
/// times: a following instance that never answers is left to the checks
/// above (its process gone, its turn ending).
const STOP_RETRY_MS: i64 = 60_000;
const STOP_ATTEMPTS: u64 = 5;

/// A stop for a run another channel follows, as it is asked of that channel.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteStop {
    pub run_id: String,
    /// The pane in the following channel.
    pub block_id: String,
    /// The channel asking: the one that fired the run.
    pub from_channel: String,
    pub infra: String,
    pub reason: String,
    pub summary: String,
}

/// How the server delivers a [`RemoteStop`] (it needs the shared registry and
/// an HTTP client, which the ledger has not). It reports back through
/// [`RunLedger::remote_stop_done`] or [`RunLedger::remote_stop_failed`].
pub type RemoteStopHook = Arc<dyn Fn(RemoteStop) + Send + Sync>;

/// The statuses a stop may close a run with, and the reasons it may give:
/// what a timeout and `cancel_other` write, so a request can't write others.
fn stop_is_known(infra_status: &str, why: &str) -> bool {
    matches!((infra_status, why), (infra::TIMEOUT, reason::TIMEOUT) | (infra::INTERRUPTED, reason::OVERLAP))
}

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
        self.follower.arm(block_id, run_id, now, self.stop_at_usd(run_id, &pane.provider));
        // This instance's Jobs tab sees the run start; its close publishes
        // again (`turn_finished`, also behind `check_remote`).
        self.changed(&run.job_id);
        tracing::info!(run_id, block_id, recorded_by = %run.recorded_by, "jobs: following a run another channel fired");
        true
    }

    /// Judge the watched cross-channel run `run_id` at `now` and write a
    /// close it calls for. `true` when it needs no more watching.
    pub fn check_remote(&self, run_id: &str, armed_at_ms: i64, now: i64) -> bool {
        let run = self.store.job_run_get(run_id).ok().flatten();
        match verdict(run.as_ref(), armed_at_ms, now, follower_alive) {
            Verdict::Done => {
                // The receiver closed it: this instance, which has the job,
                // looks at it too (its incident, §3.4).
                if let Some(run) = &run {
                    self.changed(&run.job_id);
                }
                true
            }
            Verdict::Wait => false,
            Verdict::Close { infra, summary } => {
                self.turn_finished(run_id, infra, None, summary);
                true
            }
        }
    }

    /// How to ask another channel's instance to stop a run it follows.
    pub fn install_remote_stop(&self, hook: RemoteStopHook) {
        let _ = self.remote_stop.set(hook);
    }

    /// Ask the instance that follows `run` to stop it with `infra_status`,
    /// `why` and `summary`: a timeout, or `cancel_other`. Notes the ask in
    /// the run's links and leaves the row open, for the follower to close
    /// (see the module doc). `true` when an ask went out now; `false` when
    /// nobody follows it, there is no way to ask, or an ask is already under
    /// way (or all of them were used).
    pub(super) fn request_remote_stop(&self, run: &JobRun, infra_status: &str, why: &str, summary: &str) -> bool {
        if followed_by(run).is_none() || run.block_id.is_empty() || !stop_is_known(infra_status, why) {
            return false;
        }
        let Some(hook) = self.remote_stop.get() else { return false };
        let asked = run.links.get(STOP_REQUESTED);
        let attempts = asked.and_then(|a| a.get("attempts")).and_then(|v| v.as_u64()).unwrap_or(0);
        let last = asked.and_then(|a| a.get("at_ms")).and_then(|v| v.as_i64()).unwrap_or(0);
        let now = now_ms();
        if attempts >= STOP_ATTEMPTS || now - last < STOP_RETRY_MS {
            return false;
        }
        let note = json!({ STOP_REQUESTED: {
            "at_ms": now, "attempts": attempts + 1, "by_channel": self.channel,
            "infra": infra_status, "reason": why, "summary": summary,
        }});
        if let Err(e) = self.store.job_run_add_links(&run.id, &note) {
            // Without the note the ask would repeat at every pass.
            tracing::warn!(run_id = %run.id, error = %e, "jobs: couldn't note a stop asked of another channel");
            return false;
        }
        hook(RemoteStop {
            run_id: run.id.clone(),
            block_id: run.block_id.clone(),
            from_channel: self.channel.clone(),
            infra: infra_status.to_string(),
            reason: why.to_string(),
            summary: summary.to_string(),
        });
        tracing::info!(run_id = %run.id, block_id = %run.block_id, reason = why, "jobs: asking the AgentMux that follows a run to stop it");
        true
    }

    /// The follower stopped the run: its row is closed. This instance's
    /// Jobs tab and a run waiting on this one's place move on.
    pub fn remote_stop_done(&self, run_id: &str) {
        self.run_changed(run_id);
        super::pending_wake().notify_one();
    }

    /// The ask couldn't be delivered: say why in the run's links, where the
    /// operator and the next pass see it. The row stays open.
    pub fn remote_stop_failed(&self, run_id: &str, error: &str) {
        let note = json!({ STOP_ERROR: { "at_ms": now_ms(), "error": error } });
        if let Err(e) = self.store.job_run_add_links(run_id, &note) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't note why a stop could not be delivered");
        }
        tracing::warn!(run_id, error, "jobs: the AgentMux that follows a run could not be asked to stop it");
        self.run_changed(run_id);
    }

    /// The following side of [`Self::request_remote_stop`]: close the run
    /// this instance follows, as a local stop does, and interrupt its pane's
    /// turn, when `from_channel` fired the run and asked, in the run's row,
    /// for this stop. The status, reason and summary are the row's, not the
    /// request's. `Ok(false)` when it had ended already; an error when the run
    /// is not one this process follows in `block_id`, was fired by another
    /// channel than `from_channel`, or carries no stop of that channel's.
    pub fn stop_followed_here(&self, run_id: &str, block_id: &str, from_channel: &str) -> Result<bool, String> {
        let run = self.store.job_run_get(run_id).map_err(|e| e.to_string())?.ok_or_else(|| "no such run".to_string())?;
        let follows = followed_by(&run).is_some_and(|f| f.channel == self.channel && (f.pid == 0 || f.pid == std::process::id()));
        if !follows {
            return Err("this AgentMux does not follow that run".to_string());
        }
        if run.block_id.is_empty() || run.block_id != block_id {
            return Err("that run is not in that pane".to_string());
        }
        if from_channel.is_empty() || !recorded_in(&run.recorded_by, from_channel) {
            return Err("that run was not fired by that AgentMux".to_string());
        }
        let asked = run.links.get(STOP_REQUESTED).filter(|a| a.get("by_channel").and_then(|c| c.as_str()) == Some(from_channel));
        let text = |key: &str| asked.and_then(|a| a.get(key)).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (infra_status, why, summary) = (text("infra"), text("reason"), text("summary"));
        if asked.is_none() || !stop_is_known(&infra_status, &why) {
            return Err("that AgentMux has not asked for a stop of that run".to_string());
        }
        if !run.is_open() {
            return Ok(false);
        }
        match self.store.job_run_stop(&run.id, &infra_status, &why, &summary, now_ms()) {
            Ok(true) => {
                self.interrupt_block(&run.block_id);
                self.reap_worker(&run.id);
                self.run_changed(&run.id);
                super::pending_wake().notify_one();
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;

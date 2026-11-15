// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Wait block: the run stops for a time or for an event
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "Waiting").
//!
//! Before it waits the run keeps a checkpoint that says what it waits for: the
//! time (`deadlines`) or the key of the event (`resume_on`), with the block in
//! `frontier.waiting`. A restart finds that checkpoint, runs the block again, and
//! the block takes the deadline or the key it finds there, so the wait goes on
//! where it was. A deadline that passed while AgentMux was stopped ends the wait
//! at once, saying it was `late`. A wait on an event does not remember events
//! that came while AgentMux was stopped: their source sends them again.

use serde_json::{json, Value};

use super::Scheduler;
use crate::drone::executor::schedule;
use crate::backend::drone_wait;
use crate::drone::executor::blocks::wait::{self as config, Wait};
use crate::drone::executor::engine::RunEvent;
use crate::drone::types::FlowNode;
use agentmux_common::time::now_ms;

/// Closes the wait on its key when the block ends however it ends, including
/// when the run is stopped and the block is dropped.
struct OpenKey(String);

impl Drop for OpenKey {
    fn drop(&mut self) {
        drone_wait::close(&self.0);
    }
}

impl<'a> Scheduler<'a> {
    /// Wait as `node` says, keeping the checkpoint first, and give what the wait
    /// came to: when it ended and whether it was late, or the event's payload.
    pub(super) async fn run_wait(&mut self, node: &'a FlowNode) -> Result<Value, String> {
        let id = node.id.as_str();
        let now = now_ms();
        let wait = config::parse(node, &|t| self.scope.resolve(t), now)?;
        let restored_at = self.deadlines.get(id).copied();
        let restored_key = self.resume_on.get(id).cloned();
        let restored = restored_at.is_some() || restored_key.is_some();
        // The block's own limit, kept so a restart finds how much of it is left.
        let limit_s = schedule::timeout_seconds(node)?;
        let timeout_at = self.wait_timeouts.get(id).copied().or_else(|| limit_s.map(|s| now.saturating_add(i64::try_from(s).unwrap_or(i64::MAX) * 1000)));
        let timed_out = || Err(format!("timed out after {}", super::plain_duration(std::time::Duration::from_secs(limit_s.unwrap_or(0)))));

        // What it waits for, and the way to be told.
        let (at, event) = match &wait {
            Wait::For(ms) => (Some(restored_at.unwrap_or(now.saturating_add(*ms))), None),
            Wait::Until(at) => (Some(*at), None),
            Wait::On(label) => {
                let key = restored_key.unwrap_or_else(drone_wait::new_key);
                let rx = drone_wait::open(&key);
                (None, Some((label.clone(), key, rx)))
            }
        };
        let _open = event.as_ref().map(|(_, key, _)| OpenKey(key.clone()));
        if let Some(at) = at {
            self.deadlines.insert(id.to_string(), at);
        }
        if let Some((_, key, _)) = &event {
            self.resume_on.insert(id.to_string(), key.clone());
        }
        if let Some(t) = timeout_at {
            self.wait_timeouts.insert(id.to_string(), t);
        }
        if !restored {
            // The run cannot be taken up again where it waits unless this is kept.
            self.checkpoint_required(id)?;
        }
        let _ = self.tx.send(RunEvent::Waiting {
            run_id: self.run_id.to_string(),
            block_id: id.to_string(),
            resume_at: at,
            on: event.as_ref().map(|(label, _, _)| label.clone()),
            key: event.as_ref().map(|(_, key, _)| key.clone()),
        });

        // How long until the block's limit, if it has one: it may already be past.
        let until_timeout = || timeout_at.map(|t| u64::try_from(t - now_ms()).unwrap_or(0));
        if let Some(at) = at {
            // The limit comes first when it is earlier than the time: even after a restart that found both past.
            if let Some(t) = timeout_at.filter(|t| *t < at) {
                tokio::time::sleep(std::time::Duration::from_millis(u64::try_from(t - now_ms()).unwrap_or(0))).await;
                return timed_out();
            }
            let left = at - now_ms();
            if left > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(u64::try_from(left).unwrap_or(0))).await;
            }
            // Late is what a restart makes of a wait: it was taken up past its time.
            let late_ms = if restored_at.is_some() { (now - at).max(0) } else { 0 };
            return Ok(json!({ "resume_at": at, "late": late_ms > 0, "late_ms": late_ms }));
        }
        let Some((label, _, rx)) = event else { return Err("the wait has nothing to wait for".to_string()) };
        let payload = match until_timeout() {
            Some(t) => tokio::select! {
                payload = rx => payload,
                () = tokio::time::sleep(std::time::Duration::from_millis(t)) => return timed_out(),
            },
            None => rx.await,
        }
        .map_err(|_| "the run stopped waiting for its event".to_string())?;
        Ok(json!({ "on": label, "payload": payload, "late": false, "late_ms": 0 }))
    }

    /// The wait is over, however it ended: the run is no longer waiting.
    pub(super) fn end_wait(&mut self, id: &str, late: bool) {
        self.waiting = None;
        self.deadlines.remove(id);
        self.resume_on.remove(id);
        self.wait_timeouts.remove(id);
        let _ = self.tx.send(RunEvent::WaitEnded { run_id: self.run_id.to_string(), block_id: id.to_string(), late });
    }
}

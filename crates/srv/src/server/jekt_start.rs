// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Start-then-deliver (`docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` §D4, P2b): with
//! `agents:always_available` on, a message that finds its agent stopped starts it instead of only
//! waiting for someone to open it.
//!
//! - A held message (`HoldReason::NotRunning`): `hold_with_reason` has already kept the row, and only
//!   a host-tier, full-key sender gets that far (`hold_applies`), so a LAN or WAN peer or an
//!   unverified sender never starts an agent. The start runs in the background; the replay delivers
//!   the row when the agent registers (`jekt_held`).
//! - An operator Job or cron fire, which is never held: [`deliver_cron`] starts the agent, waits for
//!   it to register and delivers once.
//!
//! Admission, the spawn gate and the lease apply as for `agent.open`; a start that fails leaves the
//! message held (or the fire missed) exactly as before.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::backend::reactive::InjectionRequest;

use super::AppState;

/// How long a Job's fire waits for a started agent to register before it is missed.
const REGISTER_WAIT: Duration = Duration::from_secs(60);
const REGISTER_POLL: Duration = Duration::from_millis(500);

/// Whether a stopped agent is started by a message (the `agents:always_available` setting).
fn enabled(state: &AppState) -> bool {
    crate::sagas::park_agents::enabled(state)
}

/// Agents with a start in flight, so a burst of messages makes one start.
fn starting() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

/// Holds an agent's slot in [`starting`] until dropped.
struct StartSlot(String);

impl StartSlot {
    /// `None` when a start of `uid` is already in flight.
    fn claim(uid: &str) -> Option<Self> {
        starting().lock().unwrap().insert(uid.to_string()).then(|| Self(uid.to_string()))
    }
}

impl Drop for StartSlot {
    fn drop(&mut self) {
        starting().lock().unwrap().remove(&self.0);
    }
}

/// Start the agent `uid` unless it is registered or a start is in flight.
async fn start_once(state: &AppState, uid: &str) -> Result<(), String> {
    if state.reactive_handler.has_uid_registration(uid) {
        return Ok(());
    }
    let Some(_slot) = StartSlot::claim(uid) else {
        return Ok(());
    };
    let block_id = super::app_api::agent_attach::start_agent(state, uid).await?;
    make_addressable(state, uid, &block_id);
    Ok(())
}

/// A started agent that has never run (no session to resume) has a controller that spawns on its first
/// turn and registers for delivery only then, and there is no pane to send that turn. Register it here, so
/// the replay (or a Job's fire) reaches it and the delivery starts that turn.
fn make_addressable(state: &AppState, uid: &str, block_id: &str) {
    if state.reactive_handler.has_uid_registration(uid) {
        return;
    }
    let slug = state.mstore.agent_def_get(uid).ok().flatten().map(|d| d.slug).filter(|s| !s.is_empty()).unwrap_or_else(|| uid.to_string());
    if let Err(e) = state.reactive_handler.register_agent_full(&slug, block_id, None, 0, Some(&slug), Some(uid), "registration.start") {
        tracing::warn!(target_uid = %uid, "start: could not register the started agent for delivery: {e}");
    }
}

/// Whether the sender may start an agent: not one whose signature failed (`sig_verified` is `Some(false)`
/// only for a sender that has a key and did not prove it). The hold's own test (`hold_applies`) has
/// already kept LAN and WAN callers out.
fn may_start(req: &InjectionRequest) -> bool {
    req.sig_verified != Some(false)
}

/// A message was held for the stopped agent `target_uid`: start it in the background. The row stays
/// held and the replay delivers it once the agent registers, so a failed start loses nothing.
pub(super) fn start_for_held(state: &AppState, req: &InjectionRequest, target_uid: &str) {
    if !enabled(state) || !may_start(req) {
        return;
    }
    let (state, uid) = (state.clone(), target_uid.to_string());
    tokio::spawn(async move {
        match start_once(&state, &uid).await {
            Ok(()) => tracing::info!(target_uid = %uid, "message for a stopped agent: started it, the held message follows"),
            Err(e) => tracing::warn!(target_uid = %uid, "message for a stopped agent: could not start it, the message stays held: {e}"),
        }
    });
}

/// Deliver a Job or cron fire. A fire to an agent that is stopped (`agent not found`) starts it, with
/// the setting on, waits for it to register and delivers once more.
pub(crate) async fn deliver_cron(state: &AppState, creator_uid: &str, req: InjectionRequest) -> serde_json::Value {
    let first = super::reactive::deliver(state, super::ReactiveAuthVia::FullAuthKey, creator_uid, req.clone()).await;
    if !enabled(state) || !is_stopped_answer(&first) {
        return first;
    }
    let mstore = state.mstore.clone();
    let target = req.target_agent.clone();
    let uid = tokio::task::spawn_blocking(move || super::reactive::defined_here(&mstore, &target)).await.ok().flatten();
    let Some((uid, _slug)) = uid else {
        return first;
    };
    if let Err(e) = start_once(state, &uid).await {
        tracing::warn!(target_uid = %uid, "cron: could not start the stopped agent: {e}");
        return first;
    }
    if !wait_registered(state, &uid).await {
        return first;
    }
    super::reactive::deliver(state, super::ReactiveAuthVia::FullAuthKey, creator_uid, req).await
}

/// Whether an inject answer says the target is not running here.
fn is_stopped_answer(body: &serde_json::Value) -> bool {
    body.get("success").and_then(|v| v.as_bool()) != Some(true)
        && body.get("error").and_then(|v| v.as_str()).is_some_and(|e| e.starts_with("agent not found"))
}

async fn wait_registered(state: &AppState, uid: &str) -> bool {
    let deadline = tokio::time::Instant::now() + REGISTER_WAIT;
    while tokio::time::Instant::now() < deadline {
        if state.reactive_handler.has_uid_registration(uid) {
            return true;
        }
        tokio::time::sleep(REGISTER_POLL).await;
    }
    state.reactive_handler.has_uid_registration(uid)
}

#[cfg(test)]
mod tests;

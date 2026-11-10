// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A change of route or model that is kept only once the first turn on it has worked (spec section 7a.3, "Thinking
//! blocks across providers"). A backend that is not Anthropic's may refuse a history it did not write: a thinking
//! block's signature, an image it cannot read, a setting it does not take. Left alone, the agent is on a place that
//! answers every turn with a 400. So a change starts a trial: the place the agent came from is kept, the first turn the
//! relaunched harness runs decides it, and a request the new place refuses puts the agent back where it was and says why.
//!
//! The trial is armed when the harness is relaunched on the new place ([`arm`], from the launch): a turn that was still
//! running on the old one when the change was recorded must not settle it. It is armed for that pane: an agent's other
//! panes, still on the old place, do not decide it. (A late frame of a process the pane has since replaced is dropped
//! by the pane's reader before it gets here.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::launch;
use super::switch::{self, Previous};
use crate::backend::storage::content::AgentContent;
use crate::backend::storage::store::Store;

/// The definition content type that holds the [`Trial`].
pub const TRIAL_TYPE: &str = "agent:model_route_trial";
/// The block meta key the pane reads to say a change was undone and why; absent when none was.
pub const UNDONE_META_KEY: &str = "agent:model_switch_undone";
/// How much of the harness's error is kept for the person.
const REASON_CHARS: usize = 300;

/// A change waiting for its first turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    /// Where the agent was before the change. An empty `route` is the harness's own sign-in.
    pub from: Previous,
    /// The harness was relaunched on the new place: its next turn settles the change.
    #[serde(default)]
    pub armed: bool,
    /// The pane whose harness was relaunched; only its turns settle the change.
    #[serde(default)]
    pub block: String,
}

/// What a turn's end says about the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The turn worked: the change stands.
    Commit,
    /// The place refused the request (this HTTP status): the change is undone.
    Refused(u16),
    /// The turn failed for something that is not about the history (a limit, a network error, a server error): the change
    /// waits for another turn.
    Wait,
}

/// The trial of the agent definition `def_id`, if one is waiting.
pub fn pending(mstore: &Store, def_id: &str) -> Option<Trial> {
    serde_json::from_str(&mstore.agent_content_get(def_id, TRIAL_TYPE).ok()??.content).ok()
}

fn save(mstore: &Store, def_id: &str, trial: &Trial) -> Result<(), String> {
    let content = serde_json::to_string(trial).map_err(|e| e.to_string())?;
    mstore
        .agent_content_set(&AgentContent { agent_id: def_id.to_string(), content_type: TRIAL_TYPE.to_string(), content, updated_at: agentmux_common::time::now_ms() })
        .map_err(|e| e.to_string())
}

fn clear(mstore: &Store, def_id: &str) {
    if let Err(e) = mstore.agent_content_delete(def_id, TRIAL_TYPE) {
        tracing::warn!(def_id, error = %e, "model route: the trial of a change was not cleared");
    }
}

/// A change from `from` to `to` was recorded. A trial that never got to run keeps the place the agent last worked on: it
/// is not moved on by a second change, and a change back to it ends the trial.
pub fn begin(mstore: &Store, def_id: &str, from: &Previous, to: &Previous) -> Result<(), String> {
    let origin = pending(mstore, def_id).filter(|t| !t.armed).map_or_else(|| from.clone(), |t| t.from);
    if &origin == to {
        clear(mstore, def_id);
        return Ok(());
    }
    save(mstore, def_id, &Trial { from: origin, armed: false, block: String::new() })
}

/// The harness of the agent in pane `block_id` was relaunched on its recorded place. The latest relaunch is the pane
/// that decides: one that was closed before its first turn cannot leave the change waiting for good.
pub fn arm(mstore: &Store, def_id: &str, block_id: &str) {
    let Some(mut trial) = pending(mstore, def_id).filter(|t| !t.armed || t.block != block_id) else { return };
    trial.armed = true;
    trial.block = block_id.to_string();
    if let Err(e) = save(mstore, def_id, &trial) {
        tracing::warn!(def_id, error = %e, "model route: the trial of a change was not armed");
    }
}

/// What the `result` frame that ends a turn says about the change.
pub fn verdict(frame: &Value) -> Verdict {
    if !crate::agents::failure::is_error_result_frame(frame) {
        return Verdict::Commit;
    }
    match api_status(frame.get("result").and_then(Value::as_str).unwrap_or_default()) {
        // The request was not accepted as it was sent: the history, a setting or the model itself.
        Some(status @ (400 | 404 | 413 | 422)) => Verdict::Refused(status),
        _ => Verdict::Wait,
    }
}

/// The status in the harness's `API Error: 400 ...`.
fn api_status(text: &str) -> Option<u16> {
    let rest = &text[text.find("API Error: ")? + "API Error: ".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|s| (100..600).contains(s))
}

/// A change that was undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undone {
    /// Where the agent is again.
    pub back_on: Previous,
    /// What it tried.
    pub tried: Previous,
}

/// A turn of the agent in pane `block_id` ended with `frame`. When it settles an armed trial, the change stands (`None`) or is undone
/// (`Some`: the record is back on the old place and the pane says why; the caller restarts the harness).
pub fn settle(mstore: &Store, def_id: &str, block_id: &str, frame: &Value) -> Option<Undone> {
    let trial = pending(mstore, def_id).filter(|t| t.armed && t.block == block_id)?;
    let status = match verdict(frame) {
        Verdict::Commit => {
            clear(mstore, def_id);
            return None;
        }
        Verdict::Wait => return None,
        Verdict::Refused(status) => status,
    };
    // An empty place is the harness's own sign-in: a change onto it can be refused and undone like any other.
    let tried = switch::place_of(launch::choice_of(mstore, def_id).as_ref());
    if let Err(e) = switch::restore(mstore, def_id, &trial.from) {
        tracing::warn!(def_id, error = %e, "model route: a refused change was not undone");
        return None;
    }
    clear(mstore, def_id);
    let reason: String = frame.get("result").and_then(Value::as_str).unwrap_or_default().chars().take(REASON_CHARS).collect();
    publish_undone(mstore, block_id, &tried, &trial.from, status, &reason);
    tracing::info!(def_id, status, from = %trial.from.model, to = %tried.model, "model route: the first turn was refused; the change is undone");
    Some(Undone { back_on: trial.from, tried })
}

fn publish_undone(mstore: &Store, block_id: &str, tried: &Previous, back_on: &Previous, status: u16, reason: &str) {
    let label = |p: &Previous| {
        if p.route.is_empty() {
            "your plan".to_string()
        } else {
            super::catalog::route(&p.route).and_then(|r| r.model(&p.model)).map_or_else(|| p.model.clone(), |m| m.label.to_string())
        }
    };
    let card = serde_json::json!({
        "at": agentmux_common::time::now_ms(),
        "tried": { "route": tried.route, "model": tried.model, "label": label(tried) },
        "back_on": { "route": back_on.route, "model": back_on.model, "label": label(back_on) },
        "status": status,
        "reason": reason,
    });
    let mut update = crate::backend::obj::MetaMapType::new();
    update.insert(UNDONE_META_KEY.to_string(), card);
    if let Err(e) = crate::backend::wcore::object_meta::update_object_meta(mstore, &format!("block:{block_id}"), &update) {
        tracing::warn!(block_id, error = %e, "model route: the pane was not told a change was undone");
    }
}

#[cfg(test)]
#[path = "trial_tests.rs"]
mod tests;

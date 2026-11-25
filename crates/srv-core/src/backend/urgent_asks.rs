// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The urgent-message watch's record (docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §2, third row):
//! an urgent message that names a task (`#N`), from a sender that was proven for that message, is an ask the recipient
//! owes an answer to. `quiet_watch` reads the asks that are still open and tells the sender when the recipient has gone
//! quiet without answering.
//!
//! What is kept, and nothing else: the sender's name, the recipient's pane, the task numbers the message named and the
//! time. Never the text. Only a delivered, urgent message whose sender was proven for it (`host-verified`,
//! `channel-verified`, `lan-verified` or `wan-verified`) is kept, so a sender that was not proven (`self-declared`,
//! `network-claimed`, `unverified`, a failed signature) can neither make an agent owe it work nor draw notes to itself.
//! The record is made after the delivery is decided and cannot change it: it takes one lock for a moment, does not
//! fail, and a panic in it is caught.

use std::sync::LazyLock;

use super::message_origin::OriginTrust;
use super::reactive::{InjectionRequest, InjectionResponse};

/// The most asks kept; the oldest go first.
const MAX_ASKS: usize = 256;
/// How long an ask is kept (unix ms): a day, then it is forgotten whether or not it was answered.
pub const MAX_AGE_MS: u64 = 24 * 60 * 60_000;

/// An urgent message to an agent that named tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// Who sent it, lowercased (the name it gave; the trust of the message says it was proven).
    pub sender: String,
    /// The recipient's pane.
    pub block_id: String,
    /// The task numbers it named.
    pub tasks: Vec<i64>,
    /// When it was delivered (unix ms).
    pub at_ms: u64,
}

static ASKS: LazyLock<parking_lot::Mutex<Vec<Ask>>> = LazyLock::new(Default::default);

/// Whether the sender of a message was proven for it.
fn proven(trust: OriginTrust) -> bool {
    matches!(trust, OriginTrust::HostVerified | OriginTrust::ChannelVerified | OriginTrust::LanVerified | OriginTrust::WanVerified)
}

/// The ask a message makes, if it makes one. Pure: every condition is here.
pub fn ask_from(priority: Option<&str>, trust: OriginTrust, sender: Option<&str>, message: &str, delivered_to: Option<&str>, now_ms: u64) -> Option<Ask> {
    if priority != Some("urgent") || !proven(trust) {
        return None;
    }
    let sender = sender.map(str::trim).filter(|s| !s.is_empty())?.to_lowercase();
    let block_id = delivered_to.filter(|b| !b.is_empty())?.to_string();
    let tasks = super::cold_hold::task_numbers(message);
    if tasks.is_empty() {
        return None;
    }
    Some(Ask { sender, block_id, tasks, at_ms: now_ms })
}

/// Keep the ask `req` makes, if it makes one, after `response` says it was delivered. Called by the handler once the
/// delivery is decided; it changes nothing about it.
pub fn note_delivery(req: &InjectionRequest, response: &InjectionResponse) {
    if !response.success {
        return;
    }
    guarded(|| {
        let tier = req.delivery_tier.as_deref().unwrap_or("host");
        let wan_pinned = tier == "wan" && req.wan_instance.is_some() && req.wan_verified == Some(true);
        let trust = super::reactive::sanitize::jekt_trust(tier, req.sig_verified, req.lan_verified, req.channel_verified, wan_pinned);
        let now = agentmux_common::time::now_ms_u64();
        if let Some(ask) = ask_from(req.priority.as_deref(), trust, req.source_agent.as_deref(), &req.message, response.block_id.as_deref(), now) {
            push(ask);
        }
    });
}

/// Run `record`, and let nothing it does reach the caller: a panic in it is caught.
fn guarded(record: impl FnOnce()) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(record));
}

/// Keep `ask`. It never waits: if the list is in use at that moment (the watch is reading it) the ask is dropped,
/// because a delivery is never held up for a note.
fn push(ask: Ask) {
    let Some(mut asks) = ASKS.try_lock() else { return };
    asks.push(ask);
    if asks.len() > MAX_ASKS {
        let extra = asks.len() - MAX_ASKS;
        asks.drain(..extra);
    }
}

/// The asks not yet forgotten at `now_ms`, oldest first.
pub fn open_asks(now_ms: u64) -> Vec<Ask> {
    let mut asks = ASKS.lock();
    asks.retain(|a| now_ms.saturating_sub(a.at_ms) < MAX_AGE_MS);
    asks.clone()
}

/// Forget the asks for which `answered` says yes (they are done being watched).
pub fn settle(mut answered: impl FnMut(&Ask) -> bool) {
    ASKS.lock().retain(|a| !answered(a));
}

/// The record is one list for the process, so the tests that read or fill it (here and in `quiet_watch`) take turns.
#[cfg(test)]
pub(crate) static TEST_TURN: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[cfg(test)]
#[path = "urgent_asks_tests.rs"]
mod tests;

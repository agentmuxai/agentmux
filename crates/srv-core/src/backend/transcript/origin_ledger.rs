// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What srv delivered to a provider that echoes the prompt into its own output, and where each
//! message came from (docs/specs/SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §8, P3).
//!
//! srv writes no user record for such a provider, so the line in its transcript is the provider's
//! own, and nothing in it says who sent the words. The delivery path says which text it handed to
//! which block ([`expect`]); the place that stores the provider's output asks whether a user line
//! is one of those ([`take`]) and, when it is, signs the origin onto the line
//! (`append::append_output_line`). Matching is by block and exact text, oldest first, the way
//! `viewer::origin` pairs a phone's message. A line nobody expected gets no origin: it is untyped,
//! and a viewer shows it in the container, never as the human.
//!
//! An entry nobody takes within [`KEEP`] (the provider never echoed it) is dropped, and a block
//! holds at most [`PER_BLOCK`], so words typed faster than a provider echoes them can't grow it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::backend::message_origin::RecordOrigin;

/// How long a delivered message waits for the provider's echo. A queued message is delivered when
/// the provider starts, so this is generous.
pub const KEEP: Duration = Duration::from_secs(600);

/// The most entries kept for one block; the oldest goes first.
pub const PER_BLOCK: usize = 64;

struct Expected {
    block_id: String,
    text: String,
    record: RecordOrigin,
    at: Instant,
}

static EXPECTED: OnceLock<parking_lot::Mutex<Vec<Expected>>> = OnceLock::new();
/// Entries waiting, read without the lock so storing a line costs nothing while none are.
static WAITING: AtomicUsize = AtomicUsize::new(0);

fn expected() -> &'static parking_lot::Mutex<Vec<Expected>> {
    EXPECTED.get_or_init(|| parking_lot::Mutex::new(Vec::new()))
}

fn prune(list: &mut Vec<Expected>, now: Instant) {
    list.retain(|e| now.duration_since(e.at) < KEEP);
    WAITING.store(list.len(), Ordering::Release);
}

/// `text` was delivered to `block_id`'s provider, from `record`.
pub fn expect(block_id: &str, text: &str, record: RecordOrigin) {
    let mut list = expected().lock();
    let now = Instant::now();
    prune(&mut list, now);
    if list.iter().filter(|e| e.block_id == block_id).count() >= PER_BLOCK {
        if let Some(oldest) = list.iter().position(|e| e.block_id == block_id) {
            list.remove(oldest);
        }
    }
    list.push(Expected { block_id: block_id.to_string(), text: text.trim().to_string(), record, at: now });
    WAITING.store(list.len(), Ordering::Release);
}

/// The record for the user line `text` of `block_id`'s output, taken out of the ledger; `None`
/// when srv delivered no such message (or already paired it).
pub fn take(block_id: &str, text: &str) -> Option<RecordOrigin> {
    if WAITING.load(Ordering::Acquire) == 0 {
        return None;
    }
    let text = text.trim();
    let mut list = expected().lock();
    prune(&mut list, Instant::now());
    let at = list.iter().position(|e| e.block_id == block_id && e.text == text)?;
    let record = list.remove(at).record;
    WAITING.store(list.len(), Ordering::Release);
    Some(record)
}

#[cfg(test)]
#[path = "origin_ledger_tests.rs"]
mod tests;

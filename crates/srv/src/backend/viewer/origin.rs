// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which user messages a paired device sent, so the transcript can say so
//! (agentmux-mobile's SPEC_SEND_FROM_THE_PHONE_2026_10_13 §5.3).
//!
//! A message sent from the phone reaches the agent as a plain user turn: the
//! text the agent reads carries no label. The record AgentMux stores for that
//! turn (`{"type":"user","message":{…}}` in the block's `output` file) gets
//! one extra top-level field, which only srv writes:
//!
//! ```json
//! {"type":"user","message":{"role":"user","content":"Run the tests"},
//!  "amx":{"via":"viewer","device":"Pixel 9","message_id":"0b6f…"}}
//! ```
//!
//! The desktop pane shows "From Pixel 9" from it, and the phone matches its
//! "Sending…" line to the record by `message_id`. The CLI's stdin gets the
//! line without it.
//!
//! The send route says which text it is about to deliver to which block
//! ([`expect`]); the places that store a user record ask whether the line
//! they store is one of those ([`stamp`]), and take the entry when it is.
//! Matching is by block and exact text, oldest first. An entry nobody stores
//! within [`KEEP`] (the agent never took it) is dropped.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long an expected message waits to be stored. A message queued while
/// the agent's process starts is stored when it is delivered, so this is
/// generous.
pub const KEEP: Duration = Duration::from_secs(600);

/// Who sent a user message, as recorded beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceOrigin {
    /// The device's name, as it paired.
    pub device: String,
    /// The phone's id for the message.
    pub message_id: String,
}

struct Expected {
    block_id: String,
    text: String,
    origin: DeviceOrigin,
    at: Instant,
}

static EXPECTED: OnceLock<parking_lot::Mutex<Vec<Expected>>> = OnceLock::new();
/// Entries waiting, read without the lock so storing a line costs nothing
/// while no device is sending.
static WAITING: AtomicUsize = AtomicUsize::new(0);

fn expected() -> &'static parking_lot::Mutex<Vec<Expected>> {
    EXPECTED.get_or_init(|| parking_lot::Mutex::new(Vec::new()))
}

fn prune(list: &mut Vec<Expected>, now: Instant) {
    list.retain(|e| now.duration_since(e.at) < KEEP);
    WAITING.store(list.len(), Ordering::Release);
}

/// The send route is about to deliver `text` to `block_id` for a device.
pub fn expect(block_id: &str, text: &str, origin: DeviceOrigin) {
    let mut list = expected().lock();
    let now = Instant::now();
    prune(&mut list, now);
    list.push(Expected { block_id: block_id.to_string(), text: text.to_string(), origin, at: now });
    WAITING.store(list.len(), Ordering::Release);
}

/// Delivery failed: nothing will be stored for this message.
pub fn forget(block_id: &str, message_id: &str) {
    let mut list = expected().lock();
    list.retain(|e| !(e.block_id == block_id && e.origin.message_id == message_id));
    WAITING.store(list.len(), Ordering::Release);
}

/// The line to store for user record `line` (one NDJSON line, a trailing
/// newline kept) of `block_id`: with its sender added when a device sent it,
/// else `None` and the line is stored as it is.
pub fn stamp(block_id: &str, line: &[u8]) -> Option<Vec<u8>> {
    if WAITING.load(Ordering::Acquire) == 0 {
        return None;
    }
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let mut record: serde_json::Value = serde_json::from_slice(body).ok()?;
    if record.get("type").and_then(|t| t.as_str()) != Some("user") {
        return None;
    }
    let text = record.pointer("/message/content").and_then(|c| c.as_str())?.to_string();
    let origin = {
        let mut list = expected().lock();
        prune(&mut list, Instant::now());
        let at = list.iter().position(|e| e.block_id == block_id && e.text == text)?;
        let origin = list.remove(at).origin;
        WAITING.store(list.len(), Ordering::Release);
        origin
    };
    record["amx"] = serde_json::json!({
        "via": "viewer",
        "device": origin.device,
        "message_id": origin.message_id,
    });
    let mut out = serde_json::to_vec(&record).ok()?;
    if line.ends_with(b"\n") {
        out.push(b'\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Vec<u8> {
        let mut l = serde_json::json!({ "type": "user", "message": { "role": "user", "content": text } })
            .to_string()
            .into_bytes();
        l.push(b'\n');
        l
    }

    fn origin(id: &str) -> DeviceOrigin {
        DeviceOrigin { device: "Pixel 9".into(), message_id: id.into() }
    }

    // One test, so parallel tests never share the global list mid-check.
    #[test]
    fn a_device_message_is_stamped_once_on_its_own_block_and_nothing_else_is() {
        let block = format!("origin-test-{}", uuid::Uuid::new_v4());
        let other = format!("origin-test-{}", uuid::Uuid::new_v4());
        assert!(stamp(&block, &line("hi")).is_none(), "nothing expected");

        expect(&block, "hi", origin("m1"));
        assert!(stamp(&other, &line("hi")).is_none(), "another block");
        assert!(stamp(&block, &line("hello")).is_none(), "other text");
        let assistant = br#"{"type":"assistant","message":{"content":"hi"}}"#;
        assert!(stamp(&block, assistant).is_none(), "not a user record");

        let stamped = stamp(&block, &line("hi")).expect("stamped");
        assert!(stamped.ends_with(b"\n"), "keeps the newline");
        let v: serde_json::Value = serde_json::from_slice(&stamped).unwrap();
        assert_eq!(v["amx"], serde_json::json!({ "via": "viewer", "device": "Pixel 9", "message_id": "m1" }));
        assert_eq!(v["message"]["content"], "hi", "the record itself is unchanged");
        assert!(stamp(&block, &line("hi")).is_none(), "taken once: the same words typed again are the user's own");

        // Two sends of the same words are stamped in order.
        expect(&block, "again", origin("a"));
        expect(&block, "again", origin("b"));
        let first: serde_json::Value = serde_json::from_slice(&stamp(&block, &line("again")).unwrap()).unwrap();
        let second: serde_json::Value = serde_json::from_slice(&stamp(&block, &line("again")).unwrap()).unwrap();
        assert_eq!((first["amx"]["message_id"].as_str(), second["amx"]["message_id"].as_str()), (Some("a"), Some("b")));

        // A failed delivery is forgotten.
        expect(&block, "lost", origin("x"));
        forget(&block, "x");
        assert!(stamp(&block, &line("lost")).is_none());
    }
}

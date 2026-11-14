// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A message handed to the cloud relay is queued, not delivered: the relay keeps it for a short
//! window and drops it if nobody picks it up, and the sender never heard. This tells the sender,
//! with one AgentMux system note for its next turn, when a message it queued is still unconfirmed
//! after that window, naming the target and the message id, so it can ask again or reach the
//! target another way instead of waiting on a message that went nowhere.
//!
//! Nothing here learns whether a message was picked up: the one thing srv can see is the target
//! writing to the sender, which shows the target is reachable and so cancels the note. Every other
//! queued message is reported as unconfirmed, in those words, never as lost.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::backend::reactive::handler::ReactiveHandler;
use crate::backend::reactive::types::InjectionResponse;
use crate::muxbus::delivery_status::notes::NOTE_MARKER;

/// How long the relay keeps a message nobody picked up; what the send answer tells the sender.
pub const RELAY_HOLD_MS: u64 = 30 * 60 * 1000;

/// Waited past the window before the note, so a message picked up at the last moment and a reply
/// to it are not reported as unconfirmed.
const SLACK_MS: u64 = 60 * 1000;

/// The most queued messages watched at once; past it the oldest is dropped from the book.
const MAX_WATCHED: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Watched {
    id: String,
    /// The agent that sent it, and gets the note.
    from: String,
    /// The agent it was addressed to.
    to: String,
    queued_ms: u64,
}

#[derive(Debug, Default)]
struct Book {
    entries: VecDeque<Watched>,
}

impl Book {
    fn add(&mut self, w: Watched) {
        if self.entries.len() >= MAX_WATCHED {
            self.entries.pop_front();
        }
        self.entries.push_back(w);
    }

    /// The target wrote to the sender: it is reachable, so nothing it was sent is reported.
    fn heard_from(&mut self, sender: &str, receiver: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|w| !(w.to.eq_ignore_ascii_case(sender) && w.from.eq_ignore_ascii_case(receiver)));
        before - self.entries.len()
    }

    fn take(&mut self, id: &str) -> Option<Watched> {
        let i = self.entries.iter().position(|w| w.id == id)?;
        self.entries.remove(i)
    }
}

static BOOK: Mutex<Book> = Mutex::new(Book { entries: VecDeque::new() });

fn book() -> std::sync::MutexGuard<'static, Book> {
    BOOK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The note for a message still unconfirmed after the relay's window.
fn unconfirmed_note(w: &Watched) -> String {
    format!(
        "{NOTE_MARKER} Your message {id} to {to} went to the cloud relay {mins} minutes ago and was never confirmed \
         delivered. The relay drops a message nobody has picked up after {mins} minutes, and {to} has not written to \
         you since, so treat it as not received: send it again once {to} is running, or reach {to} another way \
         (Agents list shows who is running).",
        id = w.id,
        to = w.to,
        mins = RELAY_HOLD_MS / 60_000,
    )
}

/// Watch a message just queued at the relay for `to` by `from`: once the window has passed with no
/// word from `to`, `from` gets the note at its next turn.
pub fn watch(handler: &'static ReactiveHandler, from: &str, to: &str, id: &str) {
    if from.is_empty() || to.is_empty() || id.is_empty() {
        return;
    }
    let w = Watched { id: id.to_string(), from: from.to_string(), to: to.to_string(), queued_ms: agentmux_common::time::now_ms_u64() };
    book().add(w.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(RELAY_HOLD_MS + SLACK_MS)).await;
        let Some(due) = book().take(&w.id) else { return };
        if let Err(e) = handler.hold_system_note(&due.from, &format!("relay-unconfirmed:{}", due.id), &unconfirmed_note(&due)) {
            tracing::debug!(error = %e, id = %due.id, queued_ms = due.queued_ms, "relay unconfirmed note not held");
        }
    });
}

/// A message from `sender` to `receiver` arrived: if `receiver` had queued one for `sender`, `sender`
/// is reachable and that one is no longer watched.
pub fn heard_from(sender: Option<&str>, receiver: &str) {
    if let Some(sender) = sender.filter(|s| !s.is_empty()) {
        book().heard_from(sender, receiver);
    }
}

/// The answer to a send that the relay queued: the usual response, plus what srv knows that the
/// sender can act on at once. `target_in_account` is the relay's word on whether the name is one of
/// the sender's own account's agents; `lan_peers` are the LAN peers this machine can see, in words
/// (`peer_words::describe_peer`), because the message got this far only because the target is not
/// running here or on any of them.
pub fn queued_answer(request_id: String, target_in_account: Option<bool>, lan_peers: Vec<String>, timestamp: u64) -> Value {
    let body = InjectionResponse {
        deferred: None,
        success: true,
        request_id,
        block_id: None,
        error: None,
        timestamp,
        // No receiver has seen the message yet, so there is no applied
        // tier or STOP decision to report — deliberately left unset
        // rather than guessed at.
        effective_tier: None,
        requires_stop: None,
        channel_verified: None,
    };
    let mut body = serde_json::to_value(&body).unwrap_or_default();
    if let Some(obj) = body.as_object_mut() {
        // Passed through for the `Message` tool, which warns of a likely typo
        // on `false` (PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md R-3).
        if let Some(in_account) = target_in_account {
            obj.insert("target_in_account".to_string(), Value::Bool(in_account));
        }
        obj.insert("lan_peers".to_string(), json!(lan_peers));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(id: &str, from: &str, to: &str) -> Watched {
        Watched { id: id.into(), from: from.into(), to: to.into(), queued_ms: 1 }
    }

    #[test]
    fn a_reply_from_the_target_stops_the_note_and_nothing_else_does() {
        let mut b = Book::default();
        b.add(w("inj-1", "agent2-5", "lazo"));
        b.add(w("inj-2", "agent2-5", "posa"));
        // Someone else writing to the sender, or the sender writing, says nothing about the target.
        assert_eq!(b.heard_from("manoz", "agent2-5"), 0);
        assert_eq!(b.heard_from("agent2-5", "lazo"), 0);
        // The target writing back, in any case, drops only what was queued for it.
        assert_eq!(b.heard_from("Lazo", "AGENT2-5"), 1);
        assert!(b.take("inj-1").is_none());
        assert_eq!(b.take("inj-2").map(|x| x.to), Some("posa".to_string()));
        assert!(b.take("inj-2").is_none(), "a message is reported once");
    }

    #[test]
    fn the_book_is_bounded_and_forgets_the_oldest() {
        let mut b = Book::default();
        for i in 0..(MAX_WATCHED + 5) {
            b.add(w(&format!("inj-{i}"), "a", "b"));
        }
        assert_eq!(b.entries.len(), MAX_WATCHED);
        assert!(b.take("inj-0").is_none());
        assert!(b.take(&format!("inj-{}", MAX_WATCHED + 4)).is_some());
    }

    #[test]
    fn the_note_names_the_target_and_the_message_and_never_says_lost() {
        let note = unconfirmed_note(&w("inj-w-ab84", "agent2-5", "lazo"));
        assert!(note.starts_with(NOTE_MARKER), "{note}");
        assert!(note.contains("inj-w-ab84") && note.contains("lazo") && note.contains("30 minutes"), "{note}");
        assert!(note.contains("never confirmed delivered"), "{note}");
    }

    #[test]
    fn the_window_is_what_the_send_answer_promises() {
        assert_eq!(RELAY_HOLD_MS, 30 * 60 * 1000);
    }

    #[test]
    fn the_queued_answer_carries_the_hint_fields() {
        let v = queued_answer("inj-w-1".into(), Some(false), vec!["Area54/stable (Microsoft Store, no agents)".into()], 7);
        assert_eq!(v["success"], true);
        assert_eq!(v["request_id"], "inj-w-1");
        assert_eq!(v["target_in_account"], false);
        assert_eq!(v["lan_peers"][0], "Area54/stable (Microsoft Store, no agents)");
        // An older relay that doesn't say leaves the field out; the peers list is always there.
        let v = queued_answer("inj-w-2".into(), None, vec![], 7);
        assert!(v.get("target_in_account").is_none());
        assert_eq!(v["lan_peers"], json!([]));
    }
}

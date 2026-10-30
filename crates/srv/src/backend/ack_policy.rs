// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Replies that say nothing are not sent (`SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md` §6).
//!
//! A message that starts a turn makes its recipient read its whole context again, so a bare
//! "got it" costs far more than it says. Two cases are refused at the sender's srv, before
//! anything is delivered:
//!
//! - a message that answers (`in_reply_to`) one sent with `reply: none`;
//! - a bare acknowledgement ("ok", "thanks", "on it") to an agent that sent the sender a
//!   message in the last [`ACK_WINDOW_MS`].
//!
//! The refusal says why and what to do instead. The setting [`SETTING_REFUSE_BARE_ACKS`] turns
//! the second off. Memory only: a restart forgets, and a forgotten message is delivered as
//! before.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The setting: refuse a bare acknowledgement (default on; `false` turns it off).
pub const SETTING_REFUSE_BARE_ACKS: &str = "message:refuse_bare_acks";

/// A message from the target this recent makes a bare acknowledgement an answer to it.
pub const ACK_WINDOW_MS: i64 = 30 * 60 * 1000;
/// How long a `reply: none` message's id is remembered.
const REMEMBER_MS: i64 = 60 * 60 * 1000;
/// Entries kept in either table before the stale ones are dropped.
const MAX_KEPT: usize = 4096;
/// A bare acknowledgement is at most this many characters once trimmed.
const MAX_ACK_CHARS: usize = 24;

/// What a bare acknowledgement says, lower-case, without punctuation.
const ACK_WORDS: &[&str] = &[
    "ack", "acked", "ok", "okay", "k", "kk", "got it", "gotcha", "thanks", "thank you", "thx", "ty", "noted", "will do",
    "on it", "done", "roger", "understood", "received", "copy", "copy that", "+1", "sounds good", "perfect", "great",
];

/// Whether `text` is nothing but an acknowledgement.
pub fn is_bare_ack(text: &str) -> bool {
    let trimmed = text.trim();
    // A question ("ok?") asks for an answer, so it is not an acknowledgement.
    if trimmed.is_empty() || trimmed.chars().count() > MAX_ACK_CHARS || trimmed.contains('?') {
        return false;
    }
    let kept: String = trimmed
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '+')
        .collect();
    let words = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    // Only symbols or emoji ("👍") is an acknowledgement too.
    words.is_empty() || ACK_WORDS.contains(&words.as_str())
}

#[derive(Default)]
pub struct AckPolicy {
    /// MSGID -> (sender, target, sent at) of messages sent with `reply: none`.
    no_reply: HashMap<String, (String, String, i64)>,
    /// (sender, target) -> when the sender last sent the target a message.
    last_sent: HashMap<(String, String), i64>,
}

fn route(name: &str) -> String {
    crate::backend::reactive::sanitize::route_name(name).unwrap_or_default().to_lowercase()
}

impl AckPolicy {
    /// Remember a message passing through: who sent it to whom, and whether it asked for no reply.
    pub fn record(&mut self, from: &str, to: &str, msg_id: Option<&str>, no_reply: bool, now_ms: i64) {
        let (from, to) = (route(from), route(to));
        if from.is_empty() || to.is_empty() {
            return;
        }
        self.last_sent.insert((from.clone(), to.clone()), now_ms);
        if let (true, Some(id)) = (no_reply, msg_id.filter(|id| !id.is_empty())) {
            self.no_reply.insert(id.to_string(), (from, to, now_ms));
        }
        if self.no_reply.len() > MAX_KEPT || self.last_sent.len() > MAX_KEPT {
            self.no_reply.retain(|_, (_, _, at)| now_ms.saturating_sub(*at) <= REMEMBER_MS);
            self.last_sent.retain(|_, at| now_ms.saturating_sub(*at) <= ACK_WINDOW_MS);
        }
    }

    /// Why `from` may not send `text` to `to`, or `Ok`.
    pub fn check(
        &self,
        from: &str,
        to: &str,
        text: &str,
        in_reply_to: Option<&str>,
        refuse_bare_acks: bool,
        now_ms: i64,
    ) -> Result<(), String> {
        let (from, to) = (route(from), route(to));
        if from.is_empty() || to.is_empty() || from == to {
            return Ok(());
        }
        if let Some((sender, target, at)) = in_reply_to.and_then(|id| self.no_reply.get(id)) {
            if *sender == to && *target == from && now_ms.saturating_sub(*at) <= REMEMBER_MS {
                return Err(format!(
                    "that message asked for no reply. If you have a new fact for {to}, send it as its own message"
                ));
            }
        }
        if refuse_bare_acks && is_bare_ack(text) {
            let recent = self.last_sent.get(&(to.clone(), from)).is_some_and(|at| now_ms.saturating_sub(*at) <= ACK_WINDOW_MS);
            if recent {
                return Err(format!(
                    "a bare acknowledgement only makes {to} read its whole context again. Say something new, or say nothing"
                ));
            }
        }
        Ok(())
    }
}

static POLICY: LazyLock<Mutex<AckPolicy>> = LazyLock::new(Default::default);

/// [`AckPolicy::record`] on the process-wide table.
pub fn record(from: &str, to: &str, msg_id: Option<&str>, no_reply: bool) {
    let mut p = POLICY.lock().unwrap_or_else(|e| e.into_inner());
    p.record(from, to, msg_id, no_reply, agentmux_common::time::now_ms());
}

/// [`AckPolicy::check`] on the process-wide table.
pub fn check(from: &str, to: &str, text: &str, in_reply_to: Option<&str>, refuse_bare_acks: bool) -> Result<(), String> {
    let p = POLICY.lock().unwrap_or_else(|e| e.into_inner());
    p.check(from, to, text, in_reply_to, refuse_bare_acks, agentmux_common::time::now_ms())
}

/// Whether the settings leave bare acknowledgements refused.
pub fn refuses_bare_acks(extra: &HashMap<String, serde_json::Value>) -> bool {
    extra.get(SETTING_REFUSE_BARE_ACKS).and_then(|v| v.as_bool()).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_short_list_of_bare_words_is_an_acknowledgement() {
        for yes in ["ok", "Got it!", "  thanks. ", "👍", "on it", "+1", "Thank you!!", "copy that"] {
            assert!(is_bare_ack(yes), "{yes}");
        }
        for no in [
            "",
            "ok, and the PR is #5850",
            "got it, merging now",
            "thanks for the review of the cache hold spec",
            "done: #5850 merged",
            "ok?",
        ] {
            assert!(!is_bare_ack(no), "{no}");
        }
    }

    const NOW: i64 = 1_000_000_000;

    #[test]
    fn a_reply_to_a_no_reply_message_is_refused_and_a_new_message_is_not() {
        let mut p = AckPolicy::default();
        p.record("AgentX", "agent1", Some("m1"), true, NOW);
        let refused = p.check("agent1", "agentx", "pausing now, nothing in flight", Some("m1"), true, NOW + 1000);
        assert!(refused.unwrap_err().contains("asked for no reply"));
        assert!(p.check("agent1", "agentx", "pausing now", None, true, NOW + 1000).is_ok(), "no in_reply_to, not an ack");
        assert!(p.check("agent1", "agentx", "pausing now", Some("other"), true, NOW + 1000).is_ok());
        assert!(p.check("camper", "agentx", "pausing now", Some("m1"), true, NOW + 1000).is_ok(), "it was not sent to camper");
        let later = NOW + REMEMBER_MS + 1;
        assert!(p.check("agent1", "agentx", "pausing now", Some("m1"), true, later).is_ok(), "forgotten after an hour");
    }

    #[test]
    fn a_bare_acknowledgement_is_refused_only_soon_after_a_message_from_the_target() {
        let mut p = AckPolicy::default();
        assert!(p.check("agent1", "agentx", "got it", None, true, NOW).is_ok(), "agentx sent nothing");
        p.record("agentx", "agent1", Some("m2"), false, NOW);
        assert!(p.check("agent1", "agentx", "got it", None, true, NOW + 60_000).unwrap_err().contains("bare acknowledgement"));
        assert!(p.check("agent1", "agentx", "got it", None, false, NOW + 60_000).is_ok(), "the setting is off");
        assert!(p.check("agent1", "agentx", "got it", None, true, NOW + ACK_WINDOW_MS + 1).is_ok(), "too long ago");
        assert!(p.check("agent1", "agentx", "got it, merged #5850", None, true, NOW + 60_000).is_ok(), "says something");
        assert!(p.check("agent1", "agenty", "got it", None, true, NOW + 60_000).is_ok(), "agenty sent nothing");
    }

    #[test]
    fn an_agent_may_say_anything_to_itself() {
        let p = AckPolicy::default();
        assert!(p.check("agent1", "Agent1", "ok", Some("m1"), true, NOW).is_ok());
    }

    #[test]
    fn the_setting_defaults_to_refusing() {
        let mut extra = HashMap::new();
        assert!(refuses_bare_acks(&extra));
        extra.insert(SETTING_REFUSE_BARE_ACKS.to_string(), serde_json::json!(false));
        assert!(!refuses_bare_acks(&extra));
    }
}

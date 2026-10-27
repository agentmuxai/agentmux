// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The user turns srv writes to a CLI's stdin, by uuid.
//!
//! Launched with `--replay-user-messages`, the CLI echoes each user turn it is
//! given back on stdout as an `isReplay` frame carrying the uuid the turn came
//! with. The pane already records what it sent, so that echo is dropped; but the
//! CLI also marks frames of its own `isReplay` without the flag (a local
//! command's output such as `/compact`'s card), and those must stay. A uuid srv
//! put on a turn it wrote is the only thing that tells the two apart.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Turns remembered; far more than the CLI can be behind by.
const REMEMBERED: usize = 1024;

#[derive(Clone, Default)]
pub struct SentPrompts(Arc<Mutex<VecDeque<String>>>);

impl SentPrompts {
    fn remember(&self, uuid: String) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() >= REMEMBERED {
            g.pop_front();
        }
        g.push_back(uuid);
    }

    /// True, and forgotten, if `uuid` is a turn srv wrote.
    pub fn take(&self, uuid: &str) -> bool {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match g.iter().position(|u| u == uuid) {
            Some(i) => {
                g.remove(i);
                true
            }
            None => false,
        }
    }

    /// `line` as it is written to the CLI: a user turn gets a uuid (its own if it has
    /// one), remembered here. Anything else (a control response, text that is not
    /// JSON) is written as it is.
    pub fn tag(&self, line: &str) -> String {
        let Ok(Value::Object(mut turn)) = serde_json::from_str::<Value>(line) else { return line.to_string() };
        if turn.get("type").and_then(Value::as_str) != Some("user") {
            return line.to_string();
        }
        let uuid = match turn.get("uuid").and_then(Value::as_str).filter(|u| !u.is_empty()) {
            Some(own) => own.to_string(),
            None => {
                let fresh = uuid::Uuid::new_v4().to_string();
                turn.insert("uuid".to_string(), Value::String(fresh.clone()));
                fresh
            }
        };
        self.remember(uuid);
        serde_json::to_string(&Value::Object(turn)).unwrap_or_else(|_| line.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid_of(line: &str) -> String {
        serde_json::from_str::<Value>(line).unwrap()["uuid"].as_str().unwrap().to_string()
    }

    #[test]
    fn a_user_turn_gets_a_uuid_that_is_remembered_once() {
        let sent = SentPrompts::default();
        let tagged = sent.tag(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#);
        let id = uuid_of(&tagged);
        assert!(sent.take(&id));
        assert!(!sent.take(&id), "an echo is dropped once");
        assert!(serde_json::from_str::<Value>(&tagged).unwrap()["message"]["content"] == "hi");
    }

    #[test]
    fn a_turn_that_has_a_uuid_keeps_it() {
        let sent = SentPrompts::default();
        let tagged = sent.tag(r#"{"type":"user","uuid":"own-1","message":{"role":"user","content":"hi"}}"#);
        assert_eq!(uuid_of(&tagged), "own-1");
        assert!(sent.take("own-1"));
    }

    #[test]
    fn control_responses_and_non_json_are_written_as_they_are() {
        let sent = SentPrompts::default();
        for line in [r#"{"type":"control_response","response":{"request_id":"r"}}"#, "not json", r#"[1,2]"#] {
            assert_eq!(sent.tag(line), line);
        }
        assert!(!sent.take(""));
    }

    #[test]
    fn a_uuid_srv_never_wrote_is_not_taken() {
        let sent = SentPrompts::default();
        sent.tag(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#);
        assert!(!sent.take("the-clis-own-frame"));
    }

    #[test]
    fn the_oldest_are_forgotten_past_the_bound() {
        let sent = SentPrompts::default();
        for i in 0..REMEMBERED + 5 {
            sent.tag(&format!(r#"{{"type":"user","uuid":"u{i}","message":{{"role":"user","content":"x"}}}}"#));
        }
        assert!(!sent.take("u0"));
        assert!(sent.take(&format!("u{}", REMEMBERED + 4)));
    }
}

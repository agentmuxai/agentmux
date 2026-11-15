// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one-trip first connect with GitHub, desktop side: the relay half.
//!
//! GitHub's install page takes no `redirect_uri`, so after the person installs the App and
//! authorizes it, GitHub sends the browser to the App's first callback URL: the cloud relay's hosted
//! page `/github-callback`. The page hands `{code, installation_id, setup_action}` to the relay under
//! the SHA-256 of the install `state` the broker issued; this module registers that trip before the
//! browser opens and polls for the outcome. The code goes to the broker, which holds the App's
//! client secret and checks the `state` is one this account was issued; nothing here reads it.
//!
//! The poll key is the hash, never the state itself, so a poll address in a log is no use to anyone.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Where the relay's hosted page is, and where GitHub is told to send the browser.
pub const CALLBACK_PATH: &str = "/github-callback";

/// An address on the relay, whether or not its base ends in a slash.
fn relay_url(relay_base: &str, path: &str) -> String {
    format!("{}{path}", relay_base.trim_end_matches('/'))
}

/// The address the install page's callback lands on.
pub fn callback_uri(relay_base: &str) -> String {
    relay_url(relay_base, CALLBACK_PATH)
}

/// The relay's key for a trip: the SHA-256 hex of the install `state`.
pub fn relay_key(state: &str) -> String {
    hex::encode(Sha256::digest(state.as_bytes()))
}

/// What the relay answered to a poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Polled {
    /// Registered, nothing back yet (or a hiccup worth asking again about).
    Pending,
    /// GitHub's authorization, with the installation it came with.
    Ready { code: String, installation_id: u64, setup_action: String },
    /// GitHub reported a refusal (the person said no, or it failed); the words are GitHub's.
    Failed(String),
    /// The relay has no such trip: it expired or was already collected.
    Gone,
}

/// Reads a poll's answer. Anything the relay should not send is read as "ask again", never as a code.
pub fn parse_poll(status: u16, body: &Value) -> Polled {
    if status == 404 {
        return Polled::Gone;
    }
    if !(200..300).contains(&status) {
        return Polled::Pending;
    }
    match body.get("status").and_then(Value::as_str) {
        Some("ready") => {
            let code = body.get("code").and_then(Value::as_str).filter(|c| !c.is_empty());
            let installation_id = body.get("installation_id").and_then(Value::as_u64).filter(|id| *id > 0);
            let action = body.get("setup_action").and_then(Value::as_str).filter(|a| !a.is_empty());
            match (code, installation_id, action) {
                (Some(code), Some(installation_id), Some(action)) => {
                    Polled::Ready { code: code.to_string(), installation_id, setup_action: action.to_string() }
                }
                _ => Polled::Failed("the relay's answer was incomplete".to_string()),
            }
        }
        Some("failed") => Polled::Failed(body.get("error").and_then(Value::as_str).unwrap_or("unknown").to_string()),
        _ => Polled::Pending,
    }
}

/// Tell the relay a trip is coming, before the browser opens (a flow nobody registered is refused).
pub async fn register(http: &reqwest::Client, relay_base: &str, key: &str) -> Result<(), String> {
    let reply = http
        .post(relay_url(relay_base, "/api/login-relay/github"))
        .json(&serde_json::json!({ "key": key }))
        .send()
        .await
        .map_err(|e| format!("could not reach AgentMux Cloud's relay: {}", e.without_url()))?;
    let status = reply.status();
    if status.is_success() {
        return Ok(());
    }
    Err(format!("AgentMux Cloud's relay refused the connection ({status})"))
}

/// Ask the relay once. A network error or a server error reads as "not yet": the caller asks again.
pub async fn poll_once(http: &reqwest::Client, relay_base: &str, key: &str) -> Polled {
    let Ok(reply) = http.get(relay_url(relay_base, &format!("/api/login-relay/github/{key}"))).send().await else {
        return Polled::Pending;
    };
    let status = reply.status().as_u16();
    let body: Value = reply.json().await.unwrap_or(Value::Null);
    parse_poll(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_callback_is_the_relays_hosted_page() {
        assert_eq!(callback_uri("https://muxbus.example.test"), "https://muxbus.example.test/github-callback");
        assert_eq!(callback_uri("https://muxbus.example.test/"), "https://muxbus.example.test/github-callback");
    }

    #[test]
    fn a_base_with_a_trailing_slash_gives_the_same_addresses() {
        for base in ["https://muxbus.example.test", "https://muxbus.example.test/"] {
            assert_eq!(relay_url(base, "/api/login-relay/github"), "https://muxbus.example.test/api/login-relay/github");
        }
    }

    #[test]
    fn the_key_is_the_sha256_hex_of_the_state_and_never_the_state() {
        assert_eq!(relay_key("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let state = "kJ3-Gh_7xQ0aBcDeFgHiJkLmNoPqRsTuVwXyZ012345";
        assert!(!relay_key(state).contains(state));
        assert_eq!(relay_key(state).len(), 64);
    }

    #[test]
    fn a_broker_that_does_not_say_it_is_ready_for_a_trip_is_not_one() {
        use crate::muxbus::github_broker::InstallState;
        let older: InstallState = serde_json::from_value(json!({ "state": "s", "install_url": "https://github.com/apps/x" })).unwrap();
        assert!(!older.one_trip);
        let ready: InstallState = serde_json::from_value(json!({ "state": "s", "install_url": "https://github.com/apps/x", "one_trip": true })).unwrap();
        assert!(ready.one_trip);
    }

    #[test]
    fn a_ready_answer_needs_the_code_the_installation_and_the_action() {
        let ready = json!({ "status": "ready", "code": "c1", "installation_id": 4242, "setup_action": "install" });
        assert_eq!(parse_poll(200, &ready), Polled::Ready { code: "c1".into(), installation_id: 4242, setup_action: "install".into() });
        for missing in ["code", "installation_id", "setup_action"] {
            let mut cut = ready.clone();
            cut.as_object_mut().unwrap().remove(missing);
            assert!(matches!(parse_poll(200, &cut), Polled::Failed(_)), "{missing}");
        }
        assert!(matches!(parse_poll(200, &json!({ "status": "ready", "code": "", "installation_id": 1, "setup_action": "install" })), Polled::Failed(_)));
        assert!(matches!(parse_poll(200, &json!({ "status": "ready", "code": "c", "installation_id": 0, "setup_action": "install" })), Polled::Failed(_)));
        assert!(matches!(parse_poll(200, &json!({ "status": "ready", "code": "c", "installation_id": "4242", "setup_action": "install" })), Polled::Failed(_)));
    }

    #[test]
    fn pending_failed_and_gone_read_as_what_they_are() {
        assert_eq!(parse_poll(200, &json!({ "status": "pending" })), Polled::Pending);
        assert_eq!(parse_poll(200, &json!({ "status": "failed", "error": "access_denied" })), Polled::Failed("access_denied".into()));
        assert_eq!(parse_poll(200, &json!({ "status": "failed" })), Polled::Failed("unknown".into()));
        assert_eq!(parse_poll(404, &Value::Null), Polled::Gone);
    }

    #[test]
    fn a_server_hiccup_or_an_unknown_answer_is_asked_again_never_taken_for_a_code() {
        assert_eq!(parse_poll(500, &json!({ "status": "ready", "code": "c", "installation_id": 1, "setup_action": "install" })), Polled::Pending);
        assert_eq!(parse_poll(429, &Value::Null), Polled::Pending);
        assert_eq!(parse_poll(200, &Value::Null), Polled::Pending);
        assert_eq!(parse_poll(200, &json!({ "status": "something-new" })), Polled::Pending);
    }
}

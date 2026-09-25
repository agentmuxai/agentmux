// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One live instance per agent across the WAN — the client side
//! (`docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` Phase 5, §4.5).
//!
//! The muxbus relay holds one lease per agent (agentmux-cloud
//! `agent-lease-store.ts`). This instance claims it for every agent it polls
//! jekts for, renews it while the agent stays registered, and releases it
//! when the agent goes away. While another instance holds it, the relay
//! answers this instance's pending pulls with 409 — so it cannot consume the
//! other instance's jekts — and this module reports the agent as held
//! elsewhere, which fences the local holder (the newcomer yields, D1).
//!
//! Fails open everywhere (D2): a relay without the lease routes (404), an
//! unreachable relay or a transient error never stops delivery or spawning.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How often a held lease is renewed. The relay's TTL is 60 s.
pub(crate) const RENEW_EVERY: Duration = Duration::from_secs(20);

/// A relay answering 404 on the lease routes predates them: stop asking for
/// this long.
const UNSUPPORTED_BACKOFF: Duration = Duration::from_secs(600);

/// How long a "held elsewhere" answer stays good enough to refuse a spawn
/// without asking again.
const HELD_ELSEWHERE_FRESH: Duration = Duration::from_secs(90);

/// This AgentMux instance's id at the relay: host + channel. Stable across
/// restarts of the same install, distinct between two installs on one host.
pub(crate) fn instance_id() -> String {
    format!(
        "{}/{}",
        crate::backend::reactive::registry::local_host_label(),
        crate::backend::reactive::registry::local_channel_id()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// This instance holds the agent's lease.
    Held,
    /// Another instance holds it; the text describes where.
    HeldElsewhere(String),
    /// No answer (relay unreachable, too old, an error): fail open.
    Unknown,
}

#[derive(Default)]
struct Entry {
    held: bool,
    last_ok: Option<Instant>,
    elsewhere: Option<(String, Instant)>,
}

static STATE: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);
static UNSUPPORTED_UNTIL: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(Default::default);

fn key(agent_id: &str) -> String {
    agent_id.trim().to_lowercase()
}

/// `Some(where)` when a recent answer said another instance holds `agent`.
/// The early admission check refuses on it without a network round trip.
pub(crate) fn held_elsewhere(agent: &str) -> Option<String> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let (desc, at) = state.get(&key(agent))?.elsewhere.clone()?;
    (at.elapsed() < HELD_ELSEWHERE_FRESH).then_some(desc)
}

/// Describe the relay's `held_by` for a refusal: "computer X, channel Y, vZ".
pub(crate) fn describe_holder(held_by: &serde_json::Value) -> String {
    let field = |k: &str| held_by.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut parts = Vec::new();
    let host = field("host");
    if !host.is_empty() {
        parts.push(format!("computer {host}"));
    }
    let channel = field("channel");
    if !channel.is_empty() {
        parts.push(format!("channel {channel}"));
    }
    let version = field("version");
    if !version.is_empty() {
        parts.push(format!("v{version}"));
    }
    parts.join(", ")
}

fn record(agent_id: &str, outcome: &Outcome) {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let e = state.entry(key(agent_id)).or_default();
    match outcome {
        Outcome::Held => {
            e.held = true;
            e.last_ok = Some(Instant::now());
            e.elsewhere = None;
        }
        Outcome::HeldElsewhere(desc) => {
            e.held = false;
            e.last_ok = None;
            e.elsewhere = Some((desc.clone(), Instant::now()));
        }
        Outcome::Unknown => {}
    }
}

/// Note a 409 `not_holder` the relay returned on a pending pull or an ack.
pub(crate) fn note_not_holder(agent_id: &str, body: &serde_json::Value) -> String {
    let desc = body.get("held_by").map(describe_holder).unwrap_or_default();
    record(agent_id, &Outcome::HeldElsewhere(desc.clone()));
    desc
}

/// Claim or renew `agent_id`'s WAN lease, at most every [`RENEW_EVERY`].
/// `base` is the relay's REST base URL ([`super::relay::rest_base_url`]).
pub(crate) async fn ensure(base: &str, agent_id: &str, token: &str, http: &reqwest::Client) -> Outcome {
    if UNSUPPORTED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t > Instant::now()) {
        return Outcome::Unknown;
    }
    let renew_due = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        match state.get(&key(agent_id)) {
            Some(e) if e.held => match e.last_ok {
                Some(at) if at.elapsed() < RENEW_EVERY => return Outcome::Held,
                _ => true,
            },
            _ => false,
        }
    };
    let outcome = if renew_due {
        match call(base, agent_id, token, http, "/agents/lease/renew").await {
            // Nobody holds it any more (`held_by: null`): claim again.
            Some((409, body)) if body.get("held_by").is_some_and(|h| h.is_null()) => {
                claim_outcome(call(base, agent_id, token, http, "/agents/lease").await)
            }
            other => claim_outcome(other),
        }
    } else {
        claim_outcome(call(base, agent_id, token, http, "/agents/lease").await)
    };
    record(agent_id, &outcome);
    outcome
}

fn claim_outcome(resp: Option<(u16, serde_json::Value)>) -> Outcome {
    match resp {
        Some((200..=299, _)) => Outcome::Held,
        Some((409, body)) => Outcome::HeldElsewhere(body.get("held_by").map(describe_holder).unwrap_or_default()),
        Some((404, _)) | Some((405, _)) => {
            *UNSUPPORTED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + UNSUPPORTED_BACKOFF);
            tracing::info!("wan_lease: relay has no agent-lease routes yet — WAN tier inactive for now");
            Outcome::Unknown
        }
        _ => Outcome::Unknown,
    }
}

/// POST `path` for `agent_id`; `(status, json body)` or `None` on transport error.
async fn call(base: &str, agent_id: &str, token: &str, http: &reqwest::Client, path: &str) -> Option<(u16, serde_json::Value)> {
    let body = serde_json::json!({
        "agent_id": agent_id,
        "instance": instance_id(),
        "host": crate::backend::reactive::registry::local_host_label(),
        "channel": crate::backend::reactive::registry::local_channel_id(),
        "version": env!("CARGO_PKG_VERSION"),
    });
    let resp = http
        .post(format!("{base}{path}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Agent-ID", agent_id)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let json = r.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
            Some((status, json))
        }
        Err(e) => {
            tracing::debug!(agent_id, path, error = %e, "wan_lease: relay unreachable");
            None
        }
    }
}

/// Release `agent_id`'s lease (best effort) and forget it.
pub(crate) async fn release(base: &str, agent_id: &str, token: &str, http: &reqwest::Client) {
    let was_held = STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key(agent_id))
        .is_some_and(|e| e.held);
    if was_held {
        let _ = call(base, agent_id, token, http, "/agents/lease/release").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_holder_is_described_without_account_or_instance() {
        let held_by = serde_json::json!({ "host": "desk", "channel": "stable", "version": "0.58.0", "acquired_at": 1 });
        assert_eq!(describe_holder(&held_by), "computer desk, channel stable, v0.58.0");
        assert_eq!(describe_holder(&serde_json::Value::Null), "");
    }

    #[test]
    fn claim_answers_map_to_outcomes() {
        assert_eq!(claim_outcome(Some((200, serde_json::json!({ "ok": true, "epoch": 1 })))), Outcome::Held);
        assert_eq!(
            claim_outcome(Some((409, serde_json::json!({ "held_by": { "host": "desk" } })))),
            Outcome::HeldElsewhere("computer desk".into())
        );
        assert_eq!(claim_outcome(None), Outcome::Unknown, "unreachable relay: fail open");
        assert_eq!(claim_outcome(Some((500, serde_json::Value::Null))), Outcome::Unknown);
    }

    #[test]
    fn a_not_holder_answer_is_remembered_for_the_early_check_and_cleared_by_a_grant() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        assert!(held_elsewhere(&agent).is_none());
        let desc = note_not_holder(&agent.to_uppercase(), &serde_json::json!({ "held_by": { "host": "desk" } }));
        assert_eq!(desc, "computer desk");
        assert_eq!(held_elsewhere(&agent).as_deref(), Some("computer desk"), "keyed case-insensitively, like the relay");
        record(&agent, &Outcome::Held);
        assert!(held_elsewhere(&agent).is_none());
    }

    #[test]
    fn the_instance_id_is_host_and_channel() {
        let id = instance_id();
        assert_eq!(id.matches('/').count(), 1, "{id}");
        assert!(id.ends_with(&crate::backend::reactive::registry::local_channel_id()));
    }
}

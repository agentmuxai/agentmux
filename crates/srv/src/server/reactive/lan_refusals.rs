// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The key a LAN peer showed for a name whose pinned key is different, kept
//! in memory so the operator's pin screen can offer to accept it. It is not
//! stored: after a restart the next refused message records it again.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The last key refused for one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub key_b64: String,
    pub last_seen_at: i64,
    pub count: u32,
    /// The certificate the peer sent with the key, if any (`lan_continuity`).
    pub cert: Option<agentmux_common::jekt_sign::WanKeyRecord>,
    /// Why it was not vouched for, in a sentence.
    pub why: Option<String>,
}

static REFUSALS: LazyLock<Mutex<HashMap<String, Refusal>>> = LazyLock::new(Default::default);

fn refusals() -> std::sync::MutexGuard<'static, HashMap<String, Refusal>> {
    REFUSALS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A message signed by `observed_key_b64` for `agent` was refused because
/// the name is pinned to another key: log it and remember the key.
pub(crate) fn pin_mismatch(agent: &str, observed_key_b64: &str) {
    pin_mismatch_with(agent, observed_key_b64, None, None);
}

/// [`pin_mismatch`], with the certificate that came with the key and the reason
/// it did not move the pin.
pub(crate) fn pin_mismatch_with(
    agent: &str,
    observed_key_b64: &str,
    cert: Option<agentmux_common::jekt_sign::WanKeyRecord>,
    why: Option<String>,
) {
    tracing::warn!(agent_id = %agent, why = why.as_deref().unwrap_or(""), "LAN pubkey mismatch against pinned key — possible identity spoofing attempt");
    note(agent, observed_key_b64, agentmux_common::time::now_secs(), cert, why);
}

fn note(
    agent: &str,
    observed_key_b64: &str,
    now: i64,
    cert: Option<agentmux_common::jekt_sign::WanKeyRecord>,
    why: Option<String>,
) {
    let mut map = refusals();
    let entry = map
        .entry(agent.to_lowercase())
        .or_insert_with(|| Refusal { key_b64: String::new(), last_seen_at: now, count: 0, cert: None, why: None });
    if entry.key_b64 != observed_key_b64 {
        *entry = Refusal { key_b64: observed_key_b64.to_string(), last_seen_at: now, count: 0, cert: None, why: None };
    }
    entry.count += 1;
    entry.last_seen_at = now;
    entry.cert = cert;
    entry.why = why;
}

/// The last key refused for `agent`, if any since this srv started.
pub(crate) fn get(agent: &str) -> Option<Refusal> {
    refusals().get(&agent.to_lowercase()).cloned()
}

/// Forget it: the pin was moved or cleared.
pub(crate) fn clear(agent: &str) {
    refusals().remove(&agent.to_lowercase());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_refused_key_is_kept_with_a_count() {
        note("Maop-t", "key-a", 10, None, None);
        note("maop-t", "key-a", 20, None, None);
        let r = get("MAOP-T").unwrap();
        assert_eq!((r.key_b64.as_str(), r.last_seen_at, r.count), ("key-a", 20, 2));
        note("maop-t", "key-b", 30, None, None);
        let r = get("maop-t").unwrap();
        assert_eq!((r.key_b64.as_str(), r.count), ("key-b", 1), "a different key starts over");
        clear("maop-t");
        assert!(get("maop-t").is_none());
    }
}

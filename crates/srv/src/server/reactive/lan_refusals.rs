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
    /// The name has no pin: this is a first key nobody vouched for, not a different key for a pinned name.
    pub first_contact: bool,
}

/// First keys held at once; past it the one seen longest ago is dropped, so a peer that claims many names
/// cannot grow this map or the pin screen's list without limit.
const MAX_FIRST_CONTACT: usize = 64;

static REFUSALS: LazyLock<Mutex<HashMap<String, Refusal>>> = LazyLock::new(Default::default);

fn refusals() -> std::sync::MutexGuard<'static, HashMap<String, Refusal>> {
    REFUSALS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A message signed by `observed_key_b64` for `agent` was refused because
/// the name is pinned to another key: log it and remember the key.
#[cfg(test)]
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

/// A name with no pin showed `observed_key_b64` and no trusted install vouched for it: nothing is pinned,
/// and the key is kept for the pin screen to offer to accept.
pub(crate) fn first_contact_with(
    agent: &str,
    observed_key_b64: &str,
    cert: Option<agentmux_common::jekt_sign::WanKeyRecord>,
    why: Option<String>,
) {
    tracing::info!(agent_id = %agent, why = why.as_deref().unwrap_or(""), "LAN first contact: no trusted install vouched for the key, so it is not pinned and its messages stay unproven");
    note(agent, observed_key_b64, agentmux_common::time::now_secs(), cert, why);
    let mut map = refusals();
    if let Some(entry) = map.get_mut(&agent.to_lowercase()) {
        entry.first_contact = true;
    }
    while map.values().filter(|r| r.first_contact).count() > MAX_FIRST_CONTACT {
        let Some(oldest) = map.iter().filter(|(_, r)| r.first_contact).min_by_key(|(_, r)| r.last_seen_at).map(|(name, _)| name.clone()) else { break };
        map.remove(&oldest);
    }
}

/// Every name with a key held for it, for the names that have no pin to list them under.
pub(crate) fn all() -> Vec<(String, Refusal)> {
    refusals().iter().map(|(name, r)| (name.clone(), r.clone())).collect()
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
        .or_insert_with(|| Refusal { key_b64: String::new(), last_seen_at: now, count: 0, cert: None, why: None, first_contact: false });
    if entry.key_b64 != observed_key_b64 {
        *entry = Refusal { key_b64: observed_key_b64.to_string(), last_seen_at: now, count: 0, cert: None, why: None, first_contact: false };
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
    fn first_keys_held_are_capped_and_the_oldest_goes_first() {
        for n in 0..(MAX_FIRST_CONTACT + 10) {
            first_contact_with(&format!("cap-name-{n}"), "key", None, None);
            // Seen at distinct times: the oldest is the one dropped.
            let mut map = refusals();
            map.get_mut(&format!("cap-name-{n}")).unwrap().last_seen_at = n as i64;
        }
        let held: Vec<String> = all().into_iter().filter(|(name, r)| name.starts_with("cap-name-") && r.first_contact).map(|(n, _)| n).collect();
        assert!(held.len() <= MAX_FIRST_CONTACT, "{}", held.len());
        assert!(!held.contains(&"cap-name-0".to_string()), "the oldest was dropped");
        assert!(held.contains(&format!("cap-name-{}", MAX_FIRST_CONTACT + 9)), "the newest is kept");
        // A different key for a pinned name is not counted against the cap.
        pin_mismatch("cap-pinned", "key");
        assert!(get("cap-pinned").is_some_and(|r| !r.first_contact));
    }

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

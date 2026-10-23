//! One delivery per signed jekt (https://docs.agentmux.ai/internals/jekt-protocol/
//! §5): a host, cross-channel or LAN signature that this instance has already
//! delivered fails on a second delivery, the same rule the WAN path applies
//! with `wan_seen_sigs` (`muxbus/wan_verify.rs`). A signature stays fresh for
//! minutes; within that time it proves its sender for one delivery.
//!
//! A message counts as delivered when this instance gave it to a local agent;
//! a request this instance only forwarded is not recorded, so a forward that
//! comes back here is still delivered once. Entries live longer than any
//! signature's freshness window.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::InjectionRequest;

/// Longer than every freshness window it guards (host and channel 300 s,
/// LAN 600 s, either way), so a signature can't outlive its entry.
const WINDOW_SECS: i64 = 1_260;
/// Entries kept at most; the oldest go first past this.
const MAX_ENTRIES: usize = 20_000;

type Key = (String, String, String);

#[derive(Default)]
pub(super) struct Seen {
    entries: HashMap<Key, i64>,
}

impl Seen {
    fn key(req: &InjectionRequest) -> Option<Key> {
        let msgid = req.request_id.as_deref().filter(|m| !m.is_empty())?;
        let source = req.source_agent.as_deref().filter(|s| !s.is_empty())?;
        Some((source.to_ascii_lowercase(), req.target_agent.to_ascii_lowercase(), msgid.to_string()))
    }

    fn signed(req: &InjectionRequest) -> bool {
        [req.sig_verified, req.lan_verified, req.channel_verified].contains(&Some(true))
    }

    /// Turn every verified verdict on `req` into a failure when this exact
    /// signed message (sender, target, message id) was already delivered here.
    pub(super) fn check(&self, req: &mut InjectionRequest, now: i64) -> bool {
        if !Self::signed(req) {
            return false;
        }
        let Some(key) = Self::key(req) else {
            return false;
        };
        let seen = self.entries.get(&key).is_some_and(|until| *until > now);
        if seen {
            for verdict in [&mut req.sig_verified, &mut req.lan_verified, &mut req.channel_verified] {
                if *verdict == Some(true) {
                    *verdict = Some(false);
                }
            }
            tracing::warn!(source = %key.0, target = %key.1, msgid = %key.2, "signed jekt already delivered here; counted as failed");
        }
        seen
    }

    /// Remember a signed message that was just delivered to a local agent.
    pub(super) fn record(&mut self, req: &InjectionRequest, now: i64) {
        if !Self::signed(req) {
            return;
        }
        let Some(key) = Self::key(req) else {
            return;
        };
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.retain(|_, until| *until > now);
            if self.entries.len() >= MAX_ENTRIES {
                if let Some(oldest) = self.entries.iter().min_by_key(|(_, until)| **until).map(|(k, _)| k.clone()) {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key, now + WINDOW_SECS);
    }
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// [`Seen::check`] against this process's memory.
pub(super) fn check(req: &mut InjectionRequest, now: i64) {
    if let Ok(seen) = seen().lock() {
        seen.check(req, now);
    }
}

/// [`Seen::record`] into this process's memory.
pub(super) fn record(req: &InjectionRequest, now: i64) {
    if let Ok(mut seen) = seen().lock() {
        seen.record(req, now);
    }
}

#[cfg(test)]
#[path = "../tests/reactive/replay_tests.rs"]
mod tests;

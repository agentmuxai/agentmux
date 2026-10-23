//! One delivery per signed jekt (https://docs.agentmux.ai/internals/jekt-protocol/
//! §5): a host, cross-channel or LAN signature that this instance has already
//! delivered fails on a second delivery, the same rule the WAN path applies
//! with `wan_seen_sigs` (`muxbus/wan_verify.rs`). A signature stays fresh for
//! minutes; within that time it proves its sender for one delivery.
//!
//! Used by `deliver()` and by the files path (`jekt_files::receive`). The
//! check and the reservation are one step under one lock, so two copies
//! arriving together can't both pass. A message counts once it is delivered
//! to a local agent; when this instance doesn't deliver it (it forwards it,
//! or delivery fails), the reservation is dropped, so a forward that comes
//! back here is still delivered once. Entries live longer than any
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

    /// Check and reserve in one step. A signed message already seen here has
    /// every verified verdict turned into a failure and reserves nothing; a
    /// signed message not seen yet is reserved and its key returned.
    pub(super) fn reserve(&mut self, req: &mut InjectionRequest, now: i64) -> Option<Key> {
        if !Self::signed(req) {
            return None;
        }
        let key = Self::key(req)?;
        if self.entries.get(&key).is_some_and(|until| *until > now) {
            for verdict in [&mut req.sig_verified, &mut req.lan_verified, &mut req.channel_verified] {
                if *verdict == Some(true) {
                    *verdict = Some(false);
                }
            }
            tracing::warn!(source = %key.0, target = %key.1, msgid = %key.2, "signed jekt already delivered here; counted as failed");
            return None;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.retain(|_, until| *until > now);
            if self.entries.len() >= MAX_ENTRIES {
                if let Some(oldest) = self.entries.iter().min_by_key(|(_, until)| **until).map(|(k, _)| k.clone()) {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key.clone(), now + WINDOW_SECS);
        Some(key)
    }

    /// Drop a reservation for a message this instance didn't deliver.
    pub(super) fn release(&mut self, key: &Key) {
        self.entries.remove(key);
    }
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// A reservation in this process's memory, dropped unless
/// [`Reservation::delivered`] is called.
pub(in crate::server) struct Reservation(Option<Key>);

impl Reservation {
    /// The message reached a local agent: keep it counted.
    pub(in crate::server) fn delivered(mut self) {
        self.0 = None;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(key) = self.0.take() {
            if let Ok(mut seen) = seen().lock() {
                seen.release(&key);
            }
        }
    }
}

/// [`Seen::reserve`] against this process's memory.
pub(in crate::server) fn reserve(req: &mut InjectionRequest, now: i64) -> Reservation {
    Reservation(seen().lock().ok().and_then(|mut seen| seen.reserve(req, now)))
}

#[cfg(test)]
#[path = "../tests/reactive/replay_tests.rs"]
mod tests;

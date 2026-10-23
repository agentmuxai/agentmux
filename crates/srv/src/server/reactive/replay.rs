//! One delivery per signed jekt (https://docs.agentmux.ai/internals/jekt-protocol/
//! §5): a host, cross-channel or LAN signature that this instance has already
//! delivered fails on a second delivery, the same rule the WAN path applies
//! with `wan_seen_sigs` (`muxbus/wan_verify.rs`). A signature stays fresh for
//! minutes; within that time it proves its sender for one delivery.
//!
//! Used by `deliver()` and by the files path (`jekt_files::receive`). The
//! check and the reservation are one step under one lock, so two copies
//! arriving together can't both pass. A message counts once it is delivered
//! to a local agent or held here for one, by whichever copy; when no copy is
//! taken (it is forwarded, or delivery fails), the reservation is dropped, so
//! a forward that comes back here is still delivered once. Entries live
//! longer than any signature's freshness window.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::InjectionRequest;

/// Longer than every freshness window it guards (host and channel 300 s,
/// LAN 600 s, either way), so a signature can't outlive its entry.
const WINDOW_SECS: i64 = 1_260;
/// Entries kept at most; the oldest go first past this.
const MAX_ENTRIES: usize = 20_000;

type Key = (String, String, String);

struct Entry {
    until: i64,
    /// A copy of the message was delivered or held here: the entry stays.
    taken: bool,
}

/// What [`Seen::reserve`] hands out for a signed message: its key, and whether
/// this copy made the reservation (the first) or found it (a later copy).
pub(super) struct Claim {
    key: Key,
    first: bool,
}

#[derive(Default)]
pub(super) struct Seen {
    entries: HashMap<Key, Entry>,
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

    /// Carries a signature someone could capture and send again. A caller
    /// proven by its token alone (`muxsh agent send`) carries none.
    fn carries_signature(req: &InjectionRequest) -> bool {
        [&req.jekt_sig, &req.lan_sig, &req.lan_sig_v2, &req.channel_sig, &req.channel_sig_v2]
            .iter()
            .any(|sig| sig.as_deref().is_some_and(|s| !s.is_empty()))
    }

    fn fail_verdicts(req: &mut InjectionRequest) {
        for verdict in [&mut req.sig_verified, &mut req.lan_verified, &mut req.channel_verified] {
            if *verdict == Some(true) {
                *verdict = Some(false);
            }
        }
    }

    /// Check and reserve in one step. A signed message already seen here has
    /// every verified verdict turned into a failure; a signed message not seen
    /// yet is reserved. Either way the claim lets a delivered copy keep the
    /// entry, so the first copy failing later can't free it for a third. A
    /// signed message with no message id or sender can't be counted, so it
    /// fails too (every signer sends both); one proven by its token alone
    /// has nothing to replay and is left as it is.
    pub(super) fn reserve(&mut self, req: &mut InjectionRequest, now: i64) -> Option<Claim> {
        if !Self::signed(req) {
            return None;
        }
        let Some(key) = Self::key(req) else {
            if !Self::carries_signature(req) {
                return None;
            }
            Self::fail_verdicts(req);
            tracing::warn!(target = %req.target_agent, "signed jekt without a message id or sender; counted as failed");
            return None;
        };
        if self.entries.get(&key).is_some_and(|e| e.until > now) {
            Self::fail_verdicts(req);
            tracing::warn!(source = %key.0, target = %key.1, msgid = %key.2, "signed jekt already delivered here; counted as failed");
            return Some(Claim { key, first: false });
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.retain(|_, e| e.until > now);
            if self.entries.len() >= MAX_ENTRIES {
                if let Some(oldest) = self.entries.iter().min_by_key(|(_, e)| e.until).map(|(k, _)| k.clone()) {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(key.clone(), Entry { until: now + WINDOW_SECS, taken: false });
        Some(Claim { key, first: true })
    }

    /// A copy of the message was delivered or held here: keep its entry.
    pub(super) fn taken(&mut self, claim: &Claim) {
        if let Some(entry) = self.entries.get_mut(&claim.key) {
            entry.taken = true;
        }
    }

    /// The copy that made the reservation wasn't taken here: drop the entry,
    /// unless another copy was.
    pub(super) fn release(&mut self, claim: &Claim) {
        if claim.first && self.entries.get(&claim.key).is_some_and(|e| !e.taken) {
            self.entries.remove(&claim.key);
        }
    }
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// A claim in this process's memory, released on drop unless
/// [`Reservation::delivered`] is called.
pub(in crate::server) struct Reservation(Option<Claim>);

impl Reservation {
    /// The message reached a local agent, or is held here for one: keep it
    /// counted.
    pub(in crate::server) fn delivered(&mut self) {
        if let Some(claim) = self.0.take() {
            if let Ok(mut seen) = seen().lock() {
                seen.taken(&claim);
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(claim) = self.0.take() {
            if let Ok(mut seen) = seen().lock() {
                seen.release(&claim);
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

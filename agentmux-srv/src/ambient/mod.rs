// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ambient Model Call (AMC) gateway — the single mandatory path for every
//! non-user-driven ("ambient") model call in the backend: cheap, background
//! LLM calls that augment the UX (e.g. summarizing what an agent is doing
//! for the pane header / Swarm tree) rather than answering a user's own
//! turn. See `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`.
//!
//! Every ambient call is keyed by `(entity_id, purpose)` and carries a
//! caller-supplied generation (monotonic per entity — e.g. a turn counter).
//! The gateway:
//!   - cancels a lower-generation in-flight call for the same key the moment
//!     a higher-generation one is admitted (real subprocess cancellation via
//!     the returned `CancellationToken`, not just a discard-on-arrival check)
//!   - rejects a request whose generation is not strictly greater than the
//!     highest one already admitted for that key (stale-on-arrival — the
//!     caller does no work at all, not even a discarded one)
//!
//! This does NOT track token usage itself — callers record usage through the
//! normal per-call-site accounting path (tagged by purpose), same as any
//! other RPC result. The gateway's job is coalescing/cancellation only.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tokio_util::sync::CancellationToken;

/// Identifies one class of ambient call for a specific entity, e.g.
/// `(block_id, "activity_summary")`. `purpose` is a short stable string —
/// also suitable for tagging cost/usage dashboards by call category.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AmbientCallKey {
    pub entity_id: String,
    pub purpose: &'static str,
}

impl AmbientCallKey {
    pub fn new(entity_id: impl Into<String>, purpose: &'static str) -> Self {
        Self { entity_id: entity_id.into(), purpose }
    }
}

struct InFlight {
    generation: u64,
    cancel: CancellationToken,
}

#[derive(Default)]
struct GatewayState {
    inflight: HashMap<AmbientCallKey, InFlight>,
}

pub struct AmbientGateway {
    state: Mutex<GatewayState>,
}

static GATEWAY: OnceLock<AmbientGateway> = OnceLock::new();

/// The process-wide ambient-call gateway singleton.
pub fn gateway() -> &'static AmbientGateway {
    GATEWAY.get_or_init(|| AmbientGateway { state: Mutex::new(GatewayState::default()) })
}

/// Outcome of admitting a request for a key at a given generation.
pub enum Admission<'a> {
    /// This is the newest request seen for the key — proceed. Holding the
    /// guard for the duration of the call gives access to its cancellation
    /// token; dropping the guard (end of scope, any return path) clears the
    /// in-flight entry unless a newer request has already replaced it.
    Proceed(AmbientCallGuard<'a>),
    /// A request at this generation-or-newer is already in flight, or has
    /// already completed, for this key — do no work at all.
    StaleOnArrival,
}

impl AmbientGateway {
    /// Admit a request for `key` at `generation`. If an older in-flight
    /// request exists for the same key, it is cancelled (its cancellation
    /// token is triggered — the caller holding that token is responsible
    /// for actually killing its subprocess). If `generation` is not
    /// strictly greater than the generation already recorded for this key,
    /// returns `StaleOnArrival` without mutating any state.
    pub fn admit(&self, key: AmbientCallKey, generation: u64) -> Admission<'_> {
        let mut state = self.state.lock().unwrap();
        if let Some(existing) = state.inflight.get(&key) {
            if generation <= existing.generation {
                return Admission::StaleOnArrival;
            }
            existing.cancel.cancel();
        }
        let cancel = CancellationToken::new();
        state.inflight.insert(
            key.clone(),
            InFlight { generation, cancel: cancel.clone() },
        );
        Admission::Proceed(AmbientCallGuard {
            gateway: self,
            key,
            generation,
            cancel,
        })
    }
}

/// Held for the duration of one admitted ambient call.
pub struct AmbientCallGuard<'a> {
    gateway: &'a AmbientGateway,
    key: AmbientCallKey,
    generation: u64,
    cancel: CancellationToken,
}

impl AmbientCallGuard<'_> {
    /// Cancellation token for this call. Race it against the subprocess
    /// (`tokio::select!`) and kill the child if it fires — a newer request
    /// for the same key has superseded this one.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

impl Drop for AmbientCallGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.gateway.state.lock().unwrap();
        // Only clear if we're still the entry of record — a newer request
        // may have already replaced it (and its own guard owns the clear).
        if let Some(entry) = state.inflight.get(&self.key) {
            if entry.generation == self.generation {
                state.inflight.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str) -> AmbientCallKey {
        AmbientCallKey::new(id, "activity_summary")
    }

    #[test]
    fn first_request_is_admitted() {
        let gw = AmbientGateway { state: Mutex::new(GatewayState::default()) };
        match gw.admit(key("block-1"), 1) {
            Admission::Proceed(_) => {}
            Admission::StaleOnArrival => panic!("first request should be admitted"),
        };
    }

    #[test]
    fn older_or_equal_generation_is_stale_on_arrival() {
        let gw = AmbientGateway { state: Mutex::new(GatewayState::default()) };
        let _guard = match gw.admit(key("block-1"), 5) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("gen 5 should be admitted"),
        };
        match gw.admit(key("block-1"), 5) {
            Admission::StaleOnArrival => {}
            Admission::Proceed(_) => panic!("equal generation must be rejected"),
        }
        match gw.admit(key("block-1"), 3) {
            Admission::StaleOnArrival => {}
            Admission::Proceed(_) => panic!("older generation must be rejected"),
        };
    }

    #[test]
    fn newer_generation_cancels_the_older_inflight_request() {
        let gw = AmbientGateway { state: Mutex::new(GatewayState::default()) };
        let guard1 = match gw.admit(key("block-1"), 1) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("gen 1 should be admitted"),
        };
        let cancel1 = guard1.cancellation();
        assert!(!cancel1.is_cancelled());

        let _guard2 = match gw.admit(key("block-1"), 2) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("gen 2 should be admitted"),
        };
        assert!(cancel1.is_cancelled(), "admitting gen 2 must cancel gen 1's token");
    }

    #[test]
    fn different_keys_do_not_interfere() {
        let gw = AmbientGateway { state: Mutex::new(GatewayState::default()) };
        let guard1 = match gw.admit(key("block-1"), 1) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("block-1 gen 1 should be admitted"),
        };
        match gw.admit(key("block-2"), 1) {
            Admission::Proceed(_) => {}
            Admission::StaleOnArrival => panic!("block-2 gen 1 should be admitted independently"),
        }
        assert!(!guard1.cancellation().is_cancelled(), "unrelated key must not cancel block-1");
    }

    #[test]
    fn dropping_the_guard_clears_the_slot_for_a_later_retry_at_the_same_generation() {
        let gw = AmbientGateway { state: Mutex::new(GatewayState::default()) };
        {
            let _guard = match gw.admit(key("block-1"), 1) {
                Admission::Proceed(g) => g,
                Admission::StaleOnArrival => panic!("gen 1 should be admitted"),
            };
        } // guard dropped here — slot cleared
        match gw.admit(key("block-1"), 1) {
            Admission::Proceed(_) => {}
            Admission::StaleOnArrival => {
                panic!("gen 1 should be admittable again once the prior call finished")
            }
        };
    }

    #[test]
    fn stale_guard_dropping_does_not_clear_a_newer_inflight_entry() {
        let gw = AmbientGateway { state: Mutex::new(GatewayState::default()) };
        let guard1 = match gw.admit(key("block-1"), 1) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("gen 1 should be admitted"),
        };
        let _guard2 = match gw.admit(key("block-1"), 2) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("gen 2 should be admitted"),
        };
        drop(guard1); // superseded guard drops — must NOT clear gen 2's slot
        match gw.admit(key("block-1"), 2) {
            Admission::StaleOnArrival => {}
            Admission::Proceed(_) => {
                panic!("gen 2 is still in flight; dropping the stale gen-1 guard must not clear it")
            }
        };
    }
}

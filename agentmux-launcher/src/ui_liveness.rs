// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 Phase 1 — UI-thread liveness
//! telemetry (observe-only).
//!
//! The supervisor's prober sends `Command::ProbeUiThread { nonce }` over the
//! host pipe on a low-rate interval; the host answers with
//! `Command::ReportUiThreadAlive { nonce }` from a posted CEF UI task —
//! i.e. a reply is evidence the UI thread pumped AFTER the probe was sent.
//! Silence is the signal: a wedged UI thread never replies, and there is no
//! host-side timeout.
//!
//! Phase 1 records `last_alive` + logs round-trip latency; Phase 2's armed
//! teardown rule will consume `last_alive()`. Kept as a tiny standalone
//! module (not reducer state): this is transport+thread telemetry about the
//! host process, not domain state the reducer owns.
//!
//! All logic lives on the `UiLiveness` struct (unit-testable in isolation —
//! reagent P1 on the first version: tests sharing the process-global cell
//! interleave under parallel execution); the module-level functions are
//! thin delegates to the single process-global instance.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Default)]
pub struct UiLiveness {
    /// Most recent probe: (nonce, when sent). One outstanding probe at a
    /// time is enough at the Phase-1 rate; an unanswered probe is simply
    /// overwritten by the next tick (the gap shows up as `last_alive`
    /// growing stale, which is the exact signal Phase 2 consumes).
    probe_sent: Option<(u64, Instant)>,
    /// Last time ANY `ReportUiThreadAlive` arrived — proof the UI thread
    /// pumped at that moment, regardless of nonce matching.
    last_alive: Option<Instant>,
}

impl UiLiveness {
    /// Record a probe about to be sent. Returns the previous probe's
    /// `(nonce, sent)` if it was never answered — the caller logs the miss.
    pub fn record_probe_sent(&mut self, nonce: u64) -> Option<(u64, Instant)> {
        let unanswered = self.probe_sent.take();
        self.probe_sent = Some((nonce, Instant::now()));
        unanswered
    }

    /// Retract an outstanding probe whose SEND failed (reagent P1: a
    /// transport failure must never age into a false "UI thread did not
    /// pump" report — the probe was never delivered, so its silence is
    /// transport evidence, not liveness evidence). No-op if a different
    /// probe is outstanding.
    pub fn retract_probe(&mut self, nonce: u64) {
        if matches!(self.probe_sent, Some((n, _)) if n == nonce) {
            self.probe_sent = None;
        }
    }

    /// Record a `ReportUiThreadAlive`. Returns the round-trip latency when
    /// the nonce matches the outstanding probe (`None` for a late/unmatched
    /// reply — still recorded as aliveness, since ANY reply proves the UI
    /// thread pumped after its probe was sent).
    pub fn record_alive(&mut self, nonce: u64) -> Option<std::time::Duration> {
        self.last_alive = Some(Instant::now());
        match self.probe_sent {
            Some((sent_nonce, sent_at)) if sent_nonce == nonce => {
                self.probe_sent = None;
                Some(sent_at.elapsed())
            }
            _ => None,
        }
    }

    /// Phase-2 consumer surface: when did the host's UI thread last prove
    /// itself alive? `None` = never (host hasn't answered a single probe
    /// yet — startup, or standalone mode with no pipe).
    pub fn last_alive(&self) -> Option<Instant> {
        self.last_alive
    }
}

fn cell() -> &'static Mutex<UiLiveness> {
    static S: OnceLock<Mutex<UiLiveness>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(UiLiveness::default()))
}

/// See [`UiLiveness::record_probe_sent`]. Process-global instance.
pub fn record_probe_sent(nonce: u64) -> Option<(u64, Instant)> {
    cell().lock().unwrap().record_probe_sent(nonce)
}

/// See [`UiLiveness::retract_probe`]. Process-global instance.
pub fn retract_probe(nonce: u64) {
    cell().lock().unwrap().retract_probe(nonce)
}

/// See [`UiLiveness::record_alive`]. Process-global instance.
pub fn record_alive(nonce: u64) -> Option<std::time::Duration> {
    cell().lock().unwrap().record_alive(nonce)
}

/// See [`UiLiveness::last_alive`]. Process-global instance.
#[allow(dead_code)] // consumed by Phase 2's armed teardown rule
pub fn last_alive() -> Option<Instant> {
    cell().lock().unwrap().last_alive()
}

#[cfg(test)]
mod tests {
    use super::UiLiveness;

    // Each test owns its OWN UiLiveness instance — no process-global state,
    // no parallel-execution interleaving (reagent P1 on the first version).

    #[test]
    fn matched_reply_yields_latency_and_clears_outstanding() {
        let mut l = UiLiveness::default();
        l.record_probe_sent(1);
        let latency = l.record_alive(1);
        assert!(latency.is_some(), "matched nonce must yield a latency");
        // Second reply to the same nonce: still aliveness, no latency.
        assert!(l.record_alive(1).is_none());
        assert!(l.last_alive().is_some());
    }

    #[test]
    fn unmatched_reply_is_aliveness_without_latency() {
        let mut l = UiLiveness::default();
        l.record_probe_sent(1);
        assert!(l.record_alive(999).is_none(), "wrong nonce must not match");
        assert!(l.last_alive().is_some(), "but it still proves aliveness");
        // The outstanding probe survives an unmatched reply and can still
        // be answered.
        assert!(l.record_alive(1).is_some());
    }

    #[test]
    fn overwriting_an_unanswered_probe_reports_the_miss() {
        let mut l = UiLiveness::default();
        l.record_probe_sent(1);
        let missed = l.record_probe_sent(2);
        assert!(
            matches!(missed, Some((1, _))),
            "the unanswered probe must be surfaced to the caller"
        );
    }

    #[test]
    fn retracted_send_failure_is_not_reported_as_a_miss() {
        // reagent P1: a probe whose send failed must not age into a false
        // "UI thread did not pump" miss on the next tick.
        let mut l = UiLiveness::default();
        l.record_probe_sent(1);
        l.retract_probe(1);
        assert!(
            l.record_probe_sent(2).is_none(),
            "a retracted probe must not be surfaced as unanswered"
        );
        // Retract only touches the matching nonce.
        l.retract_probe(999);
        let missed = l.record_probe_sent(3);
        assert!(
            matches!(missed, Some((2, _))),
            "unrelated retract must not clear a live probe"
        );
    }
}

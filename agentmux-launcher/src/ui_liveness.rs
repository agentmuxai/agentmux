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
//! teardown rule will consume `last_alive()` / `last_probe_sent()`. Kept as
//! a tiny standalone module (not reducer state): this is transport+thread
//! telemetry about the host process, not domain state the reducer owns.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Default)]
struct UiLiveness {
    /// Most recent probe: (nonce, when sent). One outstanding probe at a
    /// time is enough at the Phase-1 rate; an unanswered probe is simply
    /// overwritten by the next tick (the gap shows up as `last_alive`
    /// growing stale, which is the exact signal Phase 2 consumes).
    probe_sent: Option<(u64, Instant)>,
    /// Last time ANY `ReportUiThreadAlive` arrived — proof the UI thread
    /// pumped at that moment, regardless of nonce matching.
    last_alive: Option<Instant>,
}

fn cell() -> &'static Mutex<UiLiveness> {
    static S: OnceLock<Mutex<UiLiveness>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(UiLiveness::default()))
}

/// Record a probe about to be sent. Returns the previous probe's
/// `(nonce, sent)` if it was never answered — the caller logs the miss.
pub fn record_probe_sent(nonce: u64) -> Option<(u64, Instant)> {
    let mut s = cell().lock().unwrap();
    let unanswered = s.probe_sent.take();
    s.probe_sent = Some((nonce, Instant::now()));
    unanswered
}

/// Record a `ReportUiThreadAlive`. Returns the round-trip latency when the
/// nonce matches the outstanding probe (`None` for a late/unmatched reply —
/// still recorded as aliveness, since ANY reply proves the UI thread pumped
/// after its probe was sent).
pub fn record_alive(nonce: u64) -> Option<std::time::Duration> {
    let mut s = cell().lock().unwrap();
    s.last_alive = Some(Instant::now());
    match s.probe_sent {
        Some((sent_nonce, sent_at)) if sent_nonce == nonce => {
            s.probe_sent = None;
            Some(sent_at.elapsed())
        }
        _ => None,
    }
}

/// Phase-2 consumer surface: when did the host's UI thread last prove
/// itself alive? `None` = never (host hasn't answered a single probe yet —
/// startup, or standalone mode with no pipe).
#[allow(dead_code)] // consumed by Phase 2's armed teardown rule
pub fn last_alive() -> Option<Instant> {
    cell().lock().unwrap().last_alive
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: tests share the process-global cell; each uses disjoint nonce
    // ranges so parallel execution can't cross-contaminate matches.

    #[test]
    fn matched_reply_yields_latency_and_clears_outstanding() {
        record_probe_sent(1001);
        let latency = record_alive(1001);
        assert!(latency.is_some(), "matched nonce must yield a latency");
        // Second reply to the same nonce: still aliveness, no latency.
        assert!(record_alive(1001).is_none());
        assert!(last_alive().is_some());
    }

    #[test]
    fn unmatched_reply_is_aliveness_without_latency() {
        record_probe_sent(2001);
        assert!(record_alive(2999).is_none(), "wrong nonce must not match");
        assert!(last_alive().is_some(), "but it still proves aliveness");
    }

    #[test]
    fn overwriting_an_unanswered_probe_reports_the_miss() {
        record_probe_sent(3001);
        let missed = record_probe_sent(3002);
        assert!(
            matches!(missed, Some((3001, _))),
            "the unanswered probe must be surfaced to the caller"
        );
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Background-service audit log — issue #2977 Workstream 4,
//! `docs/specs/SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §6.
//!
//! ## What this is for
//!
//! WS4 requires an "audit log of what the background service did while
//! unattended, surfaced next time a window/panel opens". The point is
//! accountability, not diagnostics: once AgentMux can keep running with no
//! window, the user loses the ability to see what it did, and the design doc's
//! §6 cites real precedents (Zoom 2019, Recall 2024) where a background
//! component acting unobserved was the whole problem. So the contract is
//! specifically **"tell the user afterwards"**, which is why entries are
//! consumed on read rather than merely written to a log file nobody opens.
//!
//! ## Why the host owns it
//!
//! The host is the only process that knows both halves: it decides when the
//! instance enters background mode (its own suppressed-drain gate), and it is
//! what the frontend talks to when a window appears. Putting it in the
//! launcher would need a second IPC hop for no benefit.
//!
//! ## Scope, honestly
//!
//! This records the **lifecycle** of unattended periods — when the instance
//! went unattended and when it was observed again. It does not yet enumerate
//! individual agent turns that ran during the window, because that data lives
//! in `srv` and would need a new host→srv query; the entry carries the period
//! so that enrichment can be added without changing the surfacing contract.
//! Recorded as a deliberate first cut rather than implied to be complete.

use std::collections::VecDeque;

/// One thing the background service did (or had happen to it) while the user
/// had no window open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Milliseconds since the Unix epoch. Stored as a plain number rather
    /// than a formatted string so the frontend can localize it.
    pub at_ms: u64,
    pub kind: AuditKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    /// The last user window closed and the instance kept running instead of
    /// exiting — the moment it became unattended.
    WentUnattended,
    /// A window opened again, ending an unattended period.
    Observed,
}

impl AuditKind {
    /// Stable identifier for the IPC payload. Explicit rather than derived so
    /// renaming a variant can't silently change the wire format the frontend
    /// matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            AuditKind::WentUnattended => "went_unattended",
            AuditKind::Observed => "observed",
        }
    }
}

/// How many entries to retain. An unattended period produces two entries, so
/// this is ~64 unattended stretches — far more than a user will ever be shown
/// at once, and bounded so a long-lived background instance cannot grow this
/// without limit. Oldest are dropped first.
const MAX_ENTRIES: usize = 128;

/// The audit log itself.
///
/// In memory only, by design. Persisting it would turn "what happened while
/// you were away" into a durable record of usage patterns surviving restarts
/// — more data at rest than the feature needs, and §6's whole concern is user
/// trust. A restart is also a natural boundary: the user was present to cause
/// it, so anything before it is no longer "while unattended".
#[derive(Debug, Default)]
pub struct BackgroundAudit {
    entries: VecDeque<AuditEntry>,
    /// True between `WentUnattended` and the next `Observed`. Guards against
    /// recording a second `WentUnattended` for one unattended period — the
    /// reducer can report a zero-window transition more than once (e.g. a
    /// pool window churns) and the user should see one period, not several.
    unattended: bool,
}

impl BackgroundAudit {
    /// Record that the instance just became unattended. No-op if it already
    /// is — see `unattended`.
    pub fn went_unattended(&mut self, at_ms: u64) {
        if self.unattended {
            return;
        }
        self.unattended = true;
        self.push(AuditEntry { at_ms, kind: AuditKind::WentUnattended });
    }

    /// Record that a window opened. No-op if the instance was not unattended,
    /// so ordinary window-opening during normal use records nothing.
    pub fn observed(&mut self, at_ms: u64) {
        if !self.unattended {
            return;
        }
        self.unattended = false;
        self.push(AuditEntry { at_ms, kind: AuditKind::Observed });
    }

    fn push(&mut self, entry: AuditEntry) {
        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Take everything recorded so far, clearing it.
    ///
    /// Consuming (rather than peeking) is the surfacing contract: WS4 asks for
    /// the log to be *shown* next time a window opens, so once the frontend
    /// has it, it has been surfaced. A peek would either re-show the same
    /// events on every window open or need separate read-state bookkeeping.
    pub fn take(&mut self) -> Vec<AuditEntry> {
        self.entries.drain(..).collect()
    }

    /// Whether the instance is currently in an unattended period. Exposed for
    /// the surfacing path, which wants to tell the user "you were away" only
    /// when there is something to say.
    pub fn is_unattended(&self) -> bool {
        self.unattended
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Wall-clock milliseconds since the Unix epoch, or 0 if the clock is before
/// the epoch (which would mean a badly misconfigured machine; a zero
/// timestamp is a visible oddity rather than a panic).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// IPC handler: take everything the background service recorded while
/// unattended, as JSON for the frontend to display.
///
/// This is WS4's "surfaced next time a window/panel opens" half. The frontend
/// calls it during init; an empty `entries` array means there is nothing to
/// tell the user, which is the overwhelmingly common case (background-service
/// mode off, or no unattended period since the last time it was shown).
///
/// Shape is stable and flat on purpose — `{ entries: [{ at_ms, kind }], … }` —
/// so adding richer per-turn detail later (which needs a host→srv query, see
/// the module docs) extends entries rather than reshaping the payload.
pub fn background_audit_take(
    state: &std::sync::Arc<crate::state::AppState>,
) -> Result<serde_json::Value, String> {
    let (entries, unattended) = {
        let mut audit = state.background_audit.lock();
        (audit.take(), audit.is_unattended())
    };
    if !entries.is_empty() {
        tracing::info!(
            target: "wrr",
            count = entries.len(),
            "[audit] surfacing background-service activity to a newly opened window"
        );
    }
    Ok(serde_json::json!({
        "entries": entries
            .iter()
            .map(|e| serde_json::json!({ "at_ms": e.at_ms, "kind": e.kind.as_str() }))
            .collect::<Vec<_>>(),
        // True when the instance is STILL unattended as far as the audit log
        // knows — i.e. this window's own `observed` transition has not landed
        // yet. Lets the frontend distinguish "here is what you missed" from
        // "you are looking at it now".
        "unattended": unattended,
    }))
}

#[cfg(test)]
mod background_audit_tests {
    use super::*;

    #[test]
    fn records_an_unattended_period_as_a_pair() {
        let mut a = BackgroundAudit::default();
        a.went_unattended(1_000);
        a.observed(5_000);
        let entries = a.take();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, AuditKind::WentUnattended);
        assert_eq!(entries[0].at_ms, 1_000);
        assert_eq!(entries[1].kind, AuditKind::Observed);
        assert_eq!(entries[1].at_ms, 5_000);
    }

    #[test]
    fn a_single_unattended_period_is_recorded_once() {
        // The reducer can report a zero-window state more than once for one
        // period (pool churn, repeated reconciles). The user should see one
        // period, not a burst.
        let mut a = BackgroundAudit::default();
        a.went_unattended(1_000);
        a.went_unattended(1_100);
        a.went_unattended(1_200);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn opening_a_window_during_normal_use_records_nothing() {
        // `observed` fires on every window open; it must only mean something
        // if the instance was actually unattended first.
        let mut a = BackgroundAudit::default();
        a.observed(1_000);
        a.observed(2_000);
        assert!(a.take().is_empty());
    }

    #[test]
    fn periods_alternate_correctly_across_several_cycles() {
        let mut a = BackgroundAudit::default();
        for i in 0..3u64 {
            a.went_unattended(i * 100);
            assert!(a.is_unattended());
            a.observed(i * 100 + 50);
            assert!(!a.is_unattended());
        }
        assert_eq!(a.take().len(), 6, "three complete periods");
    }

    #[test]
    fn taking_clears_so_the_same_events_are_not_surfaced_twice() {
        let mut a = BackgroundAudit::default();
        a.went_unattended(1);
        assert_eq!(a.take().len(), 1);
        assert!(
            a.take().is_empty(),
            "a second window opening must not re-show what was already shown"
        );
    }

    #[test]
    fn take_does_not_end_the_unattended_period() {
        // Surfacing is about the ENTRIES, not the state. A window opening
        // calls `observed` separately; if `take` also cleared the flag, the
        // matching `Observed` entry would never be recorded.
        let mut a = BackgroundAudit::default();
        a.went_unattended(1);
        let _ = a.take();
        assert!(a.is_unattended());
        a.observed(2);
        assert_eq!(a.take()[0].kind, AuditKind::Observed);
    }

    #[test]
    fn the_log_is_bounded_and_drops_oldest_first() {
        let mut a = BackgroundAudit::default();
        // Two entries per cycle, so this overshoots MAX_ENTRIES.
        for i in 0..(MAX_ENTRIES as u64) {
            a.went_unattended(i * 10);
            a.observed(i * 10 + 1);
        }
        assert_eq!(a.len(), MAX_ENTRIES, "a long-lived instance must not grow this forever");
        let entries = a.take();
        assert!(
            entries[0].at_ms > 0,
            "the earliest entries should have been dropped, not the latest"
        );
    }

    #[test]
    fn wire_identifiers_are_stable() {
        // The frontend matches on these strings; a variant rename must not
        // silently change them.
        assert_eq!(AuditKind::WentUnattended.as_str(), "went_unattended");
        assert_eq!(AuditKind::Observed.as_str(), "observed");
    }
}

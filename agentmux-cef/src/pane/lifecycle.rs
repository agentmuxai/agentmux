// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure lifecycle state machine for browser panes.
//!
//! No CEF, no Win32, no `AppState` — just a `HashMap<block_id, PaneEntry>`
//! guarded by a mutex, and the operations that mutate it. `BrowserPaneManager`
//! composes this with the CEF-side ops to form the full pane lifecycle.
//!
//! Every method is small, deterministic, and fully unit-testable. Tests live
//! at the bottom of this file.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

/// Monotonic counter appended to every pane label so a close-then-recreate of
/// the same block_id doesn't collide: if the old browser's `on_before_close`
/// fires after the new pane's `create()` has already run, `drain_by_label`
/// would otherwise find and wipe the NEW entry.
static PANE_LABEL_SEQ: AtomicU64 = AtomicU64::new(1);

/// Per-pane lifecycle phase. Simplified from the full state machine in
/// `SPEC_BROWSER_PANE_LIFECYCLE.md` §6 — pre-create states are represented
/// by map absence so we only distinguish Live from Closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneLifecycle {
    /// Browser requested; CEF may still be creating it. Ops proceed because
    /// the Browser will be present in `state.browsers` by the time the IPC
    /// reaches it (or the lookup miss is harmless).
    Live,
    /// `close()` has been called. All further IPC for this pane must no-op.
    /// The entry stays until `on_before_close` drains it so concurrent
    /// `defocus_all` / `resize` see the Closing flag and skip, rather than
    /// seeing a stale browser ref.
    Closing,
}

struct PaneEntry {
    label: String,
    state: PaneLifecycle,
}

/// Outcome of `try_register_live`. Distinguishes "first create" (caller
/// should post a CreatePaneTask) from "already exists" (caller should
/// re-navigate) from "closing" (caller should reject the re-create).
#[derive(Debug)]
pub enum RegisterResult {
    /// Entry did not exist; a new Live entry was inserted under `label`.
    Fresh(String),
    /// Entry already existed and is Live; reuse `label` to re-navigate.
    AlreadyLive(String),
    /// Entry exists and is Closing; caller should reject the re-create
    /// because the old Browser's teardown callback would otherwise drain
    /// the NEW entry when it fires.
    Closing,
}

pub struct PaneStateMachine {
    panes: Mutex<HashMap<String, PaneEntry>>,
}

impl PaneStateMachine {
    pub fn new() -> Self {
        Self { panes: Mutex::new(HashMap::new()) }
    }

    /// Attempt to register `block_id` as a new Live pane. If an entry
    /// already exists, returns either `AlreadyLive(label)` (caller should
    /// navigate the existing browser) or `Closing` (caller should error).
    /// Otherwise generates a unique label via `PANE_LABEL_SEQ` and inserts.
    pub fn try_register_live(&self, block_id: &str) -> RegisterResult {
        let mut panes = self.panes.lock();
        if let Some(entry) = panes.get(block_id) {
            return match entry.state {
                PaneLifecycle::Live => RegisterResult::AlreadyLive(entry.label.clone()),
                PaneLifecycle::Closing => RegisterResult::Closing,
            };
        }
        let seq = PANE_LABEL_SEQ.fetch_add(1, Ordering::Relaxed);
        let label = format!("browser-pane-{}-{}", block_id, seq);
        panes.insert(
            block_id.to_string(),
            PaneEntry { label: label.clone(), state: PaneLifecycle::Live },
        );
        RegisterResult::Fresh(label)
    }

    /// Flip the state of `block_id` to Closing. Returns the entry's label
    /// iff it was Live. Returns `None` if the entry is missing or already
    /// Closing — the caller should no-op in both cases.
    pub fn try_mark_closing(&self, block_id: &str) -> Option<String> {
        let mut panes = self.panes.lock();
        let entry = panes.get_mut(block_id)?;
        if entry.state == PaneLifecycle::Closing {
            return None;
        }
        entry.state = PaneLifecycle::Closing;
        Some(entry.label.clone())
    }

    /// Remove an entry by block_id. Called after `close_with` completes
    /// its side effects so the block_id can be reused.
    pub fn remove(&self, block_id: &str) {
        self.panes.lock().remove(block_id);
    }

    /// Remove an entry by label. Called from CEF's `on_before_close` if
    /// it fires. Idempotent — if the entry was already removed by the
    /// explicit close path, this is a no-op. Returns the drained
    /// `block_id` iff an entry was actually removed; `None` otherwise.
    ///
    /// (Phase H.1.a — was `bool`; now returns `Option<String>` so the
    /// caller has the block_id needed to dispatch `CompletePaneClose` to
    /// the host reducer in parallel writes.)
    pub fn drain_by_label(&self, label: &str) -> Option<String> {
        let mut panes = self.panes.lock();
        let victim = panes
            .iter()
            .find(|(_, e)| e.label == label)
            .map(|(k, _)| k.clone());
        if let Some(block_id) = victim {
            panes.remove(&block_id);
            Some(block_id)
        } else {
            None
        }
    }

    /// Return the label for `block_id` iff the entry is Live. Used to gate
    /// focus/resize/navigate ops against concurrent close.
    pub fn live_label_of(&self, block_id: &str) -> Option<String> {
        let panes = self.panes.lock();
        let entry = panes.get(block_id)?;
        if entry.state == PaneLifecycle::Live {
            Some(entry.label.clone())
        } else {
            None
        }
    }

    /// Snapshot of labels for all Live panes. Used by `defocus_all` — it
    /// needs the list without holding the panes lock across CEF calls.
    pub fn live_labels(&self) -> Vec<String> {
        self.panes
            .lock()
            .values()
            .filter(|e| e.state == PaneLifecycle::Live)
            .map(|e| e.label.clone())
            .collect()
    }

    // ── test helpers ────────────────────────────────────────────────────
    #[cfg(test)]
    pub(crate) fn test_has_entry(&self, block_id: &str) -> bool {
        self.panes.lock().contains_key(block_id)
    }

    #[cfg(test)]
    pub(crate) fn test_entry_state(&self, block_id: &str) -> Option<PaneLifecycle> {
        self.panes.lock().get(block_id).map(|e| e.state)
    }

    #[cfg(test)]
    pub(crate) fn test_entry_label(&self, block_id: &str) -> Option<String> {
        self.panes.lock().get(block_id).map(|e| e.label.clone())
    }

    #[cfg(test)]
    pub(crate) fn test_insert_live(&self, block_id: &str, label: &str) {
        self.panes.lock().insert(
            block_id.to_string(),
            PaneEntry { label: label.to_string(), state: PaneLifecycle::Live },
        );
    }

    #[cfg(test)]
    pub(crate) fn test_mark_closing(&self, block_id: &str) {
        if let Some(entry) = self.panes.lock().get_mut(block_id) {
            entry.state = PaneLifecycle::Closing;
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let m = PaneStateMachine::new();
        assert!(!m.test_has_entry("any"));
        assert!(m.live_labels().is_empty());
    }

    #[test]
    fn try_register_live_fresh_creates_entry_with_sequential_label() {
        let m = PaneStateMachine::new();
        let label = match m.try_register_live("b1") {
            RegisterResult::Fresh(l) => l,
            other => panic!("expected Fresh, got {:?}", other),
        };
        assert!(label.starts_with("browser-pane-b1-"));
        assert_eq!(m.test_entry_state("b1"), Some(PaneLifecycle::Live));
        assert_eq!(m.test_entry_label("b1").as_deref(), Some(label.as_str()));
    }

    #[test]
    fn try_register_live_returns_already_live_for_duplicate() {
        let m = PaneStateMachine::new();
        let first_label = match m.try_register_live("b1") {
            RegisterResult::Fresh(l) => l,
            other => panic!("unexpected {:?}", other),
        };
        let second = m.try_register_live("b1");
        match second {
            RegisterResult::AlreadyLive(l) => assert_eq!(l, first_label),
            other => panic!("expected AlreadyLive, got {:?}", other),
        }
    }

    #[test]
    fn try_register_live_returns_closing_for_closing_entry() {
        let m = PaneStateMachine::new();
        m.try_register_live("b1");
        m.try_mark_closing("b1");
        match m.try_register_live("b1") {
            RegisterResult::Closing => {}
            other => panic!("expected Closing, got {:?}", other),
        }
    }

    #[test]
    fn try_mark_closing_flips_and_returns_label() {
        let m = PaneStateMachine::new();
        let expected_label = match m.try_register_live("b1") {
            RegisterResult::Fresh(l) => l,
            _ => unreachable!(),
        };
        let label = m.try_mark_closing("b1").expect("expected Some");
        assert_eq!(label, expected_label);
        assert_eq!(m.test_entry_state("b1"), Some(PaneLifecycle::Closing));
    }

    #[test]
    fn try_mark_closing_returns_none_for_already_closing() {
        let m = PaneStateMachine::new();
        m.try_register_live("b1");
        assert!(m.try_mark_closing("b1").is_some());
        assert!(m.try_mark_closing("b1").is_none(),
            "second mark_closing must return None to prevent double-close ops");
    }

    #[test]
    fn try_mark_closing_returns_none_for_missing_entry() {
        let m = PaneStateMachine::new();
        assert!(m.try_mark_closing("never-registered").is_none());
    }

    #[test]
    fn remove_clears_entry() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        m.remove("b1");
        assert!(!m.test_has_entry("b1"));
    }

    #[test]
    fn remove_missing_is_noop() {
        let m = PaneStateMachine::new();
        m.remove("nonexistent"); // must not panic
    }

    #[test]
    fn drain_by_label_removes_matching_entry() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        assert_eq!(m.drain_by_label("browser-pane-b1-1"), Some("b1".to_string()));
        assert!(!m.test_has_entry("b1"));
    }

    #[test]
    fn drain_by_label_returns_none_on_miss() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        assert_eq!(m.drain_by_label("browser-pane-other-99"), None);
        assert!(m.test_has_entry("b1"));
    }

    #[test]
    fn drain_by_label_idempotent() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        assert_eq!(m.drain_by_label("browser-pane-b1-1"), Some("b1".to_string()));
        assert_eq!(m.drain_by_label("browser-pane-b1-1"), None);
    }

    #[test]
    fn drain_after_mark_closing_still_works() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        m.test_mark_closing("b1");
        assert_eq!(m.drain_by_label("browser-pane-b1-1"), Some("b1".to_string()));
        assert!(!m.test_has_entry("b1"));
    }

    #[test]
    fn live_label_of_returns_only_live() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        assert_eq!(m.live_label_of("b1").as_deref(), Some("browser-pane-b1-1"));

        m.test_mark_closing("b1");
        assert_eq!(m.live_label_of("b1"), None,
            "live_label_of must not return a label for a Closing entry");
    }

    #[test]
    fn live_label_of_returns_none_for_missing() {
        let m = PaneStateMachine::new();
        assert_eq!(m.live_label_of("never"), None);
    }

    #[test]
    fn live_labels_skips_closing_panes() {
        let m = PaneStateMachine::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        m.test_insert_live("b2", "browser-pane-b2-2");
        m.test_insert_live("b3", "browser-pane-b3-3");
        m.test_mark_closing("b2");

        let mut labels = m.live_labels();
        labels.sort();
        assert_eq!(labels, vec!["browser-pane-b1-1", "browser-pane-b3-3"]);
    }

    #[test]
    fn label_sequence_is_monotonic_across_registers() {
        let m = PaneStateMachine::new();
        let a_label = match m.try_register_live("a") {
            RegisterResult::Fresh(l) => l, _ => unreachable!(),
        };
        let b_label = match m.try_register_live("b") {
            RegisterResult::Fresh(l) => l, _ => unreachable!(),
        };
        let a_seq: u64 = a_label.rsplit('-').next().unwrap().parse().unwrap();
        let b_seq: u64 = b_label.rsplit('-').next().unwrap().parse().unwrap();
        assert!(b_seq > a_seq, "PANE_LABEL_SEQ must advance between registers: {} vs {}", a_seq, b_seq);
    }

    #[test]
    fn close_recreate_cycle_label_differs() {
        // Simulates: register b1 → mark closing → drain → register b1 again.
        // The second label must NOT match the first — that's the invariant
        // that prevents on_before_close for the old browser from eating the
        // new entry.
        let m = PaneStateMachine::new();
        let first = match m.try_register_live("b1") {
            RegisterResult::Fresh(l) => l, _ => unreachable!(),
        };
        m.try_mark_closing("b1");
        m.drain_by_label(&first);

        let second = match m.try_register_live("b1") {
            RegisterResult::Fresh(l) => l, _ => unreachable!(),
        };
        assert_ne!(first, second);
    }
}

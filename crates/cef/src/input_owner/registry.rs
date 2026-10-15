// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every window's input owner, and the one way to step one: under a lock, with
//! the time, logging each change as one `[input]` line. The platform adapter
//! runs the effects it returns. docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §2.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use parking_lot::Mutex;

use super::machine::{Effect, Event, Have, Surface, WindowInput};

/// Each window's machine, by window label (`main`, `floating-<uuid>`, …).
static OWNERS: LazyLock<Mutex<HashMap<String, WindowInput>>> = LazyLock::new(Default::default);

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Milliseconds on a monotonic clock, for the machine's timing.
pub(super) fn monotonic_ms() -> u64 {
    EPOCH.elapsed().as_millis() as u64
}

/// Step `window`'s machine with `event` and return the effects to run. Logs a
/// line when the state changes or there is something to do.
pub fn step(window: &str, event: Event) -> Vec<Effect> {
    let now = monotonic_ms();
    let (before, after, effects) = {
        let mut owners = OWNERS.lock();
        let input = owners.entry(window.to_string()).or_default();
        let before = input.describe();
        let effects = input.step(event.clone(), now);
        (before, input.describe(), effects)
    };
    if before != after || !effects.is_empty() {
        let fx: Vec<String> = effects.iter().map(ToString::to_string).collect();
        tracing::info!("[input] win={window} {event}: {before} -> {after} fx=[{}]", fx.join(", "));
    }
    effects
}

/// Whether `window`'s machine counts the window as active.
pub fn is_active(window: &str) -> bool {
    OWNERS.lock().get(window).is_some_and(|w| w.active)
}

/// `window`'s machine as the diagnostics need it: whether it is active, where
/// it last saw the keyboard, and its one-line description.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn snapshot(window: &str) -> Option<(bool, Have, String)> {
    OWNERS.lock().get(window).map(|w| (w.active, w.have.clone(), w.describe()))
}

/// The last selection each window's frontend made that was applied: the page
/// load it came from (`epoch`) and its number in that load.
static SELECTIONS: LazyLock<Mutex<HashMap<String, (String, u64)>>> = LazyLock::new(Default::default);

/// Whether selection `seq` from page load `epoch` of `window` comes after the
/// last one applied; records it if so. Requests travel as separate IPCs and can
/// arrive out of order; an older one must not undo a newer one. A new page load
/// (a reload) starts its own numbering.
pub fn accept_selection(window: &str, epoch: &str, seq: u64) -> bool {
    let mut selections = SELECTIONS.lock();
    match selections.get(window) {
        Some((e, last)) if e == epoch && seq <= *last => false,
        _ => {
            selections.insert(window.to_string(), (epoch.to_string(), seq));
            true
        }
    }
}

/// What a window's frontend is told about its input: whether the window is
/// active, and where its machine last saw the keyboard (`None`: not yet known).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub active: bool,
    pub keys: Option<Surface>,
}

impl Report {
    /// The `input-owner-changed` payload: `{ active, keys: "app" | "pane" |
    /// "elsewhere" | null, block_id }`.
    pub fn to_json(&self) -> serde_json::Value {
        let (keys, block) = match &self.keys {
            Some(Surface::App) => (Some("app"), None),
            Some(Surface::Pane(b)) => (Some("pane"), Some(b.as_str())),
            Some(Surface::Elsewhere) => (Some("elsewhere"), None),
            None => (None, None),
        };
        serde_json::json!({ "active": self.active, "keys": keys, "block_id": block })
    }
}

/// The report each window's frontend was last sent.
static REPORTED: LazyLock<Mutex<HashMap<String, Report>>> = LazyLock::new(Default::default);

/// `window`'s report now. While a request is pending, the keyboard is where it
/// was last seen.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn report(window: &str) -> Option<Report> {
    let (active, seen) = {
        let owners = OWNERS.lock();
        let w = owners.get(window)?;
        let seen = match &w.have {
            Have::Owned(s) => Some(s.clone()),
            _ => None,
        };
        (w.active, seen)
    };
    let last = REPORTED.lock().get(window).and_then(|r| r.keys.clone());
    Some(Report { active, keys: seen.or(last) })
}

/// `window`'s report if it differs from the one last sent; records it as sent.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn report_if_changed(window: &str) -> Option<Report> {
    let now = report(window)?;
    let mut reported = REPORTED.lock();
    if reported.get(window) == Some(&now) {
        return None;
    }
    reported.insert(window.to_string(), now.clone());
    Some(now)
}

/// The window whose machine knows pane `block`.
pub fn window_of_pane(block: &str) -> Option<String> {
    OWNERS.lock().iter().find(|(_, w)| w.panes.contains_key(block)).map(|(k, _)| k.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_owner::machine::Owner;

    // Each test uses its own window label: the registry is process-wide.

    #[test]
    fn a_selection_older_than_one_applied_is_dropped() {
        assert!(accept_selection("t-order", "load-1", 1));
        assert!(accept_selection("t-order", "load-1", 3));
        assert!(!accept_selection("t-order", "load-1", 2), "arrived after #3");
        assert!(!accept_selection("t-order", "load-1", 3), "the same one twice");
        assert!(accept_selection("t-order", "load-1", 4));
    }

    #[test]
    fn a_reloaded_page_starts_its_own_numbering() {
        assert!(accept_selection("t-reload", "load-1", 40));
        assert!(accept_selection("t-reload", "load-2", 1));
        assert!(!accept_selection("t-reload", "load-2", 1));
    }

    #[test]
    fn windows_are_numbered_apart() {
        assert!(accept_selection("t-win-a", "e", 5));
        assert!(accept_selection("t-win-b", "e", 1));
    }

    #[test]
    fn the_report_changes_with_activation_and_keeps_the_last_seen_keys_while_pending() {
        let w = "t-report";
        step(w, Event::Activated);
        let first = report_if_changed(w).expect("first report");
        assert!(first.active);
        assert_eq!(first.keys, None, "asked for, not yet seen");
        step(w, Event::Verify { seq: 1, surface: Surface::App });
        assert_eq!(report_if_changed(w).map(|r| r.keys), Some(Some(Surface::App)));
        assert_eq!(report_if_changed(w), None, "nothing changed");
        // A new request is pending: the keys are still where they were seen.
        step(w, Event::PaneLive("p".into()));
        step(w, Event::Select(Owner::Pane("p".into())));
        assert_eq!(report(w).and_then(|r| r.keys), Some(Surface::App));
        step(w, Event::Deactivated);
        let r = report_if_changed(w).expect("deactivation is reported");
        assert!(!r.active);
    }

    #[test]
    fn the_payload_names_the_pane() {
        let r = Report { active: true, keys: Some(Surface::Pane("abc".into())) };
        assert_eq!(r.to_json(), serde_json::json!({ "active": true, "keys": "pane", "block_id": "abc" }));
        let r = Report { active: false, keys: None };
        assert_eq!(r.to_json(), serde_json::json!({ "active": false, "keys": null, "block_id": null }));
    }
}

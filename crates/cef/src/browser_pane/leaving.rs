// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A pane that is being closed doesn't ask its page "Leave site?".
//!
//! Closing a pane tears its window down, and CEF's own close then runs the
//! page's `beforeunload`: for a pane no agent drives (`client/jsdialog.rs`)
//! that was Chrome's native "Leave / Cancel" box, over a pane that is already
//! gone. The decision to close was taken before CEF asked: by the person, who
//! was asked first when the page would prompt (`pane-close-guard.ts`); by an
//! agent or AgentMux, whose close already gave the person the veto (the Stop
//! countdown); or by AgentMux quitting. So the answer is always Leave, and
//! this records who closed the pane for the log.

use std::collections::HashMap;
use std::sync::OnceLock;

use parking_lot::Mutex;

/// Who closed a pane, as far as the host can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedBy {
    /// The pane's × (or Close), after the window asked the person if the page
    /// would prompt.
    Person,
    /// srv's block-removed event reached the host first: an agent's Stop, or a
    /// tab or workspace closing.
    Removed,
    /// The window closed the pane without the person's ×. Usually the same
    /// removals as `Removed`, when the window saw the block go before the
    /// event arrived; so "not the person" is what this says, not which of them.
    App,
}

impl ClosedBy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ClosedBy::Person => "person",
            ClosedBy::Removed => "removed",
            ClosedBy::App => "app",
        }
    }

    /// From the `closed_by` the window sends with `browser_pane_close`.
    pub(crate) fn from_arg(arg: Option<&str>) -> ClosedBy {
        match arg {
            Some("person") => ClosedBy::Person,
            _ => ClosedBy::App,
        }
    }
}

struct Closing {
    pane: String,
    by: ClosedBy,
}

fn closing() -> &'static Mutex<HashMap<i32, Closing>> {
    static C: OnceLock<Mutex<HashMap<i32, Closing>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// `browser_id`'s pane is being closed. Ends with `forget` when its browser is gone.
pub(crate) fn mark(browser_id: i32, pane: &str, by: ClosedBy) {
    closing().lock().insert(browser_id, Closing { pane: pane.to_string(), by });
}

pub(crate) fn forget(browser_id: i32) {
    closing().lock().remove(&browser_id);
}

/// Who answers a page's "Leave site?" now, as `(pane, closed_by)`: the pane
/// being closed, or any page while AgentMux quits. None for a page that
/// is staying open: its own navigation keeps the dialog it always had.
pub(crate) fn leave_answer(browser_id: i32, quitting: bool) -> Option<(String, &'static str)> {
    decide(closing().lock().get(&browser_id), quitting)
}

fn decide(note: Option<&Closing>, quitting: bool) -> Option<(String, &'static str)> {
    match note {
        Some(c) => Some((c.pane.clone(), c.by.as_str())),
        None if quitting => Some((String::new(), "quit")),
        None => None,
    }
}

/// `scheme://host[:port]` of a page's address, for the log: never its path or query.
pub(crate) fn origin_for_log(url: &str) -> String {
    let Some(origin) = crate::client::app_navigation::url_origin(url) else { return String::new() };
    let Some(scheme_end) = origin.find("://").map(|i| i + 3) else { return String::new() };
    let (scheme, authority) = origin.split_at(scheme_end);
    // Not a sign-in typed into the address.
    format!("{scheme}{}", authority.rsplit('@').next().unwrap_or(authority))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_being_closed_answers_leave_and_says_who_closed_it() {
        let c = Closing { pane: "p1".into(), by: ClosedBy::Person };
        assert_eq!(decide(Some(&c), false), Some(("p1".to_string(), "person")));
        let c = Closing { pane: "p2".into(), by: ClosedBy::Removed };
        assert_eq!(decide(Some(&c), false), Some(("p2".to_string(), "removed")));
    }

    #[test]
    fn quitting_answers_leave_for_any_page() {
        assert_eq!(decide(None, true), Some((String::new(), "quit")));
    }

    #[test]
    fn a_page_that_stays_open_keeps_its_dialog() {
        assert_eq!(decide(None, false), None);
    }

    #[test]
    fn a_marked_browser_is_found_until_it_is_forgotten() {
        mark(900_001, "pane-x", ClosedBy::App);
        assert_eq!(leave_answer(900_001, false), Some(("pane-x".to_string(), "app")));
        forget(900_001);
        assert_eq!(leave_answer(900_001, false), None);
    }

    #[test]
    fn only_the_windows_person_flag_counts_as_a_person() {
        assert_eq!(ClosedBy::from_arg(Some("person")), ClosedBy::Person);
        assert_eq!(ClosedBy::from_arg(Some("anything else")), ClosedBy::App);
        assert_eq!(ClosedBy::from_arg(None), ClosedBy::App);
    }

    #[test]
    fn the_log_gets_an_origin_never_a_path() {
        assert_eq!(origin_for_log("https://portal.azure.com/#view/x?token=1"), "https://portal.azure.com");
        assert_eq!(origin_for_log("https://user:secret@example.com:8443/a"), "https://example.com:8443");
        assert_eq!(origin_for_log("not a url"), "");
    }
}

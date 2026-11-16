// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether an agent's own input is reaching a browser pane
//! (SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.4 item 1).
//!
//! The `Browser` tool clicks and types with `Input.dispatchMouseEvent`,
//! `Input.dispatchKeyEvent` and `Input.insertText`, sent over the DevTools
//! protocol. The page sees an ordinary trusted click or key press, and so does
//! the page's listener, so **neither can tell an agent's from the person's.**
//! The host can: everything an agent does goes through [`CdpSession::call`], which
//! marks the pane for as long as such a call is outstanding ([`begin`]). The
//! person's own clicks and keys never pass through it.
//!
//! The list of saved sign-ins opens, moves and picks only while the pane is
//! [`quiet`]: no agent input in flight and none lately. A click the person makes
//! in the second after an agent's does not open the list; that is the price of
//! a rule that needs no signal from the operating system.
//!
//! [`CdpSession::call`]: crate::browser_api::cdp::CdpSession::call

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// How long after an agent's last input the list stays shut: the page's report
/// of the click reaches the host a moment after the protocol call's own reply.
pub(crate) const GRACE: Duration = Duration::from_millis(1500);

#[derive(Default)]
struct Pane {
    in_flight: u32,
    last_end: Option<Instant>,
}

static PANES: LazyLock<Mutex<HashMap<i32, Pane>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// An agent input call on `browser_id` is outstanding while this lives.
pub(crate) struct Guard(i32);

/// Mark an agent input call on `browser_id` as started; it ends when the guard drops.
pub(crate) fn begin(browser_id: i32) -> Guard {
    PANES.lock().entry(browser_id).or_default().in_flight += 1;
    Guard(browser_id)
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut panes = PANES.lock();
        if let Some(pane) = panes.get_mut(&self.0) {
            pane.in_flight = pane.in_flight.saturating_sub(1);
            pane.last_end = Some(Instant::now());
        }
    }
}

/// No agent input is reaching `browser_id`, and none lately.
pub(crate) fn quiet(browser_id: i32) -> bool {
    quiet_after(browser_id, GRACE)
}

fn quiet_after(browser_id: i32, grace: Duration) -> bool {
    PANES.lock().get(&browser_id).is_none_or(|p| p.in_flight == 0 && p.last_end.is_none_or(|at| at.elapsed() >= grace))
}

/// The pane's browser closed.
pub(crate) fn forget(browser_id: i32) {
    PANES.lock().remove(&browser_id);
}

/// Whether a DevTools method sends input to the page: the calls an agent clicks and types with.
pub(crate) fn is_input(method: &str) -> bool {
    method.starts_with("Input.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_no_agent_has_touched_is_quiet() {
        assert!(quiet_after(910_001, Duration::from_secs(60)));
    }

    #[test]
    fn a_pane_is_not_quiet_while_an_agent_call_is_out_nor_for_the_grace_after() {
        let id = 910_002;
        let guard = begin(id);
        assert!(!quiet_after(id, Duration::ZERO), "in flight");
        drop(guard);
        assert!(!quiet_after(id, Duration::from_secs(60)), "just ended: still inside the grace");
        assert!(quiet_after(id, Duration::ZERO), "past the grace");
    }

    #[test]
    fn two_calls_in_flight_keep_the_pane_shut_until_both_end() {
        let id = 910_003;
        let (a, b) = (begin(id), begin(id));
        drop(a);
        assert!(!quiet_after(id, Duration::ZERO));
        drop(b);
        assert!(quiet_after(id, Duration::ZERO));
    }

    #[test]
    fn another_pane_is_not_held_by_this_one() {
        let _held = begin(910_004);
        assert!(quiet_after(910_005, Duration::from_secs(60)));
    }

    #[test]
    fn only_input_methods_count_as_an_agent_acting() {
        for m in ["Input.dispatchMouseEvent", "Input.dispatchKeyEvent", "Input.insertText"] {
            assert!(is_input(m), "{m}");
        }
        for m in ["Runtime.evaluate", "Page.navigate", "DOM.resolveNode", "Page.setWebLifecycleState"] {
            assert!(!is_input(m), "{m}");
        }
    }

    #[test]
    fn a_closed_browser_is_forgotten() {
        let id = 910_006;
        drop(begin(id));
        forget(id);
        assert!(quiet_after(id, Duration::from_secs(60)));
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! When an agent last drove each browser pane, from srv's own record, so a
//! click in a pane can be told from the person's and the agent's.
//!
//! A pane an agent limited to some sites (`browser_allowlist`) stops its page
//! going anywhere else, which is right for what the agent does there and wrong
//! for the person: an email link clicked in such a pane opens a popup that
//! inherited the limit, and the first redirect hop off the list left a blank
//! pane (the Microsoft sign-in from a Gmail pane, #18229). The agent's own
//! clicks are real input events too, so the page can't say who clicked; srv can:
//! every `Browser` call resolves its pane through `ui_handlers::target_block_id`,
//! which notes it here. Kept in memory, like ownership.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long after an agent's last call on a pane a click there may still be the
/// agent's. A click that opens a window does it within the page's own handler,
/// well inside this; a person's click after the agent has gone quiet is theirs.
pub(crate) const AGENT_CLICK_WINDOW: Duration = Duration::from_secs(10);

/// More panes than anyone has open: past it the oldest notes go.
const MAX_NOTED: usize = 512;

fn calls() -> &'static Mutex<HashMap<String, Instant>> {
    static CALLS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    CALLS.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Instant>> {
    calls().lock().unwrap_or_else(|p| p.into_inner())
}

/// An agent called `Browser` on `pane` (the pane whose banner holds, for a popup window).
pub(crate) fn note(pane: &str) {
    note_at(&mut lock(), pane, Instant::now());
}

fn note_at(map: &mut HashMap<String, Instant>, pane: &str, now: Instant) {
    if map.len() >= MAX_NOTED && !map.contains_key(pane) {
        map.retain(|_, at| now.saturating_duration_since(*at) < AGENT_CLICK_WINDOW);
        if map.len() >= MAX_NOTED {
            return;
        }
    }
    map.insert(pane.to_string(), now);
}

/// `pane` closed: nothing to remember.
pub(crate) fn forget(pane: &str) {
    lock().remove(pane);
}

fn agent_was_there(map: &HashMap<String, Instant>, pane: &str, now: Instant) -> bool {
    map.get(pane)
        .is_some_and(|at| now.saturating_duration_since(*at) < AGENT_CLICK_WINDOW)
}

/// Did the person open the popup `opener`'s page asked for? A click (`user_gesture`)
/// while no agent had called `Browser` on the pane within [`AGENT_CLICK_WINDOW`].
/// When both are possible it counts as the agent's, so the limit stays.
pub(crate) fn person_clicked(opener: &str, user_gesture: bool) -> bool {
    user_gesture && !agent_was_there(&lock(), opener, Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_right_after_the_agents_call_is_the_agents() {
        let mut m = HashMap::new();
        let t = Instant::now();
        note_at(&mut m, "p", t);
        assert!(agent_was_there(&m, "p", t + Duration::from_secs(2)));
        assert!(!agent_was_there(&m, "p", t + AGENT_CLICK_WINDOW));
        assert!(!agent_was_there(&m, "other", t));
    }

    #[test]
    fn a_click_with_no_agent_call_on_the_pane_is_the_persons_only_when_it_is_a_click() {
        assert!(person_clicked("never-driven-pane", true));
        assert!(!person_clicked("never-driven-pane", false));
    }

    #[test]
    fn a_call_makes_the_next_click_the_agents_until_it_goes_quiet() {
        note("driven-pane");
        assert!(!person_clicked("driven-pane", true));
        forget("driven-pane");
        assert!(person_clicked("driven-pane", true));
    }

    #[test]
    fn the_notes_stay_bounded() {
        let mut m = HashMap::new();
        let t = Instant::now();
        for i in 0..MAX_NOTED + 50 {
            note_at(&mut m, &format!("p{i}"), t);
        }
        assert_eq!(m.len(), MAX_NOTED);
        // Once they have gone quiet, new panes fit again.
        note_at(&mut m, "fresh", t + AGENT_CLICK_WINDOW);
        assert!(m.contains_key("fresh"));
        assert!(m.len() <= MAX_NOTED);
    }
}

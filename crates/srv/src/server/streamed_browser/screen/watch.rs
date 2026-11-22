// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The screen's windows, as the person would count them. A window the browser opens by itself (a pop-up) shows on the screen as it
//! would on a person's desktop; what srv adds is what the streamed pane does for the same event (`windows.rs`): a notice, and the
//! keys held for a moment so what is being typed does not go to the new window by mistake.

use std::time::Instant;

use super::super::windows::{NoticeLimiter, KEY_GAP};

/// Did one more window open between a look that saw `before` and one that saw `count`? The browser's first window is the pane itself coming
/// up, not a new window.
pub fn opened_one(before: usize, count: usize) -> bool {
    before > 0 && count > before
}

#[derive(Default)]
pub struct Watch {
    /// How many windows were there at the last look (0 until the browser's first window has come up).
    count: usize,
    held_until: Option<Instant>,
    limiter: NoticeLimiter,
}

impl Watch {
    /// A look at the screen at `now` saw `count` windows. The notice to show when one more window opened.
    pub fn seen(&mut self, count: usize, now: Instant) -> Option<String> {
        let before = std::mem::replace(&mut self.count, count);
        if !opened_one(before, count) {
            return None;
        }
        self.held_until = Some(now + KEY_GAP);
        self.limiter.allow(now).then(|| "A new window opened in this browser. Typing was held for half a second so that it did not go to the new window by mistake.".to_string())
    }

    /// Are keys held at `now`?
    pub fn keys_held(&self, now: Instant) -> bool {
        self.held_until.is_some_and(|t| now < t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_browsers_first_window_is_not_news() {
        let mut w = Watch::default();
        let t = Instant::now();
        assert_eq!(w.seen(0, t), None);
        assert_eq!(w.seen(1, t), None);
        assert!(!w.keys_held(t));
    }

    #[test]
    fn only_a_window_after_the_first_that_adds_to_the_count_is_news() {
        assert!(!opened_one(0, 1), "the pane's own first window");
        assert!(!opened_one(2, 2) && !opened_one(2, 1), "no more windows, or fewer");
        assert!(opened_one(1, 2) && opened_one(2, 4));
    }

    #[test]
    fn a_second_window_is_told_and_holds_the_keys_for_half_a_second() {
        let mut w = Watch::default();
        let t = Instant::now();
        w.seen(1, t);
        let notice = w.seen(2, t).expect("a notice for a new window");
        assert!(notice.contains("new window"), "{notice}");
        assert!(w.keys_held(t) && w.keys_held(t + Duration::from_millis(499)));
        assert!(!w.keys_held(t + KEY_GAP));
    }

    #[test]
    fn a_window_that_closes_says_nothing_and_a_burst_is_one_notice_though_each_holds_keys() {
        let mut w = Watch::default();
        let t = Instant::now();
        w.seen(1, t);
        assert!(w.seen(2, t).is_some());
        assert_eq!(w.seen(1, t + Duration::from_secs(1)), None);
        let later = t + Duration::from_secs(1);
        assert_eq!(w.seen(2, later), None, "throttled: a notice at most every few seconds");
        assert!(w.keys_held(later), "but the keys are held again");
        assert!(w.seen(3, t + Duration::from_secs(10)).is_some(), "after the pause a notice is allowed again");
    }
}

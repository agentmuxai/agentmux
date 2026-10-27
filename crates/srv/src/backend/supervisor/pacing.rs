// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How fast the Supervisor continues, with no I/O: the wait between a turn's
//! end and its continuation, the fleet budget of continuations per hour, and
//! which notice group a stop belongs to. Spec §6.2.

use std::collections::VecDeque;

use super::policy::Action;
use super::settings::NoticeGroup;
use super::Verdict;

const HOUR_MS: u64 = 60 * 60 * 1000;

/// Continuations sent in the last hour, across every pane. In memory: a restart starts a fresh hour.
#[derive(Debug, Default)]
pub struct Budget {
    sent: VecDeque<u64>,
}

impl Budget {
    /// Count a continuation sent at `now_ms` if fewer than `cap` went in the last
    /// hour (`cap` 0 is no cap); else `Err` with the milliseconds until one is free.
    pub fn spend(&mut self, now_ms: u64, cap: u32) -> Result<(), u64> {
        while self.sent.front().is_some_and(|t| now_ms.saturating_sub(*t) >= HOUR_MS) {
            self.sent.pop_front();
        }
        if cap > 0 && self.sent.len() >= cap as usize {
            let oldest = self.sent.front().copied().unwrap_or(now_ms);
            return Err((oldest + HOUR_MS).saturating_sub(now_ms));
        }
        self.sent.push_back(now_ms);
        Ok(())
    }
}

impl Budget {
    /// Take back the continuation counted at `now_ms` when it turned out not to be sent.
    pub fn refund(&mut self, now_ms: u64) {
        if self.sent.back() == Some(&now_ms) {
            self.sent.pop_back();
        }
    }
}

/// The sentence a pane shows when the fleet budget stops its continuation.
pub fn budget_reached(cap: u32, wait_ms: u64) -> String {
    let minutes = wait_ms.div_ceil(60_000).max(1);
    format!("Fleet budget reached: {cap} continuations in the last hour. It can continue again in about {minutes} min.")
}

/// What a waiting continuation checks when its wait is over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaitOver {
    /// A newer turn end or a mode change has taken the pane over.
    pub superseded: bool,
    pub switched_off: bool,
    pub mode_changed: bool,
    /// A turn is running now: the user's message, a jekt, a job, a Schedule injection.
    pub turn_running: bool,
    /// The user's Stop ended the turn.
    pub stopped: bool,
}

impl WaitOver {
    /// Why the continuation is dropped instead of sent, or `None` to send it.
    /// A continuation never lands after a different turn than the one the judge saw.
    pub fn drop_reason(&self) -> Option<&'static str> {
        if self.superseded {
            Some("superseded")
        } else if self.switched_off {
            Some("switched off")
        } else if self.mode_changed {
            Some("mode changed")
        } else if self.turn_running {
            Some("another turn started")
        } else if self.stopped {
            Some("stopped")
        } else {
            None
        }
    }
}

/// The notice group of a stop: a question or "blocked" needs the user; a
/// Continue verdict that a limit turned into a pause hit a limit; an agent
/// that can't run, or a continuation that couldn't be delivered, failed.
pub fn pause_group(action: &Action, verdict: Option<Verdict>, undeliverable: bool) -> NoticeGroup {
    match (action, verdict) {
        _ if undeliverable => NoticeGroup::Failed,
        (Action::Pause(_), Some(Verdict::NeedsUser | Verdict::Blocked)) => NoticeGroup::NeedsYou,
        (Action::Pause(_), _) => NoticeGroup::Limit,
        _ => NoticeGroup::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_stops_at_the_cap_and_frees_a_slot_as_the_hour_rolls() {
        let mut b = Budget::default();
        assert_eq!(b.spend(1_000, 2), Ok(()));
        assert_eq!(b.spend(2_000, 2), Ok(()));
        assert_eq!(b.spend(3_000, 2), Err(HOUR_MS - 2_000), "the first one frees a slot an hour after it");
        assert_eq!(b.spend(1_000 + HOUR_MS, 2), Ok(()), "an hour on, the oldest no longer counts");
    }

    #[test]
    fn a_continuation_that_was_not_sent_gives_its_slot_back() {
        let mut b = Budget::default();
        b.spend(1_000, 1).unwrap();
        assert!(b.spend(2_000, 1).is_err());
        b.refund(1_000);
        assert_eq!(b.spend(3_000, 1), Ok(()));
    }

    #[test]
    fn a_cap_of_zero_is_no_cap() {
        let mut b = Budget::default();
        for t in 0..2_000 {
            assert_eq!(b.spend(t, 0), Ok(()));
        }
    }

    #[test]
    fn lowering_the_cap_applies_to_what_was_already_sent() {
        let mut b = Budget::default();
        for t in 0..5 {
            b.spend(t, 10).unwrap();
        }
        assert!(b.spend(10, 3).is_err());
    }

    #[test]
    fn the_budget_sentence_rounds_up_to_whole_minutes() {
        assert!(budget_reached(30, 61_000).contains("about 2 min"));
        assert!(budget_reached(30, 1).contains("about 1 min"));
    }

    #[test]
    fn a_waiting_continuation_is_sent_only_when_nothing_has_changed() {
        assert_eq!(WaitOver::default().drop_reason(), None);
        let drops = [
            WaitOver { superseded: true, ..Default::default() },
            WaitOver { switched_off: true, ..Default::default() },
            WaitOver { mode_changed: true, ..Default::default() },
            WaitOver { turn_running: true, ..Default::default() },
            WaitOver { stopped: true, ..Default::default() },
        ];
        for d in drops {
            assert!(d.drop_reason().is_some(), "{d:?}");
        }
    }

    #[test]
    fn each_stop_belongs_to_one_notice_group() {
        let pause = Action::Pause("x".into());
        assert_eq!(pause_group(&pause, Some(Verdict::NeedsUser), false), NoticeGroup::NeedsYou);
        assert_eq!(pause_group(&pause, Some(Verdict::Blocked), false), NoticeGroup::NeedsYou);
        assert_eq!(pause_group(&pause, Some(Verdict::Continue), false), NoticeGroup::Limit);
        assert_eq!(pause_group(&pause, Some(Verdict::Continue), true), NoticeGroup::Failed);
        assert_eq!(pause_group(&Action::Done("x".into()), Some(Verdict::Done), false), NoticeGroup::Done);
        assert_eq!(pause_group(&Action::NoVerdict, None, false), NoticeGroup::Done);
    }
}

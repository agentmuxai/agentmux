// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How fast the Supervisor continues, with no I/O: the wait between a turn's
//! end and its continuation, the fleet budget of continuations per hour, and
//! which notice group a stop belongs to. Spec §6.2.

use std::collections::VecDeque;

use super::policy::Action;
use super::settings::NoticeGroup;
use super::{SupervisorState, SupervisorStatus, Verdict};

const HOUR_MS: u64 = 60 * 60 * 1000;

/// Continuations sent in the last hour, across every pane. In memory: a restart starts a fresh hour.
#[derive(Debug, Default)]
pub struct Budget {
    /// (reservation number, when) in the order they were taken.
    sent: VecDeque<(u64, u64)>,
    next: u64,
}

impl Budget {
    /// Count a continuation sent at `now_ms` if fewer than `cap` went in the last
    /// hour (`cap` 0 is no cap) and return its reservation number for [`Budget::refund`];
    /// else `Err` with the milliseconds until one is free.
    pub fn spend(&mut self, now_ms: u64, cap: u32) -> Result<u64, u64> {
        while self.sent.front().is_some_and(|(_, t)| now_ms.saturating_sub(*t) >= HOUR_MS) {
            self.sent.pop_front();
        }
        if cap > 0 && self.sent.len() >= cap as usize {
            let oldest = self.sent.front().map_or(now_ms, |(_, t)| *t);
            return Err((oldest + HOUR_MS).saturating_sub(now_ms));
        }
        self.next += 1;
        self.sent.push_back((self.next, now_ms));
        Ok(self.next)
    }

    /// How many continuations went in the hour before `now_ms`.
    pub fn sent_in_hour(&self, now_ms: u64) -> usize {
        self.sent.iter().filter(|(_, t)| now_ms.saturating_sub(*t) < HOUR_MS).count()
    }
}

impl Budget {
    /// Take back reservation `ticket` when its continuation turned out not to be sent,
    /// whatever was reserved after it.
    pub fn refund(&mut self, ticket: u64) {
        self.sent.retain(|(n, _)| *n != ticket);
    }
}

/// The sentence a pane shows when the fleet budget stops its continuation: what went out in the
/// last hour, the cap, and when there is room again.
pub fn budget_reached(sent: usize, cap: u32, wait_ms: u64) -> String {
    let minutes = wait_ms.div_ceil(60_000).max(1);
    let noun = if sent == 1 { "continuation" } else { "continuations" };
    format!(
        "Fleet budget reached: {sent} {noun} in the last hour (the cap is {cap}). It can continue again in about {minutes} min."
    )
}

/// The start of a pane's reason while its continuation waits ("Continuing after a 30 s wait: ...").
/// It names the wait, not the time left, so it is true for as long as the pane shows it.
pub const WAIT_PREFIX: &str = "Continuing after a ";
/// What v0.60.3 wrote, which a pane saved by it may still carry.
const LEGACY_WAIT_PREFIX: &str = "Continuing in ";

/// What a pane says when AgentMux restarted during a continuation's wait: the wait is not persisted.
pub const RESTARTED: &str = "Not continued: AgentMux restarted.";

/// Clear a "Continuing after a ..." line left by a previous run of srv, whose wait is gone. Whether it changed.
pub fn drop_restarted_wait(state: &mut SupervisorState) -> bool {
    if state.status != SupervisorStatus::Waiting || !(state.reason.starts_with(WAIT_PREFIX) || state.reason.starts_with(LEGACY_WAIT_PREFIX)) {
        return false;
    }
    state.status = SupervisorStatus::Watching;
    state.reason = RESTARTED.to_string();
    true
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
        assert!(b.spend(1_000, 2).is_ok());
        assert!(b.spend(2_000, 2).is_ok());
        assert_eq!(b.spend(3_000, 2), Err(HOUR_MS - 2_000), "the first one frees a slot an hour after it");
        assert!(b.spend(1_000 + HOUR_MS, 2).is_ok(), "an hour on, the oldest no longer counts");
    }

    #[test]
    fn a_continuation_that_was_not_sent_gives_its_slot_back() {
        let mut b = Budget::default();
        let ticket = b.spend(1_000, 1).unwrap();
        assert!(b.spend(2_000, 1).is_err());
        b.refund(ticket);
        assert!(b.spend(3_000, 1).is_ok());
    }

    #[test]
    fn a_refund_takes_back_its_own_reservation_not_the_latest() {
        let mut b = Budget::default();
        let first = b.spend(1_000, 2).unwrap();
        b.spend(2_000, 2).unwrap();
        b.refund(first);
        assert!(b.spend(3_000, 2).is_ok(), "the first one's slot is free again");
        assert!(b.spend(4_000, 2).is_err(), "the second one still counts");
    }

    #[test]
    fn a_cap_of_zero_is_no_cap() {
        let mut b = Budget::default();
        for t in 0..2_000 {
            assert!(b.spend(t, 0).is_ok());
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
        assert!(budget_reached(30, 30, 61_000).contains("about 2 min"));
        assert!(budget_reached(30, 30, 1).contains("about 1 min"));
    }

    #[test]
    fn the_budget_sentence_says_what_went_out_and_the_cap() {
        assert_eq!(
            budget_reached(1, 1, 3_420_000),
            "Fleet budget reached: 1 continuation in the last hour (the cap is 1). It can continue again in about 57 min."
        );
        assert!(budget_reached(3, 1, 60_000).starts_with("Fleet budget reached: 3 continuations in the last hour (the cap is 1)."));
    }

    #[test]
    fn the_budget_counts_only_the_last_hour() {
        let mut b = Budget::default();
        b.spend(0, 0).unwrap();
        b.spend(1_000, 0).unwrap();
        assert_eq!(b.sent_in_hour(2_000), 2);
        assert_eq!(b.sent_in_hour(HOUR_MS + 500), 1);
    }

    #[test]
    fn a_wait_left_by_a_previous_run_is_cleared_and_nothing_else_is() {
        let waiting = |reason: &str, status| SupervisorState {
            status,
            reason: reason.to_string(),
            ..SupervisorState::default()
        };
        let mut s = waiting("Continuing after a 30 s wait: still has work", SupervisorStatus::Waiting);
        assert!(drop_restarted_wait(&mut s));
        assert_eq!((s.status, s.reason.as_str()), (SupervisorStatus::Watching, RESTARTED));
        assert!(!drop_restarted_wait(&mut s), "a second pass changes nothing");
        // "Waiting" is also the status of a pane held for its background work.
        // A wait saved by v0.60.3 reads "Continuing in N s: ...".
        let mut old = waiting("Continuing in 30 s: still has work", SupervisorStatus::Waiting);
        assert!(drop_restarted_wait(&mut old));
        let mut bg = waiting("Waiting for its background work to finish.", SupervisorStatus::Waiting);
        assert!(!drop_restarted_wait(&mut bg));
        assert_eq!(bg.status, SupervisorStatus::Waiting);
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

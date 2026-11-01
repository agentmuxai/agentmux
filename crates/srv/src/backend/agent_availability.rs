// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether an agent can take a message now: the fact ([`Availability`]) and the
//! rule that turns it into a decision for one message ([`verdict`]), in one
//! place (`docs/specs/SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md` §3).
//! The fact comes from what `agent_state` already reads; nothing here reads a
//! controller or a store. The jekt path (`server/jekt_hold.rs`, `jekt_cold.rs`)
//! applies the verdict.

use super::agent_state::{AgentState, Unavailable};

/// What can be said about an agent on this instance for a message to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Defined here, no live process: never started, ended, or its pane closed.
    NotRunning { exited_with_error: bool },
    /// Alive, no turn in flight. `quiet_for_ms`: since its last output, when the controller knows it.
    Idle { quiet_for_ms: Option<u64> },
    /// A turn is in flight (working, or waiting on a question).
    Running { waiting_on_user: bool },
    /// Out of quota; `resets_at_ms` when the provider said when it lifts.
    Limited { resets_at_ms: Option<u64>, since_ms: u64 },
    SignedOut { since_ms: u64 },
    /// Nothing tracks its turns (a terminal pane): the tiers below decide.
    Unknown,
}

/// Why a message waits. The wire strings are the existing `held_reason` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    NotRunning,
    NeedsLogin,
    UsageLimit,
    /// The session has gone cold (`cold_hold`); named `idle` on the wire.
    Cold,
    /// Quota mode is tight and no turn is in flight (`quota_mode`).
    Quota,
}

impl WaitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRunning => "not_running",
            Self::NeedsLogin => "needs_login",
            Self::UsageLimit => "usage_limit",
            Self::Cold => "idle",
            Self::Quota => "quota",
        }
    }
}

impl WaitReason {
    /// What ends the wait, as the answer names it (`retry_when`): the sender decides from this,
    /// never from the wording of the error.
    pub fn retry_when(self) -> &'static str {
        match self {
            Self::NotRunning => "start",
            Self::NeedsLogin => "sign_in",
            Self::UsageLimit => "reset_or_account_move",
            Self::Cold => "session_active",
            Self::Quota => "mode_off_or_session_active",
        }
    }
}

/// What to do with one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Deliver,
    /// Keep it, or refuse it where it can't be kept (the caller's rule), until `until_ms` when known.
    Wait { reason: WaitReason, until_ms: Option<u64> },
}

impl Verdict {
    pub fn waits(self) -> bool {
        matches!(self, Verdict::Wait { .. })
    }
}

/// The fact for a registered pane: its [`AgentState`], and while it is `limited` or `signed_out`
/// the [`Unavailable`] that says why. `quiet_for_ms` is only for an idle one.
pub fn availability(state: Option<AgentState>, unavailable: Option<&Unavailable>, quiet_for_ms: Option<u64>) -> Availability {
    match state {
        None => Availability::Unknown,
        Some(AgentState::Working) => Availability::Running { waiting_on_user: false },
        Some(AgentState::Waiting) => Availability::Running { waiting_on_user: true },
        Some(AgentState::Idle) => Availability::Idle { quiet_for_ms },
        Some(AgentState::Stopped) => Availability::NotRunning { exited_with_error: false },
        Some(AgentState::Error) => Availability::NotRunning { exited_with_error: true },
        Some(AgentState::Limited) => {
            Availability::Limited { resets_at_ms: unavailable.and_then(|u| u.resets_at_ms), since_ms: unavailable.map_or(0, |u| u.since_ms) }
        }
        Some(AgentState::SignedOut) => Availability::SignedOut { since_ms: unavailable.map_or(0, |u| u.since_ms) },
    }
}

/// The fact for an unavailable agent (`Unavailable` is only ever limited or signed out).
pub fn of_unavailable(u: &Unavailable) -> Availability {
    availability(Some(u.state), Some(u), None)
}

/// The decision for a message to an agent in `availability`.
///
/// - `urgent`: the sender asked for it now (`cold_hold::asks_to_wake`). It wakes a cold session; it
///   does not deliver into an agent out of quota or signed out, which would only fail the turn.
/// - `routine`: it may wait (`cold_hold::is_routine`); anything else goes to a cold session.
/// - `cold_after_ms`: the setting's quiet time after which a session is cold; `None` turns the hold off.
/// - `hands_over_work`: asked only for a routine message to a cold session.
///
/// A limited agent with no known reset is delivered to: a turn is how it finds out it can work again.
pub fn verdict(
    availability: &Availability,
    urgent: bool,
    routine: bool,
    cold_after_ms: Option<u64>,
    hands_over_work: impl FnOnce() -> bool,
) -> Verdict {
    match *availability {
        Availability::Limited { resets_at_ms: Some(until), .. } => Verdict::Wait { reason: WaitReason::UsageLimit, until_ms: Some(until) },
        Availability::SignedOut { .. } => Verdict::Wait { reason: WaitReason::NeedsLogin, until_ms: None },
        Availability::NotRunning { .. } => Verdict::Wait { reason: WaitReason::NotRunning, until_ms: None },
        Availability::Idle { quiet_for_ms: Some(quiet) } => {
            let cold = cold_after_ms.is_some_and(|after| quiet > after);
            if cold && routine && !urgent && !hands_over_work() {
                Verdict::Wait { reason: WaitReason::Cold, until_ms: None }
            } else {
                Verdict::Deliver
            }
        }
        Availability::Idle { quiet_for_ms: None } | Availability::Running { .. } | Availability::Limited { resets_at_ms: None, .. } | Availability::Unknown => {
            Verdict::Deliver
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::failure::FailureClass;

    const AFTER: u64 = 55 * 60_000;

    fn unavailable(state: AgentState, resets_at_ms: Option<u64>) -> Unavailable {
        let code = if state == AgentState::Limited { FailureClass::UsageLimit } else { FailureClass::Auth };
        Unavailable { state, code, title: String::new(), resets_at_ms, since_ms: 7, account_id: None }
    }

    fn never() -> bool {
        panic!("asked whether the message hands over work, but it could not matter")
    }

    #[test]
    fn each_state_maps_to_its_fact() {
        let lim = unavailable(AgentState::Limited, Some(9));
        let out = unavailable(AgentState::SignedOut, None);
        let cases = [
            (None, None, Availability::Unknown),
            (Some(AgentState::Working), None, Availability::Running { waiting_on_user: false }),
            (Some(AgentState::Waiting), None, Availability::Running { waiting_on_user: true }),
            (Some(AgentState::Idle), None, Availability::Idle { quiet_for_ms: Some(5) }),
            (Some(AgentState::Stopped), None, Availability::NotRunning { exited_with_error: false }),
            (Some(AgentState::Error), None, Availability::NotRunning { exited_with_error: true }),
            (Some(AgentState::Limited), Some(&lim), Availability::Limited { resets_at_ms: Some(9), since_ms: 7 }),
            (Some(AgentState::SignedOut), Some(&out), Availability::SignedOut { since_ms: 7 }),
        ];
        for (state, u, want) in cases {
            assert_eq!(availability(state, u, Some(5)), want, "{state:?}");
        }
        assert_eq!(of_unavailable(&lim), Availability::Limited { resets_at_ms: Some(9), since_ms: 7 });
    }

    #[test]
    fn the_table_of_verdicts() {
        let wait = |reason, until_ms| Verdict::Wait { reason, until_ms };
        let v = |a: Availability, urgent| verdict(&a, urgent, true, Some(AFTER), || false);
        // Out of quota until a known reset, and signed out: held, even for an urgent message.
        for urgent in [false, true] {
            assert_eq!(v(Availability::Limited { resets_at_ms: Some(50), since_ms: 1 }, urgent), wait(WaitReason::UsageLimit, Some(50)));
            assert_eq!(v(Availability::SignedOut { since_ms: 1 }, urgent), wait(WaitReason::NeedsLogin, None));
            assert_eq!(v(Availability::NotRunning { exited_with_error: false }, urgent), wait(WaitReason::NotRunning, None));
            // A limit with no reset known: delivered, a turn finds out.
            assert_eq!(v(Availability::Limited { resets_at_ms: None, since_ms: 1 }, urgent), Verdict::Deliver);
            assert_eq!(v(Availability::Running { waiting_on_user: true }, urgent), Verdict::Deliver);
            assert_eq!(v(Availability::Unknown, urgent), Verdict::Deliver);
            assert_eq!(v(Availability::Idle { quiet_for_ms: None }, urgent), Verdict::Deliver);
            assert_eq!(v(Availability::Idle { quiet_for_ms: Some(AFTER) }, urgent), Verdict::Deliver, "quiet exactly as long as the limit is not cold");
        }
        let cold = Availability::Idle { quiet_for_ms: Some(AFTER + 1) };
        assert_eq!(v(cold, false), wait(WaitReason::Cold, None));
        assert_eq!(v(cold, true), Verdict::Deliver, "urgent wakes a cold session");
        assert_eq!(verdict(&cold, false, true, Some(AFTER), || true), Verdict::Deliver, "a message that hands over work goes");
        assert_eq!(verdict(&cold, false, false, Some(AFTER), never), Verdict::Deliver, "only a routine message waits");
        assert_eq!(verdict(&cold, false, true, None, never), Verdict::Deliver, "the setting off turns the hold off");
        assert_eq!(verdict(&Availability::Idle { quiet_for_ms: Some(5) }, false, true, Some(AFTER), never), Verdict::Deliver, "warm");
    }

    #[test]
    fn the_wire_strings_are_the_existing_held_reasons() {
        let all = [WaitReason::NotRunning, WaitReason::NeedsLogin, WaitReason::UsageLimit, WaitReason::Cold, WaitReason::Quota];
        assert_eq!(all.map(WaitReason::as_str), ["not_running", "needs_login", "usage_limit", "idle", "quota"]);
        assert_eq!(all.map(WaitReason::retry_when), ["start", "sign_in", "reset_or_account_move", "session_active", "mode_off_or_session_active"]);
    }

    /// What `jekt_hold::holding_unavailable` filtered before it asked this module: out of quota with
    /// a known reset, or signed out.
    fn legacy_holds(u: &Unavailable) -> bool {
        u.state == AgentState::SignedOut || (u.state == AgentState::Limited && u.resets_at_ms.is_some())
    }

    /// What `jekt_cold::decide` and `cold_hold::is_cold` answered before it asked this module.
    fn legacy_cold_holds(last_output_ms: Option<i64>, now_ms: i64, after_ms: i64, urgent: bool, hands_over: bool) -> bool {
        !urgent && crate::backend::cold_hold::is_cold(last_output_ms, now_ms, after_ms) && !hands_over
    }

    #[test]
    fn unavailable_agents_wait_exactly_when_the_old_filter_held_them() {
        for state in [AgentState::Limited, AgentState::SignedOut] {
            for resets in [None, Some(100)] {
                let u = unavailable(state, resets);
                for urgent in [false, true] {
                    let got = verdict(&of_unavailable(&u), urgent, true, Some(AFTER), || false).waits();
                    assert_eq!(got, legacy_holds(&u), "{state:?} reset={resets:?} urgent={urgent}");
                }
            }
        }
    }

    #[test]
    fn a_cold_session_waits_exactly_when_the_old_decision_held_it() {
        let now: i64 = 100_000_000;
        for last in [None, Some(now - 1), Some(now - AFTER as i64), Some(now - AFTER as i64 - 1), Some(now - 10 * AFTER as i64)] {
            for urgent in [false, true] {
                for hands_over in [false, true] {
                    let quiet = last.map(|at| (now - at) as u64);
                    let got = verdict(&Availability::Idle { quiet_for_ms: quiet }, urgent, true, Some(AFTER), || hands_over).waits();
                    assert_eq!(got, legacy_cold_holds(last, now, AFTER as i64, urgent, hands_over), "last={last:?} urgent={urgent} hands_over={hands_over}");
                }
            }
        }
    }
}

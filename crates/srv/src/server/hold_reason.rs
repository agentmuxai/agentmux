// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Why a jekt is held (`held_reason` in the inject answer;
//! SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md §5.1).

use crate::backend::agent_state::AgentState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldReason {
    /// A known agent with no process here.
    NotRunning,
    /// The agent is here but its spawn gate refused: not signed in. Also an
    /// agent whose sign-in was refused (`AgentState::SignedOut`): to the
    /// sender it is the same condition.
    NeedsLogin,
    /// The agent is here but its account is out of quota
    /// (`AgentState::Limited`). Held from step 2c of
    /// `docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §5.
    Limited,
}

impl HoldReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            HoldReason::NotRunning => "not_running",
            HoldReason::NeedsLogin => "needs_login",
            HoldReason::Limited => "usage_limit",
        }
    }

    /// The reason a message to an agent in `state` is held, or `None` when
    /// that state doesn't hold messages.
    pub(crate) fn for_state(state: AgentState) -> Option<Self> {
        match state {
            AgentState::Limited => Some(HoldReason::Limited),
            AgentState::SignedOut => Some(HoldReason::NeedsLogin),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unavailable_agent_holds_with_its_reason_and_the_rest_do_not() {
        assert_eq!(HoldReason::for_state(AgentState::Limited).map(HoldReason::as_str), Some("usage_limit"));
        assert_eq!(HoldReason::for_state(AgentState::SignedOut).map(HoldReason::as_str), Some("needs_login"));
        for s in [AgentState::Idle, AgentState::Working, AgentState::Waiting, AgentState::Stopped, AgentState::Error] {
            assert_eq!(HoldReason::for_state(s), None, "{s:?}");
        }
    }
}

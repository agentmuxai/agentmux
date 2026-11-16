// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether an agent is asked at all (spec §3.2). Only one that has something to lose and can take a
//! turn to say so; for every other agent the action runs at once, as it did before this existed.

use crate::backend::agent_state::AgentState;
use crate::server::AppState;

use super::Prep;

/// Ask the agent, or run the action now and say why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Ask,
    /// The reason, in the words of the audit row (`not_asked:<reason>`).
    Skip(&'static str),
}

/// What the decision reads, gathered once.
struct Inputs {
    /// The state of each of the agent's panes (`None`: no controller).
    states: Vec<Option<AgentState>>,
    /// Whether a message to the pane that would be asked starts a turn its offline model API fails.
    api_offline: bool,
    /// What a restart would cost the agent, apart from a turn in flight.
    lost: Option<&'static str>,
}

fn can_take_a_turn(s: Option<AgentState>) -> bool {
    matches!(s, Some(AgentState::Working | AgentState::Waiting | AgentState::Idle))
}

fn in_a_turn(s: Option<AgentState>) -> bool {
    matches!(s, Some(AgentState::Working | AgentState::Waiting))
}

fn decide_from(i: &Inputs) -> Decision {
    if !i.states.iter().any(|s| can_take_a_turn(*s)) {
        // Out of quota, signed out, stopped, or not up: a prep turn would only be held.
        return Decision::Skip("cannot_take_a_turn");
    }
    if i.api_offline {
        return Decision::Skip("api_offline");
    }
    if i.states.iter().any(|s| in_a_turn(*s)) || i.lost.is_some() {
        return Decision::Ask;
    }
    Decision::Skip("nothing_to_lose")
}

/// [`Decision`] for `prep`. BLOCKING (store reads).
pub fn decide(state: &AppState, prep: &Prep) -> Decision {
    let states: Vec<Option<AgentState>> = prep.blocks.iter().map(|b| crate::backend::agent_state::current_state(&state.mstore, b)).collect();
    let Some(asked) = prep.blocks.iter().zip(&states).find(|(_, s)| can_take_a_turn(**s)).map(|(b, _)| b) else {
        return decide_from(&Inputs { states, api_offline: false, lost: None });
    };
    let api_offline = crate::server::jekt_hold::api_is_offline_for(&state.mstore, asked, &crate::connectivity::status(), agentmux_common::time::now_ms_u64());
    let lost = crate::backend::continuity_facts::something_to_lose(&state.mstore, Some(&state.identity_store), asked);
    let decision = decide_from(&Inputs { states, api_offline, lost });
    tracing::debug!(agent = %prep.agent_name, ?decision, lost = ?lost, "prep: decided");
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(states: &[Option<AgentState>], api_offline: bool, lost: Option<&'static str>) -> Inputs {
        Inputs { states: states.to_vec(), api_offline, lost }
    }

    #[test]
    fn an_agent_in_a_turn_is_asked() {
        for s in [AgentState::Working, AgentState::Waiting] {
            assert_eq!(decide_from(&inputs(&[Some(s)], false, None)), Decision::Ask, "{s:?}");
        }
    }

    #[test]
    fn an_idle_agent_is_asked_only_when_it_holds_something() {
        assert_eq!(decide_from(&inputs(&[Some(AgentState::Idle)], false, Some("holds a claimed task"))), Decision::Ask);
        assert_eq!(decide_from(&inputs(&[Some(AgentState::Idle)], false, None)), Decision::Skip("nothing_to_lose"));
    }

    #[test]
    fn an_agent_that_cannot_take_a_turn_is_never_asked() {
        for s in [Some(AgentState::Limited), Some(AgentState::SignedOut), Some(AgentState::Stopped), Some(AgentState::Error), None] {
            assert_eq!(decide_from(&inputs(&[s], false, Some("holds a claimed task"))), Decision::Skip("cannot_take_a_turn"), "{s:?}");
        }
        assert_eq!(decide_from(&inputs(&[], false, Some("x"))), Decision::Skip("cannot_take_a_turn"), "no pane");
    }

    #[test]
    fn a_pane_that_can_answer_is_enough_when_another_cannot() {
        assert_eq!(decide_from(&inputs(&[Some(AgentState::Limited), Some(AgentState::Working)], false, None)), Decision::Ask);
    }

    #[test]
    fn an_offline_api_is_not_asked() {
        assert_eq!(decide_from(&inputs(&[Some(AgentState::Working)], true, None)), Decision::Skip("api_offline"));
    }
}

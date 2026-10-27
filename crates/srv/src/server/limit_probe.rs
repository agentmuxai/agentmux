// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Finding out that a limited agent can work again when nothing else will say
//! so. A stored usage limit ends at its reset, a clean turn, a sign-in or an
//! account move; if the account was changed out of band (the plan was raised,
//! the limit was lifted by hand) none of those happens, and every message to
//! the agent stays held. So a message that must not wait (urgent, or naming a
//! task that waits on the agent) is still held, and in addition one small
//! turn is started on the pane. A clean turn clears the failure and wakes the
//! replay, which then sends what was held; a turn that fails again leaves the
//! agent limited and the message held. The message itself never rides the
//! probe, so a failed turn loses nothing. Spec:
//! `docs/specs/SPEC_AGENT_AVAILABILITY_ONE_STATE_2026_10_26.md`.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::backend::agent_state::{AgentState, Unavailable};
use crate::backend::reactive::InjectionRequest;

use super::AppState;

/// The least time between two probes of one agent.
pub(super) const PROBE_EVERY_MS: u64 = 15 * 60 * 1000;

/// What the agent is asked; any reply is a clean turn.
pub(super) const PROBE_NOTE: &str =
    "[AgentMux] A message for you is waiting while your account was out of quota. Reply with the single word: ok";

static LAST_PROBE: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

/// Whether a held message to this agent warrants a probe: the agent is out of
/// quota until a known reset (without one, messages are delivered and a turn
/// finds out), and the message cannot wait.
pub(super) fn wants_probe(unavailable: &Unavailable, cannot_wait: bool) -> bool {
    cannot_wait && unavailable.state == AgentState::Limited && unavailable.resets_at_ms.is_some()
}

/// Takes the probe slot of `key` if its last probe is at least
/// [`PROBE_EVERY_MS`] ago; `false` leaves it as it was.
pub(super) fn take_slot(key: &str, now_ms: u64) -> bool {
    let mut last = LAST_PROBE.lock().unwrap_or_else(|e| e.into_inner());
    if last.get(key).is_some_and(|at| now_ms.saturating_sub(*at) < PROBE_EVERY_MS) {
        return false;
    }
    last.insert(key.to_string(), now_ms);
    true
}

/// After `req` was held for a limited agent: start a probe turn if it
/// warrants one and the agent's slot is free.
pub(super) async fn maybe_probe(state: &AppState, req: &InjectionRequest, unavailable: &Unavailable) {
    if !wants_probe(unavailable, true) {
        return;
    }
    let urgent = crate::backend::cold_hold::asks_to_wake(req.priority.as_deref());
    let (mstore, identity, request) = (state.mstore.clone(), state.work_store.clone(), req.clone());
    let now_ms = agentmux_common::time::now_ms_u64();
    let key = tokio::task::spawn_blocking(move || {
        let (uid, slug) = super::reactive::defined_here(&mstore, &request.target_agent)?;
        let cannot_wait =
            urgent || super::jekt_cold::hands_over_work(&identity, &request, &uid, &slug, agentmux_common::time::now_ms());
        cannot_wait.then_some(uid)
    })
    .await
    .ok()
    .flatten();
    let Some(key) = key else { return };
    if !take_slot(&key, now_ms) {
        return;
    }
    match state.reactive_handler.deliver_system_note(&req.target_agent, PROBE_NOTE) {
        Ok(()) => tracing::info!(target = %req.target_agent, "limit probe: started a turn to see whether the account can work again"),
        Err(e) => tracing::info!(target = %req.target_agent, error = %e, "limit probe: not started"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::failure::FailureClass;

    fn unavailable(state: AgentState, resets_at_ms: Option<u64>) -> Unavailable {
        let code = if state == AgentState::Limited { FailureClass::UsageLimit } else { FailureClass::Auth };
        Unavailable { state, code, title: String::new(), resets_at_ms, since_ms: 0, account_id: None }
    }

    #[test]
    fn only_a_message_that_cannot_wait_probes_an_agent_limited_until_a_known_reset() {
        assert!(wants_probe(&unavailable(AgentState::Limited, Some(5)), true));
        assert!(!wants_probe(&unavailable(AgentState::Limited, Some(5)), false), "a routine message waits for the reset");
        assert!(!wants_probe(&unavailable(AgentState::Limited, None), true), "no reset known: messages are delivered");
        assert!(!wants_probe(&unavailable(AgentState::SignedOut, None), true), "a sign-in is what ends that");
    }

    #[test]
    fn an_agent_is_probed_at_most_once_per_interval() {
        let key = "probe-test-agent";
        assert!(take_slot(key, 1_000_000));
        assert!(!take_slot(key, 1_000_000 + PROBE_EVERY_MS - 1));
        assert!(take_slot(key, 1_000_000 + PROBE_EVERY_MS));
        assert!(take_slot("probe-test-other", 1_000_001), "another agent has its own slot");
    }
}

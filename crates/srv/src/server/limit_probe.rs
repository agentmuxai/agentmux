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

use crate::agents::failure::FailureClass;
use crate::backend::agent_state::{failure_of_block, AgentState, Unavailable};
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;
use crate::backend::reactive::InjectionRequest;

use super::AppState;

/// The least time between two probes of one agent.
pub(super) const PROBE_EVERY_MS: u64 = 15 * 60 * 1000;

/// What the agent is asked; any reply is a clean turn.
pub(super) const PROBE_NOTE: &str =
    "[AgentMux] A message for you is waiting while your account was out of quota. Reply with the single word: ok";

/// How long after it started a probe's turn is taken to still be running.
const PROBE_WINDOW_MS: u64 = 10 * 60 * 1000;

static LAST_PROBE: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);
/// The panes with a probe turn in flight, and when it began.
static PROBING: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

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

/// The limit of the agent on `block_id` while its probe turn runs. A turn in
/// flight makes the pane `working`, so the usual check would let a replay pass
/// release held messages into the probe, and a probe that fails would lose
/// them. The hold lasts until the probe clears the stored failure (a clean
/// turn) or [`PROBE_WINDOW_MS`] has passed. BLOCKING.
pub(super) fn unavailable_while_probing(mstore: &Store, block_id: &str, now_ms: u64) -> Option<Unavailable> {
    {
        let mut probing = PROBING.lock().unwrap_or_else(|e| e.into_inner());
        let started = *probing.get(block_id)?;
        if now_ms.saturating_sub(started) >= PROBE_WINDOW_MS {
            probing.remove(block_id);
            return None;
        }
    }
    let failure = failure_of_block(&mstore.get::<Block>(block_id).ok()??)?;
    let resets_at_ms = failure.resets_at_ms.filter(|r| *r > now_ms)?;
    (failure.code == FailureClass::UsageLimit).then(|| Unavailable {
        state: AgentState::Limited,
        code: failure.code,
        title: failure.title,
        resets_at_ms: Some(resets_at_ms),
        since_ms: failure.at_ms.unwrap_or(now_ms).min(now_ms),
        account_id: None,
    })
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
    let Some(block_id) = state.reactive_handler.block_for_uid(&key) else { return };
    if !take_slot(&key, now_ms) {
        return;
    }
    // Before the turn starts, so no replay pass can see the pane working and unheld.
    PROBING.lock().unwrap_or_else(|e| e.into_inner()).insert(block_id, now_ms);
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

    fn limited_block(store: &Store, oid: &str, now: u64, with_failure: bool) {
        let mut b = Block { oid: oid.to_string(), ..Default::default() };
        if with_failure {
            b.meta.insert(
                crate::backend::blockcontroller::core::META_LAST_FAILURE.to_string(),
                serde_json::json!({"code": "usage_limit", "title": "Usage limit reached", "detail": "", "retryable": false, "resetsAtMs": now + 60_000, "atMs": now - 1_000}),
            );
        }
        store.insert(&mut b).unwrap();
    }

    #[test]
    fn the_limit_holds_while_a_probe_runs_and_ends_with_the_probe_or_its_failure() {
        let store = Store::open_in_memory().unwrap();
        let now = 5_000_000;
        limited_block(&store, "probe-test-limited", now, true);
        limited_block(&store, "probe-test-cleared", now, false);
        assert_eq!(unavailable_while_probing(&store, "probe-test-limited", now), None, "no probe in flight: the usual check decides");
        PROBING.lock().unwrap().insert("probe-test-limited".to_string(), now);
        PROBING.lock().unwrap().insert("probe-test-cleared".to_string(), now);
        let held = unavailable_while_probing(&store, "probe-test-limited", now + 1).expect("held while the probe runs");
        assert_eq!((held.state, held.resets_at_ms, held.since_ms), (AgentState::Limited, Some(now + 60_000), now - 1_000));
        assert_eq!(unavailable_while_probing(&store, "probe-test-cleared", now + 1), None, "a clean turn cleared the failure: replay goes");
        assert_eq!(unavailable_while_probing(&store, "probe-test-limited", now + 60_001), None, "the reset passed");
        assert_eq!(unavailable_while_probing(&store, "probe-test-limited", now + PROBE_WINDOW_MS), None, "a probe that never ends stops holding");
    }
}

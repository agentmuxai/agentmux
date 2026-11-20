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

/// The least time between two checks a person's message starts on one agent.
const PERSON_CHECK_EVERY_MS: u64 = 60 * 1000;

/// What [`check_now`] did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Check {
    /// A check turn was started.
    Started,
    /// One is running, or one started less than a minute ago: its result is the answer.
    Running,
    /// None was started, and why (the agent isn't registered, or the note wasn't delivered).
    NotStarted(String),
}

/// Starts a check turn on the agent of `block_id` because a person's message waits for it: it does
/// not wait for the 15-minute slot a message between agents does, but one turn runs at a time and a
/// new one starts no sooner than a minute after the last. The message itself never rides the turn.
pub(super) fn check_now(block_id: &str, now_ms: u64) -> Check {
    let handler = crate::backend::reactive::get_global_handler();
    let Some(target) = handler
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == block_id)
        .map(|r| r.uid.filter(|u| !u.is_empty()).unwrap_or(r.agent_id))
    else {
        return Check::NotStarted("the agent is not registered".to_string());
    };
    if !take_person_check(block_id, now_ms) {
        return Check::Running;
    }
    match handler.deliver_system_note(&target, PROBE_NOTE) {
        Ok(()) => Check::Started,
        Err(e) => {
            PROBING.lock().unwrap_or_else(|e| e.into_inner()).remove(block_id);
            Check::NotStarted(e)
        }
    }
}

/// Takes the right to start a check on `block_id`, and marks its turn in flight (the hold lasts until it clears
/// the limit, [`unavailable_while_probing`]). `false` when one runs or ran less than
/// [`PERSON_CHECK_EVERY_MS`] ago.
fn take_person_check(block_id: &str, now_ms: u64) -> bool {
    let mut probing = PROBING.lock().unwrap_or_else(|e| e.into_inner());
    if probing.get(block_id).is_some_and(|started| now_ms.saturating_sub(*started) < PERSON_CHECK_EVERY_MS) {
        return false;
    }
    probing.insert(block_id.to_string(), now_ms);
    true
}

/// The pause between the starts of the check turns of one account.
const CHECK_STAGGER_MS: u64 = 2_000;

/// How long after the first each check of one account starts: the first at once, the others
/// [`CHECK_STAGGER_MS`] apart, so the agents of an account do not all start a turn in the same second.
/// `accounts[i]` is pane `i`'s account (`None`: its own).
pub(super) fn stagger_delays(accounts: &[Option<&str>]) -> Vec<u64> {
    let mut seen: HashMap<&str, u64> = HashMap::new();
    accounts
        .iter()
        .map(|account| match account {
            Some(account) => {
                let n = seen.entry(account).or_insert(0);
                *n += 1;
                (*n - 1) * CHECK_STAGGER_MS
            }
            None => 0,
        })
        .collect()
}

/// "Check now" for the panes `block_ids`: each one out of quota gets a check turn (`check_now`),
/// staggered by account. `Ok` when its check started, runs, or starts in a moment; `Err` says why
/// not (not out of quota, a route out of credits, not registered). The way to say "the plan changed,
/// look again", so nobody needs to know that an urgent message is what starts one.
pub(super) fn check_limited(mstore: &Store, block_ids: &[String]) -> Vec<(String, Result<(), String>)> {
    let now_ms = agentmux_common::time::now_ms_u64();
    let found: Vec<Result<Unavailable, String>> = block_ids
        .iter()
        .map(|id| match crate::backend::agent_state::unavailable_of_block(mstore, None, id, now_ms) {
            Some(u) if super::person_hold::is_kept_for_check(&u) => Ok(u),
            Some(u) => Err(format!("{}: a check cannot change that", u.reason())),
            None => Err("not out of quota".to_string()),
        })
        .collect();
    let accounts: Vec<Option<&str>> = found.iter().map(|f| f.as_ref().ok().and_then(|u| u.account_id.as_deref())).collect();
    let delays = stagger_delays(&accounts);
    block_ids
        .iter()
        .zip(found.iter().zip(delays))
        .map(|(id, (found, delay_ms))| (id.clone(), found.as_ref().map_err(Clone::clone).and_then(|_| start_check(id, delay_ms, now_ms))))
        .collect()
}

fn start_check(block_id: &str, delay_ms: u64, now_ms: u64) -> Result<(), String> {
    if delay_ms == 0 {
        return match check_now(block_id, now_ms) {
            Check::NotStarted(why) => Err(why),
            Check::Started | Check::Running => Ok(()),
        };
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return Err("no runtime to start the check on".to_string()) };
    let block_id = block_id.to_string();
    runtime.spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        if let Check::NotStarted(why) = check_now(&block_id, agentmux_common::time::now_ms_u64()) {
            tracing::info!(block_id = %block_id, why = %why, "limit check: not started");
        }
    });
    Ok(())
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

    #[test]
    fn the_checks_of_one_account_start_two_seconds_apart_and_other_accounts_do_not_wait_for_it() {
        let delays = stagger_delays(&[Some("a"), Some("b"), Some("a"), None, Some("a"), None]);
        assert_eq!(delays, vec![0, 0, CHECK_STAGGER_MS, 0, 2 * CHECK_STAGGER_MS, 0]);
    }

    #[test]
    fn only_a_pane_out_of_its_plans_quota_is_checked() {
        let store = Store::open_in_memory().unwrap();
        let now = agentmux_common::time::now_ms_u64();
        limited_block(&store, "check-test-limited", now, true);
        let mut healthy = Block { oid: "check-test-healthy".to_string(), ..Default::default() };
        store.insert(&mut healthy).unwrap();
        let answers = check_limited(&store, &["check-test-healthy".to_string(), "check-test-gone".to_string()]);
        assert_eq!(
            answers,
            vec![
                ("check-test-healthy".to_string(), Err("not out of quota".to_string())),
                ("check-test-gone".to_string(), Err("not out of quota".to_string())),
            ]
        );
        // A limited pane with a process but no agent registered on it: the check cannot start, and says so.
        crate::backend::agent_state::test_support::register_stub(
            "check-test-limited",
            crate::backend::blockcontroller::BLOCK_CONTROLLER_PERSISTENT,
            crate::backend::blockcontroller::STATUS_RUNNING,
            false,
        );
        let answers = check_limited(&store, &["check-test-limited".to_string()]);
        crate::backend::blockcontroller::delete_controller("check-test-limited");
        assert_eq!(answers, vec![("check-test-limited".to_string(), Err("the agent is not registered".to_string()))]);
    }

    #[test]
    fn a_persons_check_starts_at_once_but_one_runs_at_a_time_and_not_again_within_a_minute() {
        let id = "check-test-pane";
        assert!(take_person_check(id, 1_000_000), "no wait for the 15-minute slot");
        assert!(!take_person_check(id, 1_000_000 + PERSON_CHECK_EVERY_MS - 1), "one is in flight");
        assert!(take_person_check(id, 1_000_000 + PERSON_CHECK_EVERY_MS));
        assert!(take_person_check("check-test-other-pane", 1_000_001), "another agent has its own");
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

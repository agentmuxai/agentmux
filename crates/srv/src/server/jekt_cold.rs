// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Holding a routine message for an agent whose session has gone cold
//! (`SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_2026_10_25.md`). The rules are
//! `backend::cold_hold`; the held row and its replay are the durable-jekt
//! hold (`jekt_hold`, `jekt_held`), with the reason `idle`.

use serde_json::json;

use crate::backend::cold_hold::{self, REASON_IDLE};
use crate::backend::reactive::handler::LookupOutcome;
use crate::backend::reactive::InjectionRequest;
use crate::backend::storage::jekt_held::HoldOutcome;
use crate::backend::storage::store::Store;

use super::reactive::{replay, HoldReason};
use super::AppState;

/// What to do with a message for a running agent.
#[derive(Debug)]
pub(super) enum Decision {
    /// Deliver it now.
    Deliver,
    /// Keep it until the session is next active.
    Hold,
}

/// Whether the message hands `target` work, so it must not wait: it names a
/// task (`#N`) that is waiting on the target, or its sender filed the target
/// a task a moment ago.
pub(super) fn hands_over_work(identity: &Store, req: &InjectionRequest, target_uid: &str, target_slug: &str, now_ms: i64) -> bool {
    let named = cold_hold::task_numbers(&req.message);
    if identity.work_numbers_waiting_on(&named, target_uid, target_slug).unwrap_or(false) {
        return true;
    }
    let sender = req.source_agent.as_deref().unwrap_or("");
    identity
        .work_filed_for_since(&req.audit_source_uid, sender, target_uid, target_slug, now_ms - cold_hold::FILED_WINDOW_MS)
        .unwrap_or(false)
}

/// The decision for a routine message to a running agent. Pure of the
/// controllers: `last_output_ms` is `session_last_output`'s answer.
#[allow(clippy::too_many_arguments)]
pub(super) fn decide(
    mstore: &Store,
    identity: &Store,
    req: &InjectionRequest,
    target_uid: &str,
    target_slug: &str,
    last_output_ms: Option<i64>,
    now_ms: i64,
    after_ms: i64,
) -> Decision {
    if cold_hold::asks_to_wake(req.priority.as_deref()) {
        // A sender who was told "held" and sent it again to wake the agent:
        // the held copy would arrive a second time behind it.
        let source = req.source_agent.clone().unwrap_or_default();
        let message = crate::backend::reactive::sanitize::sanitize_message(&req.message);
        let _ = mstore.jekt_held_delete_idle_copies(target_uid, &source, &message);
        return Decision::Deliver;
    }
    if !cold_hold::is_cold(last_output_ms, now_ms, after_ms) {
        return Decision::Deliver;
    }
    if hands_over_work(identity, req, target_uid, target_slug, now_ms) {
        return Decision::Deliver;
    }
    Decision::Hold
}

/// Hold `req` when its target is a running Claude session gone cold and the
/// message may wait. `None` delivers it as usual, which every doubt does.
pub(super) async fn hold_if_cold(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    replay: &mut replay::Reservation,
) -> Option<serde_json::Value> {
    // The same set the durable hold takes (`hold_applies`): a full-key,
    // host-tier request with an id, not a periodic `cron` fire. Not a message
    // an agent sends itself (its own loops and reminders).
    let request_id = req.request_id.clone().filter(|id| !id.is_empty())?;
    if auth_via != super::ReactiveAuthVia::FullAuthKey
        || req.delivery_tier.as_deref() != Some("host")
        || req.source_agent.as_deref().is_none_or(|s| s.is_empty() || s == "cron")
    {
        return None;
    }
    let route = crate::backend::reactive::sanitize::route_name(&req.target_agent)?;
    if req.source_agent.as_deref().map(crate::backend::reactive::sanitize::route_name).flatten().as_deref() == Some(route.as_str()) {
        return None;
    }
    let after_ms = cold_hold::cold_after_ms(&state.config_watcher.get_full_config().settings.extra)?;
    let LookupOutcome::One(reg) = state.reactive_handler.lookup_delivery_target(&route) else {
        return None;
    };
    let target_uid = reg.uid.clone()?;
    let (mstore, identity, request) = (state.mstore.clone(), state.identity_store.clone(), req.clone());
    let now = agentmux_common::time::now_ms();
    let held = tokio::task::spawn_blocking(move || {
        // Urgent or not routine: the sender's own words decide, so a message
        // that is not routine skips even the wake-up bookkeeping.
        if !cold_hold::is_routine(&request) && !cold_hold::asks_to_wake(request.priority.as_deref()) {
            return None;
        }
        let last_output = cold_hold::session_last_output(&mstore, &reg.block_id);
        match decide(&mstore, &identity, &request, &target_uid, &reg.agent_id, last_output, now, after_ms) {
            Decision::Deliver => None,
            Decision::Hold => {
                let mut row = super::jekt_hold::held_row(&request, request_id, now);
                row.target_uid = target_uid;
                row.hold_reason = REASON_IDLE.to_string();
                match mstore.jekt_held_insert(&row) {
                    Ok(HoldOutcome::Held) | Ok(HoldOutcome::AlreadyHeld) => Some(row),
                    // Full, or a store fault: deliver it rather than lose or refuse it.
                    Ok(HoldOutcome::Full) | Err(_) => None,
                }
            }
        }
    })
    .await
    .ok()
    .flatten()?;
    replay.delivered();
    crate::backend::agent_resolve::record_uid_fallback("jekt.held");
    tracing::info!(target = %req.target_agent, "reactive inject: session is cold, holding a routine message until it is next active");
    let target = &req.target_agent;
    Some(json!({
        "success": false,
        "held": true,
        "held_reason": HoldReason::Idle.as_str(),
        "request_id": held.request_id,
        "error": format!(
            "agent {target}'s session has been idle — held until it is next active (for up to 24 h). \
             To wake it now, send it again with urgent: true"
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::work_queue::WorkItem;

    const AFTER: i64 = 55 * 60_000;
    const NOW: i64 = 100_000_000;
    const COLD: Option<i64> = Some(NOW - AFTER - 1);
    const WARM: Option<i64> = Some(NOW - 60_000);

    fn req(message: &str) -> InjectionRequest {
        InjectionRequest {
            target_agent: "loap".into(),
            message: message.into(),
            source_agent: Some("agentx".into()),
            audit_source_uid: "uid-x".into(),
            sig_verified: Some(true),
            ..Default::default()
        }
    }

    /// The channel store and an identity store (which holds the task queue); the
    /// directory must outlive the test.
    fn stores() -> (Store, Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let identity = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
        (Store::open_in_memory().unwrap(), identity, dir)
    }

    fn task(id: &str, filer_uid: &str, target_uid: &str, created_at: i64) -> WorkItem {
        WorkItem {
            id: id.into(),
            title: format!("task {id}"),
            payload: "do it".into(),
            kind: String::new(),
            target_agent: "loap".into(),
            target_group: String::new(),
            priority: 0,
            state: String::new(),
            claimed_by: String::new(),
            target_agent_uid: target_uid.into(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: "agentx".into(),
            created_by_uid: filer_uid.into(),
            created_at,
            updated_at: created_at,
            not_before: None,
            result: String::new(),
        }
    }

    fn decision(m: &Store, i: &Store, r: &InjectionRequest, last: Option<i64>) -> &'static str {
        match decide(m, i, r, "uid-l", "loap", last, NOW, AFTER) {
            Decision::Deliver => "deliver",
            Decision::Hold => "hold",
        }
    }

    #[test]
    fn a_routine_message_to_a_cold_session_is_held_and_to_a_warm_one_is_not() {
        let (m, i, _d) = stores();
        assert_eq!(decision(&m, &i, &req("thanks, noted"), COLD), "hold");
        assert_eq!(decision(&m, &i, &req("thanks, noted"), WARM), "deliver");
        assert_eq!(decision(&m, &i, &req("thanks, noted"), None), "deliver", "no output yet: nothing to lose");
    }

    #[test]
    fn urgent_wakes_a_cold_session_and_drops_the_copy_it_replaces() {
        let (m, i, _d) = stores();
        let first = req("deploy is done");
        let mut row = super::super::jekt_hold::held_row(&first, "r1".into(), NOW - 1000);
        row.target_uid = "uid-l".into();
        row.source_agent = "agentx".into();
        row.hold_reason = REASON_IDLE.into();
        assert_eq!(m.jekt_held_insert(&row).unwrap(), HoldOutcome::Held);

        // Another sender's identical words are kept.
        let mut other = row.clone();
        other.request_id = "r2".into();
        other.source_agent = "agenty".into();
        m.jekt_held_insert(&other).unwrap();

        let mut again = req("deploy is done");
        again.priority = Some("urgent".into());
        assert_eq!(decision(&m, &i, &again, COLD), "deliver");
        let left = m.jekt_held_for_target("uid-l", 10).unwrap();
        assert_eq!(left.len(), 1, "{left:?}");
        assert_eq!(left[0].source_agent, "agenty");
    }

    #[test]
    fn naming_a_task_waiting_on_the_target_is_work_and_wakes_it() {
        let (m, i, _d) = stores();
        i.work_queue_enqueue(&task("t1", "uid-z", "uid-l", 1)).unwrap();
        let num = i.work_card_create("t1", &Default::default(), 1).unwrap();
        assert_eq!(decision(&m, &i, &req(&format!("#{num} is yours")), COLD), "deliver");
        assert_eq!(decision(&m, &i, &req("#9999 is yours"), COLD), "hold", "no such task");
        // A task for someone else is not work for this agent.
        i.work_queue_enqueue(&task("t2", "uid-z", "uid-o", 1)).unwrap();
        let other = i.work_card_create("t2", &Default::default(), 1).unwrap();
        assert_eq!(decision(&m, &i, &req(&format!("#{other} is yours")), COLD), "hold");
    }

    #[test]
    fn a_task_the_sender_just_filed_for_the_target_makes_the_next_message_urgent() {
        let (m, i, _d) = stores();
        i.work_queue_enqueue(&task("t1", "uid-x", "uid-l", NOW - 5 * 60_000)).unwrap();
        i.work_card_create("t1", &Default::default(), 1).unwrap();
        assert_eq!(decision(&m, &i, &req("take a look when you can"), COLD), "deliver");
        // Filed an hour ago, or by someone else: no longer a nudge.
        let (m2, i2, _d2) = stores();
        i2.work_queue_enqueue(&task("t1", "uid-x", "uid-l", NOW - 60 * 60_000)).unwrap();
        i2.work_card_create("t1", &Default::default(), 1).unwrap();
        assert_eq!(decision(&m2, &i2, &req("take a look when you can"), COLD), "hold");
        let (m3, i3, _d3) = stores();
        i3.work_queue_enqueue(&task("t1", "uid-z", "uid-l", NOW - 5 * 60_000)).unwrap();
        i3.work_card_create("t1", &Default::default(), 1).unwrap();
        assert_eq!(decision(&m3, &i3, &req("take a look when you can"), COLD), "hold");
    }

    #[test]
    fn a_block_with_no_claude_session_is_never_cold() {
        let m = Store::open_in_memory().unwrap();
        assert_eq!(cold_hold::session_last_output(&m, "no-such-block"), None);
    }
}

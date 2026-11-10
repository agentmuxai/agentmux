// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Holding a routine message for an agent whose session has gone cold
//! (`SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_2026_10_25.md`). The rules are
//! `backend::cold_hold`; the held row and its replay are the durable-jekt
//! hold (`jekt_hold`, `jekt_held`), with the reason `idle`.

use serde_json::json;

use crate::backend::cold_hold::{self, REASON_IDLE, REASON_QUOTA};
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

/// The decision for a routine message to a running agent, and the words for why it was not held.
/// Pure of the controllers: `last_output_ms` is `session_last_output`'s answer.
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
    decide_why(mstore, identity, req, target_uid, target_slug, last_output_ms, now_ms, after_ms).0
}

/// [`decide`], with the reason: what the log says for a message delivered rather than held.
#[allow(clippy::too_many_arguments)]
pub(super) fn decide_why(
    mstore: &Store,
    identity: &Store,
    req: &InjectionRequest,
    target_uid: &str,
    target_slug: &str,
    last_output_ms: Option<i64>,
    now_ms: i64,
    after_ms: i64,
) -> (Decision, String) {
    if cold_hold::asks_to_wake(req.priority.as_deref()) {
        // A sender who was told "held" and sent it again to wake the agent:
        // the held copy would arrive a second time behind it.
        let source = req.source_agent.clone().unwrap_or_default();
        let message = crate::backend::reactive::sanitize::sanitize_message(&req.message);
        let _ = mstore.jekt_held_delete_idle_copies(target_uid, &source, &message);
        return (Decision::Deliver, "the sender asked for it to be delivered now (urgent)".to_string());
    }
    let Some(last) = last_output_ms else {
        return (Decision::Deliver, "the session has written no output, is not a Claude session, or has a turn in flight".to_string());
    };
    use crate::backend::agent_availability::{verdict, Availability};
    let quiet_ms = now_ms.saturating_sub(last);
    let idle = Availability::Idle { quiet_for_ms: Some(u64::try_from(quiet_ms).unwrap_or(0)) };
    let rule = verdict(&idle, false, true, u64::try_from(after_ms).ok(), || hands_over_work(identity, req, target_uid, target_slug, now_ms));
    if rule.waits() {
        (Decision::Hold, "idle".to_string())
    } else if quiet_ms > after_ms {
        (Decision::Deliver, format!("it hands the agent work (a task it names or one its sender just filed); quiet for {} min", quiet_ms / 60_000))
    } else {
        (Decision::Deliver, format!("the session is warm: quiet for {} min of {}", quiet_ms / 60_000, after_ms / 60_000))
    }
}

/// Why the cold hold does not look at this message at all, or `None` when it may apply: the hold
/// takes a message with an id, from a named sender that is not cron, that came from a full-key
/// host-tier caller or a peer on another machine that proved who it is (`verified_peer`).
fn outside_the_hold(auth_via: super::ReactiveAuthVia, req: &InjectionRequest) -> Option<String> {
    if req.request_id.as_deref().is_none_or(str::is_empty) {
        return Some("it has no request id".to_string());
    }
    match req.source_agent.as_deref() {
        None | Some("") => return Some("it has no named sender".to_string()),
        Some("cron") => return Some("it is a cron fire".to_string()),
        _ => {}
    }
    let host = auth_via == super::ReactiveAuthVia::FullAuthKey && req.delivery_tier.as_deref() == Some("host");
    if host || super::jekt_hold::verified_peer(req) {
        return None;
    }
    let tier = req.delivery_tier.as_deref().unwrap_or("none");
    Some(match tier {
        "lan" | "wan" => format!("it came over the {tier} and its sender is not proven"),
        _ => format!("it is not from a full-key host-tier caller (tier {tier})"),
    })
}

/// One line in the log for a message the cold hold let through, with the reason.
fn delivered_because(req: &InjectionRequest, why: &str) {
    tracing::info!(
        target = %req.target_agent,
        source = req.source_agent.as_deref().unwrap_or(""),
        tier = req.delivery_tier.as_deref().unwrap_or(""),
        why,
        "reactive inject: the cold hold did not hold this message"
    );
}

/// Whether quota mode holds a routine message to a running agent: no turn in flight (a message that
/// joins a turn costs no new context read), and it does not hand the target work. `activity` is
/// `cold_hold::session_activity`'s answer, `None` with nothing running. An urgent message is never held.
pub(super) fn quota_holds(identity: &Store, req: &InjectionRequest, target_uid: &str, target_slug: &str, activity: Option<(bool, Option<i64>)>, now_ms: i64) -> bool {
    !cold_hold::asks_to_wake(req.priority.as_deref())
        && matches!(activity, Some((false, _)))
        && !hands_over_work(identity, req, target_uid, target_slug, now_ms)
}

/// Hold `req` when its target is a running Claude session gone cold, or quota mode is tight and the
/// target has no turn in flight, and the message may wait. `None` delivers it as usual, which every
/// doubt does.
pub(super) async fn hold_if_cold(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    replay: &mut replay::Reservation,
) -> Option<serde_json::Value> {
    // The messages the hold takes (`outside_the_hold`): a full-key host-tier caller, or a peer on
    // another machine that proved who it is, with an id and a named sender that is not cron. Not a
    // message an agent sends itself (its own loops and reminders). Every other one is let through
    // with its reason in the log, since a hold that does not say why it let a message by cannot be
    // checked (#2257).
    if let Some(why) = outside_the_hold(auth_via, req) {
        delivered_because(req, &why);
        return None;
    }
    let request_id = req.request_id.clone().filter(|id| !id.is_empty())?;
    let Some(route) = crate::backend::reactive::sanitize::route_name(&req.target_agent) else {
        delivered_because(req, "its target is not an agent name");
        return None;
    };
    if req.source_agent.as_deref().map(crate::backend::reactive::sanitize::route_name).flatten().as_deref() == Some(route.as_str()) {
        delivered_because(req, "the agent sent it to itself");
        return None;
    }
    let extra = state.config_watcher.get_full_config().settings.extra.clone();
    let quota_tight = crate::backend::quota_mode::is_tight(&extra);
    // The cold hold turned off leaves quota mode alone: no session is ever quiet for i64::MAX.
    let after_ms = match cold_hold::cold_after_ms(&extra) {
        Some(ms) => ms,
        None if quota_tight => i64::MAX,
        None => {
            delivered_because(req, "the cold hold is off (jekt:cold_hold_minutes is 0)");
            return None;
        }
    };
    let LookupOutcome::One(reg) = state.reactive_handler.lookup_delivery_target(&route) else {
        delivered_because(req, "its target is not exactly one running agent here");
        return None;
    };
    let Some(target_uid) = reg.uid.clone() else {
        delivered_because(req, "its target agent has no uid");
        return None;
    };
    let (mstore, identity, request) = (state.mstore.clone(), state.work_store.clone(), req.clone());
    let now = agentmux_common::time::now_ms();
    let proven_peer = super::jekt_hold::verified_peer(req);
    let held = tokio::task::spawn_blocking(move || {
        // Urgent or not routine: the sender's own words decide, so a message
        // that is not routine skips even the wake-up bookkeeping.
        if let Some(why) = cold_hold::why_not_routine(&request, proven_peer) {
            if !cold_hold::asks_to_wake(request.priority.as_deref()) {
                return Err(format!("it is not a routine message: {why}"));
            }
        }
        let last_output = cold_hold::session_last_output(&mstore, &reg.block_id);
        let (decision, why) = decide_why(&mstore, &identity, &request, &target_uid, &reg.agent_id, last_output, now, after_ms);
        let reason = match decision {
            Decision::Hold => REASON_IDLE,
            Decision::Deliver
                if quota_tight && quota_holds(&identity, &request, &target_uid, &reg.agent_id, cold_hold::session_activity(&reg.block_id), now) =>
            {
                REASON_QUOTA
            }
            Decision::Deliver => return Err(why),
        };
        let mut row = super::jekt_hold::held_row(&request, request_id, now);
        row.target_uid = target_uid;
        row.hold_reason = reason.to_string();
        match mstore.jekt_held_insert(&row) {
            Ok(HoldOutcome::Held) | Ok(HoldOutcome::AlreadyHeld) => Ok(row),
            // Full, or a store fault: deliver it rather than lose or refuse it.
            Ok(HoldOutcome::Full) | Err(_) => Err("the held-message store is full or failed, so it is delivered rather than lost".to_string()),
        }
    })
    .await;
    let held = match held {
        Ok(Ok(row)) => row,
        Ok(Err(why)) => {
            delivered_because(req, &why);
            return None;
        }
        Err(_) => return None,
    };
    replay.delivered();
    crate::backend::agent_resolve::record_uid_fallback("jekt.held");
    let reason = if held.hold_reason == REASON_QUOTA { HoldReason::Quota } else { HoldReason::Idle };
    tracing::info!(target = %req.target_agent, reason = reason.as_str(), "reactive inject: holding a routine message until the agent is next active");
    Some(held_answer(&req.target_agent, &held.request_id, reason))
}

/// The answer to a routine message held because the target's session is cold (`Idle`), or because
/// quota mode is tight and the target has no turn in flight (`Quota`).
fn held_answer(target: &str, request_id: &str, reason: HoldReason) -> serde_json::Value {
    let error = match reason {
        HoldReason::Quota => format!(
            "quota mode is tight — held until agent {target} is next active or the mode is off (for up to 24 h). \
             To deliver it now, send it again with urgent: true"
        ),
        _ => format!(
            "agent {target}'s session has been idle — held until it is next active (for up to 24 h). \
             To wake it now, send it again with urgent: true"
        ),
    };
    json!({
        "success": false,
        "held": true,
        "held_reason": reason.as_str(),
        "retry_when": reason.wait_reason().retry_when(),
        "request_id": request_id,
        "error": error,
    })
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

    fn with_id(mut r: InjectionRequest, tier: &str) -> InjectionRequest {
        r.request_id = Some("r1".into());
        r.delivery_tier = Some(tier.into());
        r
    }

    #[test]
    fn a_verified_peer_on_the_lan_is_inside_the_hold_and_an_unproven_one_says_why_not() {
        let host = super::super::ReactiveAuthVia::FullAuthKey;
        assert_eq!(outside_the_hold(host, &with_id(req("hi"), "host")), None);
        let mut lan = with_id(req("hi"), "lan");
        lan.sig_verified = None;
        let why = outside_the_hold(super::super::ReactiveAuthVia::FullAuthKey, &lan).unwrap();
        assert!(why.contains("lan") && why.contains("not proven"), "{why}");
        lan.lan_verified = Some(true);
        assert_eq!(outside_the_hold(host, &lan), None, "a LAN message whose signature checked out may wait like a local one");
    }

    #[test]
    fn a_message_outside_the_hold_names_the_reason() {
        let host = super::super::ReactiveAuthVia::FullAuthKey;
        let mut no_id = req("hi");
        no_id.delivery_tier = Some("host".into());
        assert!(outside_the_hold(host, &no_id).unwrap().contains("no request id"));
        let mut cron = with_id(req("hi"), "host");
        cron.source_agent = Some("cron".into());
        assert!(outside_the_hold(host, &cron).unwrap().contains("cron"));
        let mut nobody = with_id(req("hi"), "host");
        nobody.source_agent = None;
        assert!(outside_the_hold(host, &nobody).unwrap().contains("no named sender"));
        assert!(outside_the_hold(host, &with_id(req("hi"), "cross-channel")).unwrap().contains("cross-channel"));
    }

    #[test]
    fn a_delivered_message_says_why_it_was_not_held() {
        let (m, i, _d) = stores();
        let why = |r: &InjectionRequest, last| decide_why(&m, &i, r, "uid-l", "loap", last, NOW, AFTER).1;
        assert!(why(&req("thanks"), WARM).contains("warm"));
        assert!(why(&req("thanks"), None).contains("no output"));
        let mut urgent = req("thanks");
        urgent.priority = Some("urgent".into());
        assert!(why(&urgent, COLD).contains("urgent"));
        i.work_queue_enqueue(&task("t1", "uid-x", "uid-l", NOW - 5 * 60_000)).unwrap();
        i.work_card_create("t1", &Default::default(), 1).unwrap();
        assert!(why(&req("take a look"), COLD).contains("hands the agent work"));
        let (m2, i2, _d2) = stores();
        assert_eq!(decide_why(&m2, &i2, &req("thanks"), "uid-l", "loap", COLD, NOW, AFTER).1, "idle");
    }

    #[test]
    fn the_idle_held_answer_says_when_to_retry() {
        let a = held_answer("loap", "req-1", HoldReason::Idle);
        assert_eq!(a["held_reason"], "idle");
        assert_eq!(a["retry_when"], "session_active");
        assert_eq!(a["request_id"], "req-1");
    }

    #[test]
    fn the_quota_held_answer_names_the_mode_and_says_when_to_retry() {
        let a = held_answer("loap", "req-2", HoldReason::Quota);
        assert_eq!(a["held_reason"], "quota");
        assert_eq!(a["retry_when"], "mode_off_or_session_active");
        assert!(a["error"].as_str().unwrap().contains("urgent: true"), "{a}");
    }

    /// No turn in flight, last wrote a minute ago.
    const IDLE_NOW: Option<(bool, Option<i64>)> = Some((false, WARM));

    #[test]
    fn quota_mode_holds_a_routine_message_only_to_an_agent_with_no_turn_in_flight() {
        let (_m, i, _d) = stores();
        let holds = |r: &InjectionRequest, activity| quota_holds(&i, r, "uid-l", "loap", activity, NOW);
        assert!(holds(&req("thanks, noted"), IDLE_NOW));
        assert!(holds(&req("thanks, noted"), Some((false, None))), "no output yet is still no turn");
        assert!(!holds(&req("thanks, noted"), Some((true, WARM))), "a turn in flight: the message joins it");
        assert!(!holds(&req("thanks, noted"), None), "nothing running: not this rule's to hold");
    }

    #[test]
    fn quota_mode_never_holds_an_urgent_message_or_one_that_hands_over_work() {
        let (_m, i, _d) = stores();
        let mut urgent = req("thanks, noted");
        urgent.priority = Some("urgent".into());
        assert!(!quota_holds(&i, &urgent, "uid-l", "loap", IDLE_NOW, NOW));
        let mut high = req("thanks, noted");
        high.priority = Some("high".into());
        assert!(!quota_holds(&i, &high, "uid-l", "loap", IDLE_NOW, NOW));

        i.work_queue_enqueue(&task("t1", "uid-z", "uid-l", 1)).unwrap();
        let num = i.work_card_create("t1", &Default::default(), 1).unwrap();
        assert!(!quota_holds(&i, &req(&format!("#{num} is yours")), "uid-l", "loap", IDLE_NOW, NOW), "naming its task is work");
        assert!(quota_holds(&i, &req("#9999 is yours"), "uid-l", "loap", IDLE_NOW, NOW), "no such task");
    }

    #[test]
    fn a_block_with_no_claude_session_is_never_cold() {
        let m = Store::open_in_memory().unwrap();
        assert_eq!(cold_hold::session_last_output(&m, "no-such-block"), None);
        assert_eq!(cold_hold::session_activity("no-such-block"), None, "nothing running: the quota hold leaves it alone");
    }
}

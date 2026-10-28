// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Holding a jekt this instance can't deliver now: its agent is defined here
//! but not running (`SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md`), not signed in,
//! or out of quota (`SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §5). The replay is
//! `jekt_held`. Moved out of `reactive.rs`, whose `deliver` calls these.

use serde_json::json;

use crate::backend::reactive::InjectionRequest;

use super::reactive::{defined_here, echo_local_delivery, replay, HoldReason};
use super::AppState;

/// Hold a jekt no tier could take, when every condition of
/// `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` §2.1 holds: a full-key,
/// host-tier request (a LAN caller never fills the hold); not a periodic
/// `cron` fire; and a target that is a known agent of this channel —
/// resolved to its row's UID, so a typo or a name registered later by
/// another block never receives the backlog. `None` keeps today's error.
///
/// The same rules hold a jekt the target's spawn gate refused
/// ([`HoldReason::NeedsLogin`]); the replay then waits for sign-in
/// (`jekt_held::still_needs_login`). Delivered by UID or held, a message
/// keeps its one-delivery `replay` reservation.
pub(in crate::server) async fn hold_for_absent_target(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
    reason: HoldReason,
    replay: &mut replay::Reservation,
) -> Option<serde_json::Value> {
    hold_with_reason(state, auth_via, req, resp, reason, None, replay).await
}

/// Why messages to the agent on `block_id` wait now: it is signed out, or
/// out of quota until a known reset. A confirmed sign-in clears auth failures.
/// Limits without a reset still allow turns to discover recovery. While a probe turn
/// runs (`limit_probe`) the pane counts as working, but its limit still holds
/// until that turn clears it. BLOCKING.
pub(in crate::server) fn holding_unavailable(
    mstore: &crate::backend::storage::store::Store,
    block_id: &str,
) -> Option<crate::backend::agent_state::Unavailable> {
    let now_ms = agentmux_common::time::now_ms_u64();
    use crate::backend::agent_availability::{of_unavailable, verdict, Verdict, WaitReason};
    crate::backend::agent_state::unavailable_of_block(mstore, None, block_id, now_ms)
        .filter(|unavailable| {
            let rule = verdict(&of_unavailable(unavailable), false, true, None, || false);
            matches!(rule, Verdict::Wait { reason: WaitReason::UsageLimit | WaitReason::NeedsLogin, .. })
        })
        .or_else(|| super::limit_probe::unavailable_while_probing(mstore, block_id, now_ms))
}

/// Whether a peer's answer to a forwarded jekt is a refusal by an instance
/// that has the agent (its files refused, or the agent out of quota), to be
/// returned as is: not a stale route to evict and route around.
pub(super) fn peer_refused(body: &serde_json::Value) -> bool {
    body.get("files_refused").and_then(|v| v.as_bool()) == Some(true)
        || body.get("refused_reason").and_then(|v| v.as_str()).is_some()
        || body.get("error").and_then(|v| v.as_str()).is_some_and(crate::backend::reactive::is_unavailable_refusal)
}

/// [`holding_unavailable`] for the agent of this instance that `target`
/// names. `None` for an agent that can work, and for a target that isn't
/// registered here (an absent or remote one: the tiers below decide, and the
/// receiving instance checks again).
pub(super) async fn local_target_unavailable(state: &AppState, target: &str) -> Option<crate::backend::agent_state::Unavailable> {
    use crate::backend::reactive::handler::LookupOutcome;
    let target = crate::backend::reactive::sanitize::route_name(target)?;
    let LookupOutcome::One(reg) = state.reactive_handler.lookup_delivery_target(&target) else {
        return None;
    };
    let mstore = state.mstore.clone();
    tokio::task::spawn_blocking(move || holding_unavailable(&mstore, &reg.block_id)).await.ok().flatten()
}

/// What delivery checks before a message reaches a running agent: unavailable (held, or
/// refused where it can't be held), or a session gone cold (`jekt_cold`). `None` delivers.
pub(super) async fn hold_before_delivery(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    replay: &mut replay::Reservation,
) -> Option<serde_json::Value> {
    if let Some(unavailable) = local_target_unavailable(state, &req.target_agent).await {
        return Some(hold_for_unavailable(state, auth_via, req, &unavailable, replay).await);
    }
    super::jekt_cold::hold_if_cold(state, auth_via, req, replay).await
}

/// Hold a message for an agent that can't work now (`unavailable`), or,
/// where the hold doesn't apply (a LAN or cross-channel hop, a cron fire),
/// refuse it with the reason: either way it is not injected into a turn
/// that would fail and lose it. The refusal begins with
/// [`crate::backend::reactive::UNAVAILABLE_PREFIX`].
pub(super) async fn hold_for_unavailable(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    unavailable: &crate::backend::agent_state::Unavailable,
    replay: &mut replay::Reservation,
) -> serde_json::Value {
    use crate::backend::agent_state::AgentState;
    let reason = HoldReason::for_state(unavailable.state).unwrap_or(HoldReason::Limited);
    let target = &req.target_agent;
    let why = match (unavailable.state, unavailable.resets_at_ms.and_then(local_time_label)) {
        (AgentState::SignedOut, _) => format!("{target} needs to sign in again"),
        (_, Some(until)) => format!("{target}'s account is out of quota until {until}"),
        (_, None) => format!("{target}'s account is out of quota"),
    };
    let resp = crate::backend::reactive::types::InjectionResponse {
        deferred: None,
        success: false,
        request_id: req.request_id.clone().filter(|id| !id.is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        block_id: None,
        error: Some(format!("{}{why}", crate::backend::reactive::UNAVAILABLE_PREFIX)),
        timestamp: agentmux_common::time::now_ms_u64(),
        effective_tier: None,
        requires_stop: None,
        channel_verified: None,
    };
    tracing::info!(target = %target, state = ?unavailable.state, "reactive inject: target can't work now, holding or refusing");
    if let Some(held) = hold_with_reason(state, auth_via, req, &resp, reason, unavailable.resets_at_ms, replay).await {
        if held.get("held").and_then(|h| h.as_bool()) == Some(true) {
            super::limit_probe::maybe_probe(state, req, unavailable).await;
        }
        return held;
    }
    with_refusal(serde_json::to_value(&resp).unwrap_or_default(), reason.wait_reason(), unavailable.resets_at_ms)
}

/// `body`, the answer to a message refused (not kept) because the agent can't take it now, with
/// the reason as data: `refused_reason` (the `held_reason` strings), `retry_when` and, when
/// known, `until_ms`. A sender reads these instead of the wording of `error`.
pub(super) fn with_refusal(
    mut body: serde_json::Value,
    reason: crate::backend::agent_availability::WaitReason,
    until_ms: Option<u64>,
) -> serde_json::Value {
    body["refused_reason"] = json!(reason.as_str());
    body["retry_when"] = json!(reason.retry_when());
    if let Some(until) = until_ms {
        body["until_ms"] = json!(until);
    }
    body
}

/// [`with_refusal`] for a spawn gate that refused: the agent is not signed in.
pub(super) fn needs_login_refusal(body: serde_json::Value) -> serde_json::Value {
    with_refusal(body, crate::backend::agent_availability::WaitReason::NeedsLogin, None)
}

/// "Sun 7:00 PM": a reset time in this machine's zone, as the Kanban and
/// Swarm show it (`agent_state::Unavailable::reason`).
fn local_time_label(ms: u64) -> Option<String> {
    let t = chrono::DateTime::from_timestamp_millis(i64::try_from(ms).ok()?)?;
    Some(t.with_timezone(&chrono::Local).format("%a %-I:%M %p").to_string())
}

/// [`hold_for_absent_target`], with the reset time of a usage limit for the
/// answer (`held_until_ms`).
async fn hold_with_reason(
    state: &AppState,
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
    reason: HoldReason,
    until_ms: Option<u64>,
    replay: &mut replay::Reservation,
) -> Option<serde_json::Value> {
    use crate::backend::storage::jekt_held::HoldOutcome;
    if !hold_applies(auth_via, req, resp) {
        return None;
    }
    let mut held = held_row(req, resp.request_id.clone(), agentmux_common::time::now_ms());
    let mstore = state.mstore.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let (uid, _slug) = defined_here(&mstore, &held.target_agent)?;
        held.target_uid = uid;
        Some(held)
    })
    .await
    .ok()
    .flatten()?;
    // Registered under another of its names (the handler binds only the
    // display and stable ones): it is running, so deliver by UID now rather
    // than tell the sender it is not. Not for a spawn-gate refusal: that
    // target is registered, and a retry now would only be refused again.
    if reason == HoldReason::NotRunning && state.reactive_handler.has_uid_registration(&outcome.target_uid) {
        let mut by_uid = req.clone();
        by_uid.target_agent = outcome.target_uid.clone();
        let resp = state.reactive_handler.inject_message(by_uid);
        if resp.success {
            replay.delivered();
            // As addressed, so the sender's echo names whom it wrote to
            // (ReAgent P1 on #3632: this path skipped the echo).
            echo_local_delivery(state, req, &resp);
        }
        // A refusal is the target's answer; but if it unregistered between
        // the check and the delivery, fall through and hold it (Codex P2).
        let raced_away = resp.error.as_deref().is_some_and(|e| e.starts_with("agent not found"));
        if !raced_away {
            return Some(serde_json::to_value(&resp).unwrap_or_default());
        }
    }
    let target_uid = outcome.target_uid.clone();
    let mstore = state.mstore.clone();
    let outcome = tokio::task::spawn_blocking(move || mstore.jekt_held_insert(&outcome))
        .await
        .ok()?;
    let target = &req.target_agent;
    match outcome {
        Ok(HoldOutcome::Held) | Ok(HoldOutcome::AlreadyHeld) => {
            replay.delivered();
            crate::backend::agent_resolve::record_uid_fallback("jekt.held");
            if reason == HoldReason::NotRunning {
                super::jekt_start::start_for_held(state, &target_uid);
            }
            let error = match reason {
                HoldReason::NotRunning => format!(
                    "agent {target} is not running — held for delivery on this AgentMux instance for up to 24 h"
                ),
                HoldReason::NeedsLogin => {
                    if resp.error.as_deref().is_some_and(super::jekt_held::is_spawn_gate_refusal) {
                        super::jekt_held::mark_needs_login(&target_uid);
                    }
                    format!(
                        "agent {target} is not signed in — held for delivery on this AgentMux instance for up to 24 h, delivered when it signs in"
                    )
                }
                HoldReason::Limited => format!(
                    "agent {target}'s account is out of quota — held for delivery on this AgentMux instance for up to 24 h, delivered when it can work again (an account move or the reset)"
                ),
                // Held by `jekt_cold`, which has its own answer; never through here.
                HoldReason::Idle => format!("agent {target}'s session is idle — held until it is next active"),
            };
            let mut answer = json!({
                "success": false,
                "held": true,
                "held_reason": reason.as_str(),
                "retry_when": reason.wait_reason().retry_when(),
                "request_id": resp.request_id,
                "error": error,
            });
            if let Some(until) = until_ms.filter(|_| reason == HoldReason::Limited) {
                answer["held_until_ms"] = json!(until);
                if let Some(label) = local_time_label(until) {
                    answer["held_until"] = json!(label);
                }
            }
            Some(answer)
        }
        Ok(HoldOutcome::Full) => Some(json!({
            "success": false,
            "request_id": resp.request_id,
            "error": format!(
                "{} (hold full: too many messages already held for {target})",
                resp.error.clone().unwrap_or_default()
            ),
        })),
        Err(e) => {
            tracing::warn!(error = %e, target = %target, "durable jekt: hold failed");
            None
        }
    }
}

/// The row to keep for `req`: what the recipient would be given, with the
/// verdicts it was accepted with. `target_uid` is filled in by the caller.
pub(super) fn held_row(req: &InjectionRequest, request_id: String, now: i64) -> crate::backend::storage::jekt_held::HeldJekt {
    use crate::backend::storage::jekt_held::{HeldJekt, HELD_TTL_MS};
    HeldJekt {
        request_id,
        target_uid: String::new(),
        target_agent: req.target_agent.clone(),
        source_agent: req.source_agent.clone().unwrap_or_default(),
        audit_source_uid: req.audit_source_uid.clone(),
        // What the recipient would receive — sanitized and truncated to the
        // delivery limit — never the unbounded body (Codex P2 on #3632).
        message: crate::backend::reactive::sanitize::sanitize_message(&req.message),
        priority: req.priority.clone().unwrap_or_default(),
        jekt_tier: req
            .jekt_tier
            .as_ref()
            .and_then(|t| serde_json::to_value(t).ok())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        delivery_tier: "host".to_string(),
        sig_verified: req.sig_verified,
        reagent_verified: req.reagent_verified,
        lan_verified: req.lan_verified,
        channel_verified: req.channel_verified,
        is_transcript_request: req.is_transcript_request,
        transcript_request_escalate_forced: req.transcript_request_escalate_forced,
        files_json: req
            .files
            .as_ref()
            .and_then(|f| serde_json::to_string(f).ok())
            .unwrap_or_default(),
        sent_at_ms: now,
        expires_at_ms: now + HELD_TTL_MS,
        attempts: 0,
        last_error: String::new(),
        hold_reason: String::new(),
    }
}

/// The conditions under which this instance may hold a jekt at all (durable
/// jekt spec §2.1): a full-key, host-tier request (a LAN caller never fills
/// the hold); not a periodic `cron` fire; and an id to key the row by.
pub(super) fn hold_applies(
    auth_via: super::ReactiveAuthVia,
    req: &InjectionRequest,
    resp: &crate::backend::reactive::types::InjectionResponse,
) -> bool {
    auth_via == super::ReactiveAuthVia::FullAuthKey
        && req.delivery_tier.as_deref() == Some("host")
        && req.source_agent.as_deref() != Some("cron")
        && !resp.request_id.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_availability::WaitReason;

    #[test]
    fn a_refusal_carries_its_reason_and_what_ends_the_wait_as_data() {
        let body = with_refusal(json!({"success": false, "error": "agent unavailable: X needs to sign in again"}), WaitReason::NeedsLogin, None);
        assert_eq!((body["refused_reason"].as_str(), body["retry_when"].as_str()), (Some("needs_login"), Some("sign_in")));
        assert!(body.get("until_ms").is_none());
        let limited = with_refusal(json!({"success": false}), WaitReason::UsageLimit, Some(42));
        assert_eq!((limited["refused_reason"].as_str(), limited["retry_when"].as_str(), limited["until_ms"].as_u64()), (Some("usage_limit"), Some("reset_or_account_move"), Some(42)));
    }

    #[test]
    fn a_peer_refusal_is_known_by_its_reason_or_by_the_old_wording() {
        assert!(peer_refused(&json!({"refused_reason": "usage_limit", "error": "anything"})));
        assert!(peer_refused(&json!({"error": "agent unavailable: X's account is out of quota"})));
        assert!(peer_refused(&json!({"files_refused": true})));
        assert!(!peer_refused(&json!({"error": "agent not found: X"})));
    }
}

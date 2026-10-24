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

/// Why messages to the agent on `block_id` wait now: its account is out of
/// quota (`limited`). BLOCKING.
///
/// Not `signed_out` yet: that state lasts until a clean turn, and a sign-in
/// to the same account doesn't clear it, so holding every message would
/// keep the turn that clears it from ever running (muxreview P1 on #5419).
/// It joins once a confirmed sign-in clears the failure (Kanban #2237).
pub(in crate::server) fn holding_unavailable(
    mstore: &crate::backend::storage::store::Store,
    block_id: &str,
) -> Option<crate::backend::agent_state::Unavailable> {
    crate::backend::agent_state::unavailable_of_block(mstore, None, block_id, agentmux_common::time::now_ms_u64())
        .filter(|u| u.state == crate::backend::agent_state::AgentState::Limited)
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
        return held;
    }
    serde_json::to_value(&resp).unwrap_or_default()
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
    use crate::backend::storage::jekt_held::{HeldJekt, HoldOutcome, HELD_TTL_MS};
    if !hold_applies(auth_via, req, resp) {
        return None;
    }
    let now = agentmux_common::time::now_ms();
    let mut held = HeldJekt {
        request_id: resp.request_id.clone(),
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
    };
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
            let error = match reason {
                HoldReason::NotRunning => format!(
                    "agent {target} is not running — held for delivery on this AgentMux instance for up to 24 h"
                ),
                HoldReason::NeedsLogin => {
                    // The send itself was refused just now: the replay waits
                    // for sign-in rather than retrying at once.
                    super::jekt_held::mark_needs_login(&target_uid);
                    format!(
                        "agent {target} is not signed in — held for delivery on this AgentMux instance for up to 24 h, delivered when it signs in"
                    )
                }
                HoldReason::Limited => format!(
                    "agent {target}'s account is out of quota — held for delivery on this AgentMux instance for up to 24 h, delivered when it can work again (an account move or the reset)"
                ),
            };
            let mut answer = json!({
                "success": false,
                "held": true,
                "held_reason": reason.as_str(),
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

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Replay of jekts held for an absent agent —
//! `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` §2.3. The hold itself is in
//! `reactive::deliver`; this delivers what was held once the agent is back.

use std::time::Duration;

use crate::backend::reactive::types::{InjectionRequest, JektTier};
use crate::backend::storage::jekt_held::HeldJekt;

use super::AppState;

/// Deliveries per pass, so a backlog cannot monopolise the handler (and the
/// first-contact spawn it may trigger) in one go.
const REPLAY_PER_PASS: usize = 8;
/// Failed deliveries to a present target before a held message is dropped.
const MAX_ATTEMPTS: i64 = 20;
const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// Replay held jekts: once now, then whenever an agent registers and every
/// 30 s. Installed after `AppState` exists.
pub(crate) fn install(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            replay_pass(&state).await;
            tokio::select! {
                _ = crate::backend::reactive::held_jekt_wake().notified() => {}
                _ = tokio::time::sleep(SWEEP_EVERY) => {}
            }
        }
    });
}

/// How one replay attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayOutcome {
    Delivered,
    /// Still absent, rate-limited, or its queue full — try again later, no
    /// attempt counted.
    Deferred,
    /// Refused while present; counted.
    Failed,
    /// Refused `MAX_ATTEMPTS` times; dropped.
    Dropped,
}

/// One pass: drop expired rows, then deliver to every target that is now
/// registered, oldest first, at most [`REPLAY_PER_PASS`]. A target that is
/// still absent costs a map lookup — no rate-limit token, no audit entry.
pub(crate) async fn replay_pass(state: &AppState) -> Vec<(String, ReplayOutcome)> {
    let mstore = state.mstore.clone();
    let now = agentmux_common::time::now_ms();
    let targets = tokio::task::spawn_blocking(move || {
        match mstore.jekt_held_delete_expired(now) {
            Ok(n) if n > 0 => {
                for _ in 0..n {
                    crate::backend::agent_resolve::record_uid_fallback("jekt.held_expired");
                }
                tracing::info!(expired = n, "durable jekt: held messages expired");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "durable jekt: expiry failed"),
        }
        mstore.jekt_held_targets().unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    // Rotate the starting target every pass, so targets that keep failing
    // cannot take the budget ahead of a healthy one forever (Codex P2).
    let mut targets = targets;
    if !targets.is_empty() {
        static ROTATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = targets.len();
        targets.rotate_left(ROTATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % n);
    }
    let mut outcomes = Vec::new();
    let mut spent = 0;
    for uid in targets {
        if spent >= REPLAY_PER_PASS {
            break;
        }
        if !state.reactive_handler.has_uid_registration(&uid) {
            continue;
        }
        let mstore = state.mstore.clone();
        let budget = REPLAY_PER_PASS - spent;
        let rows = tokio::task::spawn_blocking(move || mstore.jekt_held_for_target(&uid, budget))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        for row in rows {
            let id = row.request_id.clone();
            let outcome = replay_one(state, row).await;
            outcomes.push((id, outcome));
            // A deferral (queue full, starting) applies to this target's
            // later rows too: move on, spending no budget. A failure also
            // ends this target's turn, spending one slot — so a target that
            // keeps refusing takes at most one slot per pass and cannot
            // starve the others (Codex P2 on #3632).
            match outcome {
                ReplayOutcome::Deferred => break,
                ReplayOutcome::Failed | ReplayOutcome::Dropped => {
                    spent += 1;
                    break;
                }
                ReplayOutcome::Delivered => spent += 1,
            }
        }
    }
    outcomes
}

/// The request a held row is delivered as: addressed by the target's UID,
/// with the verdicts it was accepted with — never re-verified (a signature
/// is valid for five minutes) and never raised — and its original send time.
pub(crate) fn request_from_held(row: &HeldJekt) -> InjectionRequest {
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let mut req = InjectionRequest {
        target_agent: row.target_uid.clone(),
        message: row.message.clone(),
        source_agent: non_empty(&row.source_agent),
        request_id: Some(row.request_id.clone()),
        priority: non_empty(&row.priority),
        jekt_tier: non_empty(&row.jekt_tier)
            .and_then(|t| serde_json::from_value::<JektTier>(serde_json::Value::String(t)).ok()),
        delivery_tier: Some("host".to_string()),
        ..Default::default()
    };
    req.sig_verified = row.sig_verified;
    req.reagent_verified = row.reagent_verified;
    req.lan_verified = row.lan_verified;
    req.channel_verified = row.channel_verified;
    req.is_transcript_request = row.is_transcript_request;
    req.transcript_request_escalate_forced = row.transcript_request_escalate_forced;
    req.audit_source_uid = row.audit_source_uid.clone();
    req.held_sent_at_ms = Some(row.sent_at_ms);
    req
}

async fn replay_one(state: &AppState, row: HeldJekt) -> ReplayOutcome {
    // The transcript-request fields are restored as accepted, never
    // recomputed: the replay addresses the target by UID, and the slug-keyed
    // recompute would lose a forced escalation.
    let req = request_from_held(&row);
    let resp = state.reactive_handler.inject_held(req);
    let mstore = state.mstore.clone();
    let id = row.request_id.clone();
    if resp.success {
        let _ = tokio::task::spawn_blocking(move || mstore.jekt_held_delete(&id)).await;
        crate::backend::agent_resolve::record_uid_fallback("jekt.held_delivered");
        return ReplayOutcome::Delivered;
    }
    let error = resp.error.unwrap_or_default();
    // Transient: still absent, rate-limited, its queue full, or the agent
    // starting, restarting or stopping ("… try again shortly").
    let transient = error.starts_with("agent not found")
        || error.contains("rate limit")
        || error.contains("queue is full")
        || error.contains("try again shortly");
    if transient {
        return ReplayOutcome::Deferred;
    }
    let attempts = tokio::task::spawn_blocking({
        let mstore = mstore.clone();
        let id = id.clone();
        let error = error.clone();
        // A store fault is not a failed delivery: never drop on it.
        move || mstore.jekt_held_record_failure(&id, &error).unwrap_or(0)
    })
    .await
    .unwrap_or(0);
    if attempts >= MAX_ATTEMPTS {
        let _ = tokio::task::spawn_blocking(move || mstore.jekt_held_delete(&id)).await;
        crate::backend::agent_resolve::record_uid_fallback("jekt.held_failed");
        tracing::warn!(target_uid = %row.target_uid, error = %error, "durable jekt: dropped after repeated failures");
        return ReplayOutcome::Dropped;
    }
    ReplayOutcome::Failed
}

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
/// How often a target the spawn gate refused is re-checked for sign-in.
const GATE_RECHECK_EVERY: Duration = Duration::from_secs(60);

/// Whether a delivery error is the identity/credential spawn gate refusing to
/// start the target (`run_agent_turn`'s "identity spawn gate: …"). That is
/// recoverable — the receiver signs in — so the message is held, not dropped
/// (SPEC_JEKT_DELIVERY_STATES_AND_MAILBOX_2026_10_01.md Phase 0 item 4, G4).
pub(crate) fn is_spawn_gate_refusal(error: &str) -> bool {
    error.starts_with("identity spawn gate")
}

/// Targets the spawn gate refused, with when the gate was last checked.
///
/// A delivery attempt to such a target is not free: `run_agent_turn` writes an
/// error frame into its pane and raises a failure event each time. So the
/// replay never attempts one blind; it first runs the gate on its own, which
/// has no side effect beyond a log line, at most every [`GATE_RECHECK_EVERY`],
/// and delivers once it passes. In memory: after a restart the first replay
/// attempt finds the gate closed again and re-marks the target, one frame.
fn needs_login() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>> {
    static MAP: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
}

/// Mark `uid` as refused by the spawn gate, checked just now.
pub(crate) fn mark_needs_login(uid: &str) {
    needs_login().lock().unwrap().insert(uid.to_string(), std::time::Instant::now());
}

/// What the replay should do about a target's sign-in, given when its gate was
/// last checked (`None`: never refused).
#[derive(Debug, PartialEq, Eq)]
enum GateCheck {
    /// Not refused before: deliver as usual.
    Open,
    /// Checked within [`GATE_RECHECK_EVERY`]: skip without checking.
    Wait,
    /// Due: run the gate.
    Recheck,
}

fn gate_check_due(last: Option<std::time::Instant>, now: std::time::Instant) -> GateCheck {
    match last {
        None => GateCheck::Open,
        Some(t) if now.duration_since(t) < GATE_RECHECK_EVERY => GateCheck::Wait,
        Some(_) => GateCheck::Recheck,
    }
}

/// Whether `uid` is still waiting for sign-in. Runs the spawn gate quietly
/// (no pane frame, no failure event) when a recheck is due.
async fn still_needs_login(state: &AppState, uid: &str) -> bool {
    let last = needs_login().lock().unwrap().get(uid).copied();
    match gate_check_due(last, std::time::Instant::now()) {
        GateCheck::Open => return false,
        GateCheck::Wait => return true,
        GateCheck::Recheck => {}
    }
    let Some(block_id) = state.reactive_handler.block_for_uid(uid) else {
        return true;
    };
    let (mstore, id_store, identity_store) =
        (state.mstore.clone(), state.id_store.clone(), state.identity_store.clone());
    let passed = tokio::task::spawn_blocking(move || {
        crate::identity::inject_identity_env(mstore, id_store, identity_store, &block_id, &mut Default::default())
            .is_ok()
    })
    .await
    // A panicked check never opens the gate.
    .unwrap_or(false);
    if passed {
        needs_login().lock().unwrap().remove(uid);
        tracing::info!(target_uid = %uid, "durable jekt: signed in, delivering held messages");
        return false;
    }
    mark_needs_login(uid);
    true
}

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
        if still_needs_login(state, &uid).await {
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
    // Not restored: only host-tier jekts are held (`hold_for_absent_target`)
    // and reagent verification is WAN-only, so a held row never carries a
    // real reagent verdict — and a replayed `Some(true)` without its key id
    // would read as a failed verification.
    req.reagent_verified = None;
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
    // Not signed in: wait for sign-in, uncounted (G6 counted it, so a
    // signed-out receiver lost its messages after 20 tries).
    if is_spawn_gate_refusal(&error) {
        mark_needs_login(&row.target_uid);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_spawn_gate_refusal_is_recognised_and_nothing_else_is() {
        assert!(is_spawn_gate_refusal("identity spawn gate: no credentials for claude: …"));
        for other in ["agent not found: x", "rate limit exceeded", "persistent process not running", ""] {
            assert!(!is_spawn_gate_refusal(other), "{other}");
        }
    }

    /// A refused target is re-checked at most every GATE_RECHECK_EVERY, so the
    /// 30 s replay never attempts a delivery blind (each would write an error
    /// frame into the receiver's pane).
    #[test]
    fn the_sign_in_check_runs_at_most_once_a_minute() {
        let now = Instant::now();
        assert_eq!(gate_check_due(None, now), GateCheck::Open);
        assert_eq!(gate_check_due(Some(now), now), GateCheck::Wait);
        assert_eq!(gate_check_due(Some(now), now + Duration::from_secs(59)), GateCheck::Wait);
        assert_eq!(gate_check_due(Some(now), now + GATE_RECHECK_EVERY), GateCheck::Recheck);
    }
}

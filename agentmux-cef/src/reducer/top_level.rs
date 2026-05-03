// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Top-level window creation runner (Phase H.6) reducer handlers. Extracted from reducer/mod.rs in
//! task #182 PR-F-2 for navigability.

use std::time::Instant;

use crate::state::*;

use super::{DispatchOutput, HostEvent, HostState, emit_error, TOP_LEVEL_CREATION_HISTORY_CAP};

// ── H.6 — top-level window creation runner ───────────────────────────────

pub(super) fn handle_enqueue_top_level_window(
    state: &mut HostState,
    request: TopLevelCreationRequest,
) -> DispatchOutput {
    if state.quit_state != QuitState::Running {
        return emit_error(state, format!("enqueue_top_level_window: not Running (label={})", request.label));
    }

    // Fail-fast for User-initiated requests when in-flight is occupied.
    // Background (pool refill) requests queue silently.
    if state.top_level_creation.in_flight.is_some()
        && request.source == TopLevelSource::User
    {
        return emit_error(state, format!("enqueue_top_level_window: busy in-flight (label={})", request.label));
    }

    state.top_level_creation.queue.push_back(request);
    let queue_len = state.top_level_creation.queue.len();
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelQueueLengthChanged { len: queue_len, version: v }],
        ..Default::default()
    };
    // If idle, start immediately; chain the start arm's events.
    if state.top_level_creation.in_flight.is_none() {
        let started = start_next_top_level_if_idle(state);
        out.events.extend(started.events);
    }
    out
}

/// Internal helper: if in_flight is None and queue has work, pop the front
/// and start it. Emits `TopLevelCreationRequested`, `TopLevelCreationStarted`,
/// `Effect::PostCreateWindow`, and updated queue length.
///
/// **Quit gating** (codex P1 PR #654 round 1): if `quit_state != Running`,
/// don't start anything — queued background requests stay queued but will
/// never fire (host is exiting; in-memory queue dies with the process).
/// Without this guard, an in-flight completion during `Draining` would pop
/// a queued background pool refill and emit `Effect::PostCreateWindow`,
/// creating a new window mid-shutdown and preventing drain completion.
pub(super) fn start_next_top_level_if_idle(state: &mut HostState) -> DispatchOutput {
    if state.top_level_creation.in_flight.is_some() {
        return DispatchOutput::default();
    }
    if state.quit_state != QuitState::Running {
        return DispatchOutput::default();
    }
    let request = match state.top_level_creation.queue.pop_front() {
        Some(r) => r,
        None => return DispatchOutput::default(),
    };
    state.top_level_creation.next_creation_id =
        state.top_level_creation.next_creation_id.wrapping_add(1);
    let creation_id = state.top_level_creation.next_creation_id;
    let now = Instant::now();
    state.top_level_creation.in_flight = Some(InFlightCreation {
        creation_id,
        label: request.label.clone(),
        started_at: now,
        phase: CreationPhase::Started,
    });
    let label = request.label.clone();
    let source = request.source.clone();
    let queue_len = state.top_level_creation.queue.len();
    let v_req = state.bump_version();
    let v_started = state.bump_version();
    let v_eff = state.bump_version();
    let v_qlen = state.bump_version();
    DispatchOutput {
        events: vec![
            HostEvent::TopLevelCreationRequested {
                creation_id,
                source,
                label: label.clone(),
                version: v_req,
            },
            HostEvent::TopLevelCreationStarted {
                creation_id,
                label: label.clone(),
                version: v_started,
            },
            HostEvent::Effect {
                effect: EffectKind::PostCreateWindow { request, creation_id },
                version: v_eff,
            },
            HostEvent::TopLevelQueueLengthChanged { len: queue_len, version: v_qlen },
        ],
        ..Default::default()
    }
}

pub(super) fn handle_top_level_callback_fired(state: &mut HostState, label: String) -> DispatchOutput {
    let matches_in_flight = state
        .top_level_creation
        .in_flight
        .as_ref()
        .map(|c| c.label == label)
        .unwrap_or(false);
    if !matches_in_flight {
        // Orphan callback: a CEF browser fired on_after_created with a
        // label we don't have in flight. Could be from a previously-evicted
        // creation (won't happen in PR #1 since we don't evict) or a stale
        // label. Emit an effect to close the orphan, preventing collision.
        let orphan_browser = state.browsers.get(&label).map(|h| h.browser.clone());
        if let Some(browser) = orphan_browser {
            let v = state.bump_version();
            return DispatchOutput {
                events: vec![HostEvent::Effect {
                    effect: EffectKind::CloseOrphanBrowser { browser },
                    version: v,
                }],
                ..Default::default()
            };
        }
        return DispatchOutput::default();
    }
    let inflight = state.top_level_creation.in_flight.take().unwrap();
    let now = Instant::now();
    let latency_ms = now.duration_since(inflight.started_at).as_millis() as u64;
    push_top_level_history(
        state,
        CompletedCreation {
            creation_id: inflight.creation_id,
            label: inflight.label.clone(),
            outcome: TopLevelCreationOutcome::Completed,
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: CreationPhase::BrowserCallbackFired,
        },
    );
    let v_done = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelCreationCompleted {
            creation_id: inflight.creation_id,
            label: inflight.label,
            latency_ms,
            version: v_done,
        }],
        ..Default::default()
    };
    let next = start_next_top_level_if_idle(state);
    out.events.extend(next.events);
    out
}

pub(super) fn handle_top_level_renderer_terminated(
    state: &mut HostState,
    label: String,
    status: String,
) -> DispatchOutput {
    let matches = state
        .top_level_creation
        .in_flight
        .as_ref()
        .map(|c| c.label == label)
        .unwrap_or(false);
    if !matches {
        return DispatchOutput::default();
    }
    let inflight = state.top_level_creation.in_flight.take().unwrap();
    let now = Instant::now();
    let outcome = TopLevelCreationOutcome::RendererTerminated { status };
    push_top_level_history(
        state,
        CompletedCreation {
            creation_id: inflight.creation_id,
            label: inflight.label.clone(),
            outcome: outcome.clone(),
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: inflight.phase,
        },
    );
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelCreationFailed {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome,
            version: v,
        }],
        ..Default::default()
    };
    let next = start_next_top_level_if_idle(state);
    out.events.extend(next.events);
    out
}

pub(super) fn handle_top_level_externally_closed(state: &mut HostState, label: String) -> DispatchOutput {
    let matches = state
        .top_level_creation
        .in_flight
        .as_ref()
        .map(|c| c.label == label)
        .unwrap_or(false);
    if !matches {
        return DispatchOutput::default();
    }
    let inflight = state.top_level_creation.in_flight.take().unwrap();
    let now = Instant::now();
    let outcome = TopLevelCreationOutcome::ExternallyClosed;
    push_top_level_history(
        state,
        CompletedCreation {
            creation_id: inflight.creation_id,
            label: inflight.label.clone(),
            outcome: outcome.clone(),
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: inflight.phase,
        },
    );
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelCreationFailed {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome,
            version: v,
        }],
        ..Default::default()
    };
    let next = start_next_top_level_if_idle(state);
    out.events.extend(next.events);
    out
}

pub(super) fn push_top_level_history(state: &mut HostState, entry: CompletedCreation) {
    if state.top_level_creation.history.len() >= TOP_LEVEL_CREATION_HISTORY_CAP {
        state.top_level_creation.history.pop_front();
    }
    state.top_level_creation.history.push_back(entry);
}


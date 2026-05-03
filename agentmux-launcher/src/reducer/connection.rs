// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Connection-lifecycle reducer handlers. Extracted from
//! reducer/mod.rs in task #182 PR-E for navigability.
//!
//! Covers: register, goodbye, get_snapshot, plus the
//! enforce_host_only gate and host_is_running predicate that
//! window/saga handlers also call (via super::).

use agentmux_common::ipc::{ClientKind, ErrorCode, Event};

use crate::reducer::Ctx;
use crate::state::{LifecyclePhase, ProcessRecord, ProcessState, State};

/// Phase B.4 — gate window-mirror reports to Host clients only. The
/// host is the only source of truth about its own window lifecycle;
/// allowing Renderer/Srv/Tool clients to mutate the mirror would let
/// any registered process spoof open/close traffic and break the
/// host-authoritative model. (codex P1 PR #576 round-1.)
///
/// Returns `Some(Error)` if the calling connection is not a Host;
/// `None` if the call is allowed to proceed. Looks up the kind by
/// PID rather than threading it through `Ctx` because `processes`
/// is already the canonical source — single source of truth, no
/// extra plumbing.
pub(super) fn enforce_host_only(state: &mut State, ctx: &Ctx, op: &'static str) -> Option<Event> {
    let kind = ctx
        .registered_pid
        .and_then(|pid| state.processes.get(&pid).map(|r| r.kind));
    if kind == Some(ClientKind::Host) {
        return None;
    }
    let v = state.bump_version();
    Some(Event::Error {
        code: ErrorCode::NotRegistered,
        message: format!(
            "{} is Host-only; caller kind={:?}",
            op, kind
        ),
        fatal: false,
        version: v,
    })
}


/// Phase B.9.3 — does state.processes contain a Host in the
/// `Running` lifecycle? Used by the OrphanInstance transition
/// check; without this guard, a stale Exited record would fire
/// HostShouldQuit on every benign close. Tool-only sessions
/// (`agentmux.exe --diag`) also correctly skip the saga because
/// they never register a Host.
pub(super) fn host_is_running(state: &State) -> bool {
    use crate::state::ProcessState;
    state.processes.values().any(|r| {
        r.kind == ClientKind::Host && matches!(r.state, ProcessState::Running)
    })
}

/// Phase D.1 — clone the reducer's canonical state into a `Snapshot`
/// event. Read-only; doesn't mutate state except for bumping
/// `event_version` so the snapshot's version is monotonically
/// distinct from prior events (subscribers applying snapshot + delta
/// events know the snapshot is "as-of" this version).
///
/// Sorted-vec serialization (rather than HashMap-as-JSON-object) for:
/// 1. Deterministic ordering across snapshots (idempotent diffs in
///    operator output, easier test assertions).
/// 2. Wire compatibility with `Vec<(K, V)>` decoders that don't
///    require canonical-string-keyed JSON objects.
pub(super) fn handle_get_snapshot(state: &mut State) -> Vec<Event> {
    let v = state.bump_version();

    let mut windows: Vec<agentmux_common::ipc::WindowSnapshot> = state
        .windows
        .values()
        .map(|w| agentmux_common::ipc::WindowSnapshot {
            label: w.label.clone(),
            kind: w.kind,
            parent_label: w.parent_label.clone(),
            hwnd: w.hwnd,
            visible: w.visible,
            iconic: w.iconic,
            last_rect: w.last_rect,
            foregrounded_since_open: w.foregrounded_since_open,
        })
        .collect();
    windows.sort_by(|a, b| a.label.cmp(&b.label));

    let mut pool: Vec<String> = state.pool.iter().cloned().collect();
    pool.sort();

    let mut instance_registry: Vec<(String, u32)> =
        state.instance_registry.iter().map(|(k, v)| (k.clone(), *v)).collect();
    instance_registry.sort_by(|a, b| a.0.cmp(&b.0));

    let mut backend_window_ids: Vec<(String, String)> = state
        .backend_window_ids
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    backend_window_ids.sort_by(|a, b| a.0.cmp(&b.0));

    vec![Event::Snapshot {
        version: v,
        lifecycle: state.lifecycle,
        windows,
        pool,
        instance_registry,
        backend_window_ids,
        monitors: state.monitors.clone(),
    }]
}

pub(super) fn handle_register(
    state: &mut State,
    ctx: &Ctx,
    kind: ClientKind,
    pid: u32,
    version: String,
) -> Vec<Event> {
    let mut out = Vec::with_capacity(3);

    // Cross-connection invariant: only ONE live ProcessRecord per
    // PID. We DO allow re-registration if the existing record is
    // Exited — the OS recycles PIDs over a long-running launcher,
    // so a new process can legitimately end up with a PID that was
    // previously held by a process that has cleanly Goodbye'd.
    // Without this carve-out, the process map would accumulate dead
    // records and the launcher would reject increasingly many real
    // registrations. (gemini MEDIUM PR #574 round-1.)
    let existing_state = state.processes.get(&pid).map(|r| r.state);
    if let Some(existing_state) = existing_state {
        if !matches!(existing_state, ProcessState::Exited { .. }) {
            let v = state.bump_version();
            out.push(Event::Error {
                code: ErrorCode::AlreadyRegistered,
                message: format!(
                    "pid {} already in process registry (state={:?})",
                    pid, existing_state
                ),
                fatal: true,
                version: v,
            });
            return out;
        }
        // Else: fall through. The insert below replaces the Exited
        // record with the new live one — same entry, fresh state.
    }

    let record = ProcessRecord {
        pid,
        kind,
        state: ProcessState::Running,
        spawned_at: ctx.now_rfc3339.clone(),
        version: version.clone(),
    };
    state.processes.insert(pid, record);

    let spawned_v = state.bump_version();
    out.push(Event::ProcessSpawned {
        pid,
        kind,
        client_version: version,
        version: spawned_v,
    });

    // Lifecycle: Starting → Running when the first Host registers.
    // Subsequent Host re-registers (after a host crash + restart in
    // some future world) won't double-fire because we'd already be
    // in Running. Other client kinds (Renderer, Srv, Tool) don't
    // drive the transition.
    if state.lifecycle == LifecyclePhase::Starting && kind == ClientKind::Host {
        let from = state.lifecycle;
        state.lifecycle = LifecyclePhase::Running;
        let v = state.bump_version();
        out.push(Event::LifecyclePhaseChanged {
            from,
            to: LifecyclePhase::Running,
            version: v,
        });
    }

    let registered_v = state.bump_version();
    let client_id = state.alloc_client_id();
    out.push(Event::Registered {
        client_id,
        // launcher_pid + launcher_version are filled in by the
        // server before broadcast — they don't belong in the pure
        // reducer (env reads). We use a sentinel here; the server
        // patches these before sending.
        launcher_pid: 0,
        launcher_version: String::new(),
        version: registered_v,
    });

    out
}

pub(super) fn handle_goodbye(state: &mut State, pid: u32) -> Vec<Event> {
    if pid == 0 {
        // No pid known for this connection (B.3 limitation). Just
        // emit a synthetic ProcessExited with pid=0 to signal the
        // graceful close; the server will log + close.
        let v = state.bump_version();
        return vec![Event::ProcessExited {
            pid: 0,
            code: 0,
            version: v,
        }];
    }
    if let Some(record) = state.processes.get_mut(&pid) {
        record.state = ProcessState::Exited { code: 0 };
    }
    let v = state.bump_version();
    vec![Event::ProcessExited {
        pid,
        code: 0,
        version: v,
    }]
}

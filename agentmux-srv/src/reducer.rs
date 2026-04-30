// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv reducer skeleton.
//
// Pure functional core: `update(&mut State, Command, &Ctx) -> Vec<Event>`.
// Never blocks, never awaits, never does I/O. Same discipline as
// `agentmux-launcher::reducer`. Mutex held only during dispatch
// (sub-millisecond).
//
// E.1b arms: Register / Goodbye / Ping / GetSrvSnapshot / GetEvents.
// E.2+ adds domain commands (CreateWorkspace, CreateTab, etc.).
//
// `Command::GetEvents` is intercepted by the IPC server before
// reaching the reducer (server queries the event log; reducer
// stays pure). The reducer's arm exists only for match
// exhaustiveness; same pattern as the launcher reducer.

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event, LifecyclePhase};

use crate::state::{ProcessRecord, ProcessState, State, WorkspaceRecord};

/// Per-dispatch context. Currently just an RFC3339 timestamp + the
/// originating connection's `conn_id` for log correlation.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub now_rfc3339: String,
    pub conn_id: u64,
    pub registered_pid: Option<u32>,
}

pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    match cmd {
        Command::Register { kind, pid, version } => handle_register(state, ctx, kind, pid, version),
        Command::Goodbye => handle_goodbye(state, ctx),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        Command::GetSrvSnapshot => handle_get_srv_snapshot(state),
        Command::GetEvents { .. } => Vec::new(), // intercepted by server; unreachable
        Command::CreateWorkspace { name } => handle_create_workspace(state, name),
        Command::DeleteWorkspace { workspace_id } => handle_delete_workspace(state, workspace_id),
        // E.1b accepts only the above; everything else is a non-fatal
        // protocol error. E.2+ adds domain commands as new arms.
        other => {
            let v = state.bump_version();
            vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!("srv reducer does not accept: {:?}", other),
                fatal: false,
                version: v,
            }]
        }
    }
}

fn handle_register(
    state: &mut State,
    ctx: &Ctx,
    kind: ClientKind,
    pid: u32,
    version: String,
) -> Vec<Event> {
    // Idempotent on duplicate Register from the same PID — preserve
    // the original record (per launcher's pattern). Accept fresh
    // Registers only when the PID has no record OR the existing
    // record is Exited (PID recycled).
    let prior_state = state.processes.get(&pid).map(|r| r.state);
    let allow_register = match prior_state {
        None => true,
        Some(ProcessState::Exited { .. }) => true,
        _ => false,
    };
    if !allow_register {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::AlreadyRegistered,
            message: format!("pid {} is already registered with srv", pid),
            fatal: false,
            version: v,
        }];
    }

    let mut out = Vec::with_capacity(3);

    // Insert the process record.
    state.processes.insert(
        pid,
        ProcessRecord {
            pid,
            kind,
            state: ProcessState::Running,
            spawned_at: ctx.now_rfc3339.clone(),
            version: version.clone(),
        },
    );
    let v = state.bump_version();
    out.push(Event::ProcessSpawned {
        pid,
        kind,
        client_version: version,
        version: v,
    });

    // First Register transitions srv to Running. Same lifecycle
    // pattern as launcher.
    if state.lifecycle == LifecyclePhase::Starting {
        state.lifecycle = LifecyclePhase::Running;
        let v = state.bump_version();
        out.push(Event::LifecyclePhaseChanged {
            from: LifecyclePhase::Starting,
            to: LifecyclePhase::Running,
            version: v,
        });
    }

    let client_id = state.alloc_client_id();
    let v = state.bump_version();
    // Sentinel launcher_pid / launcher_version on Registered — IPC
    // server patches these to the real srv identity before broadcast.
    out.push(Event::Registered {
        client_id,
        launcher_pid: 0,
        launcher_version: String::new(),
        version: v,
    });
    out
}

fn handle_goodbye(state: &mut State, ctx: &Ctx) -> Vec<Event> {
    let Some(pid) = ctx.registered_pid else {
        return Vec::new();
    };
    let Some(record) = state.processes.get_mut(&pid) else {
        return Vec::new();
    };
    if matches!(record.state, ProcessState::Exited { .. }) {
        return Vec::new();
    }
    record.state = ProcessState::Exited { code: 0 };
    let v = state.bump_version();
    vec![Event::ProcessExited {
        pid,
        code: 0,
        version: v,
    }]
}

fn handle_get_srv_snapshot(state: &mut State) -> Vec<Event> {
    let v = state.bump_version();
    let mut workspaces: Vec<(String, String)> = state
        .workspaces
        .values()
        .map(|w| (w.workspace_id.clone(), w.name.clone()))
        .collect();
    // Stable ordering for diffability — reducer state is HashMap so
    // iteration order is non-deterministic.
    workspaces.sort_by(|a, b| a.0.cmp(&b.0));
    vec![Event::SrvSnapshot {
        version: v,
        lifecycle: state.lifecycle,
        workspaces,
    }]
}

/// Phase E.2 — create a new workspace. Reducer assigns the OID
/// (UUID), inserts into canonical state, emits WorkspaceCreated.
/// NOT idempotent on retry: each invocation generates a fresh UUID
/// and inserts a new row, so a saga that double-fires CreateWorkspace
/// would create two distinct workspaces. Saga-side dedup (correlation
/// IDs / saga state machine) is responsible for at-most-once delivery
/// when sagas land in E.5+.
fn handle_create_workspace(state: &mut State, name: String) -> Vec<Event> {
    let workspace_id = uuid::Uuid::new_v4().to_string();
    state.workspaces.insert(
        workspace_id.clone(),
        WorkspaceRecord {
            workspace_id: workspace_id.clone(),
            name: name.clone(),
        },
    );
    let v = state.bump_version();
    vec![Event::WorkspaceCreated {
        workspace_id,
        name,
        version: v,
    }]
}

/// Phase E.2 — delete a workspace from canonical state. Idempotent:
/// deleting a missing workspace is a silent no-op. Cascade to the
/// workspace's tabs is E.2b territory (tab arms not yet present).
fn handle_delete_workspace(state: &mut State, workspace_id: String) -> Vec<Event> {
    if state.workspaces.remove(&workspace_id).is_none() {
        return Vec::new();
    }
    let v = state.bump_version();
    vec![Event::WorkspaceDeleted {
        workspace_id,
        version: v,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(conn_id: u64) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-30T00:00:00Z".to_string(),
            conn_id,
            registered_pid: None,
        }
    }

    fn ctx_with_pid(conn_id: u64, pid: u32) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-30T00:00:00Z".to_string(),
            conn_id,
            registered_pid: Some(pid),
        }
    }

    fn extract_version(e: &Event) -> u64 {
        match e {
            Event::ProcessSpawned { version, .. }
            | Event::ProcessExited { version, .. }
            | Event::LifecyclePhaseChanged { version, .. }
            | Event::Registered { version, .. }
            | Event::Pong { version, .. }
            | Event::WindowOpened { version, .. }
            | Event::WindowClosed { version, .. }
            | Event::PoolWindowAdded { version, .. }
            | Event::PoolWindowRemoved { version, .. }
            | Event::WindowInstanceAssigned { version, .. }
            | Event::WindowInstanceReleased { version, .. }
            | Event::BackendWindowIdRegistered { version, .. }
            | Event::BackendWindowIdUnregistered { version, .. }
            | Event::DriftDetected { version, .. }
            | Event::HwndDriftDetected { version, .. }
            | Event::CorrectiveWindowMove { version, .. }
            | Event::HostShouldQuit { version, .. }
            | Event::Snapshot { version, .. }
            | Event::EventList { version, .. }
            | Event::SrvSnapshot { version, .. }
            | Event::SagaStarted { version, .. }
            | Event::SagaCompleted { version, .. }
            | Event::SagaFailed { version, .. }
            | Event::WorkspaceCreated { version, .. }
            | Event::WorkspaceDeleted { version, .. }
            | Event::Error { version, .. } => *version,
        }
    }

    #[test]
    fn first_register_transitions_to_running() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "0.0.0".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        assert!(state.processes.contains_key(&100));
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], Event::ProcessSpawned { pid: 100, .. }));
        assert!(matches!(
            &events[1],
            Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                ..
            }
        ));
        assert!(matches!(&events[2], Event::Registered { .. }));
    }

    #[test]
    fn second_register_does_not_re_emit_lifecycle_change() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Tool,
                pid: 200,
                version: "v2".into(),
            },
            &ctx(2),
        );
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        // ProcessSpawned + Registered, no LifecyclePhaseChanged.
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|e| !matches!(e, Event::LifecyclePhaseChanged { .. })));
    }

    #[test]
    fn duplicate_register_returns_already_registered_error() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v2".into(),
            },
            &ctx(2),
        );
        assert!(matches!(
            &events[0],
            Event::Error {
                code: ErrorCode::AlreadyRegistered,
                ..
            }
        ));
        // Original record preserved.
        assert_eq!(&state.processes[&100].version, "v1");
    }

    #[test]
    fn register_replaces_exited_record_for_recycled_pid() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let _ = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 100));
        // Re-register with same PID — allowed because prior is Exited.
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v2".into(),
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::ProcessSpawned { .. }));
        assert_eq!(&state.processes[&100].version, "v2");
        assert!(matches!(state.processes[&100].state, ProcessState::Running));
    }

    #[test]
    fn ping_returns_pong_with_same_nonce() {
        let mut state = State::default();
        let events = update(&mut state, Command::Ping { nonce: 42 }, &ctx(1));
        assert!(matches!(&events[0], Event::Pong { nonce: 42, .. }));
    }

    #[test]
    fn get_srv_snapshot_returns_lifecycle_and_bumps_version() {
        let mut state = State::default();
        let v0 = state.event_version;
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(1));
        assert_eq!(events.len(), 1);
        let Event::SrvSnapshot { version, lifecycle, .. } = events[0].clone() else {
            panic!("expected SrvSnapshot, got {:?}", events[0]);
        };
        assert_eq!(lifecycle, LifecyclePhase::Starting);
        assert!(version > v0);
    }

    #[test]
    fn unaccepted_command_returns_invalid_command_error() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: agentmux_common::ipc::WindowKind::FullInstance,
                parent_label: None,
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error {
                code: ErrorCode::InvalidCommand,
                fatal: false,
                ..
            }
        ));
    }

    #[test]
    fn versions_strictly_monotonic_across_sequence() {
        let mut state = State::default();
        let mut versions = vec![];
        for pid in [100, 200, 300] {
            let events = update(
                &mut state,
                Command::Register {
                    kind: ClientKind::Host,
                    pid,
                    version: "v".into(),
                },
                &ctx(pid as u64),
            );
            versions.extend(events.iter().map(extract_version));
        }
        for w in versions.windows(2) {
            assert!(w[1] > w[0], "version regression: {} -> {}", w[0], w[1]);
        }
    }

    #[test]
    fn create_workspace_inserts_record_and_emits_event() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateWorkspace {
                name: "myws".into(),
            },
            &ctx(1),
        );
        assert_eq!(events.len(), 1);
        let Event::WorkspaceCreated {
            workspace_id, name, ..
        } = &events[0]
        else {
            panic!("expected WorkspaceCreated, got {:?}", events[0]);
        };
        assert_eq!(name, "myws");
        assert!(state.workspaces.contains_key(workspace_id));
        assert_eq!(state.workspaces[workspace_id].name, "myws");
    }

    #[test]
    fn delete_workspace_removes_record_and_emits_event() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateWorkspace {
                name: "to-delete".into(),
            },
            &ctx(1),
        );
        let Event::WorkspaceCreated { workspace_id, .. } = &events[0] else {
            panic!();
        };
        let ws_id = workspace_id.clone();
        let events = update(
            &mut state,
            Command::DeleteWorkspace {
                workspace_id: ws_id.clone(),
            },
            &ctx(2),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::WorkspaceDeleted { workspace_id, .. } if workspace_id == &ws_id
        ));
        assert!(!state.workspaces.contains_key(&ws_id));
    }

    #[test]
    fn delete_workspace_unknown_is_silent_no_op() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::DeleteWorkspace {
                workspace_id: "does-not-exist".into(),
            },
            &ctx(1),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn snapshot_includes_workspaces_sorted_by_id() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::CreateWorkspace { name: "a".into() },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateWorkspace { name: "b".into() },
            &ctx(2),
        );
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(3));
        let Event::SrvSnapshot { workspaces, .. } = &events[0] else {
            panic!();
        };
        assert_eq!(workspaces.len(), 2);
        // Sorted by id; verify ordering deterministic (ascending).
        assert!(workspaces[0].0 < workspaces[1].0);
    }

    #[test]
    fn goodbye_marks_process_exited() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v".into(),
            },
            &ctx(1),
        );
        let events = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 100));
        assert!(matches!(
            &events[0],
            Event::ProcessExited { pid: 100, code: 0, .. }
        ));
        assert!(matches!(
            state.processes[&100].state,
            ProcessState::Exited { code: 0 }
        ));
    }
}

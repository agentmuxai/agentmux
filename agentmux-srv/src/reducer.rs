// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E — srv reducer.
//
// Pure functional core: `update(&mut State, Command, &Ctx) -> Vec<Event>`.
// Never blocks, never awaits, never does I/O. Same discipline as
// `agentmux-launcher::reducer`. Mutex held only during dispatch
// (sub-millisecond).
//
// Arms by phase:
//   * E.1b — Register / Goodbye / Ping / GetSrvSnapshot / GetEvents
//   * E.2  — CreateWorkspace / DeleteWorkspace
//   * E.2b — CreateTab / DeleteTab / SetActiveTab
//   * E.3+ — Block / Layout commands (not yet present)
//
// `Command::GetEvents` is intercepted by the IPC server before
// reaching the reducer (server queries the event log; reducer
// stays pure). The reducer's arm exists only for match
// exhaustiveness; same pattern as the launcher reducer.

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event, LifecyclePhase};

use crate::state::{ProcessRecord, ProcessState, State, TabRecord, WorkspaceRecord};

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
        Command::CreateTab { workspace_id, name } => handle_create_tab(state, workspace_id, name),
        Command::DeleteTab { workspace_id, tab_id } => handle_delete_tab(state, workspace_id, tab_id),
        Command::SetActiveTab { workspace_id, tab_id } => {
            handle_set_active_tab(state, workspace_id, tab_id)
        }
        // Anything else is a non-fatal protocol error. Future
        // phases (E.2b tabs, E.3 blocks, E.4 layouts) extend this
        // match by adding new arms above.
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
    let mut tabs: Vec<(String, String, String)> = state
        .tabs
        .values()
        .map(|t| (t.tab_id.clone(), t.workspace_id.clone(), t.name.clone()))
        .collect();
    tabs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut active_tabs: Vec<(String, String)> = state
        .workspaces
        .values()
        .filter_map(|w| {
            w.active_tab_id
                .as_ref()
                .map(|t| (w.workspace_id.clone(), t.clone()))
        })
        .collect();
    active_tabs.sort_by(|a, b| a.0.cmp(&b.0));
    vec![Event::SrvSnapshot {
        version: v,
        lifecycle: state.lifecycle,
        workspaces,
        tabs,
        active_tabs,
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
            tab_ids: Vec::new(),
            active_tab_id: None,
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
/// deleting a missing workspace is a silent no-op. Cascades to the
/// workspace's tabs (E.2b): every tab whose `workspace_id` matches
/// is removed from `state.tabs` before the workspace itself goes
/// away. Cascade events are NOT emitted individually — subscribers
/// observing `WorkspaceDeleted` are expected to drop dependent state
/// (mirrors how `wcore::delete_workspace` cascades in SQLite).
fn handle_delete_workspace(state: &mut State, workspace_id: String) -> Vec<Event> {
    let Some(removed) = state.workspaces.remove(&workspace_id) else {
        return Vec::new();
    };
    for tab_id in &removed.tab_ids {
        state.tabs.remove(tab_id);
    }
    let v = state.bump_version();
    vec![Event::WorkspaceDeleted {
        workspace_id,
        version: v,
    }]
}

/// Phase E.2b — create a tab inside a workspace. Validates the
/// parent exists; otherwise emits `Event::Error` (non-fatal). On
/// success: assigns a UUID, appends to the workspace's `tab_ids`,
/// inserts into `state.tabs`, emits `Event::TabCreated`. If the
/// workspace had no active tab, the new tab also becomes active
/// and an `Event::ActiveTabChanged` is emitted alongside.
///
/// NOT idempotent on retry (same UUID-assignment caveat as
/// `handle_create_workspace`).
fn handle_create_tab(state: &mut State, workspace_id: String, name: String) -> Vec<Event> {
    if !state.workspaces.contains_key(&workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("CreateTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    }
    let tab_id = uuid::Uuid::new_v4().to_string();
    state.tabs.insert(
        tab_id.clone(),
        TabRecord {
            tab_id: tab_id.clone(),
            workspace_id: workspace_id.clone(),
            name: name.clone(),
        },
    );
    let workspace = state.workspaces.get_mut(&workspace_id).expect("checked");
    workspace.tab_ids.push(tab_id.clone());
    let activated = if workspace.active_tab_id.is_none() {
        workspace.active_tab_id = Some(tab_id.clone());
        true
    } else {
        false
    };
    let mut events = Vec::with_capacity(2);
    let v = state.bump_version();
    events.push(Event::TabCreated {
        workspace_id: workspace_id.clone(),
        tab_id: tab_id.clone(),
        name,
        version: v,
    });
    if activated {
        let v2 = state.bump_version();
        events.push(Event::ActiveTabChanged {
            workspace_id,
            tab_id: Some(tab_id),
            version: v2,
        });
    }
    events
}

/// Phase E.2b — delete a tab from a workspace. Idempotent: deleting
/// a missing tab is a silent no-op. If the deleted tab was the
/// active tab, the workspace's active tab becomes the next tab in
/// `tab_ids` (or the previous one if the deleted was last; or None
/// if the workspace is now empty), and an `Event::ActiveTabChanged`
/// is emitted alongside `Event::TabDeleted`.
fn handle_delete_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        return Vec::new();
    };
    let Some(pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        return Vec::new();
    };
    workspace.tab_ids.remove(pos);
    let active_changed = if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        let new_active = workspace
            .tab_ids
            .get(pos)
            .or_else(|| pos.checked_sub(1).and_then(|i| workspace.tab_ids.get(i)))
            .cloned();
        workspace.active_tab_id = new_active.clone();
        Some(new_active)
    } else {
        None
    };
    state.tabs.remove(&tab_id);
    let mut events = Vec::with_capacity(2);
    let v = state.bump_version();
    events.push(Event::TabDeleted {
        workspace_id: workspace_id.clone(),
        tab_id,
        version: v,
    });
    if let Some(new_active) = active_changed {
        let v2 = state.bump_version();
        events.push(Event::ActiveTabChanged {
            workspace_id,
            tab_id: new_active,
            version: v2,
        });
    }
    events
}

/// Phase E.2b — set a workspace's active tab. No-op if already
/// active. Errors (non-fatal) if the workspace doesn't exist or the
/// tab isn't in that workspace's tab list.
fn handle_set_active_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetActiveTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    if !workspace.tab_ids.iter().any(|t| t == &tab_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "SetActiveTab: tab {} not in workspace {}",
                tab_id, workspace_id
            ),
            fatal: false,
            version: v,
        }];
    }
    if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        return Vec::new();
    }
    workspace.active_tab_id = Some(tab_id.clone());
    let v = state.bump_version();
    vec![Event::ActiveTabChanged {
        workspace_id,
        tab_id: Some(tab_id),
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
            | Event::TabCreated { version, .. }
            | Event::TabDeleted { version, .. }
            | Event::ActiveTabChanged { version, .. }
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

    fn create_workspace(state: &mut State, name: &str) -> String {
        let events = update(
            state,
            Command::CreateWorkspace { name: name.into() },
            &ctx(1),
        );
        match &events[0] {
            Event::WorkspaceCreated { workspace_id, .. } => workspace_id.clone(),
            _ => panic!("expected WorkspaceCreated"),
        }
    }

    #[test]
    fn create_tab_validates_workspace_exists() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: "no-such-ws".into(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn create_tab_first_tab_becomes_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        // First event: TabCreated; second: ActiveTabChanged.
        assert!(matches!(&events[0], Event::TabCreated { .. }));
        assert!(matches!(&events[1], Event::ActiveTabChanged { .. }));
        let workspace = &state.workspaces[&ws_id];
        assert_eq!(workspace.tab_ids.len(), 1);
        assert_eq!(workspace.active_tab_id, Some(workspace.tab_ids[0].clone()));
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn create_tab_second_tab_does_not_steal_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let first_active = state.workspaces[&ws_id].active_tab_id.clone();
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        // Only TabCreated — second tab does not become active.
        assert!(matches!(&events[0], Event::TabCreated { .. }));
        assert_eq!(events.len(), 1);
        assert_eq!(state.workspaces[&ws_id].active_tab_id, first_active);
        assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 2);
    }

    #[test]
    fn delete_tab_removes_from_state_and_workspace_list() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab2_id.clone(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        // tab2 wasn't active, so no ActiveTabChanged.
        assert_eq!(events.len(), 1);
        assert!(!state.tabs.contains_key(&tab2_id));
        assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 1);
    }

    #[test]
    fn delete_active_tab_promotes_neighbor() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
        // tab1 was created first → it's active. Delete it.
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab1_id.clone(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        assert!(matches!(
            &events[1],
            Event::ActiveTabChanged { tab_id: Some(_), .. }
        ));
        // tab2 should now be active.
        assert_eq!(state.workspaces[&ws_id].active_tab_id, Some(tab2_id));
    }

    #[test]
    fn delete_last_tab_clears_active_to_none() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab1_id,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        assert!(matches!(
            &events[1],
            Event::ActiveTabChanged { tab_id: None, .. }
        ));
        assert_eq!(state.workspaces[&ws_id].active_tab_id, None);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn delete_unknown_tab_silent_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
            },
            &ctx(1),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn set_active_tab_validates_workspace_and_tab() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        // Wrong workspace.
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: "no-such".into(),
                tab_id: "x".into(),
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        // Right workspace, wrong tab.
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn set_active_tab_idempotent_no_event_when_already_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
        // Already active (auto-activated on first tab create).
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: ws_id,
                tab_id,
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn delete_workspace_cascades_tabs() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        assert_eq!(state.tabs.len(), 2);
        let events = update(
            &mut state,
            Command::DeleteWorkspace {
                workspace_id: ws_id.clone(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::WorkspaceDeleted { .. }));
        assert!(state.tabs.is_empty());
        assert!(!state.workspaces.contains_key(&ws_id));
    }

    #[test]
    fn snapshot_includes_tabs_and_active_tabs() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(2));
        let Event::SrvSnapshot {
            tabs, active_tabs, ..
        } = &events[0]
        else {
            panic!("expected SrvSnapshot");
        };
        assert_eq!(tabs.len(), 1);
        assert_eq!(active_tabs.len(), 1);
        assert_eq!(active_tabs[0].0, ws_id);
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

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared test fixtures for the `reducer::*` submodule test suites. Only
// compiled under `#[cfg(test)]`. `pub(crate)` so sibling `reducer::*::tests`
// modules can pull these in via `use crate::reducer::test_support::*;`.

use agentmux_common::ipc::{Command, Event};

use crate::reducer::{update, Ctx};
use crate::state::State;

pub(crate) fn ctx(_conn_id: u64) -> Ctx {
    Ctx {
        now_rfc3339: "2026-04-30T00:00:00Z".to_string(),
        registered_pid: None,
    }
}

pub(crate) fn ctx_with_pid(_conn_id: u64, pid: u32) -> Ctx {
    Ctx {
        now_rfc3339: "2026-04-30T00:00:00Z".to_string(),
        registered_pid: Some(pid),
    }
}

pub(crate) fn extract_version(e: &Event) -> u64 {
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
        | Event::PoolWindowPromoted { version, .. }
        | Event::PanesReaped { version, .. }
        | Event::PoolDrained { version, .. }
        | Event::PoolNotLast { version, .. }
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
        | Event::TabReordered { version, .. }
        | Event::BlockCreated { version, .. }
        | Event::BlockDeleted { version, .. }
        | Event::SrvWindowOpened { version, .. }
        | Event::SrvWindowClosed { version, .. }
        | Event::SrvWindowWorkspaceChanged { version, .. }
        | Event::TabsReorderedBulk { version, .. }
        | Event::WorkspaceRenamed { version, .. }
        | Event::TabRenamed { version, .. }
        | Event::WorkspaceMetaUpdated { version, .. }
        | Event::TabMetaUpdated { version, .. }
        | Event::BlockMetaUpdated { version, .. }
        | Event::WindowMetaUpdated { version, .. }
        | Event::TabMoved { version, .. }
        | Event::BlockMoved { version, .. }
        | Event::FocusedNodeChanged { version, .. }
        | Event::MagnifiedNodeChanged { version, .. }
        | Event::SagaActionFailed { version, .. }
        | Event::Error { version, .. }
        // Phase E.4.B — layout tree events.
        | Event::LayoutNodeInserted { version, .. }
        | Event::LayoutNodeInsertedAtIndex { version, .. }
        | Event::LayoutNodeDeleted { version, .. }
        | Event::LayoutNodeMoved { version, .. }
        | Event::LayoutNodesSwapped { version, .. }
        | Event::LayoutNodesResized { version, .. }
        | Event::LayoutNodeReplaced { version, .. }
        | Event::LayoutSplitHorizontalApplied { version, .. }
        | Event::LayoutSplitVerticalApplied { version, .. }
        | Event::LayoutCleared { version, .. }
        | Event::LayoutBackendActionsQueued { version, .. }
        | Event::LayoutTreeReplaced { version, .. } => *version,
    }
}

pub(crate) fn create_workspace(state: &mut State, name: &str) -> String {
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

pub(crate) fn create_tab(state: &mut State, workspace_id: &str, name: &str) -> String {
    let events = update(
        state,
        Command::CreateTab {
            workspace_id: workspace_id.into(),
            name: name.into(),
        },
        &ctx(1),
    );
    match &events[0] {
        Event::TabCreated { tab_id, .. } => tab_id.clone(),
        _ => panic!("expected TabCreated"),
    }
}

pub(crate) fn create_block(state: &mut State, tab_id: &str) -> String {
    let events = update(
        state,
        Command::CreateBlock {
            tab_id: tab_id.into(),
            meta: serde_json::Value::Null,
        },
        &ctx(1),
    );
    match &events[0] {
        Event::BlockCreated { block_id, .. } => block_id.clone(),
        _ => panic!("expected BlockCreated"),
    }
}

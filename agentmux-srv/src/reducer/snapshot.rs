// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{Command, ErrorCode, Event};

use crate::state::State;

use super::Ctx;

pub(super) fn handle_get_srv_snapshot(state: &mut State) -> Vec<Event> {
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
    let mut blocks: Vec<(String, String)> = state
        .blocks
        .values()
        .map(|b| (b.block_id.clone(), b.tab_id.clone()))
        .collect();
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    vec![Event::SrvSnapshot {
        version: v,
        lifecycle: state.lifecycle,
        workspaces,
        tabs,
        active_tabs,
        blocks,
    }]
}

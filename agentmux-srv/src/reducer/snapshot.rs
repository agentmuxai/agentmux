// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::Event;

use crate::state::State;


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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::test_support::*;
    use crate::reducer::update;
    use agentmux_common::ipc::Command;

    #[test]
    fn get_srv_snapshot_returns_lifecycle_and_bumps_version() {
        let mut state = State::default();
        let v0 = state.event_version;
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(1));
        assert_eq!(events.len(), 1);
        let Event::SrvSnapshot { version, lifecycle, .. } = events[0].clone() else {
            panic!("expected SrvSnapshot, got {:?}", events[0]);
        };
        assert_eq!(lifecycle, agentmux_common::ipc::LifecyclePhase::Starting);
        assert!(version > v0);
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
    fn snapshot_includes_blocks() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id, meta: serde_json::Value::Null },
            &ctx(2),
        );
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(3));
        let Event::SrvSnapshot { blocks, .. } = &events[0] else {
            panic!("expected SrvSnapshot");
        };
        assert_eq!(blocks.len(), 1);
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
}

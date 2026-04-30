// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2 — bootstrap helper: load SQLite-persistent state into
// the srv reducer at startup. The reducer's state is a SESSION-only
// projection in E.2/E.2b — it's populated from SQLite at boot,
// mutated by pipe-originated commands during the session, and
// discarded on restart (the next bootstrap re-reads SQLite).
//
// HTTP/WS RPC continues to write to SQLite directly via wcore. So
// SQLite stays authoritative for the duration of the session even
// though the reducer's view diverges as soon as a pipe command
// runs. That's intentional: pipe-originated commands have no
// client populating them yet (saga coordinator is empty in E.1a;
// E.5+ adds saga consumers). Once those exist, E.2c adds the
// persist subscriber that mirrors pipe-event effects back to SQLite.
//
// This module DOES NOT define a persist subscriber. The HWM /
// broadcast-lag concerns codex flagged are deferred to E.2c when
// the subscriber actually exists.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::backend::obj::{Tab, Workspace};
use crate::backend::storage::wstore::WaveStore;
use crate::state::{State, TabRecord, WorkspaceRecord};

/// Phase E.2 / E.2b — load workspaces and their tabs from SQLite
/// into the reducer state. Called once at srv startup before the
/// IPC server starts accepting commands. Async because we're inside
/// the tokio runtime.
///
/// Errors are logged but non-fatal: if a SQLite read fails (fresh
/// install, empty DB, transient I/O), the reducer starts with
/// whatever loaded successfully. Workspace and tab loads are
/// independent — a workspace-load failure does not prevent the tab
/// load from being attempted (and vice versa), since pipe commands
/// later in the session can populate either map.
pub async fn bootstrap_state_from_wstore(state: &Arc<Mutex<State>>, wstore: &WaveStore) {
    let workspaces = wstore.get_all::<Workspace>().unwrap_or_else(|e| {
        tracing::warn!(
            target: "srv-persist",
            "[srv-persist] bootstrap: failed to load workspaces from wstore: {} — workspaces start empty",
            e
        );
        Vec::new()
    });
    let tabs = wstore.get_all::<Tab>().unwrap_or_else(|e| {
        tracing::warn!(
            target: "srv-persist",
            "[srv-persist] bootstrap: failed to load tabs from wstore: {} — tabs start empty",
            e
        );
        Vec::new()
    });
    let mut state = state.lock().await;
    for ws in &workspaces {
        // The persistent `Workspace` carries two ordered lists:
        // `tabids` (regular tabs) and `pinnedtabids` (sticky tabs).
        // Both are equally "owned by this workspace" for reducer
        // purposes — only their UX semantics differ. Concatenate
        // pinned-then-regular (pinning convention puts pinned tabs
        // first), then filter against the tabs we actually loaded
        // to drop dangling references defensively. (codex P1 #612.)
        let tab_ids: Vec<String> = ws
            .pinnedtabids
            .iter()
            .chain(ws.tabids.iter())
            .filter(|tid| tabs.iter().any(|t| &t.oid == *tid))
            .cloned()
            .collect();
        let active_tab_id = if !ws.activetabid.is_empty()
            && tab_ids.iter().any(|tid| tid == &ws.activetabid)
        {
            Some(ws.activetabid.clone())
        } else {
            None
        };
        state.workspaces.insert(
            ws.oid.clone(),
            WorkspaceRecord {
                workspace_id: ws.oid.clone(),
                name: ws.name.clone(),
                tab_ids,
                active_tab_id,
            },
        );
    }
    for tab in &tabs {
        // Each tab needs to know its parent workspace_id, which the
        // persistent `Tab` struct doesn't carry directly — we recover
        // it from whichever workspace lists this tab id in EITHER
        // `tabids` OR `pinnedtabids`. Tabs whose parent isn't loaded
        // are skipped (orphans). (codex P1 #612.)
        let Some(workspace_id) = workspaces
            .iter()
            .find(|ws| {
                ws.tabids
                    .iter()
                    .chain(ws.pinnedtabids.iter())
                    .any(|tid| tid == &tab.oid)
            })
            .map(|ws| ws.oid.clone())
        else {
            tracing::warn!(
                target: "srv-persist",
                "[srv-persist] bootstrap: tab {} has no parent workspace — skipping",
                tab.oid
            );
            continue;
        };
        state.tabs.insert(
            tab.oid.clone(),
            TabRecord {
                tab_id: tab.oid.clone(),
                workspace_id,
                name: tab.name.clone(),
            },
        );
    }
    tracing::info!(
        target: "srv-persist",
        "[srv-persist] bootstrap loaded {} workspace(s) + {} tab(s) from wstore",
        state.workspaces.len(),
        state.tabs.len()
    );
}

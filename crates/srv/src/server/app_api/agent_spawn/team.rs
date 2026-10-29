// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a leader's workers open: a tab of their own, "<Leader> team", in the leader's workspace,
//! made the first time and reused after. Without it a leader that lives in the user's main tab
//! fills that tab with its workers.

use crate::backend::obj::{Tab, Workspace};
use crate::backend::service::WebCallType;
use crate::backend::storage::store::Store;

use super::*;

/// The tab named `name` among `workspace`'s tabs (any case), if there is one.
fn tab_named(mstore: &Store, workspace: &Workspace, name: &str) -> Option<String> {
    workspace.tabids.iter().find(|id| mstore.get::<Tab>(id).ok().flatten().is_some_and(|t| t.name.trim().eq_ignore_ascii_case(name))).cloned()
}

/// The tab `leader`'s workers open in. `leaders_tab` (where the leader's pane is, or where it works from
/// when parked) is the way out: if the workspace can't be found or the tab can't be made, the workers
/// open there as they did before. Both callers hold the leader's lock (`leader_lock`) from before the
/// user is asked until the last worker is open, so one leader's find-or-make never overlaps itself.
pub(super) async fn team_tab(state: &AppState, leader: &Leader, leaders_tab: &str) -> String {
    let Some(workspace) = state.mstore.get_all::<Workspace>().unwrap_or_default().into_iter().find(|w| w.tabids.iter().any(|t| t == leaders_tab)) else {
        return leaders_tab.to_string();
    };
    let name = format!("{} team", leader.def.name.trim());
    if let Some(existing) = tab_named(&state.mstore, &workspace, &name) {
        return existing;
    }
    // Not shown: the user's view stays on the tab they are in.
    let call = WebCallType {
        service: "workspace".to_string(),
        method: "CreateTab".to_string(),
        uicontext: None,
        args: vec![json!(workspace.oid), json!(name), json!(false)],
    };
    let made = crate::server::service::handle_create_tab(state, &call).await;
    match made.data.as_ref().and_then(|d| d.as_str()) {
        Some(id) if made.error.is_none() => id.to_string(),
        _ => {
            tracing::warn!(leader = %leader.def.name, error = ?made.error, "agent.spawn: no team tab, the worker opens beside the leader");
            leaders_tab.to_string()
        }
    }
}

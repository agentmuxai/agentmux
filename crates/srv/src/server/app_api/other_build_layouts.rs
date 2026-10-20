// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `layout.otherBuilds` / `layout.restoreOtherBuild` — the RPC shell over
//! `server::service::session_other_builds`.

use super::*;
use crate::server::service::session_other_builds::{self as other_builds, ScanRoots};

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let list_state = state.clone();
    engine.register_typed(COMMAND_LAYOUT_OTHER_BUILDS, move |req: CommandOtherBuildLayoutsData, _ctx| {
        let state = list_state.clone();
        async move {
            let offer = state
                .mstore
                .get::<obj::Window>(&req.window_id)
                .ok()
                .flatten()
                .and_then(|w| other_builds::untouched_default_tab(&state.mstore, &w.workspaceid))
                .is_some();
            if !offer && req.offer_only.unwrap_or(false) {
                return Ok(OtherBuildLayoutsResult { offer, layouts: Vec::new() });
            }
            let Some(roots) = ScanRoots::live() else {
                return Ok(OtherBuildLayoutsResult { offer, layouts: Vec::new() });
            };
            // Opens databases: off the async workers.
            let layouts = tokio::task::spawn_blocking(move || other_builds::discover(&roots, other_builds::SCAN_BUDGET))
                .await
                .map_err(|e| format!("layout.otherBuilds: task: {e}"))?;
            Ok(OtherBuildLayoutsResult { offer, layouts })
        }
    });

    let restore_state = state.clone();
    engine.register_typed(COMMAND_LAYOUT_RESTORE_OTHER_BUILD, move |req: CommandRestoreOtherBuildLayoutData, _ctx| {
        let state = restore_state.clone();
        async move {
            let roots = ScanRoots::live()
                .ok_or_else(|| "layout.restoreOtherBuild: this AgentMux's data folders are unknown".to_string())?;
            other_builds::restore(&state, &roots, &req.window_id, &req.channel, &req.version)
                .await
                .map_err(|e| format!("layout.restoreOtherBuild: {e}"))
        }
    });
}

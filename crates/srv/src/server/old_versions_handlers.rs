// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/old-versions`: the plan of the old-version cleanup (`backend/old_versions.rs`),
//! read only. `POST /agentmux/old-versions` with `{"only": [<folder>, ...]}` removes those of the
//! plan's folders that are still removable now, and nothing else. Authed like the other agent
//! routes (`AGENTMUX_LOCAL_URL` and `X-AuthKey`).

use super::*;
use crate::backend::old_versions as ov;

#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct RemoveReq {
    #[serde(default)]
    only: Vec<String>,
}

async fn respond(state: AppState, dry_run: bool, only: Option<Vec<String>>) -> Json<serde_json::Value> {
    let Some(layout) = ov::layout_from_env() else {
        return Json(json!({ "error": "this AgentMux has no per-version folders (a dev build or a test home)" }));
    };
    let policy = crate::backend::old_versions_service::policy(&state.config_watcher);
    let shown = layout.clone();
    let run = tokio::task::spawn_blocking(move || ov::run(&layout, &policy, dry_run, only.as_deref(), std::time::SystemTime::now())).await;
    match run {
        Ok(run) => Json(json!({
            "dry_run": dry_run,
            "running": shown.running,
            "channel": shown.channel,
            "versions_root": shown.versions_root,
            "scan_ok": run.scan_ok,
            "reclaimable_bytes": run.plan.reclaimable_bytes(),
            "plan": run.plan,
            "report": run.report,
        })),
        Err(e) => Json(json!({ "error": format!("old versions: {e}") })),
    }
}

pub(super) async fn handle_get(State(state): State<AppState>) -> Json<serde_json::Value> {
    respond(state, true, None).await
}

pub(super) async fn handle_post(State(state): State<AppState>, Json(req): Json<RemoveReq>) -> Json<serde_json::Value> {
    if req.only.is_empty() {
        return Json(json!({ "error": "name the folders to remove in \"only\" (GET shows the plan first)" }));
    }
    respond(state, false, Some(req.only)).await
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.disk.cleanup`: ask this machine to clear its agents' idle build folders (spec §4.3). Like
//! `app.upgrade` it starts nothing by itself: it files the `disk_cleanup` sensitive request, so this
//! machine's super agent decides and the person here sees the banner and can veto the countdown. The
//! requester is `agent@machine`, never anything that reads as the operator.
//!
//! What runs when the countdown ends is the pass a low disk runs (`build_cache_service::clean_when_asked`):
//! orphan incremental folders, and the `target` of a clone nobody built in for days, never one with a
//! build running. The older version folders are not touched; they stay a person's call (`app.disk.report`
//! lists them).
//!
//! `{ reason }` files it and answers with the request's id. `{ status: <id> }` reads where a request this
//! caller made stands.

use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};

/// The request owner for a remote caller: not an agent here, so no local uid stands in for it.
fn owner(caller: &RemoteCaller) -> String {
    format!("remote:{}", caller.actor())
}

pub(super) async fn disk_cleanup(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let uid = owner(caller);
    if let Some(id) = params.get("status").and_then(Value::as_str) {
        return crate::server::sensitive_handlers::status_for(&uid, id).map_err(RemoteRefusal::failed);
    }
    let reason = params.get("reason").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty()).unwrap_or_else(|| caller.reason.trim());
    if reason.is_empty() {
        return Err(RemoteRefusal::failed("a reason is required"));
    }
    // Say so now rather than after a countdown that could only end in a refusal.
    if !crate::backend::build_cache_service::this_srv_may_clean() {
        return Err(RemoteRefusal::new(
            "not_available",
            "this AgentMux is not the one that cleans build folders on its computer (a development, headless or test instance, or cleaning is turned off for it)",
        ));
    }
    tracing::info!(requester = %caller.actor(), "disk cleanup: a remote agent asks as a sensitive request");
    let filed = crate::server::sensitive_handlers::file_request(state, &uid, &caller.actor(), "disk_cleanup", &std::collections::BTreeMap::new(), reason)
        .await
        .map_err(RemoteRefusal::failed)?;
    Ok(json!({
        "status": "requested",
        "id": filed["id"].as_str().unwrap_or_default(),
        "review": filed["review"].as_str().unwrap_or_default(),
    }))
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.history_reclaim`: ask this machine to give the shared pane-history store's free space
//! back at its next start, withdraw that, or read where it stands (spec §4.3). It is the local
//! `App history_reclaim` unchanged (`app_api::agent_history_reclaim`), recorded as `agent@machine`.
//! Nothing runs now, and the person sees the request pending in History > Storage.

use serde_json::Value;

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::server::app_api::agent_history_reclaim::run_for;

pub(super) async fn history_reclaim(_state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let action = params.get("action").and_then(Value::as_str).unwrap_or("status");
    if !matches!(action, "request" | "cancel" | "status") {
        return Err(RemoteRefusal::failed(format!("action is \"request\", \"cancel\" or \"status\", not {action:?}")));
    }
    run_for(action, &caller.actor()).map_err(RemoteRefusal::failed)
}

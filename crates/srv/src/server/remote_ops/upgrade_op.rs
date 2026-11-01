// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.upgrade`: ask this machine to upgrade itself (spec §4.3). A remote request never starts anything
//! by itself and is never "the user's own turn": it is filed as the same sensitive request a local agent's
//! `App upgrade` files, so this machine's super agent decides and the person here sees the banner and can
//! veto the countdown, whoever is or is not at the screen (`SPEC_SUPER_AGENT_2026_10_25.md`). The requester
//! is `agent@machine` on the request, the banner and the audit, never anything that reads as the operator.
//!
//! `{ reason, version? }` files it and answers like the local tool (`requested` with the request's id, or
//! why there is nothing to install). `{ status: <id> }` reads where a request this caller made stands.

use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::backend::upgrade_request::{self, Outcome};

/// The request owner for a remote caller: not an agent here, so no local uid stands in for it.
fn owner(caller: &RemoteCaller) -> String {
    format!("remote:{}", caller.actor())
}

pub(super) async fn upgrade(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let uid = owner(caller);
    if let Some(id) = params.get("status").and_then(Value::as_str) {
        return crate::server::sensitive_handlers::status_for(&uid, id).map_err(RemoteRefusal::failed);
    }
    let reason = params.get("reason").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty()).unwrap_or_else(|| caller.reason.trim());
    if reason.is_empty() {
        return Err(RemoteRefusal::failed("a reason is required"));
    }
    // Ask GitHub first: nothing checks on its own.
    let status = crate::backend::update_check::check_now().await.unwrap_or_else(|_| crate::backend::update_check::current_status());
    let version = match upgrade_request::installable(&status, params.get("version").and_then(Value::as_str)) {
        Ok(v) => v,
        Err(outcome) => return Ok(json!(outcome)),
    };
    tracing::info!(requester = %caller.actor(), version = %version, "upgrade: a remote agent asks as a sensitive request");
    let facts = std::collections::BTreeMap::from([("version".to_string(), version.clone())]);
    let filed = crate::server::sensitive_handlers::file_request(state, &uid, &caller.actor(), "upgrade", &facts, reason)
        .await
        .map_err(RemoteRefusal::failed)?;
    Ok(json!(Outcome::Requested {
        version,
        id: filed["id"].as_str().unwrap_or_default().to_string(),
        review: filed["review"].as_str().unwrap_or_default().to_string(),
    }))
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.version` and `app.health`: the first operations, both reads
//! (spec §4.3). Nothing here returns a credential, a path or an address.

use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteResult};

/// What the last release check found, field by field: the status struct also holds the release
/// asset's URL and the download's folder, installer path and error text, none of which leaves
/// this machine (a read needs only the first-seen pin, and discovery is unauthenticated).
fn update_summary() -> Value {
    let u = crate::backend::update_check::current_status();
    let mut out = json!({
        "state": u.state,
        "current": u.current,
        "latest": u.latest,
        "install": u.install,
        "install_first": u.install_first,
        "can_download": u.can_download,
        "published_at": u.published_at,
        "checked_ms": u.checked_ms,
    });
    if let (Some(d), Value::Object(map)) = (u.download, &mut out) {
        map.insert("download".into(), json!({ "version": d.version, "state": d.state }));
    }
    out
}

/// MuxBus delivery, without the signed-in account's email or the error text.
fn muxbus_summary() -> Value {
    let m = crate::muxbus::delivery_status::status();
    json!({
        "state": serde_json::to_value(m.state).unwrap_or(Value::Null),
        "since_ms": m.since_ms,
        "last_ok_ms": m.last_ok_ms,
        "sign_in_unreachable": m.sign_in_unreachable,
    })
}

/// Which AgentMux this is: the facts the `AGENTMUX_VERSION`, `AGENTMUX_BUILD` and
/// `AGENTMUX_INSTANCE_CHANNEL` variables carry, plus what the last release check found.
fn identity(state: &AppState) -> Value {
    let build = std::env::var("AGENTMUX_BUILD").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| state.version.clone());
    json!({
        "machine": state.hostname,
        "os": std::env::consts::OS,
        "version": state.version,
        "build": build,
        "channel": crate::backend::reactive::registry::local_channel_id(),
        "update": update_summary(),
    })
}

pub(super) async fn version(state: &AppState, _caller: &RemoteCaller, _params: Value) -> RemoteResult {
    Ok(identity(state))
}

pub(super) async fn health(state: &AppState, _caller: &RemoteCaller, _params: Value) -> RemoteResult {
    Ok(health_report(state))
}

/// The upgrade spec's health report, on demand: the identity above, and what a steward checks
/// after an upgrade (MuxBus, LAN peers, free disk, how many agents are in which state). Also
/// what `GET /api/v1/app/health` gives this machine's own agents.
pub(crate) fn health_report(state: &AppState) -> Value {
    let mut report = identity(state);
    let disk = agentmux_common::data_paths::DataPaths::from_env()
        .and_then(|paths| agentmux_common::disk_space::volume_free_total_bytes(&paths.data_dir))
        .map(|(free, total)| json!({ "free_bytes": free, "total_bytes": total }));
    let mut agents = serde_json::Map::new();
    for status in state.fleet_feed.snapshot().agent_status.values() {
        let label = serde_json::to_value(status).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "unknown".into());
        let count = agents.get(&label).and_then(Value::as_u64).unwrap_or(0);
        agents.insert(label, json!(count + 1));
    }
    if let Value::Object(map) = &mut report {
        map.insert("lan_peers".into(), json!(state.lan_discovery.get_instances().len()));
        map.insert("muxbus".into(), muxbus_summary());
        map.insert("disk".into(), disk.unwrap_or(Value::Null));
        map.insert("agents".into(), Value::Object(agents));
        map.insert("remote_ops".into(), json!(if super::enabled(state) { "on" } else { "off" }));
    }
    report
}

/// `GET /api/v1/app/health`: `App` with action `health`, for this machine.
pub(crate) async fn handle_health(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Value> {
    axum::Json(health_report(&state))
}

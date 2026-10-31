// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! `Message` action `quota_mode` (`SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md` §5):
//!
//! - `POST /api/v1/quota-mode`: set the fleet's quota mode (`off` or `tight`, and the priority floor) and tell
//!   every agent with one bulletin under the key `mode`, instead of a broadcast to each.
//! - `GET /api/v1/quota-mode`: the mode now.
//!
//! The setting is `settings.json`'s `fleet:quota_mode` and `fleet:quota_min_priority`
//! (`backend::quota_mode`); `Tasks claim` reads it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::backend::quota_mode::{self, DEFAULT_MIN_PRIORITY, SETTING_MIN_PRIORITY, SETTING_MODE};
use crate::backend::storage::bulletins::Bulletin;

use super::bulletin_handlers::push_to_running;
use super::caller::Caller;
use super::work_claims_handlers::claimer;
use super::AppState;

#[derive(Debug, Default, Deserialize)]
pub struct SetParams {
    pub mode: Option<String>,
    pub min_priority: Option<i64>,
    pub agent: Option<String>,
    pub block: Option<String>,
}

fn bad_request(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.into() }))).into_response()
}

fn current(state: &AppState) -> serde_json::Value {
    let extra = state.config_watcher.get_full_config().settings.extra.clone();
    match quota_mode::claim_floor(&extra) {
        Some(n) => json!({ "mode": "tight", "min_priority": n }),
        None => json!({ "mode": "off", "min_priority": extra.get(SETTING_MIN_PRIORITY).and_then(|v| v.as_i64()).unwrap_or(DEFAULT_MIN_PRIORITY) }),
    }
}

/// What the bulletin says about a mode.
fn notice(mode: &str, floor: i64) -> String {
    if mode == "tight" {
        format!(
            "Quota mode is tight. Tasks claim with no id now takes only your own tasks and ones with priority {floor} or \
             more; claim a named task by its id as before. Nothing else changes. No reply needed."
        )
    } else {
        "Quota mode is off. Tasks claim takes any open task again. No reply needed.".to_string()
    }
}

/// `GET /api/v1/quota-mode`.
pub(crate) async fn handle_get_quota_mode(State(state): State<AppState>) -> Response {
    Json(current(&state)).into_response()
}

/// `POST /api/v1/quota-mode`.
pub(crate) async fn handle_set_quota_mode(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(p): Json<SetParams>,
) -> Response {
    let mode = match p.mode.as_deref().map(|m| m.trim().to_ascii_lowercase()) {
        Some(m) if m == "off" || m == "tight" => m,
        _ => return bad_request("mode must be \"off\" or \"tight\""),
    };
    if p.min_priority.is_some_and(|n| !(0..=1000).contains(&n)) {
        return bad_request("min_priority must be between 0 and 1000");
    }
    if !caller.as_ref().is_some_and(|c| c.0.uid().is_some()) {
        return bad_request("who is setting it is not proven: this needs an agent started by AgentMux");
    }
    let Some((author, uid, _)) = claimer(&state, caller.as_ref().map(|c| &c.0), p.agent.as_deref(), p.block.as_deref()) else {
        return bad_request("who is setting it is unknown: this needs an agent started by AgentMux");
    };
    let mut keys = serde_json::Map::new();
    keys.insert(SETTING_MODE.to_string(), json!(mode));
    if let Some(n) = p.min_priority {
        keys.insert(SETTING_MIN_PRIORITY.to_string(), json!(n));
    }
    let disk = keys.clone();
    match tokio::task::spawn_blocking(move || crate::backend::config_watcher_fs::merge_settings_to_disk(disk)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("not set: {e}") }))).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("not set: {e}") }))).into_response(),
    }
    let merged = crate::backend::config_watcher_fs::merge_settings_into_current(&state.config_watcher, keys);
    state.config_watcher.update_settings(merged);
    crate::backend::config_watcher_fs::broadcast_full_config(&state.config_watcher, &state.event_bus);
    let now = state.config_watcher.get_full_config().settings.extra.clone();
    let floor = now.get(SETTING_MIN_PRIORITY).and_then(|v| v.as_i64()).unwrap_or(DEFAULT_MIN_PRIORITY);
    // One bulletin under the key `mode` replaces the last notice of the author's and reaches every running agent.
    let at = agentmux_common::time::now_ms();
    let post = Bulletin {
        id: uuid::Uuid::new_v4().to_string(),
        key: "mode".into(),
        author,
        author_uid: uid,
        body: notice(&mode, floor),
        targets: vec![],
        created_at: at,
        expires_at: at + if mode == "tight" { 24 * 60 } else { 60 } * 60_000,
    };
    let (store, stored, running) = (state.work_store.clone(), post.clone(), state.reactive_handler.list_agents());
    let reached = tokio::task::spawn_blocking(move || {
        store.bulletin_put(&stored, at)?;
        Ok::<_, crate::backend::storage::StoreError>(push_to_running(&store, &stored, &running))
    })
    .await;
    let reached = match reached {
        Ok(Ok(n)) => n,
        _ => 0,
    };
    let mut answer = current(&state);
    answer["bulletin_reached_running_agents"] = json!(reached);
    Json(answer).into_response()
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's `dock` and `float` of a pane it opened (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md
//! §2.2, §3): a floating pane goes back into a tab, a docked one gets a window of its own. Both run the tear-off
//! and redock the person's drag runs (`service::tear_off`), so the pane, its block id and everything that finds it
//! by that id (the `Browser*` tools, the opener record) are unchanged by the move. Only a pane the caller opened.

use agentmux_common::api_types::PanePlaceRequest;

use super::app_api::floater_place;
use super::*;
use crate::backend::obj::Block;
use crate::backend::rpc_types::{CommandPaneOpenData, PaneOpenResult};
use crate::backend::service::WebCallType;

fn refuse(status: StatusCode, text: impl Into<String>) -> Response {
    (status, Json(json!({ "error": text.into() }))).into_response()
}

/// The tab and workspace a block is in now.
async fn home_of(state: &AppState, block_id: &str) -> Option<(String, String)> {
    let s = state.srv_state.lock().await;
    let tab = s.blocks.get(block_id)?.tab_id.clone();
    let ws = s.tabs.get(&tab)?.workspace_id.clone();
    Some((tab, ws))
}

/// The redock's direction code for a side of the caller's pane (`layout_helpers::queue_target_layout_split`).
pub(crate) fn split_code(split: &str) -> Option<u8> {
    match split {
        "up" => Some(0),
        "right" => Some(1),
        "down" => Some(2),
        "left" => Some(3),
        _ => None,
    }
}

fn service_call(method: &str, args: Vec<serde_json::Value>) -> WebCallType {
    WebCallType { service: "block".to_string(), method: method.to_string(), uicontext: None, args }
}

/// Redock the floating pane `target` (in `src`, a tab and its workspace) into `tgt`: beside `beside`'s pane on its
/// side code, or, with none, where it was torn from when that place can still be found. The pane is no floater after.
async fn redock_into(
    state: &AppState,
    target: &str,
    src: (&str, &str),
    tgt: (&str, &str),
    beside: Option<(&str, u8)>,
) -> Result<serde_json::Value, String> {
    // [block, source tab, source workspace, target tab, target workspace, beside block, side, as tab, members, active, to origin]
    let call = service_call(
        "RedockFloatingPane",
        vec![
            json!(target),
            json!(src.0),
            json!(src.1),
            json!(tgt.0),
            json!(tgt.1),
            beside.map_or(serde_json::Value::Null, |(b, _)| json!(b)),
            beside.map_or(serde_json::Value::Null, |(_, c)| json!(c)),
            json!(false),
            json!(null),
            json!(null),
            json!(beside.is_none()),
        ],
    );
    let ret = crate::server::service::handle_redock_floating_pane(state, &call).await;
    if let Some(e) = ret.error {
        return Err(e);
    }
    super::pane_openers::set_floating(target, false);
    Ok(ret.data.unwrap_or_default())
}

/// A Stop of the agent `agent_uid`: each floating pane it opened is docked into the tab `home_block` (its own pane)
/// is in, where it was torn from when that place can still be found. Nothing is closed. Returns how many were docked.
pub(crate) async fn dock_all(state: &AppState, agent_uid: &str, home_block: &str) -> usize {
    let Some((tgt_tab, tgt_ws)) = home_of(state, home_block).await else { return 0 };
    let mut docked = 0;
    for pane in floater_place::live_floaters(state, agent_uid).await {
        let Some((src_tab, src_ws)) = home_of(state, &pane).await else { continue };
        if src_tab == tgt_tab {
            continue;
        }
        match redock_into(state, &pane, (&src_tab, &src_ws), (&tgt_tab, &tgt_ws), None).await {
            Ok(_) => docked += 1,
            Err(e) => tracing::warn!(pane = %pane, "agent stop: a floating pane could not be docked: {e}"),
        }
    }
    docked
}

/// `dock`: a floating pane the caller opened goes back into the caller's own tab, beside the caller's pane on the
/// side `split` names, or where it was torn from when that place can still be found. The window closes with its
/// last pane.
pub(super) async fn dock(state: &AppState, agent: &str, req: &PanePlaceRequest) -> Response {
    let p = &req.place;
    if p.width.is_some() || p.height.is_some() || p.spot.is_some() || p.x.is_some() || p.y.is_some() {
        return refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: dock takes no size or place: it takes split (right, left, down, up) to say which side of your pane, or nothing");
    }
    let target = req.target_id.as_str();
    if !super::pane_openers::opened_by(target, agent) {
        return refuse(StatusCode::FORBIDDEN, super::pane_place::NOT_YOURS);
    }
    if !super::pane_openers::is_floating(target) {
        return refuse(StatusCode::CONFLICT, format!("NOT_FLOATING: {target} is already docked in a tab"));
    }
    let split = match req.split.as_deref().filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => match split_code(s) {
            Some(code) => Some(code),
            None => return refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: split {s:?} is not one of right, left, down, up")),
        },
    };
    let Some(own) = req.block_id.as_deref().filter(|b| !b.is_empty()) else {
        return refuse(StatusCode::BAD_REQUEST, "MISSING_ARG: dock puts the pane in your own tab, and your own pane is not known here");
    };
    let (Some((src_tab, src_ws)), Some((tgt_tab, tgt_ws))) = (home_of(state, target).await, home_of(state, own).await) else {
        return refuse(StatusCode::NOT_FOUND, format!("that pane is gone: {target} or your own pane is no longer in a tab"));
    };
    if src_tab == tgt_tab {
        return refuse(StatusCode::CONFLICT, format!("NOT_FLOATING: {target} is already in your tab"));
    }
    let beside = split.map(|c| (own, c));
    let ret = match redock_into(state, target, (&src_tab, &src_ws), (&tgt_tab, &tgt_ws), beside).await {
        Ok(data) => data,
        Err(e) => return refuse(StatusCode::BAD_GATEWAY, format!("the pane could not be docked: {e}")),
    };
    let at = ret.get("placed").cloned().unwrap_or(json!(null));
    (StatusCode::OK, Json(json!({ "pane": target, "docked": true, "tab_id": tgt_tab, "placed_by": at }))).into_response()
}

/// `float`: a docked pane the caller opened gets a window of its own, placed as an open's `floating` places one
/// (the same size, spot, x and y; none gives the default, a browser's 1040 x 760 flush right).
pub(super) async fn float(state: &AppState, agent: &str, req: &PanePlaceRequest) -> Response {
    let target = req.target_id.as_str();
    let mut cmd = CommandPaneOpenData {
        floating: Some(true),
        opener: Some(agent.to_string()),
        width: req.place.width.clone(),
        height: req.place.height.clone(),
        spot: req.place.spot.clone(),
        x: req.place.x,
        y: req.place.y,
        ..Default::default()
    };
    if let Err(e) = floater_place::check(&cmd) {
        return refuse(StatusCode::BAD_REQUEST, e);
    }
    if !super::pane_openers::opened_by(target, agent) {
        return refuse(StatusCode::FORBIDDEN, super::pane_place::NOT_YOURS);
    }
    if super::pane_openers::is_floating(target) {
        return refuse(StatusCode::CONFLICT, format!("NOT_DOCKED: {target} is already a floating window: use resize, move or place"));
    }
    let Some((src_tab, src_ws)) = home_of(state, target).await else {
        return refuse(StatusCode::NOT_FOUND, format!("that pane is gone: {target} is no longer in a tab"));
    };
    let open = floater_place::live_floaters(state, agent).await;
    if open.len() >= floater_place::MAX_FLOATERS {
        return refuse(StatusCode::TOO_MANY_REQUESTS, floater_place::limit_text(&open));
    }
    if let Err(text) = super::deeplink_guard::check_place_rate(agent) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, text);
    }
    let view = state.mstore.get::<Block>(target).ok().flatten().map(|b| crate::backend::obj::meta_get_string(&b.meta, "view", "")).unwrap_or_default();
    if view == "browser" {
        cmd.floater_max_share = floater_place::browser_defaults(&mut cmd);
    }
    let call = service_call("TearOffBlock", vec![json!(target), json!(src_tab), json!(src_ws)]);
    let ret = crate::server::service::handle_tear_off_block(state, &call).await;
    if let Some(e) = ret.error {
        return refuse(StatusCode::BAD_GATEWAY, format!("the pane could not be floated: {e}"));
    }
    if let Some(updates) = &ret.updates {
        // The tear-off's updates reach only a window that asked; the others learn of the new workspace here.
        state.event_bus.broadcast_mux_obj_updates(updates);
    }
    let Some(new_ws) = ret.data.as_ref().and_then(|d| d.as_str()).map(str::to_string) else {
        return refuse(StatusCode::BAD_GATEWAY, "the pane could not be floated: no workspace was made for it");
    };
    super::pane_openers::set_floating(target, true);
    let mut result = PaneOpenResult::docked(target.to_string(), String::new(), view, false);
    super::app_api::pane::announce_floater(state, &src_ws, &new_ws, &cmd, &mut result, open.len()).await;
    let mut body = json!({ "pane": target });
    if let Some(placed) = &result.placed {
        body["placed"] = json!(placed);
    }
    if result.queued {
        body["queued"] = json!(true);
    }
    (StatusCode::OK, Json(body)).into_response()
}

#[cfg(test)]
#[path = "pane_dock_tests.rs"]
mod tests;

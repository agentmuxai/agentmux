// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's `resize`, `move` and `place` of a floating pane it opened, and where each such pane is, for
//! `Pane layout` (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.2, §2.3, §2.4, §3).
//!
//! srv cannot reach a window's screen, so it asks the floating window itself, in the `placefloatingpane`
//! event, and the window answers with `pane.floatplaced`, the same request, wait and answer an open uses
//! (`app_api::floater_place`). The window resolves the request against the work area of its own monitor,
//! sets its rect without raising or activating it, and says where it went. An agent acts only on a pane
//! its opener record names it for (`pane_openers`).

use std::time::Duration;

use agentmux_common::api_types::PanePlaceRequest;
use agentmux_common::place_args::PlaceArgs;

use super::app_api::floater_place;
use super::*;
use crate::backend::mps_events::{PlaceFloatingPane, PlaceFloatingPaneEvent};
use crate::backend::rpc_types::{CommandPaneOpenData, Placed};

/// How long `Pane layout` waits for a floating window to say where it is.
const MEASURE_WAIT: Duration = Duration::from_secs(2);

/// The refusal for a pane the caller did not open.
pub(crate) const NOT_YOURS: &str =
    "NOT_YOURS: you can place only panes you opened; ask the person to move it, or open your own";

/// What a `Pane` placement action does to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Resize,
    Move,
    Place,
}

impl Op {
    fn parse(op: &str) -> Result<Self, String> {
        match op {
            "resize" => Ok(Op::Resize),
            "move" => Ok(Op::Move),
            "place" => Ok(Op::Place),
            other => Err(format!("INVALID_ARG: {other:?} is not a placement action: use resize, move or place")),
        }
    }

    /// What the window leaves as it is: a resize its position, a move its size.
    fn keep(self) -> Option<&'static str> {
        match self {
            Op::Resize => Some("position"),
            Op::Move => Some("size"),
            Op::Place => None,
        }
    }
}

/// The action and its arguments, checked: each action takes what it changes and refuses the rest, in words
/// that say which action to use instead. The request comes back as the floating-open shape the shared checks read.
pub(crate) fn check(op: &str, p: &PlaceArgs) -> Result<(Op, CommandPaneOpenData), String> {
    let op = Op::parse(op)?;
    let sized = p.width.is_some() || p.height.is_some();
    let placed = p.spot.is_some() || p.x.is_some() || p.y.is_some();
    match op {
        Op::Resize if !sized => return Err("INVALID_ARG: resize needs width and/or height (CSS pixels such as 900, or a share such as \"60%\")".into()),
        Op::Resize if placed => return Err("INVALID_ARG: resize changes the size only: use move for a position, or place for both".into()),
        Op::Move if !placed => return Err("INVALID_ARG: move needs spot, or x and y".into()),
        Op::Move if sized => return Err("INVALID_ARG: move changes the position only: use resize for a size, or place for both".into()),
        Op::Place if !sized && !placed => return Err("INVALID_ARG: place needs a size, a position, or both".into()),
        _ => {}
    }
    let cmd = CommandPaneOpenData {
        floating: Some(true),
        width: p.width.clone(),
        height: p.height.clone(),
        spot: p.spot.clone(),
        x: p.x,
        y: p.y,
        ..Default::default()
    };
    floater_place::check(&cmd)?;
    Ok((op, cmd))
}

/// What the window did with a question.
#[derive(Debug)]
pub(crate) enum Asked {
    /// The window said it could not.
    Failed(String),
    /// The window did not answer in time (hidden, or still starting).
    NoAnswer,
}

/// The window of the floating pane `block_id`: the srv window row of the workspace it is alone in.
async fn window_of(state: &AppState, block_id: &str) -> Option<String> {
    let s = state.srv_state.lock().await;
    let ws = &s.tabs.get(&s.blocks.get(block_id)?.tab_id)?.workspace_id;
    s.windows.iter().find(|(_, w)| &w.workspace_id == ws).map(|(id, _)| id.clone())
}

/// Ask the floating window `win` to apply `place` (or, with none, to say where it is) and wait for its answer.
async fn ask(
    state: &AppState,
    win: String,
    block_id: &str,
    keep: Option<&str>,
    place: Option<&CommandPaneOpenData>,
    wait: Duration,
) -> Result<Placed, Asked> {
    let id = uuid::Uuid::new_v4().to_string();
    // Registered before the window is told, so an answer cannot beat the waiter.
    let rx = floater_place::expect(&id);
    let event = PlaceFloatingPaneEvent {
        request_id: id.clone(),
        block_id: block_id.to_string(),
        keep: keep.map(str::to_string),
        place: place.map(|cmd| floater_place::place_for(cmd, id.clone(), 0, None)),
    };
    state.broker.publish_event::<PlaceFloatingPane>(&event, vec![win], 0);
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(Ok(placed))) => Ok(placed),
        Ok(Ok(Err(why))) => Err(Asked::Failed(why)),
        _ => {
            floater_place::forget(&id);
            Err(Asked::NoAnswer)
        }
    }
}

/// Where `agent` stands with `block_id`: refused with the status and text the agent gets, or the window to ask.
async fn floater_of(state: &AppState, agent: &str, block_id: &str) -> Result<String, (StatusCode, String)> {
    if !super::pane_openers::opened_by(block_id, agent) {
        return Err((StatusCode::FORBIDDEN, NOT_YOURS.to_string()));
    }
    if !super::pane_openers::is_floating(block_id) {
        return Err((
            StatusCode::CONFLICT,
            format!("NOT_FLOATING: {block_id} is docked in a tab, not a floating window: open a pane with floating: true to place it"),
        ));
    }
    window_of(state, block_id).await.ok_or_else(|| {
        (StatusCode::CONFLICT, format!("NOT_FLOATING: the floating window of {block_id} is not open (the person may have closed it)"))
    })
}

/// `POST /api/v1/pane/place` — an agent's `resize`, `move` or `place` of a floating pane it opened. Answers with
/// where the window is now (`placed`, in CSS pixels from the work area's top-left, `clamped` when it had to be
/// pulled in).
pub(super) async fn handle_pane_place(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<PanePlaceRequest>,
) -> impl IntoResponse {
    let refuse = |status: StatusCode, text: String| (status, Json(json!({ "error": text }))).into_response();
    let Some(agent) = caller.as_ref().and_then(|c| c.0.uid()).map(str::to_string) else {
        return refuse(StatusCode::FORBIDDEN, "an agent known by its token places its own panes; the person drags a window".into());
    };
    let (op, cmd) = match check(&req.op, &req.place) {
        Ok(x) => x,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e),
    };
    let win = match floater_of(&state, &agent, &req.target_id).await {
        Ok(w) => w,
        Err((status, text)) => return refuse(status, text),
    };
    // The same ten a minute as an open, so a loop of placements is stopped.
    if let Err(text) = super::deeplink_guard::check_place_rate(&agent) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, text);
    }
    match ask(&state, win, &req.target_id, op.keep(), Some(&cmd), floater_place::WAIT).await {
        Ok(placed) => (StatusCode::OK, Json(json!({ "pane": req.target_id, "placed": placed }))).into_response(),
        Err(Asked::Failed(why)) => refuse(StatusCode::BAD_GATEWAY, format!("the floating window could not do it: {why}")),
        Err(Asked::NoAnswer) => refuse(
            StatusCode::GATEWAY_TIMEOUT,
            "NO_ANSWER: the floating window did not answer (it may be hidden or still starting): try again".into(),
        ),
    }
}

/// `Pane layout`'s report of the floating panes agents opened: each gets `floating: true` and, from its window, `rect`
/// (`x`, `y`, `width`, `height` in CSS pixels from the work area's top-left, and `display_scale`). A window that
/// does not answer gets `rect: null` and the reason. Panes nobody opened as a floater are left as they are.
pub(super) async fn report_floaters(state: &AppState, layout: &mut serde_json::Value) {
    let Some(windows) = layout.get_mut("windows").and_then(|w| w.as_array_mut()) else { return };
    let panes = || {
        windows
            .iter()
            .flat_map(|w| w.get("tabs").and_then(|t| t.as_array()).into_iter().flatten())
            .flat_map(|t| t.get("panes").and_then(|p| p.as_array()).into_iter().flatten())
            .filter_map(|p| p.get("block_id").and_then(|b| b.as_str()))
            .filter(|b| super::pane_openers::is_floating(b))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let floaters = panes();
    if floaters.is_empty() {
        return;
    }
    let measured = futures_util::future::join_all(floaters.iter().map(|block| async move {
        let Some(win) = window_of(state, block).await else { return (block.clone(), None) };
        (block.clone(), Some(ask(state, win, block, None, None, MEASURE_WAIT).await))
    }))
    .await;
    for pane in windows
        .iter_mut()
        .flat_map(|w| w.get_mut("tabs").and_then(|t| t.as_array_mut()).into_iter().flatten())
        .flat_map(|t| t.get_mut("panes").and_then(|p| p.as_array_mut()).into_iter().flatten())
    {
        let Some(block) = pane.get("block_id").and_then(|b| b.as_str()).map(str::to_string) else { continue };
        let Some((_, answer)) = measured.iter().find(|(b, _)| *b == block) else { continue };
        pane["floating"] = json!(true);
        match answer {
            Some(Ok(p)) => {
                pane["rect"] = json!({ "x": p.x, "y": p.y, "width": p.width, "height": p.height, "display_scale": p.display_scale });
            }
            Some(Err(Asked::Failed(why))) => {
                pane["rect"] = json!(null);
                pane["rect_unavailable"] = json!(format!("the window could not say: {why}"));
            }
            Some(Err(Asked::NoAnswer)) => {
                pane["rect"] = json!(null);
                pane["rect_unavailable"] = json!("the window did not answer (it may be hidden or still starting)");
            }
            None => {
                pane["rect"] = json!(null);
                pane["rect_unavailable"] = json!("the window is not open");
            }
        }
    }
}

#[cfg(test)]
#[path = "pane_place_tests.rs"]
mod tests;

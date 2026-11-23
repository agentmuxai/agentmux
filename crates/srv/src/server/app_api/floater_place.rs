// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent asked for a floating pane, checked here, resolved by the window
//! (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.1, §3).
//!
//! srv cannot reach a window's screen, so the request goes to the caller's own window in the
//! `openfloatingpane` event and the window answers with `pane.floatplaced`, the same request, wait and
//! answer shape `block.reveal` uses. This module checks the arguments, fills the defaults (a browser's
//! size and place), builds the event's `place` and holds the waits.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::backend::mps_events::FloaterPlace;
use crate::backend::rpc_types::{CommandPaneOpenData, Placed};

/// How long an open waits for its window to say where it put the pane. A window that is hidden or still
/// starting answers late: the open then says `queued`.
pub(crate) const WAIT: Duration = Duration::from_secs(5);

/// The most floating panes one agent has open at a time.
pub(crate) const MAX_FLOATERS: usize = 4;

/// A floating browser an agent opens with no size: 1040 x 760 CSS pixels, at most 90% of the work area, flush right.
const BROWSER_WIDTH: f64 = 1040.0;
const BROWSER_HEIGHT: f64 = 760.0;
const BROWSER_MAX_SHARE: f64 = 0.9;
const BROWSER_SPOT: &str = "right";

/// The named places in the work area.
pub(crate) const SPOTS: [&str; 9] =
    ["center", "right", "left", "top", "bottom", "top-right", "top-left", "bottom-right", "bottom-left"];

/// Whether the request names any placement.
pub(crate) fn names_placement(cmd: &CommandPaneOpenData) -> bool {
    cmd.width.is_some() || cmd.height.is_some() || cmd.spot.is_some() || cmd.x.is_some() || cmd.y.is_some()
}

/// One size argument: a positive number of CSS pixels, or a share of the work area (`"60%"`).
fn check_size(name: &str, v: &serde_json::Value) -> Result<(), String> {
    let bad = || format!("INVALID_ARG: {name} is a number of CSS pixels (such as 900) or a share of the work area (such as \"60%\"), not {v}");
    match v {
        serde_json::Value::Number(n) => n.as_f64().filter(|f| f.is_finite() && *f > 0.0).map(|_| ()).ok_or_else(bad),
        serde_json::Value::String(s) => {
            let share = s.trim().strip_suffix('%').and_then(|n| n.trim().parse::<f64>().ok());
            share.filter(|p| p.is_finite() && *p > 0.0 && *p <= 100.0).map(|_| ()).ok_or_else(bad)
        }
        _ => Err(bad()),
    }
}

/// The placement arguments, checked. They place a floating pane, so without `floating: true` they are refused,
/// and the text says what to add.
pub(crate) fn check(cmd: &CommandPaneOpenData) -> Result<(), String> {
    if !names_placement(cmd) {
        return Ok(());
    }
    if cmd.floating != Some(true) {
        return Err("INVALID_ARG: width, height, spot, x and y place a floating pane: add floating: true".to_string());
    }
    if let Some(w) = &cmd.width {
        check_size("width", w)?;
    }
    if let Some(h) = &cmd.height {
        check_size("height", h)?;
    }
    if let Some(spot) = cmd.spot.as_deref() {
        if !SPOTS.contains(&spot) {
            return Err(format!("INVALID_ARG: spot {spot:?} is not one of {}", SPOTS.join(", ")));
        }
    }
    match (cmd.x, cmd.y) {
        (Some(_), None) | (None, Some(_)) => {
            return Err("INVALID_ARG: x and y go together: give both, or use spot (x and y win over spot)".to_string())
        }
        (Some(x), Some(y)) if !(x.is_finite() && y.is_finite()) => return Err("INVALID_ARG: x and y are numbers of CSS pixels".to_string()),
        _ => {}
    }
    Ok(())
}

/// A floating browser an agent opens with no size is 1040 x 760 flush right, unless the request says otherwise;
/// each of width, height and the place keeps what the agent gave. Returns the share cap that goes with the default.
pub(crate) fn browser_defaults(cmd: &mut CommandPaneOpenData) -> Option<f64> {
    let sized = cmd.width.is_some() || cmd.height.is_some();
    if cmd.width.is_none() {
        cmd.width = Some(serde_json::json!(BROWSER_WIDTH));
    }
    if cmd.height.is_none() {
        cmd.height = Some(serde_json::json!(BROWSER_HEIGHT));
    }
    if cmd.spot.is_none() && cmd.x.is_none() {
        cmd.spot = Some(BROWSER_SPOT.to_string());
    }
    (!sized).then_some(BROWSER_MAX_SHARE)
}

/// A browser open's floating arguments onto the pane request, checked, with a floating browser's defaults. The
/// opener is the caller's token's UID, as `pane.open` records an agent's pane; the signed id for a caller with none.
pub(crate) fn for_browser(
    cmd: &mut CommandPaneOpenData,
    req: &agentmux_common::api_types::UiBrowserOpenRequest,
    caller: Option<&crate::server::caller::Caller>,
) -> Result<(), String> {
    let place = &req.place;
    (cmd.width, cmd.height, cmd.spot, cmd.x, cmd.y) = (place.width.clone(), place.height.clone(), place.spot.clone(), place.x, place.y);
    cmd.opener = Some(caller.and_then(|c| c.uid()).map_or_else(|| req.auth.agent_id.clone(), str::to_string));
    cmd.floating = req.floating.filter(|f| *f);
    check(cmd)?;
    // A floating browser with no size is 1040 x 760 flush right, held to 90% of the work area.
    if cmd.floating == Some(true) {
        cmd.floater_max_share = browser_defaults(cmd);
    }
    Ok(())
}

/// The `data` of a browser open's answer: the pane, what it browses as, and for a floating open where its window went.
pub(crate) fn open_answer(result: &crate::backend::rpc_types::PaneOpenResult, browsing_as: &str) -> serde_json::Value {
    let mut data = serde_json::json!({ "pane": result.block_id, "browsing_as": browsing_as });
    if let Some(placed) = &result.placed {
        data["placed"] = serde_json::json!(placed);
    }
    if result.queued {
        data["queued"] = serde_json::json!(true);
    }
    data
}

/// The event's `place` for `cmd`, with the id the window answers with. `cascade` is how many floating panes the
/// agent already has open.
pub(crate) fn place_for(cmd: &CommandPaneOpenData, request_id: String, cascade: usize, max_share: Option<f64>) -> FloaterPlace {
    FloaterPlace {
        request_id,
        width: cmd.width.clone(),
        height: cmd.height.clone(),
        spot: cmd.spot.clone(),
        x: cmd.x,
        y: cmd.y,
        max_share,
        cascade: cascade.min(MAX_FLOATERS) as u32,
        activate: false,
    }
}

/// The floating panes `agent` has open now: the ones its record names whose block is still there.
pub(crate) async fn live_floaters(state: &crate::server::AppState, agent: &str) -> Vec<String> {
    let named = crate::server::pane_openers::floaters_of(agent);
    let s = state.srv_state.lock().await;
    named.into_iter().filter(|b| s.blocks.contains_key(b)).collect()
}

/// The refusal for an agent that has [`MAX_FLOATERS`] floating panes open.
pub(crate) fn limit_text(open: &[String]) -> String {
    format!(
        "FLOATER_LIMIT: you have {} floating panes open ({}): dock or close one first",
        open.len(),
        open.join(", ")
    )
}

/// What the window said.
pub(crate) type Answer = Result<Placed, String>;

fn waiters() -> &'static Mutex<HashMap<String, tokio::sync::oneshot::Sender<Answer>>> {
    static W: OnceLock<Mutex<HashMap<String, tokio::sync::oneshot::Sender<Answer>>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Wait for the answer to the `openfloatingpane` that carries `request_id`: register before publishing.
pub(crate) fn expect(request_id: &str) -> tokio::sync::oneshot::Receiver<Answer> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    waiters().lock().unwrap_or_else(|e| e.into_inner()).insert(request_id.to_string(), tx);
    rx
}

/// Stop waiting for `request_id` (its answer did not come in time).
pub(crate) fn forget(request_id: &str) {
    waiters().lock().unwrap_or_else(|e| e.into_inner()).remove(request_id);
}

/// A window's answer (`pane.floatplaced`). Whether a caller was still waiting for it.
pub(crate) fn answered(request_id: &str, answer: Answer) -> bool {
    let tx = waiters().lock().unwrap_or_else(|e| e.into_inner()).remove(request_id);
    tx.is_some_and(|tx| tx.send(answer).is_ok())
}

#[cfg(test)]
#[path = "tests/floater_place_tests.rs"]
mod tests;

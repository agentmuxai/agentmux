// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's zoom of a browser pane it drives (docs/specs/SPEC_BROWSER_PANE_AGENT_ZOOM_2026_11_10.md).
//! The host does the zoom (`/agentmux/browser/zoom`: emulation on the pane's own browser); this checks that the
//! pane is the caller's, remembers which panes are zoomed and by whom, and gives the agent's `Browser` tool the
//! line it ends every answer with while one is, so a stale zoom is never a surprise.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::server::ui_handlers::{err_response, get_host_ipc, proxy_to_host_timeout, target_block_id};
use crate::server::AppState;

/// A pane the host has zoomed: the agent that did it, and what to tell that agent.
#[derive(Debug, Clone, PartialEq)]
struct Zoomed {
    agent: String,
    line: String,
}

fn zoomed() -> &'static Mutex<HashMap<String, Zoomed>> {
    static Z: OnceLock<Mutex<HashMap<String, Zoomed>>> = OnceLock::new();
    Z.get_or_init(Default::default)
}

/// How long the host has to fit a pane (it reflows the page and measures it).
const HOST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// `POST /api/v1/ui/browser/zoom` — zoom a pane the caller drives, or put it back (`reset`).
pub(crate) async fn handle_ui_browser_zoom(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserZoomRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let host = match get_host_ipc(&state).await {
        Ok(h) => h,
        Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let body = json!({
        "block_id": block_id,
        "factor": req.factor,
        "width": req.width,
        "fit": req.fit.unwrap_or(false),
        "reset": req.reset.unwrap_or(false),
    });
    let host_resp = match proxy_to_host_timeout(&state, &host, "zoom", body, Some(HOST_TIMEOUT)).await {
        Ok(v) => v,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    if host_resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = host_resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown host error").to_string();
        return err_response(StatusCode::BAD_REQUEST, err);
    }
    let data = host_resp.get("data").cloned().unwrap_or(Value::Null);
    set(&block_id, &req.auth.agent_id, &data);
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, "[ui-automation] browser zoom");
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// Remember the host's answer: the pane is zoomed (with the line to show), or it is not.
fn set(block_id: &str, agent: &str, data: &Value) {
    let mut map = zoomed().lock().unwrap_or_else(|p| p.into_inner());
    match (data.get("zoomed").and_then(|z| z.as_bool()), data.get("line").and_then(|l| l.as_str())) {
        (Some(true), Some(line)) => {
            map.insert(block_id.to_string(), Zoomed { agent: agent.to_string(), line: line.to_string() });
        }
        _ => {
            map.remove(block_id);
        }
    }
}

/// The lines for `agent`'s `Browser` tool to end an answer with: one per pane of theirs that is zoomed.
pub(crate) fn notes_for(agent: &str) -> Vec<Value> {
    let map = zoomed().lock().unwrap_or_else(|p| p.into_inner());
    let mut notes: Vec<Value> = map
        .iter()
        .filter(|(_, z)| z.agent == agent)
        .map(|(pane, z)| json!({ "pane": pane, "line": z.line }))
        .collect();
    notes.sort_by(|a, b| a["pane"].as_str().cmp(&b["pane"].as_str()));
    notes
}

/// The panes the host should put back, and forget here: those no longer owned by the agent that zoomed them, or
/// handed to the person. (The host also drops a zoom on a pane that is not owned at all.) `owners` is every
/// owned pane with its owner.
fn stale(owners: &BTreeMap<String, String>, handed: &[String]) -> Vec<String> {
    stale_in(&mut zoomed().lock().unwrap_or_else(|p| p.into_inner()), owners, handed)
}

fn stale_in(map: &mut HashMap<String, Zoomed>, owners: &BTreeMap<String, String>, handed: &[String]) -> Vec<String> {
    let gone: Vec<String> = map
        .iter()
        .filter(|(pane, z)| owners.get(*pane) != Some(&z.agent) || handed.contains(*pane))
        .map(|(pane, _)| pane.clone())
        .collect();
    for pane in &gone {
        map.remove(pane);
    }
    gone
}

/// Part of the push of the owned panes: forget the zoom of every pane that stopped being the zoomer's, and
/// tell the host to put each back, in case another agent now owns one (the host only knows a pane is owned).
pub(crate) async fn reconcile(state: &AppState, host: &crate::server::HostIpc, handed: &[String]) {
    for pane in stale(&crate::server::browser_owner::all_owners(), handed) {
        let _ = proxy_to_host_timeout(state, host, "zoom", json!({ "block_id": pane, "reset": true }), Some(HOST_TIMEOUT)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(zoomed: bool, line: &str) -> Value {
        json!({ "zoomed": zoomed, "line": line })
    }

    #[test]
    fn a_zoomed_pane_is_told_to_its_agent_only_until_it_is_put_back() {
        set("zt-pane-1", "agent-a", &data(true, "viewport 1280x720 CSS, shown at 50%"));
        assert_eq!(notes_for("agent-a").iter().filter(|n| n["pane"] == "zt-pane-1").count(), 1);
        assert!(notes_for("agent-b").iter().all(|n| n["pane"] != "zt-pane-1"));
        set("zt-pane-1", "agent-a", &data(false, "zoom is off (100%)"));
        assert!(notes_for("agent-a").iter().all(|n| n["pane"] != "zt-pane-1"));
    }

    #[test]
    fn a_pane_that_changed_hands_or_went_to_the_person_is_forgotten() {
        let z = |a: &str| Zoomed { agent: a.to_string(), line: "viewport 640x400 CSS, shown at 200%".to_string() };
        let mut map = HashMap::from([("p2".to_string(), z("agent-a")), ("p3".to_string(), z("agent-a")), ("p4".to_string(), z("agent-a")), ("p5".to_string(), z("agent-a"))]);
        let owners = BTreeMap::from([
            ("p2".to_string(), "agent-a".to_string()),
            ("p3".to_string(), "agent-b".to_string()),
            ("p4".to_string(), "agent-a".to_string()),
        ]);
        let mut gone = stale_in(&mut map, &owners, &["p4".to_string()]);
        gone.sort();
        // p3 changed hands, p4 went to the person, p5 is no longer owned at all; p2 stays.
        assert_eq!(gone, vec!["p3".to_string(), "p4".to_string(), "p5".to_string()]);
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["p2"]);
    }

    #[test]
    fn an_answer_without_a_line_is_not_a_zoom() {
        set("zt-pane-5", "agent-a", &json!({ "zoomed": true }));
        assert!(notes_for("agent-a").iter().all(|n| n["pane"] != "zt-pane-5"));
    }
}

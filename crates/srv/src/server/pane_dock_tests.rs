// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent's dock and float of a pane it opened (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.2).

use super::*;
use crate::backend::rpc_types::Placed;
use crate::server::pane_openers;
use crate::server::tests::test_state;
use agentmux_common::api_types::PanePlaceRequest;
use agentmux_common::ipc::{Command, Event};
use agentmux_common::place_args::PlaceArgs;
use std::sync::{Arc, Mutex};

fn request(op: &str, target: &str, own: Option<&str>) -> PanePlaceRequest {
    PanePlaceRequest {
        op: op.to_string(),
        target_id: target.to_string(),
        place: PlaceArgs::default(),
        split: None,
        block_id: own.map(str::to_string),
    }
}

fn an_agent() -> String {
    format!("uid-{}", uuid::Uuid::new_v4())
}

async fn said(resp: Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// A workspace with a window and one tab holding two panes (the agent's own and another), as `(tab, workspace, own, other)`.
async fn a_tab_with_two_panes(state: &AppState, view: &str) -> (String, String, String, String) {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateWindow { window_id: format!("win-{ws_id}"), workspace_id: ws_id.clone() }).await;
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() }).await;
    let tab_id = tab
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut blocks = Vec::new();
    for v in ["term", view] {
        let b = dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: json!({ "view": v }) }).await;
        let id = b
            .iter()
            .find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .unwrap();
        // Only the agent's own pane is a leaf of the tab's tree; the other is docked in the tab and shown by the redock.
        if blocks.is_empty() {
            crate::server::service::setup_torn_off_block_layout(state, &tab_id, &id).await.ok();
        }
        blocks.push(id);
    }
    (tab_id, ws_id, blocks[0].clone(), blocks[1].clone())
}

/// Every `openfloatingpane` the broker publishes; the window answers with `answer`.
fn watch(state: &AppState, answer: Placed) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rec = seen.clone();
    state.broker.add_observer(Arc::new(move |ev: &crate::backend::mps::MuxEvent| {
        if ev.event != "openfloatingpane" {
            return;
        }
        let data = ev.data.clone().unwrap_or_default();
        if let Some(id) = data["place"]["request_id"].as_str() {
            assert!(floater_place::answered(id, Ok(answer.clone())), "the call was waiting");
        }
        rec.lock().unwrap().push(data);
    }));
    seen
}

fn rect() -> Placed {
    Placed { x: 500.0, y: 100.0, width: 1040.0, height: 760.0, spot: Some("right".into()), clamped: false, display_scale: 1.0 }
}

#[test]
fn a_side_of_the_callers_pane_is_a_redock_direction() {
    assert_eq!(
        ["up", "right", "down", "left"].map(split_code),
        [Some(0), Some(1), Some(2), Some(3)],
        "the codes layout_helpers::queue_target_layout_split reads"
    );
    assert_eq!(split_code("middle"), None);
}

#[tokio::test]
async fn dock_is_refused_on_a_pane_the_caller_did_not_open_or_that_is_not_floating() {
    let state = test_state();
    let (owner, other) = (an_agent(), an_agent());
    pane_openers::record("pd-float", &owner, true);
    pane_openers::record("pd-docked", &owner, false);

    let (status, body) = said(dock(&state, &other, &request("dock", "pd-float", Some("own"))).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!(super::super::pane_place::NOT_YOURS));
    let (status, body) = said(dock(&state, &owner, &request("dock", "pd-docked", Some("own"))).await).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("already docked"), "{body}");
    for b in ["pd-float", "pd-docked"] {
        pane_openers::forget(b);
    }
}

#[tokio::test]
async fn dock_takes_a_side_or_nothing_and_needs_the_callers_own_pane() {
    let state = test_state();
    let agent = an_agent();
    pane_openers::record("pd-arg", &agent, true);

    let mut sized = request("dock", "pd-arg", Some("own"));
    sized.place.width = Some(json!(700));
    let (status, body) = said(dock(&state, &agent, &sized).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("dock takes no size or place"), "{body}");

    let mut bad = request("dock", "pd-arg", Some("own"));
    bad.split = Some("middle".into());
    let (status, body) = said(dock(&state, &agent, &bad).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("not one of right, left, down, up"), "{body}");

    let (status, body) = said(dock(&state, &agent, &request("dock", "pd-arg", None)).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().starts_with("MISSING_ARG"), "{body}");
    pane_openers::forget("pd-arg");
}

#[tokio::test]
async fn float_is_refused_on_a_pane_the_caller_did_not_open_or_that_already_floats_and_checks_its_values() {
    let state = test_state();
    let (owner, other) = (an_agent(), an_agent());
    pane_openers::record("pf-docked", &owner, false);
    pane_openers::record("pf-float", &owner, true);

    let (status, body) = said(float(&state, &other, &request("float", "pf-docked", None)).await).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!(super::super::pane_place::NOT_YOURS));
    let (status, body) = said(float(&state, &owner, &request("float", "pf-float", None)).await).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().starts_with("NOT_DOCKED"), "{body}");
    let mut bad = request("float", "pf-docked", None);
    bad.place.spot = Some("middle".into());
    let (status, body) = said(float(&state, &owner, &bad).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("is not one of"), "{body}");
    let (status, _) = said(float(&state, &owner, &request("float", "pf-docked", None)).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a pane that is in no tab is gone");
    for b in ["pf-docked", "pf-float"] {
        pane_openers::forget(b);
    }
}

#[tokio::test]
async fn a_docked_pane_floats_into_a_window_of_its_own_and_docks_back_beside_the_callers_pane() {
    let state = test_state();
    let (tab, ws, own, pane) = a_tab_with_two_panes(&state, "sysinfo").await;
    let agent = an_agent();
    pane_openers::record(&pane, &agent, false);
    let seen = watch(&state, rect());
    let mut asked = request("float", &pane, Some(&own));
    asked.place.width = Some(json!(900));
    asked.place.spot = Some("left".into());

    let (status, body) = said(float(&state, &agent, &asked).await).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["placed"]["width"], json!(1040.0), "the window's own answer comes back");
    assert!(pane_openers::is_floating(&pane), "the pane is a floater now");
    let events = seen.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["block_id"], json!(pane));
    assert_eq!((events[0]["place"]["width"].clone(), events[0]["place"]["spot"].clone()), (json!(900), json!("left")));
    assert_eq!(events[0]["place"]["activate"], json!(false), "never takes the foreground");
    drop(events);
    let (new_tab, new_ws) = home_of(&state, &pane).await.expect("the block is in a tab");
    assert_ne!(new_tab, tab, "the pane left the caller's tab for one of its own");
    assert_ne!(new_ws, ws);

    let (status, body) = said(dock(&state, &agent, &request("dock", &pane, Some(&own))).await).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["docked"], json!(true));
    assert!(!pane_openers::is_floating(&pane), "docked again");
    assert!(pane_openers::opened_by(&pane, &agent), "the record keeps who opened it");
    assert_eq!(home_of(&state, &pane).await.unwrap().0, tab, "back in the caller's tab");
    pane_openers::forget(&pane);
}

#[tokio::test]
async fn a_floating_browser_with_no_size_is_the_browsers_default() {
    let state = test_state();
    let (_, _, own, pane) = a_tab_with_two_panes(&state, "browser").await;
    let agent = an_agent();
    pane_openers::record(&pane, &agent, false);
    let seen = watch(&state, rect());

    let (status, body) = said(float(&state, &agent, &request("float", &pane, Some(&own))).await).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let place = seen.lock().unwrap()[0]["place"].clone();
    assert_eq!((place["width"].clone(), place["height"].clone(), place["spot"].clone()), (json!(1040.0), json!(760.0), json!("right")));
    assert_eq!(place["max_share"], json!(0.9));
    pane_openers::forget(&pane);
}

#[tokio::test]
async fn an_agent_with_four_floaters_open_cannot_float_a_fifth() {
    let state = test_state();
    let (_, _, own, docked) = a_tab_with_two_panes(&state, "sysinfo").await;
    let agent = an_agent();
    pane_openers::record(&docked, &agent, false);
    let mut open = Vec::new();
    for _ in 0..floater_place::MAX_FLOATERS {
        let (_, _, _, floater) = a_tab_with_two_panes(&state, "sysinfo").await;
        pane_openers::record(&floater, &agent, true);
        open.push(floater);
    }

    let (status, body) = said(float(&state, &agent, &request("float", &docked, Some(&own))).await).await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body["error"].as_str().unwrap().starts_with("FLOATER_LIMIT"), "{body}");
    assert!(!pane_openers::is_floating(&docked), "nothing moved");
    for b in open.iter().chain([&docked]) {
        pane_openers::forget(b);
    }
}

#[tokio::test]
async fn a_stop_docks_the_floaters_its_agent_opened_and_no_one_elses() {
    let state = test_state();
    let (tab, _, own, pane) = a_tab_with_two_panes(&state, "sysinfo").await;
    let (_, _, _, others_pane) = a_tab_with_two_panes(&state, "sysinfo").await;
    let (agent, other) = (an_agent(), an_agent());
    pane_openers::record(&pane, &agent, false);
    pane_openers::record(&others_pane, &other, false);
    watch(&state, rect());
    for (who, p) in [(&agent, &pane), (&other, &others_pane)] {
        let (status, body) = said(float(&state, who, &request("float", p, Some(&own))).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    assert_eq!(dock_all(&state, &agent, &own).await, 1);

    assert!(!pane_openers::is_floating(&pane), "the stopped agent's floater is docked");
    assert_eq!(home_of(&state, &pane).await.unwrap().0, tab, "into the tab its agent is in");
    assert!(pane_openers::is_floating(&others_pane), "another agent's floater stays where it is");
    assert_eq!(dock_all(&state, &agent, &own).await, 0, "nothing left to dock");
    assert_eq!(dock_all(&state, &agent, "no-such-pane").await, 0, "no home tab, nothing moves");
    for b in [&pane, &others_pane] {
        pane_openers::forget(b);
    }
}

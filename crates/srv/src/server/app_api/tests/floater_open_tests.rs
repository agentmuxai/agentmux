// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent's floating open (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.1, §2.4): what the
// event carries, the answer the open waits for, the record of who opened the pane, and the cap.

use super::*;
use crate::backend::rpc_types::{CommandPaneOpenData, Placed};
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;
use std::sync::{Arc, Mutex};

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// A workspace with one tab and a window on it; the tab's id.
async fn tab_in_a_window(state: &AppState) -> String {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateWindow { window_id: format!("win-{ws_id}"), workspace_id: ws_id.clone() }).await;
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id, name: "t".into() }).await;
    tab.iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

fn an_agent() -> String {
    format!("uid-{}", uuid::Uuid::new_v4())
}

fn floating(tab: &str, opener: Option<&str>) -> CommandPaneOpenData {
    CommandPaneOpenData {
        view: "sysinfo".into(),
        tab_id: Some(tab.into()),
        floating: Some(true),
        opener: opener.map(str::to_string),
        ..Default::default()
    }
}

/// Every `openfloatingpane` the broker publishes, as its JSON; `answer` makes the window reply with the rect.
fn watch_floaters(state: &AppState, answer: Option<Placed>) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rec = seen.clone();
    state.broker.add_observer(Arc::new(move |ev: &crate::backend::mps::MuxEvent| {
        if ev.event != "openfloatingpane" {
            return;
        }
        let data = ev.data.clone().unwrap_or_default();
        if let (Some(placed), Some(id)) = (&answer, data["place"]["request_id"].as_str()) {
            assert!(crate::server::app_api::floater_place::answered(id, Ok(placed.clone())), "the open was waiting");
        }
        rec.lock().unwrap().push(data);
    }));
    seen
}

fn rect() -> Placed {
    Placed { x: 100.0, y: 50.0, width: 700.0, height: 500.0, spot: Some("right".into()), clamped: false, display_scale: 1.5 }
}

#[tokio::test]
async fn an_agents_floating_open_carries_what_it_asked_for_and_answers_where_it_went() {
    let state = test_state();
    let tab = tab_in_a_window(&state).await;
    let seen = watch_floaters(&state, Some(rect()));
    let agent = an_agent();
    let cmd = CommandPaneOpenData { width: Some(json!(700)), spot: Some("right".into()), ..floating(&tab, Some(&agent)) };

    let res = open_pane(&state, cmd).await.expect("a floating open");

    assert_eq!(res.placed, Some(rect()), "the answer is where the window put it");
    assert!(!res.queued);
    let events = seen.lock().unwrap();
    assert_eq!(events.len(), 1);
    let place = &events[0]["place"];
    assert_eq!((place["width"].clone(), place["spot"].clone(), place["activate"].clone()), (json!(700), json!("right"), json!(false)));
    assert_eq!(place["cascade"], json!(0), "the first floater has nothing to step from");
    assert!(crate::server::pane_openers::opened_by(&res.block_id, &agent), "the pane is the agent's to place");
    assert_eq!(crate::server::pane_openers::floaters_of(&agent), vec![res.block_id.clone()]);
}

#[tokio::test]
async fn the_persons_own_floating_open_is_as_it_was() {
    let state = test_state();
    let tab = tab_in_a_window(&state).await;
    let seen = watch_floaters(&state, None);

    let res = open_pane(&state, floating(&tab, None)).await.expect("a floating open");

    assert_eq!(res.placed, None, "nothing waits for an answer");
    assert!(!res.queued);
    assert!(seen.lock().unwrap()[0].get("place").is_none(), "no placement rides on the person's open");
    assert_eq!(crate::server::pane_openers::opener_of(&res.block_id), None, "nobody is recorded as having opened it");
}

#[tokio::test]
async fn the_fifth_floating_pane_is_refused_with_the_four_that_are_open() {
    let state = test_state();
    let tab = tab_in_a_window(&state).await;
    let seen = watch_floaters(&state, Some(rect()));
    let agent = an_agent();
    let mut open = Vec::new();
    for n in 0..4 {
        let res = open_pane(&state, floating(&tab, Some(&agent))).await.expect("under the cap");
        open.push(res.block_id);
        assert_eq!(seen.lock().unwrap()[n]["place"]["cascade"], json!(n), "each steps the next one over");
    }
    let blocks_before = state.srv_state.lock().await.blocks.len();

    let refused = open_pane(&state, floating(&tab, Some(&agent))).await.unwrap_err();

    assert!(refused.starts_with("FLOATER_LIMIT") && refused.contains("dock or close one first"), "{refused}");
    for b in &open {
        assert!(refused.contains(b.as_str()), "the refusal names {b}");
    }
    assert_eq!(state.srv_state.lock().await.blocks.len(), blocks_before, "nothing was made for the refused open");

    // Another agent has its own four, and a docked pane does not count.
    let other = an_agent();
    assert!(open_pane(&state, floating(&tab, Some(&other))).await.is_ok());
}

#[tokio::test]
async fn a_window_that_does_not_answer_leaves_the_open_queued() {
    let state = test_state();
    let tab = tab_in_a_window(&state).await;
    let seen = watch_floaters(&state, None);
    let agent = an_agent();

    let res = open_pane(&state, floating(&tab, Some(&agent))).await.expect("a floating open");

    assert!(res.queued, "no answer in time: the pane is made and the window takes its place when it opens");
    assert_eq!(res.placed, None);
    let id = seen.lock().unwrap()[0]["place"]["request_id"].as_str().unwrap().to_string();
    assert!(!crate::server::app_api::floater_place::answered(&id, Err("late".into())), "the wait was dropped");
}

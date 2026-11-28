// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent's resize, move and place of a floating pane it opened, and the layout's report of where they are
// (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.2, §2.3, §2.4).

use super::*;
use crate::server::pane_openers;
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};
use std::sync::{Arc, Mutex};

fn args(v: serde_json::Value) -> PlaceArgs {
    PlaceArgs::of(&v).expect("valid placement arguments")
}

#[test]
fn each_action_takes_what_it_changes_and_names_the_one_to_use_for_the_rest() {
    assert!(check("resize", &args(json!({ "width": 700 }))).is_ok());
    assert!(check("resize", &args(json!({ "height": "50%" }))).is_ok());
    assert!(check("move", &args(json!({ "spot": "top-left" }))).is_ok());
    assert!(check("move", &args(json!({ "x": 10, "y": 20 }))).is_ok());
    assert!(check("place", &args(json!({ "width": 700, "spot": "right" }))).is_ok());
    assert!(check("place", &args(json!({ "x": 0, "y": 0 }))).is_ok());

    let e = |op: &str, a: serde_json::Value| check(op, &args(a)).unwrap_err();
    assert!(e("resize", json!({})).contains("resize needs width and/or height"));
    assert!(e("resize", json!({ "width": 700, "spot": "left" })).contains("use move for a position, or place for both"));
    assert!(e("move", json!({})).contains("move needs spot, or x and y"));
    assert!(e("move", json!({ "spot": "left", "height": 400 })).contains("use resize for a size, or place for both"));
    assert!(e("place", json!({})).contains("place needs a size, a position, or both"));
    assert!(e("shrink", json!({ "width": 5 })).contains("use resize, move, place, dock or float"));
}

#[test]
fn the_values_are_checked_as_an_open_checks_them() {
    let e = |op: &str, a: serde_json::Value| check(op, &args(a)).unwrap_err();
    assert!(e("resize", json!({ "width": -4 })).starts_with("INVALID_ARG"));
    assert!(e("resize", json!({ "width": "150%" })).starts_with("INVALID_ARG"));
    assert!(e("move", json!({ "spot": "middle" })).contains("is not one of"));
    assert!(e("move", json!({ "x": 5 })).contains("x and y go together"));
}

#[test]
fn a_resize_keeps_the_position_a_move_keeps_the_size_and_a_place_keeps_neither() {
    assert_eq!(check("resize", &args(json!({ "width": 700 }))).unwrap().0.keep(), Some("position"));
    assert_eq!(check("move", &args(json!({ "spot": "left" }))).unwrap().0.keep(), Some("size"));
    assert_eq!(check("place", &args(json!({ "width": 700 }))).unwrap().0.keep(), None);
}

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// A block alone in a workspace that has a window: what a floating pane is once its window is open.
async fn floater_in_a_window(state: &AppState) -> String {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "f".into() }).await;
    let ws_id = ws
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateWindow { window_id: format!("win-{ws_id}"), workspace_id: ws_id.clone() }).await;
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id, name: "t".into() }).await;
    let tab_id = tab
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let block = dispatch_apply(state, Command::CreateBlock { tab_id, meta: json!({ "view": "sysinfo" }) }).await;
    block
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

fn an_agent() -> String {
    format!("uid-{}", uuid::Uuid::new_v4())
}

fn rect() -> Placed {
    Placed { x: 120.0, y: 60.0, width: 900.0, height: 600.0, spot: None, clamped: false, display_scale: 2.0 }
}

/// Every `placefloatingpane` the broker publishes, as its JSON; `answer` makes the window reply.
fn watch(state: &AppState, answer: Option<Result<Placed, String>>) -> Arc<Mutex<Vec<serde_json::Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rec = seen.clone();
    state.broker.add_observer(Arc::new(move |ev: &crate::backend::mps::MuxEvent| {
        if ev.event != "placefloatingpane" {
            return;
        }
        let data = ev.data.clone().unwrap_or_default();
        if let (Some(a), Some(id)) = (&answer, data["request_id"].as_str()) {
            assert!(floater_place::answered(id, a.clone()), "the call was waiting");
        }
        rec.lock().unwrap().push(data);
    }));
    seen
}

#[tokio::test]
async fn a_pane_the_caller_did_not_open_is_refused_with_what_to_do_instead() {
    let state = test_state();
    let block = floater_in_a_window(&state).await;
    let (owner, other) = (an_agent(), an_agent());
    pane_openers::record(&block, &owner, true);

    let (status, text) = floater_of(&state, &other, &block).await.unwrap_err();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(text, NOT_YOURS);
    assert!(text.contains("ask the person to move it, or open your own"));
    let (status, _) = floater_of(&state, &other, "no-such-pane").await.unwrap_err();
    assert_eq!(status, StatusCode::FORBIDDEN, "a pane nobody opened is not the caller's either");
    assert!(floater_of(&state, &owner, &block).await.is_ok(), "the opener may");
    pane_openers::forget(&block);
}

#[tokio::test]
async fn a_docked_pane_is_not_a_floating_one() {
    let state = test_state();
    let block = floater_in_a_window(&state).await;
    let agent = an_agent();
    pane_openers::record(&block, &agent, false);

    let (status, text) = floater_of(&state, &agent, &block).await.unwrap_err();
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(text.starts_with("NOT_FLOATING") && text.contains("docked"), "{text}");
    pane_openers::forget(&block);
}

#[tokio::test]
async fn a_floating_pane_whose_window_is_gone_says_so() {
    let state = test_state();
    let agent = an_agent();
    pane_openers::record("no-window-block", &agent, true);

    let (status, text) = floater_of(&state, &agent, "no-window-block").await.unwrap_err();
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(text.contains("is not open"), "{text}");
    pane_openers::forget("no-window-block");
}

#[tokio::test]
async fn the_window_is_asked_with_what_to_keep_and_its_answer_comes_back() {
    let state = test_state();
    let block = floater_in_a_window(&state).await;
    let agent = an_agent();
    pane_openers::record(&block, &agent, true);
    let seen = watch(&state, Some(Ok(rect())));
    let (op, cmd) = check("resize", &args(json!({ "width": 900, "height": "60%" }))).unwrap();
    let win = floater_of(&state, &agent, &block).await.unwrap();

    let placed = ask(&state, win, &block, op.keep(), Some(&cmd), floater_place::WAIT).await.expect("the window answered");

    assert_eq!(placed, rect());
    let events = seen.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["block_id"], json!(block));
    assert_eq!(events[0]["keep"], json!("position"));
    let place = &events[0]["place"];
    assert_eq!((place["width"].clone(), place["height"].clone()), (json!(900), json!("60%")));
    assert_eq!((place["activate"].clone(), place["cascade"].clone()), (json!(false), json!(0)), "never activates, never cascades");
    pane_openers::forget(&block);
}

#[tokio::test]
async fn a_window_that_cannot_do_it_says_why_and_one_that_is_silent_is_not_waited_on_for_ever() {
    let state = test_state();
    let block = floater_in_a_window(&state).await;
    let win = {
        let agent = an_agent();
        pane_openers::record(&block, &agent, true);
        floater_of(&state, &agent, &block).await.unwrap()
    };
    let (_, cmd) = check("move", &args(json!({ "spot": "left" }))).unwrap();

    // Silent: nobody listens, and the wait is dropped.
    let silent = ask(&state, win.clone(), &block, Some("size"), Some(&cmd), Duration::from_millis(50)).await.unwrap_err();
    assert!(matches!(silent, Asked::NoAnswer));

    watch(&state, Some(Err("no such window".into())));
    let refused = ask(&state, win, &block, Some("size"), Some(&cmd), floater_place::WAIT).await.unwrap_err();
    assert!(matches!(&refused, Asked::Failed(why) if why == "no such window"), "{refused:?}");
    pane_openers::forget(&block);
}

#[tokio::test]
async fn the_layout_marks_the_floaters_an_agent_opened_with_where_they_are() {
    let state = test_state();
    let block = floater_in_a_window(&state).await;
    pane_openers::record(&block, &an_agent(), true);
    let seen = watch(&state, Some(Ok(rect())));
    let mut layout = json!({ "windows": [{ "tabs": [{ "panes": [
        { "block_id": block, "view": "sysinfo" },
        { "block_id": "docked-one", "view": "term" },
    ] }] }] });

    report_floaters(&state, &mut layout).await;

    let panes = &layout["windows"][0]["tabs"][0]["panes"];
    assert_eq!(panes[0]["floating"], json!(true));
    assert_eq!(panes[0]["rect"], json!({ "x": 120.0, "y": 60.0, "width": 900.0, "height": 600.0, "display_scale": 2.0 }));
    assert!(panes[1].get("floating").is_none() && panes[1].get("rect").is_none(), "a pane nobody floated is left as it is");
    let events = seen.lock().unwrap();
    assert_eq!(events.len(), 1, "only the floater is asked");
    assert!(events[0].get("place").is_none(), "the question changes nothing");
    pane_openers::forget(&block);
}

#[tokio::test]
async fn a_floater_whose_window_is_not_open_gets_a_null_rect_and_the_reason() {
    let state = test_state();
    pane_openers::record("floater-with-no-window", &an_agent(), true);
    let mut layout = json!({ "windows": [{ "tabs": [{ "panes": [{ "block_id": "floater-with-no-window" }] }] }] });

    report_floaters(&state, &mut layout).await;

    let pane = &layout["windows"][0]["tabs"][0]["panes"][0];
    assert_eq!(pane["floating"], json!(true));
    assert_eq!(pane["rect"], json!(null));
    assert_eq!(pane["rect_unavailable"], json!("the window is not open"));
    pane_openers::forget("floater-with-no-window");
}

#[tokio::test]
async fn a_layout_with_no_floaters_asks_nobody() {
    let state = test_state();
    let seen = watch(&state, None);
    let mut layout = json!({ "windows": [{ "tabs": [{ "panes": [{ "block_id": "plain" }] }] }] });
    let before = layout.clone();

    report_floaters(&state, &mut layout).await;

    assert_eq!(layout, before);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_layout_says_whether_the_person_tacked_a_floater_and_gives_it_no_share() {
    let state = test_state();
    let (tacked, loose) = (floater_in_a_window(&state).await, floater_in_a_window(&state).await);
    for b in [&tacked, &loose] {
        pane_openers::record(b, &an_agent(), true);
    }
    // The host writes the tack through to the block's meta (`set_floating_always_on_top`).
    let mut block = state.mstore.get::<crate::backend::obj::Block>(&tacked).unwrap().unwrap();
    block.meta.insert("pane:floating_ontop".into(), json!(true));
    state.mstore.update(&mut block).unwrap();
    watch(&state, Some(Ok(rect())));
    let mut layout = json!({ "windows": [{ "tabs": [{ "panes": [
        { "block_id": tacked, "share": 1.0 },
        { "block_id": loose, "share": 1.0 },
    ] }] }] });

    report_floaters(&state, &mut layout).await;

    let panes = &layout["windows"][0]["tabs"][0]["panes"];
    assert_eq!((panes[0]["always_on_top"].clone(), panes[1]["always_on_top"].clone()), (json!(true), json!(false)));
    assert!(panes[0].get("share").is_none() && panes[1].get("share").is_none(), "a window alone in its tab has a rect, not a share");
    for b in [&tacked, &loose] {
        pane_openers::forget(b);
    }
}

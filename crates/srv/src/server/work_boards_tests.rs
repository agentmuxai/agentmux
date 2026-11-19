// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Several boards on the MuxAPI: filing on a board, listing, claiming, the agent lists, moving a task.

use axum::extract::State;
use axum::response::Json;
use serde_json::{json, Value};

use super::{board_act, boards_view, BoardsRequest};
use crate::server::app_api::kanban::{act_as, Actor};
use crate::server::tests::test_state;
use crate::server::work_queue::{handle_work_claim, ClaimRequest};
use crate::server::work_remote::{tasks_enqueue, tasks_list_for};
use crate::server::AppState;

const ACTOR: &str = "naki@narko";

fn create(state: &AppState, name: &str, agents: &[&str]) {
    let req = BoardsRequest {
        action: "create".into(),
        name: Some(name.into()),
        agents: (!agents.is_empty()).then(|| agents.iter().map(|a| a.to_string()).collect()),
        ..Default::default()
    };
    board_act(state, None, "operator", req).unwrap();
}

fn act(state: &AppState, action: &str, board: &str) -> Result<Value, String> {
    board_act(state, None, "operator", BoardsRequest { action: action.into(), board: Some(board.into()), ..Default::default() })
}

async fn file(state: &AppState, title: &str, board: Option<&str>) -> Value {
    let mut p = json!({ "title": title, "payload": "do it" });
    if let Some(b) = board {
        p["board"] = json!(b);
    }
    tasks_enqueue(state, ACTOR, p).await.unwrap()
}

fn claim(state: &AppState, agent: &str, extra: Value) -> ClaimRequest {
    let mut body = json!({ "agent_id": agent });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let _ = state;
    serde_json::from_value(body).unwrap()
}

async fn claim_body(state: &AppState, agent: &str, extra: Value) -> Value {
    let (_, Json(v)) = handle_work_claim(State(state.clone()), None, Json(claim(state, agent, extra))).await;
    v
}

#[tokio::test]
async fn a_task_is_filed_on_the_board_asked_for_and_its_answer_says_where() {
    let state = test_state();
    create(&state, "Work", &[]);
    let a = file(&state, "on main", None).await;
    assert_eq!((a["ref"].as_str(), a["board"].as_str()), (Some("#1"), Some("main")));
    let b = file(&state, "on work", Some("work")).await;
    assert_eq!((b["ref"].as_str(), b["board"].as_str()), (Some("work#1"), Some("work")));
    let c = file(&state, "on work again", Some("Work")).await;
    assert_eq!(c["ref"].as_str(), Some("work#2"), "a board is found by its name too");
    let pair = tasks_enqueue(&state, ACTOR, json!({ "title": "Feature", "payload": "p", "first": "spec", "board": "work" })).await.unwrap();
    assert_eq!((pair["ref"].as_str(), pair["build_ref"].as_str()), (Some("work#3"), Some("work#4")));
}

#[tokio::test]
async fn a_board_that_is_unknown_archived_or_not_open_to_the_filer_takes_nothing() {
    let state = test_state();
    create(&state, "Work", &[]);
    create(&state, "Team", &["agentx"]);
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "board": "nope" })).await.unwrap_err();
    assert_eq!(e.code, "bad_params");
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "board": "team" })).await.unwrap_err();
    assert!(e.message.contains("no board named"), "a board the filer isn't listed on looks like one that doesn't exist: {}", e.message);
    act(&state, "archive", "work").unwrap();
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "board": "work" })).await.unwrap_err();
    assert!(e.message.contains("archived"), "{}", e.message);
    // The list names the filer: an agent on it files there.
    act(&state, "unarchive", "work").unwrap();
    board_act(
        &state,
        None,
        "operator",
        BoardsRequest { action: "edit".into(), board: Some("team".into()), agents: Some(vec![ACTOR.into()]), ..Default::default() },
    )
    .unwrap();
    assert_eq!(file(&state, "t", Some("team")).await["ref"].as_str(), Some("team#1"));
}

#[tokio::test]
async fn a_task_cannot_start_after_a_task_on_another_board() {
    let state = test_state();
    create(&state, "Work", &[]);
    file(&state, "first", None).await;
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "board": "work", "after": ["1"] })).await.unwrap_err();
    assert!(e.message.contains("own board"), "{}", e.message);
    // Inside the board it is fine.
    let w = file(&state, "w1", Some("work")).await;
    let ok = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "board": "work", "after": [w["id"]] })).await;
    assert!(ok.is_ok());
}

#[tokio::test]
async fn a_target_must_be_on_the_boards_agent_list() {
    let state = test_state();
    create(&state, "Team", &[ACTOR, "agentx"]);
    let mut def = crate::backend::storage::agents::test_agent_def("zed-uid", "Zed", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "board": "team", "target_agent": "zed" })).await.unwrap_err();
    assert!(e.message.contains("agent list"), "{}", e.message);
}

#[tokio::test]
async fn a_list_is_the_default_boards_unless_it_names_one_or_asks_for_all() {
    let state = test_state();
    create(&state, "Work", &[]);
    file(&state, "m", None).await;
    file(&state, "w", Some("work")).await;
    let titles = |v: &Value| v["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let main = tasks_list_for(&state, None, Value::Null).unwrap();
    assert_eq!(titles(&main), vec!["m".to_string()]);
    assert_eq!(main["items"][0]["ref"].as_str(), Some("#1"));
    let work = tasks_list_for(&state, None, json!({ "board": "work" })).unwrap();
    assert_eq!(titles(&work), vec!["w".to_string()]);
    assert_eq!(work["items"][0]["ref"].as_str(), Some("work#1"));
    let mut all = titles(&tasks_list_for(&state, None, json!({ "board": "all" })).unwrap());
    all.sort();
    assert_eq!(all, vec!["m".to_string(), "w".to_string()]);
    assert_eq!(tasks_list_for(&state, None, json!({ "board": "nope" })).unwrap_err().code, "bad_params");
}

#[tokio::test]
async fn a_list_hides_the_boards_an_agent_does_not_see_and_says_when_more_waits_elsewhere() {
    let state = test_state();
    create(&state, "Team", &["agentx", ACTOR]);
    create(&state, "Work", &[]);
    file(&state, "t", Some("team")).await;
    file(&state, "w", Some("work")).await;
    let all = tasks_list_for(&state, Some("someone@narko"), json!({ "board": "all" })).unwrap();
    let titles: Vec<&str> = all["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["w"], "the Team board's task is not shown");
    assert!(tasks_list_for(&state, Some("someone@narko"), json!({ "board": "team" })).is_err());
    // A task for the caller on another board is mentioned on the default board's list.
    let mut def = crate::backend::storage::agents::test_agent_def("zed-uid", "Zed", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let zed_uid = def.id.clone();
    tasks_enqueue(&state, ACTOR, json!({ "title": "for zed", "payload": "p", "board": "work", "target_agent": "zed" })).await.unwrap();
    let view = crate::server::work_queue::list_view(&state, "", None, None, Some(("zed", zed_uid.as_str()))).unwrap();
    let note = view["note"].as_str().unwrap_or_default().to_string();
    assert!(note.contains("1 more for you on Work"), "{note}");
}

#[tokio::test]
async fn a_claim_takes_the_default_board_first_and_a_named_board_only_there() {
    let state = test_state();
    create(&state, "Work", &[]);
    file(&state, "w", Some("work")).await;
    file(&state, "m", None).await;
    let v = claim_body(&state, "naki-2", json!({ "board": "work" })).await;
    assert_eq!((v["ref"].as_str(), v["board"].as_str()), (Some("work#1"), Some("work")), "{v}");
    let v = claim_body(&state, "naki-2", json!({})).await;
    assert_eq!(v["ref"].as_str(), Some("#1"), "{v}");
}

#[tokio::test]
async fn a_task_on_a_board_the_claimer_does_not_see_is_as_good_as_unknown() {
    let state = test_state();
    create(&state, "Team", &["agentx", ACTOR]);
    file(&state, "t", Some("team")).await;
    // Named by its ref, then by a number with the board.
    for extra in [json!({ "id": "team#1" }), json!({ "id": "1", "board": "team" })] {
        let v = claim_body(&state, "naki-2", extra).await;
        assert!(v["claimed"] == json!(false) || v["error"].is_string(), "{v}");
    }
    let v = claim_body(&state, "agentx", json!({ "id": "team#1" })).await;
    assert_eq!(v["claimed"], json!(true), "{v}");
    assert_eq!(v["ref"].as_str(), Some("team#1"));
}

#[tokio::test]
async fn an_agent_manages_the_boards_it_sees_and_nobody_deletes_the_default() {
    let state = test_state();
    create(&state, "Scratch", &[]);
    let seen = boards_view(&state, Some(("naki-2", "")), false).unwrap();
    assert_eq!(seen["boards"].as_array().unwrap().len(), 2);
    assert_eq!(seen["default"].as_str(), Some("main"));
    assert!(act(&state, "delete", "main").is_err());
    assert!(act(&state, "delete", "scratch").is_err(), "archive first");
    act(&state, "archive", "scratch").unwrap();
    assert_eq!(act(&state, "delete", "scratch").unwrap()["deleted"], json!(true));
    assert_eq!(act(&state, "default", "nope").unwrap_err(), "no board named \"nope\"");
    // An agent can't touch a board whose list leaves it out.
    create(&state, "Team", &["agentx"]);
    let e = board_act(&state, Some(("naki-2", "")), "naki-2", BoardsRequest { action: "archive".into(), board: Some("team".into()), ..Default::default() });
    assert_eq!(e.unwrap_err(), "no board named \"team\"");
}

#[tokio::test]
async fn a_move_gives_the_task_a_number_on_the_other_board_and_closes_the_original() {
    let state = test_state();
    create(&state, "Work", &[]);
    file(&state, "a", None).await;
    let moved = act_as(&state, json!({ "action": "move", "id": "#1", "board": "work" }), &Actor::agent("naki-2")).await.unwrap();
    assert_eq!((moved["ref"].as_str(), moved["from"].as_str()), (Some("work#1"), Some("#1")), "{moved}");
    let old = state.work_store.work_board_get("#1").unwrap().unwrap();
    assert_eq!(old.item.state, "done");
    let new = state.work_store.work_board_get("work#1").unwrap().unwrap();
    assert_eq!((new.item.title.as_str(), new.item.state.as_str()), ("a", "open"));
    // A move with no board is still a rank move.
    let rank = act_as(&state, json!({ "action": "move", "id": "work#1", "rank": 5.0 }), &Actor::operator()).await.unwrap();
    assert_eq!(rank["ok"], json!(true));
}

#[tokio::test]
async fn the_operator_files_on_a_board_and_the_list_follows_the_board_it_names() {
    let state = test_state();
    create(&state, "Work", &[]);
    let made = act_as(&state, json!({ "action": "create", "title": "t", "payload": "p", "board": "work" }), &Actor::operator()).await.unwrap();
    assert_eq!(made["ref"].as_str(), Some("work#1"), "{made}");
    let main = crate::server::app_api::kanban::list_for_tests(&state, json!({})).await;
    assert_eq!(main["tasks"].as_array().unwrap().len(), 0);
    let work = crate::server::app_api::kanban::list_for_tests(&state, json!({ "board": "work" })).await;
    assert_eq!(work["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(work["board"].as_str(), Some("work"));
    assert_eq!(work["default_board"].as_str(), Some("main"));
    let all = crate::server::app_api::kanban::list_for_tests(&state, json!({ "board": "all" })).await;
    assert_eq!(all["tasks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn an_agent_left_off_a_boards_list_cannot_act_on_its_tasks() {
    let state = test_state();
    create(&state, "Team", &["agentx", ACTOR]);
    let filed = file(&state, "t", Some("team")).await;
    let id = filed["id"].as_str().unwrap().to_string();
    // By ref or by id, an agent not on the list is told there is no such task.
    for key in ["team#1", id.as_str()] {
        let e = act_as(&state, json!({ "action": "cancel", "id": key }), &Actor::agent("naki-2")).await.unwrap_err();
        assert_eq!(e, "no such task", "{key}");
    }
    assert_eq!(state.work_store.work_queue_get(&id).unwrap().unwrap().state, "open", "nothing changed");
    // An agent on the list can, and so can the operator.
    act_as(&state, json!({ "action": "cancel", "id": "team#1" }), &Actor::agent("agentx")).await.unwrap();
    assert_eq!(state.work_store.work_queue_get(&id).unwrap().unwrap().state, "cancelled");
    let again = file(&state, "t2", Some("team")).await;
    let ok = act_as(&state, json!({ "action": "cancel", "id": again["id"] }), &Actor::operator()).await;
    assert!(ok.is_ok());
}


#[tokio::test]
async fn an_agent_a_boards_list_names_by_uid_can_act_on_its_tasks() {
    let state = test_state();
    create(&state, "Team", &[ACTOR, "uid-naki"]);
    let filed = file(&state, "t", Some("team")).await;
    let id = filed["id"].as_str().unwrap().to_string();
    // Without its UID the same agent is unlisted; with the UID its token proved, it is on the list.
    let e = act_as(&state, json!({ "action": "edit", "id": "team#1", "title": "x" }), &Actor::agent("naki-2")).await.unwrap_err();
    assert_eq!(e, "no such task");
    act_as(&state, json!({ "action": "edit", "id": "team#1", "title": "Renamed" }), &Actor::agent("naki-2").with_uid("uid-naki")).await.unwrap();
    assert_eq!(state.work_store.work_queue_get(&id).unwrap().unwrap().title, "Renamed");
    // A move to another board checks the source board the same way.
    state.work_store.work_board_create(&crate::backend::storage::work_boards::BoardEdit { name: Some("Work".into()), ..Default::default() }, "t", 1).unwrap();
    let moved = act_as(&state, json!({ "action": "move", "id": "team#1", "board": "work" }), &Actor::agent("naki-2").with_uid("uid-naki")).await;
    assert!(moved.is_ok(), "{moved:?}");
}


fn merged_set() -> Value {
    json!({
        "version": 1,
        "stages": [
            { "id": "backlog", "name": "Backlog", "colour": "grey", "takes": ["backlog", "specd"] },
            { "id": "doing", "name": "Doing", "colour": "magenta", "takes": ["research", "progress"] },
            { "id": "needs", "name": "Needs you", "colour": "amber", "takes": ["needs"] },
            { "id": "review", "name": "Review", "colour": "yellow", "takes": ["review"] },
            { "id": "done", "name": "Done", "colour": "green", "takes": ["merged", "released"] }
        ]
    })
}

fn set_stages(state: &AppState, who: Option<(&str, &str)>, board: &str, stages: Value, rev: Option<i64>) -> Result<Value, String> {
    let req = BoardsRequest { action: "edit".into(), board: Some(board.into()), stages: Some(stages), stages_rev: rev, ..Default::default() };
    board_act(state, who, "naki-2", req)
}

#[tokio::test]
async fn a_board_shows_the_default_stages_until_the_person_sets_their_own() {
    let state = test_state();
    let v = boards_view(&state, None, false).unwrap();
    let main = &v["boards"][0];
    assert_eq!((main["stages_default"].clone(), main["stages_rev"].clone(), main["stages_error"].clone()), (json!(true), json!(0), Value::Null));
    assert_eq!(main["stages"]["stages"].as_array().unwrap().len(), 8);
    assert_eq!(main["stages"]["stages"][4]["id"], json!("needs"));

    let out = set_stages(&state, None, "main", merged_set(), Some(0)).unwrap();
    assert_eq!((out["board"]["stages_default"].clone(), out["board"]["stages_rev"].clone()), (json!(false), json!(1)));
    let v = boards_view(&state, None, false).unwrap();
    assert_eq!(v["boards"][0]["stages"]["stages"].as_array().unwrap().len(), 5);

    let reset = board_act(&state, None, "operator", BoardsRequest { action: "edit".into(), board: Some("main".into()), stages_reset: Some(true), ..Default::default() }).unwrap();
    assert_eq!((reset["board"]["stages_default"].clone(), reset["board"]["stages_rev"].clone()), (json!(true), json!(2)));
}

#[tokio::test]
async fn a_refused_stage_set_says_why_and_a_stale_editor_is_told() {
    let state = test_state();
    let mut bad = merged_set();
    bad["stages"][0]["name"] = json!("   ");
    let why = set_stages(&state, None, "main", bad, None).unwrap_err();
    assert!(why.contains("needs a name"), "{why}");
    set_stages(&state, None, "main", merged_set(), Some(0)).unwrap();
    let why = set_stages(&state, None, "main", merged_set(), Some(0)).unwrap_err();
    assert!(why.contains("changed since you read them"), "{why}");
}

#[tokio::test]
async fn a_change_of_stages_leaves_a_line_in_the_boards_history() {
    let state = test_state();
    set_stages(&state, Some(("agentx", "")), "main", merged_set(), Some(0)).unwrap();
    // A refused change leaves nothing.
    assert!(set_stages(&state, None, "main", merged_set(), Some(0)).is_err());
    let reset = BoardsRequest { action: "edit".into(), board: Some("main".into()), stages_reset: Some(true), ..Default::default() };
    board_act(&state, None, "operator", reset).unwrap();

    let v = boards_view(&state, None, false).unwrap();
    let lines = v["boards"][0]["history"].as_array().unwrap();
    assert_eq!(lines.len(), 2, "{lines:?}");
    // Newest first; the operator's own name is never recorded, an agent's id is.
    assert_eq!(lines[0]["body"], json!("reset the stages to the default: 5 → 8"));
    assert_eq!((lines[0]["actor"].clone(), lines[0]["actor_kind"].clone()), (json!(""), json!("operator")));
    assert_eq!(lines[1]["body"], json!("changed the stages: 8 → 5"));
    assert_eq!((lines[1]["actor"].clone(), lines[1]["actor_kind"].clone(), lines[1]["kind"].clone()), (json!("agentx"), json!("agent"), json!("stages")));
    assert!(lines[1].get("detail").is_none() && lines[1]["at"].as_i64().unwrap() > 0);
    // The sets before and after are kept, for a later Restore.
    let kept = state.work_store.work_board_events("main", 5).unwrap();
    let detail: Value = serde_json::from_str(&kept[1].detail).unwrap();
    assert_eq!((detail["from"]["stages"].as_array().unwrap().len(), detail["to"]["stages"].as_array().unwrap().len()), (8, 5));
}

#[tokio::test]
async fn an_agent_sets_stages_only_on_a_board_it_sees() {
    let state = test_state();
    create(&state, "Team", &["agentx"]);
    let why = set_stages(&state, Some(("naki-2", "")), "team", merged_set(), None).unwrap_err();
    assert_eq!(why, "no board named \"team\"");
    assert!(set_stages(&state, Some(("agentx", "")), "team", merged_set(), None).is_ok());
}

#[tokio::test]
async fn the_operator_sets_stages_through_the_kanban_act() {
    let state = test_state();
    let out = act_as(&state, json!({ "action": "board_edit", "board": "main", "stages": merged_set(), "stages_rev": 0 }), &Actor::operator()).await;
    assert_eq!(out.unwrap()["board"]["stages_rev"], json!(1));
}

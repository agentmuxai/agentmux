// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `kanban.act`'s moves that the tests beside it don't reach: what each one answers, writes and
//! refuses, and whose name its history line carries.

use super::*;
use crate::backend::agent_state::test_support::register_stub;
use crate::backend::blockcontroller::{BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
use crate::server::tests::test_state;

async fn file(state: &AppState, title: &str) -> String {
    let made = act(state, json!({"action": "create", "title": title, "payload": "do it"})).await.unwrap();
    made["id"].as_str().unwrap().to_string()
}

fn board_task(state: &AppState, id: &str) -> BoardTask {
    state.work_store.work_board_get(id).unwrap().expect("on the board")
}

fn events_of(state: &AppState, id: &str) -> Vec<(String, String, String)> {
    state.work_store.work_events(id, 50).unwrap().into_iter().map(|e| (e.kind, e.actor, e.actor_kind)).collect()
}

fn has_event(state: &AppState, id: &str, kind: &str, actor: &str, actor_kind: &str) -> bool {
    events_of(state, id).iter().any(|(k, a, ak)| (k.as_str(), a.as_str(), ak.as_str()) == (kind, actor, actor_kind))
}

/// Puts a task in `state` whatever it was.
fn force_state(state: &AppState, id: &str, to: &str) {
    let conn = state.work_store.conn().lock().unwrap();
    conn.execute("UPDATE db_work_queue SET state = ?1 WHERE id = ?2", rusqlite::params![to, id]).unwrap();
}

/// A registered agent whose pane runs.
fn running_agent(state: &AppState, name: &str) -> String {
    let mut block = crate::backend::obj::Block { oid: Uuid::new_v4().to_string(), ..Default::default() };
    state.mstore.insert(&mut block).unwrap();
    register_stub(&block.oid, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    state.reactive_handler.register_agent(name, &block.oid, None).unwrap();
    block.oid
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

// ---- the request itself ----

#[tokio::test]
async fn a_request_the_board_cannot_read_or_name_is_refused() {
    let state = test_state();
    let err = act(&state, json!({"title": "no action"})).await.unwrap_err();
    assert!(err.starts_with("kanban.act: "), "{err}");
    let err = act(&state, json!({"action": "frobnicate"})).await.unwrap_err();
    assert_eq!(err, "kanban.act: unknown action \"frobnicate\"");
}

#[tokio::test]
async fn a_move_on_a_task_needs_an_id_that_names_one() {
    let state = test_state();
    for action in ["edit", "move", "retry", "cancel", "reopen", "answer", "start", "accept"] {
        let err = act(&state, json!({"action": action})).await.unwrap_err();
        assert_eq!(err, "id is required", "{action}");
        let err = act(&state, json!({"action": action, "id": "no-such-task"})).await.unwrap_err();
        assert_eq!(err, "no such task", "{action}");
    }
}

#[tokio::test]
async fn an_agent_cannot_decide_on_a_task_in_review_through_the_operators_window() {
    let state = test_state();
    let id = file(&state, "Review me").await;
    let me = Actor::agent("AgentY");
    for action in ["accept", "send_back"] {
        let err = act_as(&state, json!({"action": action, "id": id}), &me).await.unwrap_err();
        assert_eq!(err, format!("{action}: an agent decides with Tasks {action}"));
    }
}

// ---- create ----

#[tokio::test]
async fn create_needs_a_title_and_defaults_the_payload_to_it() {
    let state = test_state();
    for title in [json!(null), json!(""), json!("   ")] {
        let err = act(&state, json!({"action": "create", "title": title})).await.unwrap_err();
        assert_eq!(err, "a title is required");
    }
    let made = act(&state, json!({"action": "create", "title": "  Just a title  "})).await.unwrap();
    let t = board_task(&state, made["id"].as_str().unwrap());
    assert_eq!((t.item.title.as_str(), t.item.payload.as_str()), ("Just a title", "Just a title"));
    assert_eq!(made["num"], json!(t.card.num));
    assert_eq!(t.item.state, work_state::OPEN);
    assert!(has_event(&state, &t.item.id, "created", "operator", "operator"));
}

#[tokio::test]
async fn create_can_file_a_task_on_hold_for_an_agent_with_a_kind_priority_and_review() {
    let state = test_state();
    let made = act(
        &state,
        json!({"action": "create", "title": "Held", "payload": "p", "hold": true, "agent": "Loap-3", "kind": "fix", "priority": 5, "review": true, "repo": "o/r"}),
    )
    .await
    .unwrap();
    let t = board_task(&state, made["id"].as_str().unwrap());
    assert_eq!(t.item.state, work_state::HELD);
    assert_eq!((t.item.target_agent.as_str(), t.item.kind.as_str(), t.item.priority), ("Loap-3", "fix", 5));
    assert!(t.card.review_required);
    assert_eq!(t.card.repo, "o/r");
    assert!(events_of(&state, &t.item.id).iter().any(|(k, ..)| k == "created"));
}

#[tokio::test]
async fn create_after_another_task_waits_for_it_and_says_so() {
    let state = test_state();
    let first = file(&state, "First").await;
    let made = act(&state, json!({"action": "create", "title": "Second", "after": [first]})).await.unwrap();
    let second = board_task(&state, made["id"].as_str().unwrap());
    assert_eq!(second.card.after.len(), 1);
    let first_num = board_task(&state, &first).card.num;
    let created = state.work_store.work_events(&second.item.id, 10).unwrap().into_iter().find(|e| e.kind == "created").unwrap();
    assert_eq!(created.body, format!("starts after #{first_num}"));
    // A list that names nothing real is refused and files nothing.
    let before = state.work_store.work_board_list(0, 100, now_ms()).unwrap().len();
    assert!(act(&state, json!({"action": "create", "title": "Third", "after": ["no-such-task"]})).await.is_err());
    assert_eq!(state.work_store.work_board_list(0, 100, now_ms()).unwrap().len(), before);
}

// ---- edit ----

#[tokio::test]
async fn edit_changes_the_fields_it_is_given_and_leaves_the_rest() {
    let state = test_state();
    let id = file(&state, "Old title").await;
    let res = act(&state, json!({"action": "edit", "id": id, "title": "New title", "kind": "fix", "priority": 3, "review": true, "repo": "o/r"})).await.unwrap();
    assert_eq!(res, json!({"ok": true}));
    let t = board_task(&state, &id);
    assert_eq!((t.item.title.as_str(), t.item.payload.as_str(), t.item.kind.as_str(), t.item.priority), ("New title", "do it", "fix", 3));
    assert!(t.card.review_required);
    assert_eq!(t.card.repo, "o/r");
    assert!(has_event(&state, &id, "edited", "operator", "operator"));

    act(&state, json!({"action": "edit", "id": id, "payload": "do it differently"})).await.unwrap();
    let t = board_task(&state, &id);
    assert_eq!((t.item.title.as_str(), t.item.payload.as_str()), ("New title", "do it differently"));
}

#[tokio::test]
async fn edit_replaces_what_a_task_starts_after_and_a_refused_list_changes_nothing() {
    let state = test_state();
    let a = file(&state, "A").await;
    let b = file(&state, "B").await;
    act(&state, json!({"action": "edit", "id": b, "after": [a]})).await.unwrap();
    assert_eq!(board_task(&state, &b).card.after.len(), 1);
    let edited = state.work_store.work_events(&b, 10).unwrap().into_iter().find(|e| e.kind == "edited").unwrap();
    assert_eq!(edited.body, format!("starts after #{}", board_task(&state, &a).card.num));

    // A cycle is refused, and the title in the same request is not applied.
    let err = act(&state, json!({"action": "edit", "id": a, "title": "Renamed", "after": [b]})).await.unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(board_task(&state, &a).item.title, "A");

    // `[]` clears the list.
    act(&state, json!({"action": "edit", "id": b, "after": []})).await.unwrap();
    assert!(board_task(&state, &b).card.after.is_empty());
}

#[tokio::test]
async fn an_agents_payload_edit_is_the_agents_words_and_the_operators_is_not() {
    let state = test_state();
    let id = file(&state, "Whose words").await;
    act_as(&state, json!({"action": "edit", "id": id, "payload": "from the agent"}), &Actor::agent("AgentY")).await.unwrap();
    assert_eq!(board_task(&state, &id).card.filed_by, "AgentY");
    assert!(has_event(&state, &id, "edited", "AgentY", "agent"));

    let other = file(&state, "Operator edit").await;
    act(&state, json!({"action": "edit", "id": other, "payload": "from the operator"})).await.unwrap();
    assert_eq!(board_task(&state, &other).card.filed_by, "");
    // An agent's edit that leaves the payload alone claims nothing.
    let third = file(&state, "Title only").await;
    act_as(&state, json!({"action": "edit", "id": third, "title": "Renamed"}), &Actor::agent("AgentY")).await.unwrap();
    assert_eq!(board_task(&state, &third).card.filed_by, "");
}

// ---- move ----

#[tokio::test]
async fn move_gives_a_waiting_task_to_another_agent_or_to_anyone() {
    let state = test_state();
    let id = file(&state, "Movable").await;
    act(&state, json!({"action": "move", "id": id, "agent": "Loap-3"})).await.unwrap();
    assert_eq!(board_task(&state, &id).item.target_agent, "Loap-3");
    let assigned = state.work_store.work_events(&id, 10).unwrap().into_iter().find(|e| e.kind == "assigned").unwrap();
    assert_eq!(assigned.body, "for Loap-3");

    act(&state, json!({"action": "move", "id": id, "agent": ""})).await.unwrap();
    assert_eq!(board_task(&state, &id).item.target_agent, "");
    let anyone = state.work_store.work_events(&id, 10).unwrap().into_iter().filter(|e| e.kind == "assigned").any(|e| e.body == "for anyone");
    assert!(anyone);

    // A task an agent holds can't be handed over this way.
    state.work_store.work_start_for(&id, "Naki-2", "", now_ms(), LEASE_MS).unwrap().expect("claimed");
    let err = act(&state, json!({"action": "move", "id": id, "agent": "Loap-3"})).await.unwrap_err();
    assert_eq!(err, "only a waiting task can be moved to another agent");
}

#[tokio::test]
async fn move_holds_and_unholds_and_orders_the_backlog() {
    let state = test_state();
    let id = file(&state, "Holdable").await;
    act(&state, json!({"action": "move", "id": id, "hold": true})).await.unwrap();
    assert_eq!(board_task(&state, &id).item.state, work_state::HELD);
    assert!(has_event(&state, &id, "held", "operator", "operator"));
    // Holding a held task again moves nothing and says nothing more.
    act(&state, json!({"action": "move", "id": id, "hold": true})).await.unwrap();
    assert_eq!(events_of(&state, &id).iter().filter(|(k, ..)| k == "held").count(), 1);

    act(&state, json!({"action": "move", "id": id, "hold": false})).await.unwrap();
    assert_eq!(board_task(&state, &id).item.state, work_state::OPEN);
    assert!(has_event(&state, &id, "unheld", "operator", "operator"));

    act(&state, json!({"action": "move", "id": id, "rank": 12.5})).await.unwrap();
    assert_eq!(board_task(&state, &id).card.rank, Some(12.5));
}

#[tokio::test]
async fn a_move_by_an_agent_is_recorded_under_its_name() {
    let state = test_state();
    let id = file(&state, "Agent moves").await;
    let me = Actor::agent("AgentY");
    act_as(&state, json!({"action": "move", "id": id, "agent": "Loap-3", "hold": true}), &me).await.unwrap();
    assert!(has_event(&state, &id, "assigned", "AgentY", "agent"));
    assert!(has_event(&state, &id, "held", "AgentY", "agent"));
}

// ---- start ----

#[tokio::test]
async fn start_needs_an_agent_that_is_here_and_running() {
    let state = test_state();
    let id = file(&state, "Start me").await;
    let err = act(&state, json!({"action": "start", "id": id})).await.unwrap_err();
    assert_eq!(err, "start: which agent?");
    let err = act(&state, json!({"action": "start", "id": id, "agent": "nobody-here"})).await.unwrap_err();
    assert_eq!(err, "no agent named nobody-here here");

    // Registered, but its process is not running.
    let name = unique("idle");
    let mut block = crate::backend::obj::Block { oid: Uuid::new_v4().to_string(), ..Default::default() };
    state.mstore.insert(&mut block).unwrap();
    state.reactive_handler.register_agent(&name, &block.oid, None).unwrap();
    let err = act(&state, json!({"action": "start", "id": id, "agent": name})).await.unwrap_err();
    assert_eq!(err, format!("{name} isn't running"));
    assert_eq!(board_task(&state, &id).item.state, work_state::OPEN, "nothing was taken");
}

#[tokio::test]
async fn start_on_a_running_agent_claims_the_task_or_puts_it_back_when_the_agent_cannot_be_told() {
    let state = test_state();
    let name = unique("runner");
    running_agent(&state, &name);
    let id = file(&state, "Start me").await;
    let res = act(&state, json!({"action": "start", "id": id, "agent": name})).await;
    let t = board_task(&state, &id);
    match res {
        Ok(v) => {
            assert_eq!(v["ok"], true);
            assert_eq!(v["attempt"], json!(t.item.attempts));
            assert_eq!((t.item.state.as_str(), t.item.claimed_by.as_str()), (work_state::CLAIMED, name.as_str()));
            assert!(events_of(&state, &id).iter().any(|(k, ..)| k == "started"));
        }
        Err(e) => {
            assert_eq!(e.split(':').next(), Some(format!("couldn't reach {name}").as_str()), "{e}");
            assert_eq!(t.item.state, work_state::OPEN, "put back as it was");
            assert_eq!(t.item.attempts, 0, "its attempt not used");
            assert!(events_of(&state, &id).iter().any(|(k, ..)| k == "start_failed"));
        }
    }
    // A task that is not waiting can't be started.
    let held = file(&state, "Held").await;
    state.work_store.work_start_for(&held, "Naki-2", "", now_ms(), LEASE_MS).unwrap().expect("claimed");
    let err = act(&state, json!({"action": "start", "id": held, "agent": name})).await.unwrap_err();
    assert_eq!(err, "only a waiting task can be started");
}

// ---- answer ----

#[tokio::test]
async fn answer_needs_words_a_question_and_a_running_asker() {
    let state = test_state();
    let id = file(&state, "Asked").await;
    let err = act(&state, json!({"action": "answer", "id": id})).await.unwrap_err();
    assert_eq!(err, "the answer is empty");
    let err = act(&state, json!({"action": "answer", "id": id, "text": "yes"})).await.unwrap_err();
    assert_eq!(err, "this task isn't waiting on a question");

    force_state(&state, &id, work_state::BLOCKED);
    let err = act(&state, json!({"action": "answer", "id": id, "text": "yes"})).await.unwrap_err();
    assert_eq!(err, "its agent isn't running");
    assert_eq!(board_task(&state, &id).item.state, work_state::BLOCKED, "nothing changed");
}

#[tokio::test]
async fn answer_to_a_running_holder_either_resumes_the_task_or_leaves_it_blocked() {
    let state = test_state();
    let name = unique("asker");
    running_agent(&state, &name);
    let id = file(&state, "Asked").await;
    state.work_store.work_start_for(&id, &name, "", now_ms(), LEASE_MS).unwrap().expect("claimed");
    force_state(&state, &id, work_state::BLOCKED);
    let res = act(&state, json!({"action": "answer", "id": id, "text": "use the second one"})).await;
    let t = board_task(&state, &id);
    match res {
        Ok(v) => {
            assert_eq!(v, json!({"ok": true}));
            assert_eq!(t.item.state, work_state::CLAIMED);
            assert!(has_event(&state, &id, "answered", "operator", "operator"));
        }
        Err(e) => {
            assert!(e.starts_with(&format!("couldn't reach {name}: ")), "{e}");
            assert_eq!(t.item.state, work_state::BLOCKED);
        }
    }
}

// ---- retry ----

#[tokio::test]
async fn retry_puts_a_failed_task_back_and_refuses_any_other() {
    let state = test_state();
    let id = file(&state, "Failing").await;
    let err = act(&state, json!({"action": "retry", "id": id})).await.unwrap_err();
    assert_eq!(err, "only a failed task can be retried");

    force_state(&state, &id, work_state::FAILED);
    let res = act(&state, json!({"action": "retry", "id": id})).await.unwrap();
    assert_eq!(res, json!({"ok": true}));
    assert_eq!(board_task(&state, &id).item.state, work_state::OPEN);
    assert!(has_event(&state, &id, "retried", "operator", "operator"));
}

#[tokio::test]
async fn an_agents_retry_is_recorded_under_its_name() {
    let state = test_state();
    let id = file(&state, "Failing").await;
    force_state(&state, &id, work_state::FAILED);
    act_as(&state, json!({"action": "retry", "id": id}), &Actor::agent("AgentY")).await.unwrap();
    assert!(has_event(&state, &id, "retried", "AgentY", "agent"));
}

// ---- cancel ----

#[tokio::test]
async fn cancel_of_a_finished_task_is_refused_and_a_plain_task_is_cancelled_with_its_reason() {
    let state = test_state();
    let id = file(&state, "Not needed").await;
    let res = act(&state, json!({"action": "cancel", "id": id, "text": "no longer wanted"})).await.unwrap();
    assert_eq!(res, json!({"ok": true, "followers": 0}));
    assert_eq!(board_task(&state, &id).item.state, work_state::CANCELLED);
    let cancelled = state.work_store.work_events(&id, 10).unwrap().into_iter().find(|e| e.kind == "cancelled").unwrap();
    assert_eq!(cancelled.body, "no longer wanted");

    let err = act(&state, json!({"action": "cancel", "id": id})).await.unwrap_err();
    assert_eq!(err, "it is already finished");
}

#[tokio::test]
async fn cancelling_a_spec_whose_build_is_already_on_hold_cancels_it_with_the_spec() {
    let state = test_state();
    let v = act(&state, json!({"action": "create", "title": "Feature", "first": "spec"})).await.unwrap();
    let (spec, build) = (v["id"].as_str().unwrap().to_string(), v["build_id"].as_str().unwrap().to_string());
    act(&state, json!({"action": "move", "id": build, "hold": true})).await.unwrap();
    let res = act(&state, json!({"action": "cancel", "id": spec})).await.unwrap();
    assert_eq!(res["followers"], 1);
    assert_eq!(board_task(&state, &build).item.state, work_state::CANCELLED);
    let why = state.work_store.work_events(&build, 10).unwrap().into_iter().find(|e| e.kind == "cancelled").unwrap();
    assert_eq!(why.body, format!("its spec #{} was cancelled", board_task(&state, &spec).card.num));
}

// ---- track ----

#[tokio::test]
async fn track_needs_a_pane_that_has_an_agent() {
    let state = test_state();
    let err = act(&state, json!({"action": "track"})).await.unwrap_err();
    assert_eq!(err, "track: which agent pane?");
    let err = act(&state, json!({"action": "track", "block_id": "no-such-pane"})).await.unwrap_err();
    assert_eq!(err, "that pane has no agent");
}

#[tokio::test]
async fn track_follows_a_running_agents_work_as_a_task_it_holds() {
    let state = test_state();
    let name = unique("tracked");
    let block = running_agent(&state, &name);
    let res = act(&state, json!({"action": "track", "block_id": block, "title": "  The thing  it does "})).await.unwrap();
    assert_eq!(res["ok"], true);
    let num = res["num"].as_i64().unwrap();
    let board = list(&state, &Value::Null).await.unwrap();
    let t = board["tasks"].as_array().unwrap().iter().find(|t| t["card"]["num"] == num).cloned().expect("on the board");
    assert_eq!(t["title"], "The thing  it does", "trimmed, not collapsed");
    assert_eq!((t["state"].as_str(), t["claimed_by"].as_str(), t["target_agent"].as_str()), (Some("claimed"), Some(name.as_str()), Some(name.as_str())));
    assert_eq!(t["card"]["review_required"], true);
    let id = t["id"].as_str().unwrap();
    assert!(has_event(&state, id, "tracked", "operator", "operator"));
}

// ---- release ----

#[tokio::test]
async fn reassign_names_who_it_goes_to_and_the_old_holder_is_told() {
    let state = test_state();
    let id = file(&state, "Handed over").await;
    state.work_store.work_start_for(&id, "Naki-2", "", now_ms(), LEASE_MS).unwrap().expect("claimed");
    let err = act(&state, json!({"action": "reassign", "id": id})).await.unwrap_err();
    assert_eq!(err, "reassign: to which agent? (empty for anyone)");
    assert_eq!(board_task(&state, &id).item.state, work_state::CLAIMED, "nothing changed");

    let res = act(&state, json!({"action": "reassign", "id": id, "agent": "Loap-3", "text": "needs another pair of hands"})).await.unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["id"], json!(id));
    let ev = state.work_store.work_events(&id, 10).unwrap().into_iter().find(|e| e.kind == "reassigned").unwrap();
    assert_eq!(ev.body, "taken back from Naki-2: needs another pair of hands; now for Loap-3");
    let ev = {
        let again = file(&state, "Anyone").await;
        state.work_store.work_start_for(&again, "Naki-2", "", now_ms(), LEASE_MS).unwrap().expect("claimed");
        act(&state, json!({"action": "reassign", "id": again, "agent": ""})).await.unwrap();
        state.work_store.work_events(&again, 10).unwrap().into_iter().find(|e| e.kind == "reassigned").unwrap()
    };
    assert_eq!(ev.body, "taken back from Naki-2; now for anyone");
}

#[tokio::test]
async fn release_with_a_branch_asks_before_it_takes_the_task_back_and_says_so_in_the_answer() {
    let state = test_state();
    let id = file(&state, "Has a branch").await;
    state.work_store.work_start_for(&id, "Naki-2", "", now_ms(), LEASE_MS).unwrap().expect("claimed");
    state
        .work_store
        .work_card_update(&id, &crate::backend::storage::work_board::CardUpdate { branch: Some("naki/feature".into()), ..Default::default() }, now_ms())
        .unwrap();
    let asked = act(&state, json!({"action": "release", "id": id})).await.unwrap();
    assert_eq!(asked["ok"], false);
    assert!(asked["confirm"].as_str().unwrap().starts_with("It has a branch (naki/feature). That work stays where it is"), "{asked}");
    assert_eq!(board_task(&state, &id).item.state, work_state::CLAIMED);
    let events_before = events_of(&state, &id).len();

    let done = act(&state, json!({"action": "release", "id": id, "confirm": true})).await.unwrap();
    assert_eq!(done["ok"], true);
    assert_eq!(board_task(&state, &id).item.state, work_state::OPEN);
    assert_eq!(events_of(&state, &id).len(), events_before + 1);
    assert!(has_event(&state, &id, "taken_back", "operator", "operator"));
}


#[tokio::test]
async fn move_and_start_refuse_an_agent_the_tasks_board_leaves_out() {
    use crate::backend::storage::work_boards::BoardEdit;
    let state = test_state();
    let id = file(&state, "Team only").await;
    let name = unique("outsider");
    running_agent(&state, &name);
    let edit = BoardEdit { agents: Some(vec!["agentx".into()]), ..Default::default() };
    state.work_store.work_board_edit("main", &edit, now_ms()).unwrap();

    let err = act(&state, json!({"action": "start", "id": id, "agent": name})).await.unwrap_err();
    assert!(err.contains("isn't on board main"), "{err}");
    let err = act(&state, json!({"action": "move", "id": id, "agent": "Loap-3"})).await.unwrap_err();
    assert!(err.contains("isn't on board main"), "{err}");
    let t = board_task(&state, &id);
    assert_eq!((t.item.state.as_str(), t.item.target_agent.as_str()), (work_state::OPEN, ""), "nothing was taken or assigned");

    // On the list, it goes through; "anyone" never needed a check.
    act(&state, json!({"action": "move", "id": id, "agent": "agentx"})).await.unwrap();
    act(&state, json!({"action": "move", "id": id, "agent": ""})).await.unwrap();
}

// ---- the boards ----

#[tokio::test]
async fn the_operator_manages_boards_through_kanban_act() {
    let state = test_state();
    let made = act(&state, json!({"action": "board_create", "name": "Client X"})).await.unwrap();
    assert_eq!(made["ok"], true);
    let slug = made["board"]["slug"].as_str().unwrap().to_string();
    act(&state, json!({"action": "board_edit", "board": slug, "name": "Client Y"})).await.unwrap();
    let archived = act(&state, json!({"action": "board_archive", "board": slug})).await.unwrap();
    assert_eq!(archived["board"]["archived"], true);
    let back = act(&state, json!({"action": "board_unarchive", "board": slug})).await.unwrap();
    assert_eq!(back["board"]["archived"], false);
    let listed = list(&state, &json!({"board": "all"})).await.unwrap();
    assert!(listed["boards"].as_array().unwrap().iter().any(|b| b["name"] == "Client Y"));
    // A board needs naming.
    assert!(act(&state, json!({"action": "board_archive"})).await.is_err());
}

#[tokio::test]
async fn an_agent_manages_boards_through_tasks_not_kanban_act() {
    let state = test_state();
    let err = act_as(&state, json!({"action": "board_create", "name": "Mine"}), &Actor::agent("agent9")).await.unwrap_err();
    assert!(err.contains("Tasks board_create"), "{err}");
}

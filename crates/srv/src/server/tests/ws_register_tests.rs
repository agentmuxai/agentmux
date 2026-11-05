// Direct calls to the named handlers `register_handlers` wires: no engine, no RPC framing.

use super::*;
use crate::backend::obj::Block;
use crate::server::tests::test_state;

fn req<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn insert_block(state: &AppState) -> String {
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    state.mstore.insert(&mut block).unwrap();
    block.oid
}

#[tokio::test]
async fn full_config_is_a_json_object() {
    let state = test_state();
    assert!(get_full_config(&state).unwrap().is_object());
}

#[tokio::test]
async fn set_meta_then_get_meta_round_trips_a_key() {
    let state = test_state();
    let id = insert_block(&state);
    set_meta(&state, serde_json::json!({ "oref": format!("block:{id}"), "meta": { "ws-test:key": "v1" } })).unwrap();
    let meta = get_meta(&state, serde_json::json!({ "oref": format!("block:{id}") })).unwrap();
    assert_eq!(meta["ws-test:key"], "v1");
}

#[tokio::test]
async fn get_meta_of_a_missing_object_is_an_error() {
    let state = test_state();
    let err = get_meta(&state, serde_json::json!({ "oref": format!("block:{}", uuid::Uuid::new_v4()) })).unwrap_err();
    assert!(err.starts_with("getmeta: object "), "{err}");
}

#[tokio::test]
async fn a_sub_block_is_linked_to_its_parent_and_unlinked_on_delete() {
    let state = test_state();
    let parent = insert_block(&state);
    let oref = create_sub_block(&state, serde_json::json!({ "parentblockid": parent, "blockdef": { "meta": {} } })).unwrap();
    let child = oref.strip_prefix("block:").expect("a block: oref").to_string();
    let linked = state.mstore.must_get::<Block>(&parent).unwrap().subblockids.unwrap_or_default();
    assert_eq!(linked, vec![child.clone()]);
    assert_eq!(state.mstore.must_get::<Block>(&child).unwrap().parentoref, format!("block:{parent}"));

    delete_sub_block(&state, req(serde_json::json!({ "blockid": child }))).unwrap();
    assert!(state.mstore.get::<Block>(&child).unwrap().is_none());
    assert!(state.mstore.must_get::<Block>(&parent).unwrap().subblockids.unwrap_or_default().is_empty());
}

#[tokio::test]
async fn controller_resync_of_an_unknown_block_is_an_error() {
    let state = test_state();
    let err = controller_resync(&state, serde_json::json!({ "blockid": "no-such-block", "tabid": "t" })).unwrap_err();
    assert_eq!(err, "controllerresync: block no-such-block not found");
}

fn decision(outcome: &str, scope: &str) -> CommandToolDecisionData {
    req(serde_json::json!({ "blockid": "no-such-block", "request_id": "tu-1", "outcome": outcome, "scope": scope }))
}

#[test]
fn tool_decision_refuses_an_unknown_outcome_or_scope_and_a_block_with_no_controller() {
    assert!(tool_decision(decision("maybe", "once")).unwrap_err().contains("invalid outcome 'maybe'"));
    assert!(tool_decision(decision("allow", "forever")).unwrap_err().contains("invalid scope 'forever'"));
    assert_eq!(tool_decision(decision("allow", "once")).unwrap_err(), "tooldecision: no controller for block no-such-block");
}

#[test]
fn agent_answer_and_cancel_keep_the_errors_the_frontend_matches() {
    let answer = agent_answer(serde_json::json!({ "blockid": "b", "tool_use_id": "", "answers": {} })).unwrap_err();
    assert_eq!(answer, "agent.answer: MISSING_ARG: tool_use_id");
    let cancel = agent_cancel(req(serde_json::json!({ "blockid": "no-such-block", "tool_use_id": "tu-1" }))).unwrap_err();
    assert_eq!(cancel, "agent.cancel: no controller for block no-such-block");
}

fn dock(node: &str, block: &str, background: bool) -> CommandDockNodeStatusData {
    req(serde_json::json!({ "blockid": block, "node_id": node, "tool_name": "Bash", "status": "success", "run_in_background": background }))
}

#[tokio::test]
async fn a_background_launch_is_listed_with_its_pid_and_then_completed() {
    let state = test_state();
    let block = insert_block(&state);
    dock_node_status(&state, dock("node-1", &block, true)).unwrap();
    background_task_pid(&state, req(serde_json::json!({ "blockid": block, "node_id": "node-1", "pid": 4242 }))).unwrap();

    let list = |state: &AppState| list_background_tasks(state, req(serde_json::json!({ "blockid": block }))).unwrap();
    let tasks = list(&state);
    assert_eq!(tasks.len(), 1);
    assert_eq!((tasks[0].id.as_str(), tasks[0].pid, tasks[0].status), ("node-1", Some(4242), "running"));

    background_task_completion(&state, req(serde_json::json!({ "blockid": block, "node_id": "node-1", "status": "done" }))).unwrap();
    assert_eq!(list(&state)[0].status, "done");
}

#[tokio::test]
async fn a_status_push_that_is_not_a_background_launch_records_no_task() {
    let state = test_state();
    let block = insert_block(&state);
    dock_node_status(&state, dock("node-2", &block, false)).unwrap();
    assert!(list_background_tasks(&state, req(serde_json::json!({ "blockid": block }))).unwrap().is_empty());
}

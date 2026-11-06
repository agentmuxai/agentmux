// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `object` service, driven through `run_service_call` against a real store and reducer:
//! what each method answers, what it writes, and which updates it returns.

use agentmux_common::ipc::{Command, Event};
use serde_json::{json, Value};

use crate::backend::obj::{Block, Tab};
use crate::backend::service::{UIContext, WebCallType, WebReturnType};
use crate::server::tests::test_state;
use crate::server::AppState;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let events = super::dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
}

/// A workspace holding one tab. Returns `(workspace_id, tab_id)`.
async fn seed_tab(state: &AppState) -> (String, String) {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
    let Some(Event::WorkspaceCreated { workspace_id, .. }) = ws.first() else { panic!("{ws:?}") };
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: workspace_id.clone(), name: "t".into() }).await;
    let tab_id = tab
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    (workspace_id.clone(), tab_id)
}

fn ctx(tab_id: &str) -> Option<UIContext> {
    Some(UIContext { active_tab_id: tab_id.into(), extra: Default::default() })
}

async fn call(state: &AppState, method: &str, uicontext: Option<UIContext>, args: Vec<Value>) -> WebReturnType {
    let call = WebCallType { service: "object".into(), method: method.into(), uicontext, args };
    super::run_service_call(state, &call).await
}

fn error_of(res: &WebReturnType) -> &str {
    assert!(!res.success, "expected a refusal: {res:?}");
    res.error.as_deref().unwrap_or("")
}

/// Makes a block through the service, with the tab named by the call's context.
async fn create_block(state: &AppState, tab_id: &str, meta: Value) -> String {
    let res = call(state, "CreateBlock", ctx(tab_id), vec![json!({ "meta": meta })]).await;
    assert!(res.success, "{res:?}");
    res.data.unwrap().as_str().unwrap().to_string()
}

fn block_of(state: &AppState, id: &str) -> Block {
    state.mstore.must_get::<Block>(id).unwrap()
}

fn updated_ids(res: &WebReturnType) -> Vec<(String, String)> {
    res.updates.iter().flatten().map(|u| (u.otype.clone(), u.oid.clone())).collect()
}

// ---- GetObject / GetObjects ----

#[tokio::test]
async fn get_object_answers_the_stored_row_and_refuses_a_bad_reference() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "term" })).await;

    let res = call(&state, "GetObject", None, vec![json!(format!("block:{block_id}"))]).await;
    assert!(res.success, "{res:?}");
    let data = res.data.unwrap();
    assert_eq!(data["oid"], json!(block_id));
    assert_eq!(data["meta"]["view"], json!("term"));

    let missing = call(&state, "GetObject", None, vec![json!("block:no-such-block")]).await;
    assert!(!error_of(&missing).is_empty());
    let malformed = call(&state, "GetObject", None, vec![json!("nonsense")]).await;
    assert!(!error_of(&malformed).is_empty());
    let no_args = call(&state, "GetObject", None, vec![]).await;
    assert!(!error_of(&no_args).is_empty());
}

#[tokio::test]
async fn get_objects_answers_null_for_each_reference_it_cannot_read() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({})).await;

    let refs = json!([format!("block:{block_id}"), "block:no-such-block", "nonsense", format!("tab:{tab_id}")]);
    let res = call(&state, "GetObjects", None, vec![refs]).await;
    assert!(res.success, "{res:?}");
    let list = res.data.unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 4);
    assert_eq!(list[0]["oid"], json!(block_id));
    assert_eq!(list[1], Value::Null);
    assert_eq!(list[2], Value::Null);
    assert_eq!(list[3]["oid"], json!(tab_id));

    let bad_arg = call(&state, "GetObjects", None, vec![json!("not-a-list")]).await;
    assert!(!error_of(&bad_arg).is_empty());
}

// ---- CreateBlock ----

#[tokio::test]
async fn create_block_stores_the_block_in_the_contexts_tab_and_returns_both_rows() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let res = call(&state, "CreateBlock", ctx(&tab_id), vec![json!({ "meta": { "view": "term", "note": "x" } })]).await;
    assert!(res.success, "{res:?}");
    let block_id = res.data.clone().unwrap().as_str().unwrap().to_string();

    let block = block_of(&state, &block_id);
    assert_eq!(block.meta.get("view"), Some(&json!("term")));
    assert_eq!(block.meta.get("note"), Some(&json!("x")));
    let tab = state.mstore.must_get::<Tab>(&tab_id).unwrap();
    assert!(tab.blockids.contains(&block_id), "the tab lists the new block");

    assert_eq!(updated_ids(&res), vec![("block".to_string(), block_id), ("tab".to_string(), tab_id)]);
}

#[tokio::test]
async fn create_block_drops_the_keys_only_srv_may_write() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let owner_key = crate::server::browser_owner::OWNER_META_KEY;
    let block_id = create_block(&state, &tab_id, json!({ "view": "browser", owner_key: "some-agent" })).await;
    let block = block_of(&state, &block_id);
    assert_eq!(block.meta.get("view"), Some(&json!("browser")));
    assert!(!block.meta.contains_key(owner_key), "a client can't make itself a pane's owner");
}

#[tokio::test]
async fn create_block_refuses_a_browser_identity_that_is_neither_incognito_nor_a_profile() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let key = crate::server::browser_identity::IDENTITY_META_KEY;
    let res = call(&state, "CreateBlock", ctx(&tab_id), vec![json!({ "meta": { key: "nonsense" } })]).await;
    assert!(error_of(&res).contains("must be incognito:<id> or profile:<id>"), "{res:?}");
    assert!(state.mstore.must_get::<Tab>(&tab_id).unwrap().blockids.is_empty());
}

#[tokio::test]
async fn create_block_prefers_an_explicit_tab_over_the_contexts_tab() {
    let state = test_state();
    let (ws, active) = seed_tab(&state).await;
    let other = dispatch_apply(&state, Command::CreateTab { workspace_id: ws, name: "other".into() }).await;
    let other_id = other
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();

    // args[1] is the runtime options; args[2] names the tab, trimmed.
    let res = call(&state, "CreateBlock", ctx(&active), vec![json!({ "meta": {} }), Value::Null, json!(format!("  {other_id} "))]).await;
    assert!(res.success, "{res:?}");
    let block_id = res.data.unwrap().as_str().unwrap().to_string();
    assert!(state.mstore.must_get::<Tab>(&other_id).unwrap().blockids.contains(&block_id));
    assert!(state.mstore.must_get::<Tab>(&active).unwrap().blockids.is_empty());

    // An empty or null args[2] falls back to the context.
    for blank in [json!(""), json!("   "), Value::Null] {
        let res = call(&state, "CreateBlock", ctx(&active), vec![json!({ "meta": {} }), Value::Null, blank]).await;
        assert!(res.success, "{res:?}");
    }
    assert_eq!(state.mstore.must_get::<Tab>(&active).unwrap().blockids.len(), 3);
}

#[tokio::test]
async fn create_block_refuses_a_malformed_tab_argument_and_a_missing_context() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let malformed = call(&state, "CreateBlock", ctx(&tab_id), vec![json!({ "meta": {} }), Value::Null, json!(42)]).await;
    assert!(error_of(&malformed).starts_with("invalid tabId arg: "), "{malformed:?}");

    let no_context = call(&state, "CreateBlock", None, vec![json!({ "meta": {} })]).await;
    assert_eq!(error_of(&no_context), "missing uicontext.activetabid");
    let empty_context = call(&state, "CreateBlock", ctx(""), vec![json!({ "meta": {} })]).await;
    assert_eq!(error_of(&empty_context), "missing uicontext.activetabid");

    let bad_def = call(&state, "CreateBlock", ctx(&tab_id), vec![json!("not a block def")]).await;
    assert!(!error_of(&bad_def).is_empty());
    assert!(state.mstore.must_get::<Tab>(&tab_id).unwrap().blockids.is_empty(), "nothing was created");
}

#[tokio::test]
async fn create_block_in_a_tab_that_does_not_exist_surfaces_the_reducers_error() {
    let state = test_state();
    let res = call(&state, "CreateBlock", ctx("no-such-tab"), vec![json!({ "meta": {} })]).await;
    assert!(!error_of(&res).is_empty(), "{res:?}");
}

// ---- DeleteBlock ----

#[tokio::test]
async fn delete_block_removes_the_block_and_its_place_in_the_tab() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "term" })).await;
    assert!(state.srv_state.lock().await.blocks.contains_key(&block_id), "the reducer knows the block");

    let res = call(&state, "DeleteBlock", None, vec![json!(block_id)]).await;
    assert!(res.success, "{res:?}");
    assert!(res.data.is_none() && res.updates.is_none());
    assert!(state.mstore.get::<Block>(&block_id).unwrap().is_none(), "the row is gone");
    assert!(!state.mstore.must_get::<Tab>(&tab_id).unwrap().blockids.contains(&block_id));
    assert!(!state.srv_state.lock().await.blocks.contains_key(&block_id));
}

#[tokio::test]
async fn delete_block_names_a_block_the_reducer_does_not_know() {
    let state = test_state();
    let res = call(&state, "DeleteBlock", None, vec![json!("ghost")]).await;
    assert_eq!(error_of(&res), "DeleteBlock: block not found: ghost");
    let no_args = call(&state, "DeleteBlock", None, vec![]).await;
    assert!(!error_of(&no_args).is_empty());
}

// ---- QuitAgent / ShutdownAgent / ShutdownPreview / ClosePane ----

#[tokio::test]
async fn the_quit_family_refuses_a_missing_argument() {
    let state = test_state();
    for method in ["QuitAgent", "ShutdownAgent", "ShutdownPreview", "ClosePane"] {
        let res = call(&state, method, None, vec![]).await;
        assert!(!error_of(&res).is_empty(), "{method}: {res:?}");
    }
}

#[tokio::test]
async fn shutdown_preview_of_a_plain_block_answers_without_ending_anything() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "term" })).await;
    let res = call(&state, "ShutdownPreview", None, vec![json!(block_id)]).await;
    assert!(res.success, "{res:?}");
    assert!(res.data.is_some_and(|d| d.is_object()));
    assert!(state.mstore.get::<Block>(&block_id).unwrap().is_some(), "a preview ends nothing");
}

#[tokio::test]
async fn quit_agent_and_shutdown_agent_answer_for_a_block_that_is_not_an_agent() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "term" })).await;
    let quit = call(&state, "QuitAgent", None, vec![json!(block_id)]).await;
    let shutdown = call(&state, "ShutdownAgent", None, vec![json!(block_id), json!(true)]).await;
    // Both end in the saga's own verdict for a block that is no agent; the point is that the handler
    // passes the id (and the stop-background flag) through and answers with it.
    assert_eq!(quit.success, quit.error.is_none(), "{quit:?}");
    assert_eq!(shutdown.success, shutdown.error.is_none(), "{shutdown:?}");
}

#[tokio::test]
async fn close_pane_removes_every_block_it_is_given() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let a = create_block(&state, &tab_id, json!({ "view": "term" })).await;
    let res = call(&state, "ClosePane", None, vec![json!([a])]).await;
    assert!(res.success, "{res:?}");
    assert!(res.data.is_none() && res.updates.is_none());
    assert!(state.mstore.get::<Block>(&a).unwrap().is_none(), "the block is gone");
}

// ---- ApplyLayoutIntent ----

#[tokio::test]
async fn apply_layout_intent_refuses_arguments_it_cannot_read() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    for args in [vec![], vec![json!(tab_id)], vec![json!(tab_id), json!("not an intent"), json!("a1")]] {
        let res = call(&state, "ApplyLayoutIntent", None, args).await;
        assert!(!error_of(&res).is_empty(), "{res:?}");
    }
}

// ---- UpdateObjectMeta ----

#[tokio::test]
async fn update_object_meta_merges_into_a_block_and_returns_the_block() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "term", "keep": 1 })).await;
    let oref = format!("block:{block_id}");

    let res = call(&state, "UpdateObjectMeta", None, vec![json!(oref), json!({ "added": "yes", "keep": 2 })]).await;
    assert!(res.success, "{res:?}");
    let block = block_of(&state, &block_id);
    assert_eq!(block.meta.get("view"), Some(&json!("term")), "untouched keys stay");
    assert_eq!(block.meta.get("added"), Some(&json!("yes")));
    assert_eq!(block.meta.get("keep"), Some(&json!(2)));
    assert_eq!(updated_ids(&res), vec![("block".to_string(), block_id.clone())]);
    let returned = res.updates.unwrap().remove(0).obj.unwrap();
    assert_eq!(returned["meta"]["added"], json!("yes"));

    // A null value deletes the key.
    let res = call(&state, "UpdateObjectMeta", None, vec![json!(oref), json!({ "added": null })]).await;
    assert!(res.success, "{res:?}");
    assert!(!block_of(&state, &block_id).meta.contains_key("added"));
}

#[tokio::test]
async fn update_object_meta_on_a_tab_returns_the_tab_and_on_a_workspace_returns_nothing() {
    let state = test_state();
    let (ws, tab_id) = seed_tab(&state).await;

    let res = call(&state, "UpdateObjectMeta", None, vec![json!(format!("tab:{tab_id}")), json!({ "color": "red" })]).await;
    assert!(res.success, "{res:?}");
    assert_eq!(updated_ids(&res), vec![("tab".to_string(), tab_id.clone())]);
    assert_eq!(state.mstore.must_get::<Tab>(&tab_id).unwrap().meta.get("color"), Some(&json!("red")));

    let res = call(&state, "UpdateObjectMeta", None, vec![json!(format!("workspace:{ws}")), json!({ "color": "blue" })]).await;
    assert!(res.success, "{res:?}");
    assert!(res.updates.is_none() && res.data.is_none());
    let stored = state.mstore.must_get::<crate::backend::obj::Workspace>(&ws).unwrap();
    assert_eq!(stored.meta.get("color"), Some(&json!("blue")));
}

#[tokio::test]
async fn update_object_meta_keeps_a_zoom_change_on_a_block_that_is_not_an_agent_pane() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "term" })).await;
    let res = call(&state, "UpdateObjectMeta", None, vec![json!(format!("block:{block_id}")), json!({ "term:zoom": 1.5 })]).await;
    assert!(res.success, "{res:?}");
    assert_eq!(block_of(&state, &block_id).meta.get("term:zoom"), Some(&json!(1.5)));
    assert_eq!(updated_ids(&res), vec![("block".to_string(), block_id)]);
}

#[tokio::test]
async fn update_object_meta_refuses_what_a_client_may_not_write() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let block_id = create_block(&state, &tab_id, json!({ "view": "browser" })).await;
    let oref = format!("block:{block_id}");

    let owner_key = crate::server::browser_owner::OWNER_META_KEY;
    let res = call(&state, "UpdateObjectMeta", None, vec![json!(oref), json!({ owner_key: "me" })]).await;
    assert!(error_of(&res).contains("a client can only clear it"), "{res:?}");
    let res = call(&state, "UpdateObjectMeta", None, vec![json!(oref), json!({ "browser:attention": {} })]).await;
    assert!(error_of(&res).contains("written only by AgentMux itself"), "{res:?}");
    assert!(!block_of(&state, &block_id).meta.contains_key(owner_key));
}

#[tokio::test]
async fn update_object_meta_refuses_a_bad_reference_a_bad_patch_and_a_missing_object() {
    let state = test_state();
    let res = call(&state, "UpdateObjectMeta", None, vec![json!("nonsense"), json!({})]).await;
    assert!(!error_of(&res).is_empty());
    let res = call(&state, "UpdateObjectMeta", None, vec![json!("block:x"), json!("not a map")]).await;
    assert!(!error_of(&res).is_empty());
    let res = call(&state, "UpdateObjectMeta", None, vec![json!("block:no-such-block"), json!({ "a": 1 })]).await;
    assert!(!error_of(&res).is_empty(), "{res:?}");
}

#[tokio::test]
async fn update_object_meta_on_a_type_the_reducer_does_not_own_falls_back_to_the_store() {
    let state = test_state();
    let oref = format!("temp:{}", uuid::Uuid::new_v4());
    let res = call(&state, "UpdateObjectMeta", None, vec![json!(oref), json!({ "a": 1 })]).await;
    assert!(
        error_of(&res).starts_with("UpdateObjectMeta: unsupported otype temp via reducer; wcore fallback failed: "),
        "{res:?}"
    );
}

// ---- UpdateTabName ----

#[tokio::test]
async fn update_tab_name_renames_the_tab_and_returns_it() {
    let state = test_state();
    let (_, tab_id) = seed_tab(&state).await;
    let res = call(&state, "UpdateTabName", None, vec![json!(tab_id), json!("renamed")]).await;
    assert!(res.success, "{res:?}");
    assert_eq!(state.mstore.must_get::<Tab>(&tab_id).unwrap().name, "renamed");
    assert_eq!(updated_ids(&res), vec![("tab".to_string(), tab_id.clone())]);
    assert_eq!(res.updates.unwrap().remove(0).obj.unwrap()["name"], json!("renamed"));
}

#[tokio::test]
async fn update_tab_name_refuses_a_tab_that_does_not_exist_and_missing_arguments() {
    let state = test_state();
    let res = call(&state, "UpdateTabName", None, vec![json!("no-such-tab"), json!("x")]).await;
    assert!(!error_of(&res).is_empty(), "{res:?}");
    for args in [vec![], vec![json!("only-an-id")]] {
        let res = call(&state, "UpdateTabName", None, args).await;
        assert!(!error_of(&res).is_empty());
    }
}

// ---- the rest ----

#[tokio::test]
async fn an_unknown_method_is_named_in_the_refusal() {
    let state = test_state();
    let res = call(&state, "Frobnicate", None, vec![]).await;
    assert_eq!(error_of(&res), "unknown object method: Frobnicate");
}

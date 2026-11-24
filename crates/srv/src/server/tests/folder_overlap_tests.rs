// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host agent that shares a folder with a container agent: the person is asked where a host agent is opened or a
//! container agent's workspace or volumes are saved, and a caller that is not the person is refused with what to
//! do instead. The window's other saves and opens go on as they did.

use serde_json::{json, Value};

use crate::backend::folder_overlap as overlap;
use crate::backend::folder_overlap_asks::asks;
use crate::server::app_api::folder_overlap_ask::{ask, AskRequest, Asker};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use crate::server::AppState;

/// One folder for every test's choices; each test names its own agents.
fn choices() {
    crate::sagas::park_agents_tests::one_choices_dir_for_test();
}

async fn call(state: &AppState, ctx: RpcContext, command: &str, data: Value) -> (Value, String) {
    let (engine, mut rx) = WshRpcEngine::new();
    crate::server::websocket::register_handlers(&engine, state.clone(), "folder-overlap".to_string());
    engine.set_rpc_context(ctx);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    (resp.data.unwrap_or_default(), resp.error)
}

fn window() -> RpcContext {
    RpcContext::operator_for_tests()
}

/// An agent's connection that carries the proof, and a connection that has not proven it is the window.
fn agent() -> RpcContext {
    RpcContext { agent_id: "korp".into(), operator_proven: true, ..Default::default() }
}

fn unproven() -> RpcContext {
    RpcContext::default()
}

struct Fleet {
    state: AppState,
    root: tempfile::TempDir,
    host: String,
    container: String,
}

impl Fleet {
    fn path(&self, rest: &str) -> String {
        self.root.path().join(rest).to_string_lossy().to_string()
    }
}

fn save(state: &AppState, tag: &str, kind: &str, folder: &str, volumes: &str) -> String {
    let id = format!("{tag}-{}", uuid::Uuid::new_v4().simple());
    let mut def = test_agent_def(&id, tag, "claude", kind, 1, "");
    def.working_directory = folder.to_string();
    def.container_volumes = volumes.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    id
}

/// Hosty works in a folder inside the one Aria, a container agent, has as its workspace.
fn fleet() -> Fleet {
    choices();
    let state = test_state();
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("shared").to_string_lossy().to_string();
    let inside = root.path().join("shared/work").to_string_lossy().to_string();
    let container = save(&state, "Aria", "container", &workspace, "[]");
    let host = save(&state, "Hosty", "host", &inside, "[]");
    Fleet { state, root, host, container }
}

#[tokio::test]
async fn the_window_is_told_which_container_agents_a_host_agent_shares_a_folder_with() {
    let f = fleet();
    let (data, error) = call(&f.state, unproven(), "folderoverlapcheck", json!({ "agent_id": f.host })).await;
    assert!(error.is_empty(), "anyone may ask: {error}");
    let pair = &data["overlaps"][0];
    assert_eq!((pair["container_id"].as_str(), pair["host_name"].as_str(), pair["container_name"].as_str()), (Some(f.container.as_str()), Some("Hosty"), Some("Aria")));

    let (data, _) = call(&f.state, window(), "folderoverlapcheck", json!({ "agent_id": f.host, "folder": f.path("elsewhere") })).await;
    assert_eq!(data["overlaps"], json!([]), "opening in a folder of its own");
}

#[tokio::test]
async fn a_host_agent_that_names_no_folder_is_asked_about_the_folder_it_opens_in() {
    let f = fleet();
    let blank = save(&f.state, "Blanky", "host", "", "[]");
    let own = overlap::open_folder(&f.state.mstore.agent_def_get(&blank).unwrap().unwrap());
    let holder = save(&f.state, "Holder", "container", &own.parent().unwrap().to_string_lossy(), "[]");

    let (data, error) = call(&f.state, window(), "folderoverlapcheck", json!({ "agent_id": blank })).await;

    assert!(error.is_empty(), "{error}");
    assert_eq!(data["overlaps"][0]["container_id"].as_str(), Some(holder.as_str()), "{data}");
}

#[tokio::test]
async fn only_the_window_goes_ahead_and_the_choice_is_for_that_pair() {
    let f = fleet();
    let other = save(&f.state, "Ula", "container", &f.path("shared"), "[]");
    let both = json!({ "host_id": f.host, "container_id": f.container });
    for ctx in [agent(), unproven()] {
        let (_, error) = call(&f.state, ctx, "folderoverlapaccept", both.clone()).await;
        assert!(error.contains("only the person goes ahead") || error.contains("isn't the AgentMux window"), "{error}");
        assert!(!error.is_empty());
    }
    assert!(!overlap::accepted(&f.host, &f.container), "nothing was kept for a caller that is not the person");

    let (data, error) = call(&f.state, window(), "folderoverlapaccept", both).await;
    assert!(error.is_empty(), "{error}");
    assert_eq!(data["chosen"], json!(true));
    let (data, _) = call(&f.state, window(), "folderoverlapcheck", json!({ "agent_id": f.host })).await;
    let left: Vec<&str> = data["overlaps"].as_array().unwrap().iter().map(|o| o["container_id"].as_str().unwrap()).collect();
    assert_eq!(left, vec![other.as_str()], "the other container agent still asks");

    let (_, error) = call(&f.state, window(), "folderoverlapaccept", json!({ "host_id": f.container, "container_id": f.host })).await;
    assert!(error.contains("needs a host agent and a container agent"), "{error}");
}

#[tokio::test]
async fn creating_a_host_agent_in_a_container_agents_folder_is_refused_with_what_to_do_instead() {
    let f = fleet();
    let created = f.state.mstore.agent_def_list().unwrap().len();
    let make = json!({ "name": "Newhost", "provider": "claude", "working_directory": f.path("shared/deeper") });
    for ctx in [agent(), unproven()] {
        let (_, error) = call(&f.state, ctx, "createagent", make.clone()).await;
        assert!(error.contains("Newhost (host) and Aria (container) share a folder"), "{error}");
        assert!(error.contains("separate workspaces") && error.contains(":ro"), "{error}");
    }
    let (_, error) = call(&f.state, agent(), "agent.define", json!({ "name": "Defhost", "working_directory": f.path("shared") })).await;
    assert!(error.contains("Defhost (host) and Aria (container) share a folder"), "{error}");
    assert_eq!(f.state.mstore.agent_def_list().unwrap().len(), created, "neither was made");

    let (data, error) = call(&f.state, window(), "createagent", make.clone()).await;
    assert!(error.is_empty() && data["name"] == json!("Newhost"), "the window's own save goes on, the picker asks before it opens: {error}");

    let (data, error) = call(&f.state, agent(), "createagent", json!({ "name": "Apart", "provider": "claude", "working_directory": f.path("apart") })).await;
    assert!(error.is_empty() && data["name"] == json!("Apart"), "a folder of its own: {error}");
}

#[tokio::test]
async fn saving_a_workspace_or_a_writable_volume_over_a_host_agents_folder_is_refused_and_a_read_only_one_is_not() {
    let f = fleet();
    let calm = save(&f.state, "Calm", "container", &f.path("calm"), "[]");
    let update = |volumes: String, dir: String| json!({ "id": calm, "name": "Calm", "icon": "X", "provider": "claude", "agent_type": "container", "working_directory": dir, "container_volumes": volumes });

    let writable = json!([format!("{}:/data", f.path("shared/work"))]).to_string();
    let (_, error) = call(&f.state, agent(), "updateagent", update(writable, f.path("calm"))).await;
    assert!(error.contains("Hosty (host) and Calm (container) share a folder"), "{error}");

    let (_, error) = call(&f.state, agent(), "updateagent", update("[]".into(), f.path("shared/work/in"))).await;
    assert!(error.contains("Hosty (host) and Calm (container)"), "a workspace over the host agent's folder: {error}");

    let writable = json!([format!("{}:/data", f.path("shared/work"))]).to_string();
    let (_, error) = call(&f.state, unproven(), "updateagent", update(writable.clone(), f.path("calm"))).await;
    assert!(error.contains("Hosty (host) and Calm (container) share a folder"), "{error}");
    let (_, error) = call(&f.state, window(), "updateagent", update(writable, f.path("calm"))).await;
    assert!(error.is_empty(), "the window's own save goes on: {error}");

    let read_only = json!([format!("{}:/data:ro", f.path("shared/work"))]).to_string();
    let (_, error) = call(&f.state, agent(), "updateagent", update(read_only, f.path("calm"))).await;
    assert!(error.is_empty(), "a read-only volume can't be changed by the container agent: {error}");
}

#[tokio::test]
async fn a_save_that_leaves_the_folders_alone_goes_on_even_where_the_pair_overlaps() {
    let f = fleet();
    let host = f.state.mstore.agent_def_get(&f.host).unwrap().unwrap();
    let rename = json!({ "id": f.host, "name": "Hosty renamed", "icon": "X", "provider": "claude", "agent_type": host.agent_type, "working_directory": host.working_directory });
    let (data, error) = call(&f.state, window(), "updateagent", rename).await;
    assert!(error.is_empty() && data["name"] == json!("Hosty renamed"), "{error}");
}

// An agent that was refused asks the person to go ahead; the person answers from the window.

fn lead() -> Asker {
    Asker { slug: "lead".into(), name: "Lead".into(), block_id: "pane-of-lead".into() }
}

fn pair_of(f: &Fleet) -> AskRequest {
    AskRequest { host: f.host.clone(), container: f.container.clone() }
}

fn waiting(f: &Fleet) -> Vec<crate::backend::folder_overlap_asks::FolderAsk> {
    asks().list(agentmux_common::time::now_ms_u64()).into_iter().filter(|a| a.host_id == f.host && a.container_id == f.container).collect()
}

fn audited(f: &Fleet, action: &str) -> Vec<crate::backend::reactive::AuditLogEntry> {
    f.state
        .reactive_handler
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.reason.as_deref().is_some_and(|r| r.starts_with(action) && r.contains(&f.host)))
        .collect()
}

#[tokio::test]
async fn an_agent_asks_and_the_person_is_told_who_asked_and_which_two_agents() {
    let f = fleet();
    let asked = ask(&f.state, &lead(), &pair_of(&f), agentmux_common::time::now_ms_u64()).unwrap();
    assert_eq!(asked["status"], json!("asked"));
    let card = waiting(&f);
    assert_eq!(card.len(), 1);
    let ask = &card[0];
    assert_eq!((ask.agent.as_str(), ask.host_name.as_str(), ask.container_name.as_str(), ask.block_id.as_str()), ("Lead", "Hosty", "Aria", "pane-of-lead"));
    assert!(!overlap::accepted(&f.host, &f.container), "asking keeps nothing");

    let again = ask_again(&f);
    assert_eq!(again["status"], json!("asked"), "asking again says where it stands");
    assert_eq!(waiting(&f).len(), 1, "the pair is asked once");
    let rows = audited(&f, "folder-overlap.ask");
    assert_eq!(rows.len(), 1, "the ask is recorded once");
    assert_eq!(rows[0].source_agent.as_deref(), Some("lead"), "as the agent that asked");
}

fn ask_again(f: &Fleet) -> serde_json::Value {
    ask(&f.state, &lead(), &pair_of(f), agentmux_common::time::now_ms_u64()).unwrap()
}

#[tokio::test]
async fn an_agent_can_ask_but_cannot_answer() {
    let f = fleet();
    ask_again(&f);
    let both = json!({ "host_id": f.host, "container_id": f.container });
    for command in ["folderoverlapaccept", "folderoverlapdecline"] {
        for ctx in [agent(), unproven()] {
            let (_, error) = call(&f.state, ctx, command, both.clone()).await;
            assert!(error.contains("only the person goes ahead") || error.contains("isn't the AgentMux window"), "{command}: {error}");
        }
    }
    assert!(!overlap::accepted(&f.host, &f.container), "nothing was kept for a caller that is not the person");
    assert_eq!(waiting(&f).len(), 1, "the ask is still waiting for the person");
}

#[tokio::test]
async fn the_persons_go_ahead_answers_the_ask_and_is_recorded_as_theirs() {
    let f = fleet();
    ask_again(&f);
    let both = json!({ "host_id": f.host, "container_id": f.container });
    let (data, error) = call(&f.state, window(), "folderoverlapaccept", both).await;
    assert!(error.is_empty(), "{error}");
    assert_eq!(data["chosen"], json!(true));
    assert!(waiting(&f).is_empty(), "the ask is answered");
    assert_eq!(ask_again(&f)["status"], json!("chosen"), "the agent sees it and can retry what was refused");
    let rows = audited(&f, "folder-overlap.go-ahead");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_agent, None, "the person's window, not an agent");
}

#[tokio::test]
async fn a_declined_ask_leaves_the_pair_asking() {
    let f = fleet();
    ask_again(&f);
    let both = json!({ "host_id": f.host, "container_id": f.container });
    let (data, error) = call(&f.state, window(), "folderoverlapdecline", both).await;
    assert!(error.is_empty(), "{error}");
    assert_eq!(data["declined"], json!(true));
    assert!(waiting(&f).is_empty(), "the ask is answered");
    assert!(!overlap::accepted(&f.host, &f.container), "no choice was kept");
    let (data, _) = call(&f.state, window(), "folderoverlapcheck", json!({ "agent_id": f.host })).await;
    assert_eq!(data["overlaps"][0]["container_id"], json!(f.container), "the pair still asks");
    assert!(overlap::check(&f.state.mstore, &f.state.mstore.agent_def_get(&f.host).unwrap().unwrap(), None).is_err(), "and a caller that is not the person is still refused");

    let asked = ask_again(&f);
    assert_eq!(asked["status"], json!("declined"), "the pair is not asked about again at once");
    assert!(asked["retry_after_seconds"].as_u64().unwrap() > 0);
    assert!(waiting(&f).is_empty(), "so no new card is made");
    assert_eq!(audited(&f, "folder-overlap.decline").len(), 1);
}

#[tokio::test]
async fn an_ask_names_a_host_agent_and_a_container_agent_that_share_a_folder_and_needs_a_pane_to_show_in() {
    let f = fleet();
    let swapped = AskRequest { host: f.container.clone(), container: f.host.clone() };
    let err = ask(&f.state, &lead(), &swapped, 1).unwrap_err();
    assert!(err.message.contains("needs host: a host agent and container: a container agent"), "{}", err.message);
    let nobody = AskRequest { host: "no-such-agent".into(), container: f.container.clone() };
    assert_eq!(ask(&f.state, &lead(), &nobody, 1).unwrap_err().status, axum::http::StatusCode::NOT_FOUND);

    let elsewhere = save(&f.state, "Korp", "host", &f.path("elsewhere"), "[]");
    let apart = ask(&f.state, &lead(), &AskRequest { host: elsewhere, container: f.container.clone() }, 1).unwrap();
    assert_eq!(apart["status"], json!("not_needed"), "no shared folder, nothing to go ahead with");

    let no_pane = Asker { block_id: String::new(), ..lead() };
    let err = ask(&f.state, &no_pane, &pair_of(&f), 1).unwrap_err();
    assert_eq!(err.status, axum::http::StatusCode::CONFLICT);
    assert!(waiting(&f).is_empty());
}

#[tokio::test]
async fn deleting_an_agent_ends_the_ask_that_names_it() {
    let f = fleet();
    ask_again(&f);
    crate::server::app_api::folder_overlap::forget_asks_of(&f.state, &f.host);
    assert!(waiting(&f).is_empty());
}

#[tokio::test]
async fn a_choice_file_that_nothing_records_is_not_the_persons_go_ahead_and_the_window_says_why() {
    let f = fleet();
    overlap::write_choice_without_a_record_for_test(&f.host, &f.container);
    let host = f.state.mstore.agent_def_get(&f.host).unwrap().unwrap();

    let (data, _) = call(&f.state, window(), "folderoverlapcheck", json!({ "agent_id": f.host })).await;
    assert_eq!(data["overlaps"][0]["container_id"], json!(f.container), "the pair asks again");
    assert_eq!(data["overlaps"][0]["untrusted"], json!(true), "and the window is told why");
    let refused = overlap::check_open(&f.state.mstore, &host).unwrap_err();
    assert!(refused.starts_with("SHARED_FOLDER") && refused.contains("no record that the person made it"), "{refused}");
    assert!(audited(&f, "folder-overlap.go-ahead").is_empty());

    // The person goes ahead from the window: the record is written, and the pair then counts.
    let both = json!({ "host_id": f.host, "container_id": f.container });
    let (_, error) = call(&f.state, window(), "folderoverlapaccept", both).await;
    assert!(error.is_empty(), "{error}");
    assert!(overlap::check_open(&f.state.mstore, &host).is_ok());
    let (data, _) = call(&f.state, window(), "folderoverlapcheck", json!({ "agent_id": f.host })).await;
    assert_eq!(data["overlaps"], json!([]));
    let rows = audited(&f, "folder-overlap.go-ahead");
    assert_eq!(rows.len(), 1, "the go-ahead is one row");
    assert_eq!(rows[0].source_agent, None, "made by the window");
}

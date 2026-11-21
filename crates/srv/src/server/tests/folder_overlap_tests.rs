// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host agent that shares a folder with a container agent: the person is asked where a host agent is opened or a
//! container agent's workspace or volumes are saved, and a caller that is not the person is refused with what to
//! do instead. The window's other saves and opens go on as they did.

use serde_json::{json, Value};

use crate::backend::folder_overlap as overlap;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use crate::server::AppState;

/// One folder for every test's choices; each test names its own agents.
fn choices() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    overlap::set_choices_dir_for_test(Some(dir.path().to_path_buf()));
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

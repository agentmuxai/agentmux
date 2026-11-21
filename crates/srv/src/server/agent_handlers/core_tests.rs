// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the agent-definition RPCs in `core.rs` answer, pinned through the engine so the handlers
//! can be moved out of `register` without a change.

use super::*;
use crate::backend::rpc_types::RpcMessage;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

async fn call(state: &AppState, command: &str, data: serde_json::Value) -> RpcMessage {
    let (engine, mut out) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage {
        command: command.to_string(),
        reqid: "r1".to_string(),
        data: Some(data),
        ..Default::default()
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), out.recv())
        .await
        .expect("the command answers")
        .expect("the engine is open")
}

fn answer<T: serde::de::DeserializeOwned>(resp: &RpcMessage) -> T {
    assert!(resp.error.is_empty(), "{resp:?}");
    serde_json::from_value(resp.data.clone().expect("data")).expect("typed answer")
}

fn listed(state: &AppState) -> Vec<AgentDefinition> {
    state.mstore.agent_def_list().unwrap()
}

fn add(state: &AppState, id: &str, seeded: i64, hidden: i64) {
    let mut def = test_agent_def(id, id, "claude", "host", 1, "");
    def.is_seeded = seeded;
    def.user_hidden = hidden;
    state.mstore.agent_def_insert(&mut def).unwrap();
}

#[tokio::test]
async fn createagent_inserts_a_user_agent_with_a_slug_and_a_colour() {
    let state = test_state();
    let resp = call(&state, COMMAND_CREATE_AGENT, serde_json::json!({ "name": "Fresh Agent", "provider": "claude" })).await;
    let made: AgentDefinition = answer(&resp);
    assert_eq!((made.is_seeded, made.user_hidden), (0, 0));
    assert_eq!((made.icon.as_str(), made.agent_type.as_str(), made.container_volumes.as_str()), ("\u{2726}", "standalone", "[]"));
    assert!(!made.slug.is_empty(), "the slug is derived and returned, not left empty");
    assert!(listed(&state).iter().any(|a| a.id == made.id && a.name == "Fresh Agent"));
    let colour = state.mstore.agent_content_get(&made.id, "ui:color").unwrap().expect("a colour is assigned at creation");
    assert!(!colour.content.is_empty());
}

#[tokio::test]
async fn updateagent_keeps_what_the_caller_leaves_out() {
    let state = test_state();
    let mut def = test_agent_def("agent-up", "Before", "claude", "host", 1, "");
    def.accounts = "acct-1".to_string();
    def.container_image = "img:1".to_string();
    def.container_volumes = "[\"/a:/b\"]".to_string();
    def.container_name = "ctr".to_string();
    def.is_seeded = 0;
    def.user_hidden = 1;
    state.mstore.agent_def_insert(&mut def).unwrap();
    let slug = listed(&state).into_iter().find(|a| a.id == "agent-up").unwrap().slug;

    let update = serde_json::json!({ "id": "agent-up", "name": "After", "icon": "X", "provider": "claude" });
    let updated: AgentDefinition = answer(&call(&state, COMMAND_UPDATE_AGENT, update).await);
    assert_eq!((updated.name.as_str(), updated.icon.as_str()), ("After", "X"));
    assert_eq!(updated.slug, slug, "the slug never changes");
    assert_eq!(
        (updated.accounts.as_str(), updated.container_image.as_str(), updated.container_volumes.as_str()),
        ("acct-1", "img:1", "[\"/a:/b\"]"),
        "an omitted accounts, image or volumes keeps the stored value"
    );
    assert_eq!((updated.container_name.as_str(), updated.user_hidden), ("ctr", 1));

    let update = serde_json::json!({ "id": "agent-up", "name": "After", "icon": "X", "provider": "claude", "accounts": "acct-2", "container_image": "img:2" });
    let updated: AgentDefinition = answer(&call(&state, COMMAND_UPDATE_AGENT, update).await);
    assert_eq!((updated.accounts.as_str(), updated.container_image.as_str()), ("acct-2", "img:2"));

    let resp = call(&state, COMMAND_UPDATE_AGENT, serde_json::json!({ "id": "no-such", "name": "n", "icon": "i", "provider": "claude" })).await;
    assert!(resp.data.is_none() && resp.error.contains("updateagent: agent no-such not found"), "{resp:?}");
}

#[tokio::test]
async fn listagents_hides_hidden_templates_unless_asked_and_filters_by_seeded() {
    let state = test_state();
    add(&state, "own-agent", 0, 0);
    add(&state, "shown-template", 1, 0);
    add(&state, "hidden-template", 1, 1);
    let ids = |resp: RpcMessage| {
        let mut ids: Vec<String> = answer::<Vec<AgentDefinition>>(&resp).into_iter().map(|a| a.id).collect();
        ids.sort();
        ids
    };
    for body in [serde_json::Value::Null, serde_json::json!({}), serde_json::json!("junk")] {
        let got = ids(call(&state, COMMAND_LIST_AGENTS, body.clone()).await);
        assert!(got.contains(&"own-agent".to_string()) && got.contains(&"shown-template".to_string()), "{body}: {got:?}");
        assert!(!got.contains(&"hidden-template".to_string()), "{body}: {got:?}");
    }
    let got = ids(call(&state, COMMAND_LIST_AGENTS, serde_json::json!({ "include_hidden": true })).await);
    assert!(got.contains(&"hidden-template".to_string()), "{got:?}");
    let got = ids(call(&state, COMMAND_LIST_AGENTS, serde_json::json!({ "is_seeded": 0 })).await);
    assert!(got.contains(&"own-agent".to_string()) && !got.contains(&"shown-template".to_string()), "{got:?}");
    let got = ids(call(&state, COMMAND_LIST_AGENTS, serde_json::json!({ "is_seeded": 1, "include_hidden": true })).await);
    assert!(got.contains(&"shown-template".to_string()) && got.contains(&"hidden-template".to_string()), "{got:?}");
    assert!(!got.contains(&"own-agent".to_string()), "{got:?}");
}

#[tokio::test]
async fn agent_content_is_set_read_back_and_listed() {
    let state = test_state();
    add(&state, "agent-c", 0, 0);
    let set = |t: &str, c: &str| serde_json::json!({ "agent_id": "agent-c", "content_type": t, "content": c });
    let written: AgentContent = answer(&call(&state, COMMAND_SET_AGENT_CONTENT, set("agentmd", "hello")).await);
    assert_eq!((written.agent_id.as_str(), written.content_type.as_str(), written.content.as_str()), ("agent-c", "agentmd", "hello"));
    call(&state, COMMAND_SET_AGENT_CONTENT, set("mcp", "{}")).await;
    let got: Option<AgentContent> =
        answer(&call(&state, COMMAND_GET_AGENT_CONTENT, serde_json::json!({ "agent_id": "agent-c", "content_type": "agentmd" })).await);
    assert_eq!(got.expect("stored").content, "hello");
    let none: Option<AgentContent> =
        answer(&call(&state, COMMAND_GET_AGENT_CONTENT, serde_json::json!({ "agent_id": "agent-c", "content_type": "absent" })).await);
    assert!(none.is_none(), "a missing blob is null");
    let all: Vec<AgentContent> = answer(&call(&state, COMMAND_GET_ALL_AGENT_CONTENT, serde_json::json!({ "agent_id": "agent-c" })).await);
    let mut types: Vec<_> = all.iter().map(|c| c.content_type.as_str()).collect();
    types.sort();
    assert_eq!(types, vec!["agentmd", "mcp"]);
    let resp = call(&state, COMMAND_SET_AGENT_CONTENT, set(crate::backend::agent_seat::WORKER_OF_CONTENT, "{}")).await;
    assert!(resp.data.is_none() && resp.error.starts_with("setagentcontent: "), "a type only AgentMux writes is refused: {resp:?}");
}

fn import_row(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": name, "icon": "I", "description": "d", "provider": "codex", "shell": "sh",
        "working_directory": "/w", "agent_bus_id": "bus", "agent_type": "host", "environment": "E=1",
        "restart_on_crash": true,
        "content": { "agentmd": "notes" },
        "skills": [{ "name": "s1", "trigger": "t", "skill_type": "prompt", "description": "sd", "content": "sc" }],
    })
}

#[tokio::test]
async fn importagents_skips_a_slug_it_has_and_exportagents_hands_the_rest_back() {
    let state = test_state();
    add(&state, "have-it", 0, 0);
    let body = serde_json::json!({ "agents": [import_row("have-it", "Old"), import_row("new-one", "New One")] });
    let result: ImportAgentDefinitionsResult = answer(&call(&state, COMMAND_IMPORT_AGENTS, body).await);
    assert_eq!((result.imported, result.skipped, result.failed), (vec!["New One".to_string()], vec!["Old".to_string()], Vec::<String>::new()));

    let made = listed(&state).into_iter().find(|a| a.slug == "new-one").expect("imported by its slug");
    assert_eq!((made.is_seeded, made.provider.as_str(), made.restart_on_crash, made.auto_start), (0, "codex", 1, 0));
    assert_eq!(state.mstore.agent_content_get(&made.id, "agentmd").unwrap().unwrap().content, "notes");
    assert_eq!(state.mstore.agent_skill_list(&made.id).unwrap().len(), 1);

    let exported: ExportAgentDefinitionsResult = answer(&call(&state, COMMAND_EXPORT_AGENTS, serde_json::json!({})).await);
    assert_eq!((exported.version, exported.source.as_str()), (4, "agentmux-export"));
    let row = exported.agents.iter().find(|a| a.id == "new-one").expect("exported");
    assert_eq!((row.name.as_str(), row.provider.as_str(), row.restart_on_crash), ("New One", "codex", true));
    assert_eq!(row.content.get("agentmd").map(String::as_str), Some("notes"));
    assert_eq!(row.skills.len(), 1);
    assert_eq!((row.skills[0].name.as_str(), row.skills[0].trigger.as_str()), ("s1", "t"));
}

#[tokio::test]
async fn importagentfromclaw_reads_the_workspace() {
    let state = test_state();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".claude")).unwrap();
    std::fs::write(dir.path().join(".claude").join("settings.json"), r#"{"provider":"codex"}"#).unwrap();
    std::fs::write(dir.path().join("CLAUDE.md"), "claw notes").unwrap();
    std::fs::write(dir.path().join(".mcp.json"), "{\"servers\":{}}").unwrap();
    let path = dir.path().to_string_lossy().to_string();
    let made: AgentDefinition =
        answer(&call(&state, COMMAND_IMPORT_AGENT_FROM_CLAW, serde_json::json!({ "workspace_path": path, "agent_name": "Claw Agent" })).await);
    assert_eq!((made.name.as_str(), made.provider.as_str(), made.agent_type.as_str()), ("Claw Agent", "codex", "standalone"));
    assert_eq!(made.working_directory, path);
    assert_eq!(made.description, format!("Imported from {path}"));
    assert_eq!(state.mstore.agent_content_get(&made.id, "agentmd").unwrap().unwrap().content, "claw notes");
    assert_eq!(state.mstore.agent_content_get(&made.id, "mcp").unwrap().unwrap().content, "{\"servers\":{}}");

    let bare = tempfile::tempdir().unwrap();
    let bare_path = bare.path().to_string_lossy().to_string();
    let plain: AgentDefinition =
        answer(&call(&state, COMMAND_IMPORT_AGENT_FROM_CLAW, serde_json::json!({ "workspace_path": bare_path, "agent_name": "Plain" })).await);
    assert_eq!(plain.provider, "claude", "claude unless the workspace says otherwise");
    assert!(state.mstore.agent_content_get(&plain.id, "agentmd").unwrap().is_none());

    let resp = call(&state, COMMAND_IMPORT_AGENT_FROM_CLAW, serde_json::json!({ "workspace_path": "/no/such/dir/xyz", "agent_name": "X" })).await;
    assert!(resp.data.is_none() && resp.error.contains("importagentfromclaw: path does not exist"), "{resp:?}");
}

#[tokio::test]
async fn reseedagents_replaces_the_seeded_agents_and_leaves_the_users() {
    let state = test_state();
    add(&state, "my-own", 0, 0);
    add(&state, "stale-seed", 1, 0);
    let result: ReseedAgentsResult = answer(&call(&state, COMMAND_RESEED_AGENTS, serde_json::Value::Null).await);
    assert!(result.deleted >= 1, "the stale seeded row is deleted");
    assert!(result.created > 0, "the manifest is seeded again");
    let ids: Vec<String> = listed(&state).into_iter().map(|a| a.id).collect();
    assert!(ids.contains(&"my-own".to_string()));
    assert!(!ids.contains(&"stale-seed".to_string()));
}

#[tokio::test]
async fn deleteagent_removes_the_definition() {
    let state = test_state();
    add(&state, "doomed", 0, 0);
    let resp = call(&state, COMMAND_DELETE_AGENT, serde_json::json!({ "id": "doomed" })).await;
    assert!(resp.error.is_empty(), "{resp:?}");
    assert!(!listed(&state).iter().any(|a| a.id == "doomed"));
}

/// The names no agent may have (`backend::reserved_names`): refused at every way an agent is made or renamed.
mod reserved_names {
    use super::*;

    const NAMES: &[&str] = &["operator", "User", "ADMIN", "System", "agentmux", "Unproven", "unknown", "window", "host", "github-consumer", "muxreview", "swarm-forward", "Op3rator"];

    fn created(name: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "provider": "claude" })
    }

    fn names(state: &AppState) -> Vec<String> {
        state.mstore.agent_def_list().unwrap().into_iter().map(|d| d.name).collect()
    }

    #[tokio::test]
    async fn create_refuses_every_reserved_name_and_accepts_a_nickname() {
        let state = test_state();
        for name in NAMES {
            let resp = call(&state, "createagent", created(name)).await;
            assert!(resp.error.contains("reserved") && resp.error.contains("nickname"), "{name}: {resp:?}");
            assert!(!names(&state).iter().any(|n| n == name), "{name} was made");
        }
        let ok = call(&state, "createagent", created("Winny")).await;
        assert!(ok.error.is_empty(), "{ok:?}");
        assert!(names(&state).contains(&"Winny".to_string()));
    }

    #[tokio::test]
    async fn rename_refuses_every_reserved_name() {
        let state = test_state();
        let made: AgentDefinition = answer(&call(&state, "createagent", created("Winny")).await);
        for name in NAMES {
            let err = crate::server::agent_handlers::rename::rename_title(&state, &made.id, name, true).await.unwrap_err();
            assert!(matches!(err, crate::server::agent_handlers::rename::RenameError::Invalid(_)) && err.to_string().contains("reserved"), "{name}: {err}");
        }
        assert_eq!(state.mstore.agent_def_list().unwrap().iter().find(|d| d.id == made.id).unwrap().name, "Winny");
        assert!(crate::server::agent_handlers::rename::rename_title(&state, &made.id, "Pulp", true).await.is_ok());
    }

    #[tokio::test]
    async fn update_refuses_a_reserved_name_but_leaves_an_existing_one_alone() {
        let state = test_state();
        let made: AgentDefinition = answer(&call(&state, "createagent", created("Winny")).await);
        let update = |id: &str, name: &str| {
            serde_json::json!({ "id": id, "name": name, "icon": "x", "provider": "claude", "agent_type": "standalone" })
        };
        let resp = call(&state, "updateagent", update(&made.id, "Operator")).await;
        assert!(resp.error.contains("reserved"), "{resp:?}");
        // An agent that already has such a name (seeded directly: it was made before the rule) is not broken:
        // saving it without changing the name goes through, and it is still listed.
        let mut old = test_agent_def("old-admin", "Admin", "claude", "agent", 1, "");
        old.slug = "admin-legacy".to_string();
        state.mstore.agent_def_insert(&mut old).unwrap();
        let resp = call(&state, "updateagent", update("old-admin", "Admin")).await;
        assert!(resp.error.is_empty(), "{resp:?}");
        assert!(names(&state).contains(&"Admin".to_string()));
    }

    #[tokio::test]
    async fn an_agent_that_already_has_a_reserved_name_may_be_given_it_again_but_not_another() {
        let state = test_state();
        let mut old = test_agent_def("old-admin", "Admin", "claude", "agent", 1, "");
        old.slug = "admin-legacy".to_string();
        state.mstore.agent_def_insert(&mut old).unwrap();
        assert!(crate::server::agent_handlers::rename::rename_title(&state, "old-admin", "Admin", true).await.is_ok());
        assert!(crate::server::agent_handlers::rename::rename_title(&state, "old-admin", "Operator", true).await.is_err());
    }

    #[tokio::test]
    async fn import_from_a_workspace_refuses_a_reserved_name() {
        let state = test_state();
        let dir = std::env::temp_dir().join(format!("reserved-names-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let resp = call(&state, "importagentfromclaw", serde_json::json!({ "workspace_path": dir.to_str().unwrap(), "agent_name": "Host" })).await;
        assert!(resp.error.contains("reserved"), "{resp:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

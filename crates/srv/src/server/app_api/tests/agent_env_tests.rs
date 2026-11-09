// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents env_get` and `env_set` (`POST /api/v1/agent/env`), through the handler.

use super::*;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

fn agent(state: &AppState, agent_type: &str, seeded: bool) -> AgentDefinition {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Env {}", &id[..8]), "claude", agent_type, 1, "");
    def.is_seeded = i64::from(seeded);
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.agent_def_get(&id).unwrap().unwrap()
}

async fn call(state: &AppState, caller: Option<&AgentDefinition>, body: Value) -> (StatusCode, Value) {
    let caller = caller.map(|d| Extension(Caller::Agent { uid: d.id.clone() }));
    let req: AgentEnvRequest = serde_json::from_value(body).unwrap();
    let resp = handle_agent_env(State(state.clone()), caller, Json(req)).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn stored(state: &AppState, def: &AgentDefinition) -> Option<String> {
    state.mstore.agent_content_get(&def.id, "env").unwrap().map(|c| c.content)
}

fn audited(def: &AgentDefinition, target: &AgentDefinition) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.source_agent.as_deref() == Some(def.slug.as_str()) && e.target_agent == target.slug)
        .collect()
}

#[tokio::test]
async fn an_agent_sets_and_reads_its_own_environment() {
    let state = test_state();
    let me = agent(&state, "host", false);
    let (status, body) = call(&state, Some(&me), json!({ "action": "get" })).await;
    assert_eq!((status, body["lines"].as_str()), (StatusCode::OK, Some("")), "{body}");

    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "set": { "RUST_BACKTRACE": "vim", "RUST_LOG": "debug" } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changed"]["added"], json!({ "RUST_BACKTRACE": "vim", "RUST_LOG": "debug" }));
    assert!(body["note"].as_str().unwrap().contains("next launch"), "{body}");
    assert_eq!(stored(&state, &me).as_deref(), Some("RUST_BACKTRACE=vim\nRUST_LOG=debug\n"));

    let (_, body) = call(&state, Some(&me), json!({ "action": "set", "set": { "RUST_BACKTRACE": "nano" }, "unset": ["RUST_LOG"] })).await;
    assert_eq!(body["changed"]["changed"]["RUST_BACKTRACE"], json!({ "from": "vim", "to": "nano" }), "{body}");
    assert_eq!(body["changed"]["removed"], json!(["RUST_LOG"]));
    let (_, body) = call(&state, Some(&me), json!({ "action": "get", "agent": me.slug })).await;
    assert_eq!(body["lines"], "RUST_BACKTRACE=nano\n");
    assert_eq!(body["agent"]["id"], json!(me.id));
}

#[tokio::test]
async fn an_agent_sets_another_agents_environment_by_slug_or_id() {
    let state = test_state();
    let (me, other) = (agent(&state, "host", false), agent(&state, "host", false));
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": other.slug, "set": { "TZ": "1" } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state, &other).as_deref(), Some("TZ=1\n"));
    assert_eq!(stored(&state, &me), None);
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "agent": other.id, "set": { "LANG": "2" } })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&state, &other).as_deref(), Some("TZ=1\nLANG=2\n"));
    let (status, body) = call(&state, Some(&me), json!({ "action": "get", "agent": "no-such-agent" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn a_template_has_no_environment_to_set() {
    let state = test_state();
    let (me, template) = (agent(&state, "host", false), agent(&state, "host", true));
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": template.id, "set": { "TZ": "1" } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(stored(&state, &template), None);
}

#[tokio::test]
async fn reserved_credential_and_secret_looking_input_is_refused_and_nothing_is_written() {
    let state = test_state();
    let me = agent(&state, "host", false);
    for set in [
        json!({ "AGENTMUX_AGENT_ID": "x" }),
        json!({ "PATH": "/x" }),
        json!({ "HOME": "/x" }),
        json!({ "GH_CONFIG_DIR": "/x" }),
        json!({ "CLAUDE_CONFIG_DIR": "/x" }),
        json!({ "CODEX_HOME": "/x" }),
        json!({ "OPENAI_API_KEY": "x" }),
        json!({ "OPENAI_BASE_URL": "http://x" }),
        json!({ "ANTHROPIC_API_KEY": "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789" }),
        json!({ "ANTHROPIC_BASE_URL": "http://user:pw@x" }),
        json!({ "SERVICE_TOKEN": "x" }),
        json!({ "FORCE_COLOR": "ghp_abcdefghijklmnopqrstuvwxyz0123456789" }),
        json!({ "bad name": "x" }),
    ] {
        let (status, body) = call(&state, Some(&me), json!({ "action": "set", "set": set })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{set}: {body}");
    }
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "unset": ["PATH"] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "lines": "# ok\nPATH=/x\n" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(stored(&state, &me), None);
}

#[tokio::test]
async fn the_size_is_bounded() {
    let state = test_state();
    let me = agent(&state, "host", false);
    let lines = "TZ=1\n".repeat(65);
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "lines": lines })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("at most 64"), "{body}");
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "lines": format!("TZ={}\n", "x".repeat(5000)) })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // 64 lines, under 4 KiB: accepted.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "lines": "TZ=1\n".repeat(64) })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // An edit that would grow it past the limit is refused too.
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "set": { "LANG": "2" } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn lines_are_stored_exactly_as_written_comments_and_blanks_included() {
    let state = test_state();
    let me = agent(&state, "host", false);
    let text = "# tools\n\nRUST_BACKTRACE=vim   \n\n  # indented note\nMCP_TIMEOUT=hello world = 1\n";
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "lines": text })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state, &me).as_deref(), Some(text));
    // An edit leaves the comments and the blank lines where they were.
    let (_, _) = call(&state, Some(&me), json!({ "action": "set", "set": { "RUST_BACKTRACE": "nano" } })).await;
    assert_eq!(
        stored(&state, &me).as_deref(),
        Some("# tools\n\nRUST_BACKTRACE=nano\n\n  # indented note\nMCP_TIMEOUT=hello world = 1\n")
    );
    // A value comes back as it went in.
    let (_, body) = call(&state, Some(&me), json!({ "action": "get" })).await;
    assert!(body["lines"].as_str().unwrap().contains("MCP_TIMEOUT=hello world = 1"), "{body}");
}

#[tokio::test]
async fn a_line_that_would_be_skipped_at_launch_is_refused() {
    let state = test_state();
    let me = agent(&state, "host", false);
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "lines": "TZ=1\nnot a setting\n" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("line 2"), "{body}");
}

#[tokio::test]
async fn a_container_agent_edits_environments_as_a_host_agent_does() {
    let state = test_state();
    let (container, host) = (agent(&state, "container", false), agent(&state, "host", false));
    let (status, out) = call(&state, Some(&container), json!({ "action": "set", "set": { "TZ": "1" } })).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(stored(&state, &container).as_deref(), Some("TZ=1\n"));
    let (status, out) = call(&state, Some(&container), json!({ "action": "get" })).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, _) = call(&state, Some(&container), json!({ "action": "set", "agent": host.id, "set": { "LANG": "2" } })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&state, &host).as_deref(), Some("LANG=2\n"));
    // The rules a host caller has: a name an agent may not set is refused, and nothing is written.
    let (status, _) = call(&state, Some(&container), json!({ "action": "set", "set": { "PATH": "/x" } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(stored(&state, &container).as_deref(), Some("TZ=1\n"));
}

#[tokio::test]
async fn a_container_agents_environment_is_edited_as_a_target_like_a_hosts() {
    let state = test_state();
    let (container, host) = (agent(&state, "container", false), agent(&state, "host", false));
    let (status, out) = call(&state, Some(&host), json!({ "action": "set", "agent": container.id, "set": { "TZ": "1" } })).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(stored(&state, &container).as_deref(), Some("TZ=1\n"));
    let (status, out) = call(&state, Some(&host), json!({ "action": "set", "agent": container.slug, "lines": "LANG=2\n" })).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, out) = call(&state, Some(&host), json!({ "action": "get", "agent": container.id })).await;
    assert_eq!(status, StatusCode::OK, "{out}");
}

#[tokio::test]
async fn a_request_without_an_agent_token_is_refused_and_never_the_operator() {
    let state = test_state();
    let target = agent(&state, "host", false);
    let (status, body) = call(&state, None, json!({ "action": "set", "agent": target.id, "set": { "TZ": "1" } })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(stored(&state, &target), None);
}

#[tokio::test]
async fn the_audit_log_names_the_calling_agent_and_the_definition_without_values() {
    let state = test_state();
    let (me, other) = (agent(&state, "host", false), agent(&state, "host", false));
    let (_, body) = call(&state, Some(&me), json!({ "action": "set", "agent": other.id, "set": { "RUST_BACKTRACE": "vim-secret-looking-no" } })).await;
    let entries = audited(&me, &other);
    let entry = entries.iter().find(|e| e.request_id == body["audit_id"].as_str().unwrap()).expect("an audit entry");
    assert!(entry.success);
    assert_eq!(entry.source_agent.as_deref(), Some(me.slug.as_str()));
    let reason = entry.reason.as_deref().unwrap();
    assert!(reason.contains(&me.id) && reason.contains(&other.id) && reason.contains("added [RUST_BACKTRACE]"), "{reason}");
    assert!(!reason.contains("vim-secret"), "values stay out of the audit log: {reason}");

    // A refused attempt is recorded as one.
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "agent": other.id, "set": { "PATH": "/x" } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(audited(&me, &other).iter().any(|e| !e.success && e.reason.as_deref().is_some_and(|r| r.contains("refused"))));
}

#[tokio::test]
async fn the_operators_credential_values_are_hidden_from_an_agent_and_survive_its_edits() {
    let state = test_state();
    let me = agent(&state, "host", false);
    // The operator wrote these in My Agents, through setagentcontent.
    state
        .mstore
        .agent_content_set(&AgentContent {
            agent_id: me.id.clone(),
            content_type: "env".into(),
            content: "SERVICE_TOKEN=operator-value-123\nRUST_BACKTRACE=vim\n".into(),
            updated_at: 1,
        })
        .unwrap();
    let (_, body) = call(&state, Some(&me), json!({ "action": "get" })).await;
    assert_eq!(body["lines"], "SERVICE_TOKEN=[hidden]\nRUST_BACKTRACE=vim\n");
    // Writing the shown text back leaves the secret as it was.
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "lines": "SERVICE_TOKEN=[hidden]\nRUST_BACKTRACE=vim\n" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&state, &me).as_deref(), Some("SERVICE_TOKEN=operator-value-123\nRUST_BACKTRACE=vim\n"));
    // So does a whole text that leaves the line out, or tries a value of its own.
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "lines": "RUST_BACKTRACE=emacs\n" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&state, &me).as_deref(), Some("RUST_BACKTRACE=emacs\nSERVICE_TOKEN=operator-value-123\n"));
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "lines": "SERVICE_TOKEN=mine\n" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(stored(&state, &me).as_deref(), Some("RUST_BACKTRACE=emacs\nSERVICE_TOKEN=operator-value-123\n"));
    let (status, _) = call(&state, Some(&me), json!({ "action": "set", "set": { "RUST_BACKTRACE": "vim" } })).await;
    assert_eq!(status, StatusCode::OK);
    // An edit of another variable leaves it as the operator wrote it.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "set": { "RUST_BACKTRACE": "nano" } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state, &me).as_deref(), Some("RUST_BACKTRACE=nano\nSERVICE_TOKEN=operator-value-123\n"));
    assert!(!body.to_string().contains("operator-value-123"), "{body}");
}

#[tokio::test]
async fn a_change_with_no_effect_writes_nothing() {
    let state = test_state();
    let me = agent(&state, "host", false);
    call(&state, Some(&me), json!({ "action": "set", "set": { "TZ": "1" } })).await;
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "set": { "TZ": "1" } })).await;
    assert_eq!((status, body["note"].as_str()), (StatusCode::OK, Some("nothing changed")), "{body}");
}

#[tokio::test]
async fn a_set_that_says_what_to_set_is_required_and_lines_exclude_the_edits() {
    let state = test_state();
    let me = agent(&state, "host", false);
    for body in [
        json!({ "action": "set" }),
        json!({ "action": "set", "lines": "TZ=1\n", "set": { "LANG": "2" } }),
        json!({ "action": "delete" }),
    ] {
        let (status, out) = call(&state, Some(&me), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    }
}

const SERVER: &str = "http://198.51.100.7:11434";

fn audited_by_name(target: &AgentDefinition, source: &str) -> Vec<crate::backend::reactive::AuditLogEntry> {
    get_global_handler()
        .get_audit_log(usize::MAX)
        .into_iter()
        .filter(|e| e.source_agent.as_deref() == Some(source) && e.target_agent == target.slug)
        .collect()
}

fn point(agent: &AgentDefinition) -> Value {
    json!({ "action": "set", "agent": agent.id, "set": {
        "ANTHROPIC_BASE_URL": SERVER,
        "ANTHROPIC_AUTH_TOKEN": "ollama",
        "ANTHROPIC_MODEL": "flint:40b-a2b",
        "API_TIMEOUT_MS": "600000",
    } })
}

#[tokio::test]
async fn an_agent_points_another_at_a_model_server_with_a_notice_and_no_key_in_the_answer() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    let (status, body) = call(&state, Some(&me), point(&lolly)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["notice"]["undo_for_ms"], 30_000, "{body}");
    assert!(body["note"].as_str().unwrap().contains("Undo"), "{body}");
    assert_eq!(body["changed"]["added"]["ANTHROPIC_BASE_URL"], SERVER, "{body}");
    // The key is not echoed, in the change or in the text shown back.
    assert!(!body.to_string().contains("ollama"), "{body}");
    assert!(body["lines"].as_str().unwrap().contains("ANTHROPIC_AUTH_TOKEN=[hidden]"), "{body}");
    assert!(stored(&state, &lolly).unwrap().contains("ANTHROPIC_BASE_URL=http://198.51.100.7:11434"));
}

#[tokio::test]
async fn the_audit_row_names_the_acting_agent_and_the_endpoint_change_without_a_key() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    let (_, body) = call(&state, Some(&me), point(&lolly)).await;
    let entries = audited(&me, &lolly);
    let entry = entries.iter().find(|e| e.request_id == body["audit_id"].as_str().unwrap()).expect("an audit entry");
    assert!(entry.success);
    let reason = entry.reason.as_deref().unwrap();
    assert!(reason.contains(&me.id) && reason.contains(&lolly.id) && reason.contains("[model endpoint]"), "{reason}");
    assert!(reason.contains("ANTHROPIC_AUTH_TOKEN"), "the name is recorded: {reason}");
    assert!(!reason.contains("ollama"), "the key stays out of the audit log: {reason}");
}

#[tokio::test]
async fn a_secret_shaped_key_or_an_unusable_vault_reference_is_refused_and_nothing_is_written() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    for token in ["sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789", "vault:no-such-entry"] {
        let (status, body) = call(
            &state,
            Some(&me),
            json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": SERVER, "ANTHROPIC_AUTH_TOKEN": token } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{token}: {body}");
        assert!(!body.to_string().contains(token), "a refusal does not repeat the value: {body}");
    }
    assert_eq!(stored(&state, &lolly), None);
    assert!(audited(&me, &lolly).iter().any(|e| !e.success), "the refusals are recorded");
}

#[tokio::test]
async fn an_agent_on_a_model_route_or_in_a_container_keeps_its_own_server() {
    let state = test_state();
    let (me, routed, container) = (agent(&state, "host", false), agent(&state, "host", false), agent(&state, "container", false));
    state
        .mstore
        .agent_content_set(&AgentContent {
            agent_id: routed.id.clone(),
            content_type: crate::backend::model_gateway::launch::CONTENT_TYPE.into(),
            content: json!({ "route": "openrouter", "model": "any" }).to_string(),
            updated_at: 1,
        })
        .unwrap();
    for target in [&routed, &container] {
        let (status, body) = call(&state, Some(&me), point(target)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(stored(&state, target), None);
    }
    // Other variables on the same agents still go through.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": container.id, "set": { "TZ": "1" } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// An api-key entry in the Vault, open to the agents in `open_to` only.
fn vault_key(state: &AppState, open_to: &[&AgentDefinition]) -> String {
    crate::backend::vault::seal::use_test_key();
    let entry: crate::backend::vault::NewEntry =
        serde_json::from_value(json!({ "kind": "api-key", "label": "Studio box key", "parts": { "value": "sk-test-0123456789" } })).unwrap();
    let id = crate::backend::vault::create(&state.identity_store, entry).unwrap();
    let ids: Vec<&str> = open_to.iter().map(|d| d.id.as_str()).collect();
    assert!(state.identity_store.vault_set("personal", "Personal", "named", &json!(ids).to_string()).unwrap());
    id
}

fn with_key(agent: &AgentDefinition, key: &str) -> Value {
    json!({ "action": "set", "agent": agent.id, "set": { "ANTHROPIC_BASE_URL": SERVER, "ANTHROPIC_AUTH_TOKEN": key } })
}

#[tokio::test]
async fn a_vault_key_must_be_open_to_the_acting_agent_as_well_as_the_agent_it_is_for() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    // Open to Lolly only: an agent that could not read the key itself cannot send it to a server it names.
    let key = vault_key(&state, &[&lolly]);
    let (status, body) = call(&state, Some(&me), with_key(&lolly, &format!("vault:{key}"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(stored(&state, &lolly), None);
    // Open to the acting agent only: the agent it is for could not use it either.
    let key = vault_key(&state, &[&me]);
    let (status, body) = call(&state, Some(&me), with_key(&lolly, &format!("vault:{key}"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Open to both: accepted, and the value is not in the answer.
    let key = vault_key(&state, &[&me, &lolly]);
    let (status, body) = call(&state, Some(&me), with_key(&lolly, &format!("vault:{key}"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("sk-test-0123456789"), "{body}");
}

#[tokio::test]
async fn a_refused_vault_key_never_says_which_entries_exist_or_what_they_are_called() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    let closed = vault_key(&state, &[&lolly]);
    let mut answers = Vec::new();
    for key in [format!("vault:{closed}"), "vault:no-such-entry".to_string()] {
        let (status, body) = call(&state, Some(&me), with_key(&lolly, &key)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let error = body["error"].as_str().unwrap().replace(&key, "<key>");
        assert!(!error.contains("Studio box key") && !error.contains("no entry"), "{error}");
        answers.push(error);
    }
    assert_eq!(answers[0], answers[1], "the same answer whether or not the entry exists");
}

#[tokio::test]
async fn a_key_written_by_an_agent_on_another_machine_may_not_name_a_vault_entry() {
    let state = test_state();
    let lolly = agent(&state, "host", false);
    let key = vault_key(&state, &[&lolly]);
    let req: AgentEnvRequest = serde_json::from_value(with_key(&lolly, &format!("vault:{key}"))).unwrap();
    let (status, _) = run_remote(&state, "narko-2@elsewhere", req).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(stored(&state, &lolly), None);
}

fn write_env(state: &AppState, agent: &AgentDefinition, text: &str) {
    state
        .mstore
        .agent_content_set(&AgentContent { agent_id: agent.id.clone(), content_type: "env".into(), content: text.into(), updated_at: 1 })
        .unwrap();
}

#[tokio::test]
async fn moving_the_address_checks_the_keys_already_in_the_environment_too() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    // A Vault key open to Lolly only, named by the person: an agent that could not read it must not be able
    // to send it to an address of its own choosing by changing the address alone.
    let key = vault_key(&state, &[&lolly]);
    write_env(&state, &lolly, &format!("ANTHROPIC_BASE_URL={SERVER}\nANTHROPIC_AUTH_TOKEN=vault:{key}\n"));
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": "http://198.51.100.9:11434" } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(stored(&state, &lolly).unwrap().contains(SERVER), "nothing was written");
    // Changing something else leaves the key where it was, so it is not asked again.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "API_TIMEOUT_MS": "1000" } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Taking the address away sends nothing anywhere.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "unset": ["ANTHROPIC_BASE_URL"] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_key_the_person_wrote_in_the_clear_is_not_handed_to_an_address_an_agent_chose() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    write_env(&state, &lolly, "ANTHROPIC_API_KEY=operator-plain-key-123\n");
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": SERVER } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.to_string().contains("operator-plain-key-123"), "{body}");
    assert_eq!(stored(&state, &lolly).as_deref(), Some("ANTHROPIC_API_KEY=operator-plain-key-123\n"));
    // A placeholder is no key.
    write_env(&state, &lolly, "ANTHROPIC_API_KEY=ollama\n");
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": SERVER } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_key_is_written_only_with_the_address_of_a_server_of_its_own() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    // A key alone would be sent to Anthropic as its key, next to the person's login.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_API_KEY": "ollama" } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(stored(&state, &lolly), None);
    // Nor with Anthropic's own address.
    let (status, body) = call(
        &state,
        Some(&me),
        json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": "https://api.anthropic.com", "ANTHROPIC_AUTH_TOKEN": "ollama" } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // With the address, in one call: fine; and then a key on its own address.
    let (status, body) = call(&state, Some(&me), point(&lolly)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_API_KEY": "none" } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Taking the address away together with another setting is not a key written.
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "API_TIMEOUT_MS": "5" }, "unset": ["ANTHROPIC_BASE_URL"] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn only_a_change_of_address_opens_a_notice() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    for set in [json!({ "ANTHROPIC_MODEL": "sonnet" }), json!({ "API_TIMEOUT_MS": "1000" })] {
        // (An agent with no address of its own: nothing moved.)
        let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": set })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["notice"].is_null(), "no address moved: {body}");
        assert!(!body["note"].as_str().unwrap().contains("Undo"), "{body}");
    }
    let (_, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": SERVER } })).await;
    assert!(body["notice"]["id"].is_string(), "{body}");
    // On an agent that has an address, a key, a timeout or a model on the same address is no new address.
    for set in [json!({ "ANTHROPIC_AUTH_TOKEN": "ollama" }), json!({ "API_TIMEOUT_MS": "2000" }), json!({ "ANTHROPIC_DEFAULT_SONNET_MODEL": "flint:40b" })] {
        let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": set })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["notice"].is_null(), "the address did not move: {body}");
    }
    // Another address is a change of address.
    let (_, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "ANTHROPIC_BASE_URL": "http://198.51.100.9:11434" } })).await;
    assert!(body["notice"]["id"].is_string(), "{body}");
    // Taking the address away is a change of address too.
    let (_, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "unset": ["ANTHROPIC_BASE_URL"] })).await;
    assert!(body["notice"]["id"].is_string(), "{body}");
}

#[tokio::test]
async fn an_endpoint_name_is_written_in_capitals_only() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "anthropic_base_url": SERVER } })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(stored(&state, &lolly), None);
}

#[tokio::test]
async fn a_route_or_container_agent_can_still_set_a_model_name_and_take_endpoint_settings_out() {
    let state = test_state();
    let (me, routed, container) = (agent(&state, "host", false), agent(&state, "host", false), agent(&state, "container", false));
    state
        .mstore
        .agent_content_set(&AgentContent {
            agent_id: routed.id.clone(),
            content_type: crate::backend::model_gateway::launch::CONTENT_TYPE.into(),
            content: json!({ "route": "openrouter", "model": "any" }).to_string(),
            updated_at: 1,
        })
        .unwrap();
    for target in [&routed, &container] {
        let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": target.id, "set": { "ANTHROPIC_MODEL": "sonnet" } })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    // Settings the operator left in a container agent's environment can be removed by an agent.
    state
        .mstore
        .agent_content_set(&AgentContent {
            agent_id: container.id.clone(),
            content_type: "env".into(),
            content: format!("ANTHROPIC_BASE_URL={SERVER}\nANTHROPIC_MODEL=sonnet\n"),
            updated_at: 2,
        })
        .unwrap();
    let (status, body) = call(&state, Some(&me), json!({ "action": "set", "agent": container.id, "unset": ["ANTHROPIC_BASE_URL"] })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&state, &container).as_deref(), Some("ANTHROPIC_MODEL=sonnet\n"));
}

#[tokio::test]
async fn the_change_is_undone_inside_the_window_and_only_once() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "TZ": "1" } })).await;
    let (_, body) = call(&state, Some(&me), point(&lolly)).await;
    let notice = body["notice"]["id"].as_str().unwrap().to_string();
    undo_endpoint(&state, &notice, "you").unwrap();
    assert_eq!(stored(&state, &lolly).as_deref(), Some("TZ=1\n"));
    assert!(undo_endpoint(&state, &notice, "you").is_err(), "an undone change cannot be undone again");
    let entries = audited_by_name(&lolly, "you");
    assert!(entries.iter().any(|e| e.success && e.reason.as_deref().is_some_and(|r| r.contains("undid"))), "{entries:?}");
}

#[tokio::test]
async fn an_undo_after_the_environment_was_edited_again_is_refused() {
    let state = test_state();
    let (me, lolly) = (agent(&state, "host", false), agent(&state, "host", false));
    let (_, body) = call(&state, Some(&me), point(&lolly)).await;
    let notice = body["notice"]["id"].as_str().unwrap().to_string();
    call(&state, Some(&me), json!({ "action": "set", "agent": lolly.id, "set": { "TZ": "1" } })).await;
    let why = undo_endpoint(&state, &notice, "you").unwrap_err();
    assert!(why.contains("changed again"), "{why}");
    assert!(stored(&state, &lolly).unwrap().contains("ANTHROPIC_BASE_URL"), "the later edit and the endpoint both stand");
}

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
        json!({ "ANTHROPIC_API_KEY": "x" }),
        json!({ "OPENAI_API_KEY": "x" }),
        json!({ "ANTHROPIC_BASE_URL": "http://x" }),
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

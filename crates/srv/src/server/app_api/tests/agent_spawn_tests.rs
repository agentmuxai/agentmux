// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents` `spawn` (SPEC_AGENT_WORKERS_2026_10_15.md §3.1, §3.5, §5).

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use agentmux_common::api_types::UiAutomationAuth;
use agentmux_common::ipc::{Command, Event};

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

async fn new_tab(state: &AppState) -> String {
    let ws_id = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateTab { workspace_id: ws_id, name: "t".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

async fn new_block(state: &AppState, tab_id: &str, meta: serde_json::Value) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.to_string(), meta })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

/// A leader: a definition in My Agents, open in a pane of a tab, registered
/// under its routing id. `pane_meta` adds to the pane's meta.
async fn seed_leader(state: &AppState, provider: &str, agent_type: &str, pane_meta: serde_json::Value) -> (AgentDefinition, String, String) {
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Lead {}", &id[..8]), provider, agent_type, 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let tab_id = new_tab(state).await;
    let mut meta = json!({ "view": "agent", "agentId": def.id });
    for (k, v) in pane_meta.as_object().cloned().unwrap_or_default() {
        meta[k] = v;
    }
    let block_id = new_block(state, &tab_id, meta).await;
    state.reactive_handler.register_agent(&def.slug, &block_id, Some(&tab_id)).unwrap();
    // It runs on a signed-in account of its own.
    let own = leader_account(&def);
    state.identity_store.identity_upsert(&own).unwrap();
    state.identity_store.agent_identity_link(&def.id, &own.id, provider).unwrap();
    (def, tab_id, block_id)
}

fn account(id: &str, email: &str, status: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: "claude-oauth".to_string(),
        provider: "claude".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: crate::backend::storage::identities::SecretRef::OAuthConfigDir { dir: format!("/tmp/{id}") },
        context: json!({ "email": email }),
        status: status.to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// The account `seed_leader` gives a leader.
fn leader_account(def: &AgentDefinition) -> IdentityAccount {
    account(&format!("acct-{}", def.id), &format!("{}@example.com", def.slug), "valid")
}

fn account_of(a: &IdentityAccount) -> WorkerAccount {
    WorkerAccount { id: a.id.clone(), label: account_label(a), provider: "claude".to_string(), note: None }
}

/// A fresh `UiAutomationAuth` for `agent_id`, signed with its own key, as
/// `agentmux-mcp`'s `sign_ui_automation_auth` signs it.
fn sign_auth(state: &AppState, agent_id: &str) -> UiAutomationAuth {
    let key = state.mstore.agent_jekt_key_ensure(agent_id).unwrap();
    let ts_secs = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", agent_id, "__srv__", ts_secs, "");
    UiAutomationAuth { agent_id: agent_id.to_string(), ts_secs, sig }
}

fn request(auth: UiAutomationAuth, brief: &str) -> SpawnAgentRequest {
    SpawnAgentRequest {
        auth,
        name: Some("Lead 4".into()),
        model: Some("sonnet".into()),
        effort: None,
        account: None,
        brief: brief.into(),
        user_instruction: String::new(),
    }
}

async fn spawn(state: &AppState, req: SpawnAgentRequest) -> (StatusCode, serde_json::Value) {
    let resp = handle_agent_spawn(State(state.clone()), None, Json(req)).await;
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

fn definitions(state: &AppState) -> usize {
    state.mstore.agent_def_list().unwrap().len()
}

#[test]
fn a_worker_runs_on_the_model_asked_for_with_the_rest_of_the_leaders_runtime() {
    let leader = r#"{"permissionMode":"bypass","model":"opus","effort":"high"}"#;
    let rt = worker_runtime("claude", leader, Some("sonnet"), None).unwrap();
    assert_eq!(rt.model.as_deref(), Some("sonnet"));
    assert_eq!(rt.effort.as_deref(), Some("high"), "the leader's effort");
    let stored: serde_json::Value = serde_json::from_str(&rt.stored).unwrap();
    assert_eq!(stored, json!({ "permissionMode": "bypass", "model": "sonnet", "effort": "high" }));

    let same = worker_runtime("claude", leader, None, Some("medium")).unwrap();
    assert_eq!((same.model.as_deref(), same.effort.as_deref()), (Some("opus"), Some("medium")), "left out: the leader's model");
    assert_eq!(worker_runtime("claude", "", None, None).unwrap(), WorkerRuntime::default(), "nothing to remember");
}

#[test]
fn a_model_or_effort_a_launch_would_drop_is_refused() {
    let e = worker_runtime("claude", "", Some("gpt-5"), None).unwrap_err();
    assert!(e.starts_with("unknown model \"gpt-5\""), "{e}");
    let e = worker_runtime("claude", "", Some("sonnet"), Some("extreme")).unwrap_err();
    assert!(e.starts_with("unknown effort \"extreme\""), "{e}");
    // Only Claude's runtime is remembered across a launch (agent_runtime.rs).
    let e = worker_runtime("codex", "", Some("gpt-5.5"), None).unwrap_err();
    assert!(e.contains("only for a Claude worker"), "{e}");
    assert_eq!(worker_runtime("codex", "", None, None).unwrap(), WorkerRuntime::default());
}

#[tokio::test]
async fn live_workers_are_the_panes_in_a_tab_the_leader_coaches() {
    let state = test_state();
    let tab = new_tab(&state).await;
    let mark = |id: &str| json!({ "view": "agent", COACHED_BY_META_KEY: { "agent": "Lead", "id": id, "block_id": "b" } });
    new_block(&state, &tab, mark("lead-1")).await;
    new_block(&state, &tab, mark("lead-1")).await;
    new_block(&state, &tab, mark("lead-2")).await;
    new_block(&state, &tab, json!({ "view": "agent" })).await;
    assert_eq!(live_workers_of(&state.mstore, "lead-1"), 2);
    assert_eq!(live_workers_of(&state.mstore, "lead-2"), 1);
    assert_eq!(live_workers_of(&state.mstore, "lead-3"), 0);
}

#[tokio::test]
async fn a_call_without_a_verified_agent_is_refused() {
    let state = test_state();
    let (def, _, _) = seed_leader(&state, "claude", "host", json!({})).await;
    let mut auth = sign_auth(&state, &def.slug);
    auth.sig = "forged".into();
    let before = definitions(&state);
    let (status, body) = spawn(&state, request(auth, "implement the spec")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(definitions(&state), before);
}

#[tokio::test]
async fn a_worker_cannot_start_workers_of_its_own() {
    let state = test_state();
    let coached = json!({ COACHED_BY_META_KEY: { "agent": "Boss", "id": "boss-def", "block_id": "boss-pane" } });
    let (def, _, _) = seed_leader(&state, "claude", "host", coached).await;
    let before = definitions(&state);
    let (status, body) = spawn(&state, request(sign_auth(&state, &def.slug), "implement the spec")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("WORKER_CANNOT_SPAWN"), "{body}");
    assert_eq!(definitions(&state), before);
}

#[tokio::test]
async fn a_container_agent_cannot_start_workers() {
    let state = test_state();
    let (def, _, _) = seed_leader(&state, "claude", "container", json!({})).await;
    let (status, body) = spawn(&state, request(sign_auth(&state, &def.slug), "implement the spec")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("CONTAINER_AGENT"), "{body}");
}

#[tokio::test]
async fn a_fork_needs_a_brief_and_a_known_model() {
    let state = test_state();
    let (def, _, _) = seed_leader(&state, "claude", "host", json!({})).await;
    let (status, body) = spawn(&state, request(sign_auth(&state, &def.slug), "  ")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("a brief is required"), "{body}");
    let mut req = request(sign_auth(&state, &def.slug), "implement the spec");
    req.model = Some("gpt-5".into());
    let (status, body) = spawn(&state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("unknown model"), "{body}");
}

#[tokio::test]
async fn a_leader_with_three_workers_open_is_refused_a_fourth() {
    let state = test_state();
    let (def, tab, _) = seed_leader(&state, "claude", "host", json!({})).await;
    for _ in 0..MAX_LIVE_WORKERS {
        new_block(&state, &tab, json!({ "view": "agent", COACHED_BY_META_KEY: { "agent": def.name, "id": def.id, "block_id": "x" } })).await;
    }
    let before = definitions(&state);
    let (status, body) = spawn(&state, request(sign_auth(&state, &def.slug), "implement the spec")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("TOO_MANY_WORKERS"), "{body}");
    assert_eq!(definitions(&state), before);
}

/// Parallel forks don't each see room for one more: a fork waits while
/// another of the same leader's is under way (here, the lock held as that
/// one would hold it) and counts the worker it opened.
#[tokio::test]
async fn a_fork_waits_for_the_leaders_fork_under_way_and_counts_its_worker() {
    let state = test_state();
    let (def, tab, _) = seed_leader(&state, "claude", "host", json!({})).await;
    let worker = || json!({ "view": "agent", COACHED_BY_META_KEY: { "agent": def.name, "id": def.id, "block_id": "x" } });
    for _ in 1..MAX_LIVE_WORKERS {
        new_block(&state, &tab, worker()).await;
    }
    let lock = leader_lock(&def.id);
    let under_way = lock.lock().await;
    let second = tokio::spawn({
        let (state, req) = (state.clone(), request(sign_auth(&state, &def.slug), "implement the spec"));
        async move { spawn(&state, req).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!second.is_finished(), "waits for the fork under way");
    new_block(&state, &tab, worker()).await; // the one under way opens its worker
    drop(under_way);
    let (status, body) = second.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("TOO_MANY_WORKERS"), "{body}");
}

/// A turn the user didn't start (here: no controller can say who did) asks
/// the user; with no window to ask in, nothing is created.
#[tokio::test]
async fn without_the_users_go_ahead_nothing_is_created() {
    let state = test_state();
    let (def, _, _) = seed_leader(&state, "claude", "host", json!({})).await;
    let before = definitions(&state);
    let (status, body) = spawn(&state, request(sign_auth(&state, &def.slug), "implement the spec")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("NOT_ALLOWED"), "{body}");
    assert_eq!(definitions(&state), before);
}

/// A fork that doesn't open is deleted again (here: the leader's provider is
/// one `agent.open` doesn't know).
#[tokio::test]
async fn a_worker_that_does_not_open_leaves_no_definition_behind() {
    let state = test_state();
    let (def, tab, block_id) = seed_leader(&state, "no-such-cli", "host", json!({})).await;
    let own = account_of(&leader_account(&def));
    let leader = Leader { block_id, def };
    let before = definitions(&state);
    let err = launch(&state, &leader, "Lead 4", &tab, &WorkerRuntime::default(), &own).await.unwrap_err();
    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.message.starts_with("the worker couldn't be opened, and was removed"), "{}", err.message);
    assert_eq!(definitions(&state), before);
    assert!(state.mstore.agent_def_list().unwrap().iter().all(|d| d.name != "Lead 4"));
}

/// An open that fails after it made the worker's pane (a step after
/// `CreateBlock`) leaves neither the pane nor the definition: the leader's
/// place is free again.
#[tokio::test]
async fn a_worker_whose_open_fails_after_its_pane_exists_leaves_no_pane_behind() {
    let state = test_state();
    let (def, tab, block_id) = seed_leader(&state, "claude", "host", json!({})).await;
    let fork = crate::server::agent_handlers::copy_definition(&state.mstore, &state.id_store, &state.identity_store, &def.id, "Lead 5", false).unwrap();
    // The pane `open_agent_inner` creates, mark included, before its later steps.
    let pane = new_block(
        &state,
        &tab,
        json!({ "view": "agent", "agentId": fork.id, COACHED_BY_META_KEY: { "agent": def.name, "id": def.id, "block_id": block_id } }),
    )
    .await;
    assert_eq!(live_workers_of(&state.mstore, &def.id), 1);

    discard_worker(&state, &tab, &fork.id).await;

    assert_eq!(live_workers_of(&state.mstore, &def.id), 0, "the leader's place is free");
    let tab_now: Tab = state.mstore.must_get(&tab).unwrap();
    assert!(!tab_now.blockids.contains(&pane), "the pane is gone");
    assert!(tab_now.blockids.contains(&block_id), "the leader's own pane stays");
    assert!(state.mstore.agent_def_list().unwrap().iter().all(|d| d.id != fork.id), "and so is the definition");
}

/// The mark is srv's: a client can neither write it on an existing pane nor
/// bring it into a new one.
#[test]
fn a_client_cannot_write_who_coaches_a_pane() {
    let mut meta = MetaMapType::new();
    meta.insert(COACHED_BY_META_KEY.to_string(), json!({ "agent": "Boss", "id": "x", "block_id": "y" }));
    let e = crate::server::browser_owner::guard_client_meta_write("block:b1", &meta).unwrap_err();
    assert!(e.contains(COACHED_BY_META_KEY), "{e}");
    let removed = crate::server::browser_owner::strip_srv_only_keys(&mut meta);
    assert_eq!(removed, vec![COACHED_BY_META_KEY]);
    assert!(meta.is_empty());
}

#[test]
fn the_users_prompt_names_the_leader_the_worker_its_model_and_the_task() {
    let m = consent_message("Loap-3", Some("Loap-4"), Some("sonnet"), &account_of(&account("a1", "me@example.com", "valid")), "Implement SPEC_X.\nReport to Loap-3.");
    assert!(m.starts_with("Loap-3 wants to start a new agent, Loap-4 on sonnet, from its own instructions"), "{m}");
    assert!(m.contains("Implement SPEC_X. Report to Loap-3."), "the task, on one line: {m}");
    assert!(m.contains("It runs on the account me@example.com."), "{m}");
    assert_eq!(one_line(&"x".repeat(500), 400).chars().count(), 401);
}

/// A worker is a new agent, not a fork: the pane's Fork carries the
/// conversation, and Claude resumes a session only for a definition recorded
/// as a fork. So no lineage, a slug of its own name, and no account copied.
#[tokio::test]
async fn a_worker_is_a_new_agent_not_one_of_the_leaders_forks() {
    let state = test_state();
    let (def, _, _) = seed_leader(&state, "claude", "host", json!({})).await;
    let worker = crate::server::agent_handlers::copy_definition(&state.mstore, &state.id_store, &state.identity_store, &def.id, "Lead Helper", false).unwrap();
    assert_eq!((worker.parent_id.as_str(), worker.branch_label.as_str()), ("", ""));
    assert_eq!(worker.slug, "lead-helper");
    assert!(state.identity_store.agent_identity_list_for_agent(&worker.id).unwrap().is_empty(), "the spawn links the account it chose");
    assert_eq!(state.mstore.agent_content_get_all(&worker.id).unwrap().len(), state.mstore.agent_content_get_all(&def.id).unwrap().len());
    // The pane's Fork still makes a fork.
    let fork = crate::server::agent_handlers::copy_definition(&state.mstore, &state.id_store, &state.identity_store, &def.id, "", true).unwrap();
    assert_eq!(fork.parent_id, def.id);
    assert!(!state.identity_store.agent_identity_list_for_agent(&fork.id).unwrap().is_empty(), "a fork keeps its source's account");
    let e = crate::server::agent_handlers::copy_definition(&state.mstore, &state.id_store, &state.identity_store, &def.id, " ", false).unwrap_err();
    assert!(e.contains("needs a name"), "{e}");
}

fn leader_in(state: &AppState, def: &AgentDefinition, block_id: &str) -> Leader {
    let _ = state;
    Leader { block_id: block_id.to_string(), def: def.clone() }
}

/// A pane on `account_id`'s agent `agent_id` whose last run ended on `code`.
async fn failing_pane(state: &AppState, tab: &str, account_id: &str, code: &str) {
    let agent = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&agent, &format!("Busy {}", &agent[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.identity_store.agent_identity_link(&agent, account_id, "claude").unwrap();
    let failure = json!({ "code": code, "title": "t", "detail": "d", "retryable": false });
    new_block(state, tab, json!({ "view": "agent", "agentId": agent, LAST_FAILURE_META_KEY: failure })).await;
}

#[tokio::test]
async fn a_worker_runs_on_the_leaders_account_while_it_can() {
    let state = test_state();
    let (def, _, block) = seed_leader(&state, "claude", "host", json!({})).await;
    state.identity_store.identity_upsert(&account("other", "other@example.com", "valid")).unwrap();
    let a = choose_account(&state, &leader_in(&state, &def, &block), "claude", None).unwrap();
    assert_eq!((a.id, a.note), (leader_account(&def).id, None));
}

/// The leader's account is out of quota (a pane on it hit the plan's limit)
/// or signed out: the worker runs on another of the user's, and says why.
#[tokio::test]
async fn a_worker_moves_to_another_account_when_the_leaders_cannot_run() {
    let state = test_state();
    let (def, tab, block) = seed_leader(&state, "claude", "host", json!({})).await;
    let own = leader_account(&def);
    state.identity_store.identity_upsert(&account("signed-out", "out@example.com", "needs_reauth")).unwrap();
    state.identity_store.identity_upsert(&account("spare", "spare@example.com", "valid")).unwrap();
    failing_pane(&state, &tab, &own.id, "usage_limit").await;
    let a = choose_account(&state, &leader_in(&state, &def, &block), "claude", None).unwrap();
    assert_eq!(a.id, "spare", "skips the signed-out one too");
    let note = a.note.unwrap();
    assert!(note.contains("is out of quota") && note.ends_with("so it runs on spare@example.com instead."), "{note}");
    // A rate limit is transient, not the account's quota.
    let state = test_state();
    let (def, tab, block) = seed_leader(&state, "claude", "host", json!({})).await;
    failing_pane(&state, &tab, &leader_account(&def).id, "rate_limited").await;
    assert_eq!(choose_account(&state, &leader_in(&state, &def, &block), "claude", None).unwrap().note, None);
}

#[tokio::test]
async fn an_account_asked_for_is_used_or_refused_with_why() {
    let state = test_state();
    let (def, tab, block) = seed_leader(&state, "claude", "host", json!({})).await;
    let leader = leader_in(&state, &def, &block);
    state.identity_store.identity_upsert(&account("spare", "Spare@Example.com", "valid")).unwrap();
    state.identity_store.identity_upsert(&account("tired", "tired@example.com", "valid")).unwrap();
    failing_pane(&state, &tab, "tired", "usage_limit").await;
    assert_eq!(choose_account(&state, &leader, "claude", Some("spare@example.com")).unwrap().id, "spare", "by email, any case");
    assert_eq!(choose_account(&state, &leader, "claude", Some("spare")).unwrap().id, "spare", "by id");
    let e = choose_account(&state, &leader, "claude", Some("tired")).unwrap_err();
    assert!(e.message.starts_with("ACCOUNT_CANT_RUN: tired@example.com is out of quota"), "{}", e.message);
    let e = choose_account(&state, &leader, "claude", Some("nobody@example.com")).unwrap_err();
    assert!(e.message.starts_with("UNKNOWN_ACCOUNT"), "{}", e.message);
}

#[tokio::test]
async fn with_no_account_that_can_run_nothing_is_started() {
    let state = test_state();
    let (def, tab, block) = seed_leader(&state, "claude", "host", json!({})).await;
    failing_pane(&state, &tab, &leader_account(&def).id, "auth").await;
    let e = choose_account(&state, &leader_in(&state, &def, &block), "claude", None).unwrap_err();
    assert!(e.message.starts_with("NO_ACCOUNT: your account") && e.message.contains("had its sign-in refused"), "{}", e.message);
}

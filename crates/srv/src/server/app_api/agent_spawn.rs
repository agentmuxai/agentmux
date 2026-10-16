// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/spawn`, the `Agents` tool's `spawn`
//! (docs/specs/SPEC_AGENT_WORKERS_2026_10_15.md §3.1, Phase A): the calling
//! agent, the leader, opens a worker: a new agent built from its own
//! definition, on the model it names, in a pane to the right of its own.
//! A worker is not a fork (the pane's Fork, which carries the conversation):
//! it isn't among the leader's forks and starts with no history. The MCP
//! server then sends it the brief, signed as the leader.
//!
//! Launching an agent spends tokens until someone stops it, so (§3.5): only
//! a verified host agent spawns, a worker doesn't, a leader has at most
//! [`MAX_LIVE_WORKERS`], and unless the user's own turn asked for it the user
//! is asked first.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use agentmux_common::api_types::SpawnAgentRequest;

use super::*;

/// The worker pane's leader, `{agent, id, block_id}`: the leader's name, its
/// definition id and its pane. srv writes it when it opens the worker; no
/// client can (`browser_owner::srv_only_keys`).
pub(crate) const COACHED_BY_META_KEY: &str = "agent:coached_by";

/// Live workers a leader may have at once (§3.5).
pub(crate) const MAX_LIVE_WORKERS: usize = 3;

const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, serde::Serialize)]
pub(crate) struct SpawnAgentResult {
    /// The worker's definition id.
    pub agent_id: String,
    pub name: String,
    /// What `Message` `send` addresses it by (its AGENTMUX_AGENT_ID).
    pub message_to: String,
    pub block_id: String,
    pub tab_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The account it runs on: its name, and its id.
    pub account: String,
    pub account_id: String,
    /// Why it isn't on the account asked for or the leader's, when it isn't.
    pub account_note: Option<String>,
    /// Whether the user was asked (the turn wasn't the user's own).
    pub asked_user: bool,
}

#[derive(Debug)]
pub(crate) struct SpawnError {
    status: StatusCode,
    message: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> SpawnError {
    SpawnError { status, message: message.into() }
}

pub(crate) async fn handle_agent_spawn(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<SpawnAgentRequest>,
) -> Response {
    match spawn_worker(&state, caller.as_deref(), req).await {
        Ok(result) => Json(json!(result)).into_response(),
        Err(e) => (e.status, Json(json!({ "error": e.message }))).into_response(),
    }
}

/// The leader as srv knows it: its pane and its definition.
struct Leader {
    block_id: String,
    def: AgentDefinition,
}

async fn spawn_worker(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    req: SpawnAgentRequest,
) -> Result<SpawnAgentResult, SpawnError> {
    let leader = leader_of(state, caller, &req)?;
    let brief = req.brief.trim();
    if brief.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: a brief is required: the worker's task, its first message"));
    }
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if let Some(n) = name {
        if n.chars().count() > MAX_NAME_CHARS || n.chars().any(char::is_control) {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: a name is at most {MAX_NAME_CHARS} characters, on one line")));
        }
    }
    let provider = state.id_store.resolve_effective_provider_id(&leader.def);
    let leader_runtime = state.mstore.agent_last_runtime_get(&leader.def.id).unwrap_or_default();
    let runtime = worker_runtime(&provider, &leader_runtime, req.model.as_deref(), req.effort.as_deref())
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: {e}")))?;
    let tab_id = super::resolve_tab_id_for_block(&state.mstore, &leader.block_id)
        .map_err(|e| refuse(StatusCode::CONFLICT, format!("NO_TAB: your pane isn't in a tab: {e}")))?;
    // Held until the worker's pane exists (or nothing was made), so a second
    // call counts it: parallel spawns can't each see room for one more.
    let lock = leader_lock(&leader.def.id);
    let _one_at_a_time = lock.lock().await;
    let live = live_workers_of(&state.mstore, &leader.def.id);
    if live >= MAX_LIVE_WORKERS {
        return Err(refuse(
            StatusCode::CONFLICT,
            format!(
                "TOO_MANY_WORKERS: you have {live} workers open, the most at once; close one \
                 (Stop pane) when its work is done, then spawn again"
            ),
        ));
    }

    let account = choose_account(state, &leader, &provider, req.account.as_deref())?;

    let asked_user = consent(state, &leader, name, runtime.model.as_deref(), &account, brief, &req.user_instruction).await?;

    let name = match name {
        Some(n) => n.to_string(),
        None => next_worker_name(&state.mstore, &leader.def.name),
    };
    let (worker, opened) = launch(state, &leader, &name, &tab_id, &runtime, &account).await?;
    tracing::info!(
        leader = %leader.def.name,
        worker = %worker.name,
        block_id = %opened.block_id,
        model = ?runtime.model,
        account = %account.label,
        switched = account.note.is_some(),
        asked_user,
        "agent.spawn: a worker was launched"
    );
    Ok(SpawnAgentResult {
        agent_id: worker.id,
        name: worker.name,
        message_to: worker.slug,
        block_id: opened.block_id,
        tab_id: opened.tab_id,
        provider: opened.provider,
        model: runtime.model,
        effort: runtime.effort,
        account: account.label,
        account_id: account.id,
        account_note: account.note,
        asked_user,
    })
}

/// Build the worker from the leader's definition and open it. A worker that
/// doesn't open is removed again, with any pane the open made before it
/// failed: the user didn't get a worker, so there is no definition of one in
/// My Agents and no marked pane taking up one of the leader's places.
async fn launch(
    state: &AppState,
    leader: &Leader,
    name: &str,
    tab_id: &str,
    runtime: &WorkerRuntime,
    account: &WorkerAccount,
) -> Result<(AgentDefinition, AgentOpenResult), SpawnError> {
    let worker = crate::server::agent_handlers::copy_definition(
        &state.mstore,
        &state.id_store,
        &state.identity_store,
        &leader.def.id,
        name,
        false,
    )
    .map_err(|e| {
        let status = if e.contains("already exists") { StatusCode::CONFLICT } else { StatusCode::INTERNAL_SERVER_ERROR };
        refuse(status, e)
    })?;
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
    let linked = state
        .identity_store
        .agent_identity_link(&worker.id, &account.id, &account.provider)
        .map_err(|e| format!("link its account: {e}"));
    let opened = match linked {
        Ok(()) => open_worker(state, leader, &worker, tab_id, runtime).await,
        Err(e) => Err(e),
    };
    match opened {
        Ok(opened) => Ok((worker, opened)),
        Err(e) => {
            discard_worker(state, tab_id, &worker.id).await;
            Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("the worker couldn't be opened, and was removed: {e}")))
        }
    }
}

/// The verified caller, if it may spawn: a host agent's own pane, not a
/// worker's.
fn leader_of(
    state: &AppState,
    caller: Option<&crate::server::caller::Caller>,
    req: &SpawnAgentRequest,
) -> Result<Leader, SpawnError> {
    let block_id = crate::server::ui_handlers::verified_block_id(state, caller, &req.auth)
        .map_err(|e| refuse(StatusCode::UNAUTHORIZED, format!("could not verify which agent is asking: {e}")))?;
    let block = state
        .mstore
        .get::<Block>(&block_id)
        .ok()
        .flatten()
        .ok_or_else(|| refuse(StatusCode::CONFLICT, "NOT_AN_AGENT_PANE: your pane is gone"))?;
    if block.meta.get(COACHED_BY_META_KEY).is_some_and(|v| !v.is_null()) {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "WORKER_CANNOT_SPAWN: you are a worker; only the agent that coaches you can start workers",
        ));
    }
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    let def = state
        .mstore
        .agent_def_list()
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent.spawn: {e}")))?
        .into_iter()
        .find(|d| !def_id.is_empty() && d.id == def_id)
        .ok_or_else(|| refuse(StatusCode::CONFLICT, "NOT_AN_AGENT_PANE: your pane isn't an agent from My Agents"))?;
    if def.agent_type == "container" {
        return Err(refuse(StatusCode::FORBIDDEN, "CONTAINER_AGENT: a container agent can't start workers"));
    }
    Ok(Leader { block_id, def })
}

/// The worker's remembered runtime (`agentlastruntime`): the leader's, with
/// the requested model and effort over it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkerRuntime {
    /// The normalized JSON to store; empty when there is nothing to remember.
    stored: String,
    model: Option<String>,
    effort: Option<String>,
}

pub(crate) fn worker_runtime(
    provider: &str,
    leader_runtime: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<WorkerRuntime, String> {
    use crate::backend::agent_runtime::Remembered;
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let effort = effort.map(str::trim).filter(|e| !e.is_empty());
    let leader = Remembered::parse(provider, leader_runtime);
    if provider != "claude" {
        if model.is_some() || effort.is_some() {
            return Err(format!("a model and effort can be chosen only for a Claude worker for now; this agent runs {provider}"));
        }
        return Ok(WorkerRuntime::default());
    }
    let mut obj = serde_json::Map::new();
    if let Some(mode) = &leader.permission_mode {
        obj.insert("permissionMode".into(), json!(mode));
    }
    if let Some(m) = model.map(str::to_string).or(leader.model.clone()) {
        obj.insert("model".into(), json!(m));
    }
    if let Some(e) = effort.map(str::to_string).or(leader.effort.clone()) {
        obj.insert("effort".into(), json!(e));
    }
    let raw = serde_json::Value::Object(obj).to_string();
    // What a launch will read back: a value it would drop is refused here.
    let parsed = Remembered::parse(provider, &raw);
    if model.is_some() && parsed.model.as_deref() != model {
        return Err(format!("unknown model {:?}: use opus, sonnet, haiku or a claude-… model id", model.unwrap_or("")));
    }
    if effort.is_some() && parsed.effort.as_deref() != effort {
        return Err(format!("unknown effort {:?}: use low, medium, high, xhigh or max", effort.unwrap_or("")));
    }
    let stored = crate::backend::storage::agents::normalize_last_runtime(&raw)?;
    Ok(WorkerRuntime { stored, model: parsed.model, effort: parsed.effort })
}

/// One spawn at a time per leader, from counting its workers to opening the
/// new one, including the wait for the user. Like `agent_open`'s locks, it
/// serializes only this process's calls.
static LEADER_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(Default::default);

pub(crate) fn leader_lock(leader_def_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut locks = LEADER_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(leader_def_id.to_string()).or_default().clone()
}

/// The account a worker runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerAccount {
    pub id: String,
    /// Its name as the user sees it (display name, email, else name).
    pub label: String,
    /// The link's provider, the leader's.
    pub provider: String,
    /// Why it isn't the account asked for or the leader's, when it isn't.
    pub note: Option<String>,
}

/// The account to run the worker on (§3.5): the one asked for, else the
/// leader's, else, when the leader's can't run (`account_blocks`), the first
/// of the user's other accounts for the provider that can, with a note
/// saying why. Refused when the one asked for can't run, or none can.
fn choose_account(state: &AppState, leader: &Leader, provider: &str, requested: Option<&str>) -> Result<WorkerAccount, SpawnError> {
    use crate::backend::providers::resolve_provider_alias;
    let provider = resolve_provider_alias(provider).to_string();
    let accounts: Vec<IdentityAccount> = state
        .identity_store
        .identity_list(None)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent.spawn: accounts: {e}")))?
        .into_iter()
        .filter(|a| resolve_provider_alias(&a.provider) == provider)
        .collect();
    let blocked = account_blocks(state, &accounts);
    let pick = |a: &IdentityAccount, note: Option<String>| WorkerAccount {
        id: a.id.clone(),
        label: account_label(a),
        provider: provider.clone(),
        note,
    };
    let usable = || accounts.iter().find(|o| !blocked.contains_key(&o.id));
    if let Some(wanted) = requested.map(str::trim).filter(|w| !w.is_empty()) {
        let Some(a) = accounts.iter().find(|a| account_matches(a, wanted)) else {
            let known: Vec<String> = accounts.iter().map(account_label).collect();
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                format!("UNKNOWN_ACCOUNT: no {provider} account {wanted:?}; the user's are: {}", known.join(", ")),
            ));
        };
        if let Some(why) = blocked.get(&a.id) {
            return Err(refuse(
                StatusCode::CONFLICT,
                format!("ACCOUNT_CANT_RUN: {} {why}; name another account, or leave account out", account_label(a)),
            ));
        }
        return Ok(pick(a, None));
    }
    let leader_account = crate::identity::resolver::effective_identity_links(&state.mstore, &state.identity_store, &leader.def)
        .unwrap_or_default()
        .into_iter()
        .filter(|l| resolve_provider_alias(&l.provider) == provider)
        .find_map(|l| accounts.iter().find(|a| a.id == l.account_id));
    match (leader_account, usable()) {
        (Some(a), _) if !blocked.contains_key(&a.id) => Ok(pick(a, None)),
        (Some(a), Some(other)) => {
            let note = format!("Your account {} {}, so it runs on {} instead.", account_label(a), blocked[&a.id], account_label(other));
            Ok(pick(other, Some(note)))
        }
        (Some(a), None) => Err(refuse(
            StatusCode::CONFLICT,
            format!("NO_ACCOUNT: your account {} {}, and none of the user's other {provider} accounts can run", account_label(a), blocked[&a.id]),
        )),
        (None, Some(other)) => Ok(pick(other, Some(format!("You have no {provider} account linked, so it runs on {}.", account_label(other))))),
        (None, None) => Err(refuse(StatusCode::CONFLICT, format!("NO_ACCOUNT: none of the user's {provider} accounts can run"))),
    }
}

/// The accounts that can't take a worker now, and why: signed out (its
/// status), or a pane on it whose last run ended on the plan's usage limit or
/// a refused sign-in (`agent:last_failure`, which its next clean turn clears).
pub(crate) fn account_blocks(state: &AppState, accounts: &[IdentityAccount]) -> std::collections::HashMap<String, String> {
    use crate::agents::failure::FailureClass;
    let mut blocked = std::collections::HashMap::new();
    for a in accounts {
        if matches!(a.status.as_str(), "expired" | "needs_reauth" | "invalid") {
            blocked.insert(a.id.clone(), "needs the user to sign in again".to_string());
        }
    }
    let Ok(tabs) = state.mstore.get_all::<Tab>() else {
        return blocked;
    };
    let blocks = tabs.iter().flat_map(|t| t.blockids.iter()).filter_map(|id| state.mstore.get::<Block>(id).ok().flatten());
    for block in blocks {
        // Only its class: the stored failure leaves out empty fields that
        // reading the whole `AgentFailure` back would require.
        let code = block
            .meta
            .get(LAST_FAILURE_META_KEY)
            .and_then(|f| f.get("code").cloned())
            .and_then(|c| serde_json::from_value::<FailureClass>(c).ok());
        let why = match code {
            Some(FailureClass::UsageLimit) => "is out of quota (a pane on it hit the plan's usage limit)",
            Some(FailureClass::Auth) => "had its sign-in refused",
            _ => continue,
        };
        let agent = obj::meta_get_string(&block.meta, "agentId", "");
        for link in state.identity_store.agent_identity_list_for_agent(&agent).unwrap_or_default() {
            if accounts.iter().any(|a| a.id == link.account_id) {
                blocked.entry(link.account_id).or_insert_with(|| why.to_string());
            }
        }
    }
    blocked
}

/// `agent:last_failure` (`blockcontroller::core::META_LAST_FAILURE`).
const LAST_FAILURE_META_KEY: &str = "agent:last_failure";

fn account_label(a: &IdentityAccount) -> String {
    let email = a.context.get("email").and_then(|e| e.as_str()).unwrap_or("");
    [a.display_name.as_str(), email, a.name.as_str()]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or(&a.id)
        .to_string()
}

/// `wanted` names `a`: its id, name, display name or email, ignoring case.
fn account_matches(a: &IdentityAccount, wanted: &str) -> bool {
    let email = a.context.get("email").and_then(|e| e.as_str()).unwrap_or("");
    [a.id.as_str(), a.name.as_str(), a.display_name.as_str(), email]
        .into_iter()
        .any(|s| !s.is_empty() && s.eq_ignore_ascii_case(wanted))
}

/// "<Leader> worker N", the first N no agent is named.
fn next_worker_name(mstore: &Store, leader: &str) -> String {
    let taken: std::collections::HashSet<String> =
        mstore.agent_def_list().unwrap_or_default().into_iter().map(|d| d.name.to_lowercase()).collect();
    (1..)
        .map(|n| format!("{leader} worker {n}"))
        .find(|n| !taken.contains(&n.to_lowercase()))
        .unwrap_or_default()
}

/// How many open panes `leader_def_id` coaches: workers in some tab.
pub(crate) fn live_workers_of(mstore: &Store, leader_def_id: &str) -> usize {
    let Ok(tabs) = mstore.get_all::<Tab>() else {
        return 0;
    };
    tabs.iter()
        .flat_map(|t| t.blockids.iter())
        .filter_map(|id| mstore.get::<Block>(id).ok().flatten())
        .filter(|b| coached_by_id(&b.meta) == Some(leader_def_id))
        .count()
}

fn coached_by_id(meta: &MetaMapType) -> Option<&str> {
    meta.get(COACHED_BY_META_KEY)?.get("id")?.as_str()
}

/// May the spawn go ahead? At once when the user's own turn asked for it (the
/// `QuitSelf` rule, `self_quit::gate`), otherwise only when the user says so
/// in the approval window, which no agent can answer. Returns whether the
/// user was asked.
async fn consent(
    state: &AppState,
    leader: &Leader,
    name: Option<&str>,
    model: Option<&str>,
    account: &WorkerAccount,
    brief: &str,
    user_instruction: &str,
) -> Result<bool, SpawnError> {
    let provenance = blockcontroller::get_controller(&leader.block_id).and_then(|c| c.turn_provenance());
    let Err(why) = crate::sagas::self_quit::gate(provenance.as_ref(), user_instruction) else {
        return Ok(false);
    };
    tracing::info!(leader = %leader.def.name, why = why.as_str(), "agent.spawn: asking the user");
    let question = json!({
        "kind": "consent",
        "title": format!("{} wants to start a worker", one_line(&leader.def.name, 60)),
        "message": consent_message(&leader.def.name, name, model, account, brief),
        "checkbox": "",
        "ok_label": "Start it",
        "cancel_label": "Don't",
    });
    let answer = super::connections::ask_user(state, &leader.block_id, question)
        .await
        .map_err(|e| refuse(StatusCode::FORBIDDEN, format!("NOT_ALLOWED: starting a worker needs the user's go-ahead, and {e}")))?;
    if answer.answered && answer.approve {
        Ok(true)
    } else if answer.answered {
        Err(refuse(StatusCode::FORBIDDEN, "NOT_ALLOWED: the user said no to starting the worker"))
    } else {
        Err(refuse(StatusCode::FORBIDDEN, "NOT_ALLOWED: the user didn't answer, so no worker was started"))
    }
}

fn consent_message(leader: &str, name: Option<&str>, model: Option<&str>, account: &WorkerAccount, brief: &str) -> String {
    let who = match name {
        Some(n) => format!("a new agent, {}", one_line(n, 60)),
        None => "a new agent".to_string(),
    };
    let on = model.map(|m| format!(" on {}", one_line(m, 40))).unwrap_or_default();
    let note = account.note.as_deref().map(|n| format!(" {}", one_line(n, 200))).unwrap_or_default();
    format!(
        "{} wants to start {who}{on}, from its own instructions, in a pane next to its own, with this task:\n\n{}\n\n\
         It runs on the account {}.{note} It spends tokens until it is stopped. Start it?",
        one_line(leader, 60),
        one_line(brief, 400),
        one_line(&account.label, 80),
    )
}

/// `s` on one line, at most `max` characters.
fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>())
    }
}

/// Remember the worker's runtime, then open it right of the leader's pane,
/// marked as the leader's worker from the moment its block exists.
async fn open_worker(
    state: &AppState,
    leader: &Leader,
    worker: &AgentDefinition,
    tab_id: &str,
    runtime: &WorkerRuntime,
) -> Result<AgentOpenResult, String> {
    if !runtime.stored.is_empty() {
        state
            .mstore
            .agent_last_runtime_set(&worker.id, &runtime.stored)
            .map_err(|e| format!("remember its model: {e}"))?;
    }
    let mut meta = MetaMapType::new();
    meta.insert(
        COACHED_BY_META_KEY.to_string(),
        json!({ "agent": leader.def.name, "id": leader.def.id, "block_id": leader.block_id }),
    );
    super::agent_open::open_agent_inner(
        state,
        CommandAgentOpenData {
            agent_id: worker.id.clone(),
            tab_id: Some(tab_id.to_string()),
            split_direction: Some("right".to_string()),
            split_reference_block_id: Some(leader.block_id.clone()),
            focus: Some(false),
        },
        None,
        meta,
    )
    .await
}

/// Remove a worker whose open failed: the pane the open may have made in
/// `tab_id` before failing (closed as any pane is, `delete_block`), then its
/// definition, as the `deleteagent` RPC deletes one.
async fn discard_worker(state: &AppState, tab_id: &str, def_id: &str) {
    if let Ok(Some(block)) = super::find_agent_block(&state.mstore, tab_id, def_id) {
        if let Err(e) = crate::sagas::delete_block::run(state, tab_id.to_string(), block.oid.clone()).await {
            tracing::warn!(def_id, block_id = %block.oid, error = %e, "agent.spawn: the unopened worker's pane is left behind");
        }
    }
    if let Err(e) = state.mstore.agent_def_delete(def_id) {
        tracing::warn!(def_id, error = %e, "agent.spawn: the unopened worker's definition is left behind");
        return;
    }
    crate::server::agent_handlers::purge_identity_store_rows(&state.identity_store, &state.id_store, def_id, "agent.spawn");
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
}

#[cfg(test)]
#[path = "tests/agent_spawn_tests.rs"]
mod tests;

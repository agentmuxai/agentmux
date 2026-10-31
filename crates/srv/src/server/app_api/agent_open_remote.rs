// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agents.open` for an agent on another machine (`remote_ops::agent_ops`, spec §4.3): it opens the
//! named agent into a tab of its own for the caller, "Team (slug@machine)", made the first time and
//! reused after, the way a leader's workers open in "<Leader> team" (`agent_spawn::team`). The tab is
//! made and the agent opened without focus, so the person's view stays where it is: nothing is
//! brought forward and no tab but this one changes. With no window open the tab goes in a workspace
//! of its own, and the agent runs there until a window shows it.

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::backend::obj::Workspace;
use crate::backend::rpc_types::CommandAgentOpenData;
use crate::backend::service::WebCallType;

use super::agent_lifecycle::{is_running, target_of};
use super::agent_spawn::team::tab_named;
use super::AppState;

/// One caller's find-or-make of its tab at a time, so two opens together make one tab.
static TAB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The tab `who`'s agents open in.
fn tab_name(who: &str) -> String {
    format!("Team ({who})")
}

/// The workspace a new tab goes in: a window's, the main one first, else any that has tabs, else a
/// workspace made for it (no window is opened onto it).
async fn workspace_for_new_tab(state: &AppState, name: &str) -> Result<String, String> {
    if let Some((_, workspace_id)) = crate::server::service::live_windows(&state.mstore).into_iter().next() {
        return Ok(workspace_id);
    }
    let workspaces = state.mstore.get_all::<Workspace>().unwrap_or_default();
    match workspaces.iter().find(|w| !w.tabids.is_empty()) {
        Some(w) => Ok(w.oid.clone()),
        None => super::layout::create_empty_workspace(state, name).await,
    }
}

/// The caller's tab, made (not shown) if it isn't there yet: its id and name.
async fn callers_tab(state: &AppState, who: &str) -> Result<(String, String), String> {
    let _one_at_a_time = TAB_LOCK.lock().await;
    let name = tab_name(who);
    let workspaces = state.mstore.get_all::<Workspace>().unwrap_or_default();
    if let Some(existing) = workspaces.iter().find_map(|w| tab_named(&state.mstore, w, &name)) {
        return Ok((existing, name));
    }
    let workspace_id = workspace_for_new_tab(state, &name).await?;
    let call = WebCallType {
        service: "workspace".to_string(),
        method: "CreateTab".to_string(),
        uicontext: None,
        args: vec![json!(workspace_id), json!(name), json!(false)],
    };
    let made = crate::server::service::handle_create_tab(state, &call).await;
    match made.data.as_ref().and_then(|d| d.as_str()) {
        Some(id) if made.error.is_none() => Ok((id.to_string(), name)),
        _ => Err(format!("no tab '{name}' could be made: {}", made.error.unwrap_or_default())),
    }
}

/// The status for `agent.open`'s own error vocabulary (the HTTP route's, `http_open`), plus a live
/// instance elsewhere, which is a conflict.
fn open_status(e: &str) -> StatusCode {
    if e.starts_with("AGENT_LIVE_ELSEWHERE") {
        StatusCode::CONFLICT
    } else if ["AGENT_NOT_FOUND", "TEMPLATE_NOT_OPENABLE", "INVALID_PROVIDER", "CLI_NOT_AVAILABLE"].iter().any(|p| e.starts_with(p)) {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// `open` of `agent` for the agent `who` (`slug@machine`) on another machine. Opening an agent that
/// already runs is not a start, so it does not use the sender's minute; one that has to start counts
/// against the sender's start limit like `Agents start`.
pub(crate) async fn run_remote(state: &AppState, who: &str, agent: &str) -> Result<Value, (StatusCode, String)> {
    let blocking = state.clone();
    let named = agent.to_string();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Agents open: {e}")))?
        .map_err(|r| (r.status, r.message))?;
    if !is_running(&target) {
        crate::server::start_rate::allow(state, &format!("remote:{who}")).map_err(|e| (StatusCode::TOO_MANY_REQUESTS, e))?;
    }
    let (tab_id, tab_name) = callers_tab(state, who).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Agents open: {e}")))?;
    let cmd = CommandAgentOpenData {
        agent_id: target.agent_id.clone(),
        tab_id: Some(tab_id.clone()),
        split_direction: None,
        split_reference_block_id: None,
        focus: Some(false),
    };
    let opened = super::agent_open::open_agent_impl(state, cmd).await.map_err(|e| (open_status(&e), format!("Agents open {}: {e}", target.name)))?;
    Ok(json!({
        "agent": target.name, "agent_id": target.agent_id, "created": opened.created,
        "tab_id": opened.tab_id, "block_id": opened.block_id, "status": opened.status,
        // An agent already shown in another tab stays there.
        "tab_name": (opened.tab_id == tab_id).then_some(tab_name),
        "note": "opened without focus; Agents stop ends it",
    }))
}

#[cfg(test)]
#[path = "agent_open_remote_tests.rs"]
mod tests;

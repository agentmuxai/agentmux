// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Identity` with action `check_limits`: look again, now, at the agents marked out of quota, for
//! an agent that learns the plan was raised or the quota refilled (task #2648). Without it the only
//! way to start a check was to know that an urgent message does.
//!
//! The same check as the Swarm's "Check now" (`limit_probe::check_limited`): each limited agent gets
//! a small turn, staggered by account, and one that works clears the limit, wakes what was held for
//! the agent and clears the account's other panes (`limit_evidence`). It sends nothing in anyone's
//! name, so any agent may ask.

use serde::Serialize;

use crate::backend::agent_state::{unavailable_of_block, AgentState};
use crate::backend::obj::{self, Block};
use crate::server::AppState;

/// What `check_limits` did.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct CheckLimitsResult {
    /// The agents whose check started or runs, by name.
    pub checked: Vec<String>,
    /// The agents whose check could not start, and why.
    pub failed: Vec<CheckLimitsFailure>,
    /// A note when nothing was out of quota to check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CheckLimitsFailure {
    pub agent: String,
    pub error: String,
}

/// The panes out of quota that a check is asked for: `(block id, agent name, account id)` of every pane
/// of `blocks` that is limited by its plan. BLOCKING.
fn limited_panes(state: &AppState, blocks: &[Block], now_ms: u64) -> Vec<(String, String, Option<String>)> {
    blocks
        .iter()
        .filter_map(|b| {
            let u = unavailable_of_block(&state.mstore, Some(&state.identity_store), &b.oid, now_ms)?;
            if u.state != AgentState::Limited || u.title == crate::agents::failure::ROUTE_CREDITS_TITLE {
                return None;
            }
            let id = obj::meta_get_string(&b.meta, "agentId", "");
            let name = state.mstore.agent_def_get(&id).ok().flatten().map_or(id, |d| d.name);
            Some((b.oid.clone(), name, u.account_id))
        })
        .collect()
}

/// Checks the agents out of quota: those on `account` (its id, label or email) when given, else all of them.
/// BLOCKING.
fn check_blocking(state: &AppState, account: Option<&str>) -> Result<CheckLimitsResult, String> {
    let now_ms = agentmux_common::time::now_ms_u64();
    let accounts = state.id_store.identity_list(None).unwrap_or_default();
    let wanted: Option<String> = match account.map(str::trim).filter(|a| !a.is_empty()) {
        None => None,
        Some(wanted) => {
            let found: Vec<&_> = accounts.iter().filter(|a| crate::identity::account_numbering::names(a, wanted, &accounts)).collect();
            match found.as_slice() {
                [one] => Some(one.id.clone()),
                [] => return Err(format!("no account named \"{wanted}\"; Identity with action \"accounts\" lists them")),
                _ => return Err(format!("\"{wanted}\" names more than one account; use its id")),
            }
        }
    };
    let blocks: Vec<Block> = super::app_api::agent_block::blocks_in_tabs_or_parked(&state.mstore)?.into_iter().map(|(b, _)| b).collect();
    let panes: Vec<_> = limited_panes(state, &blocks, now_ms)
        .into_iter()
        .filter(|(_, _, pane_account)| wanted.as_ref().is_none_or(|w| pane_account.as_ref() == Some(w)))
        .collect();
    if panes.is_empty() {
        return Ok(CheckLimitsResult { note: Some("no agent is out of quota".to_string()), ..Default::default() });
    }
    let ids: Vec<String> = panes.iter().map(|(id, _, _)| id.clone()).collect();
    let mut result = CheckLimitsResult::default();
    for ((_, name, _), (_, answer)) in panes.into_iter().zip(super::limit_probe::check_limited(&state.mstore, &ids)) {
        match answer {
            Ok(()) => result.checked.push(name),
            Err(error) => result.failed.push(CheckLimitsFailure { agent: name, error }),
        }
    }
    Ok(result)
}

/// [`check_blocking`] off the async workers.
pub(crate) async fn check_account_limits(state: AppState, account: Option<String>) -> Result<CheckLimitsResult, String> {
    tokio::task::spawn_blocking(move || check_blocking(&state, account.as_deref())).await.map_err(|e| e.to_string())?
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `identity.accounts` and `identity.switch` (spec §4.3, §4.4): the signed-in accounts here with
//! their stable id and quota, and moving an agent to one. Both run the code a local `Identity`
//! call runs (`server::account_episodes`); a switch is audited as the remote agent,
//! `slug@machine`, never as the operator, and never on the strength of the signed `agent_uid`.

use serde::Deserialize;
use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::server::account_episodes::{self as episodes, AccountSwitchWhen, AgentAccountSwitchReq};

/// `identity.switch`'s params.
#[derive(Debug, Deserialize)]
struct SwitchParams {
    /// The agent on this machine (slug or definition id), or `all limited`. A remote caller names
    /// it: there is no "yourself" across machines.
    #[serde(alias = "agent")]
    agent_id: String,
    /// A stable id, a local id or label, an email with exactly one account with quota, or `any with quota`.
    account: String,
    #[serde(default)]
    when: AccountSwitchWhen,
}

fn failed(error: String) -> RemoteRefusal {
    RemoteRefusal::failed(error)
}

/// Every signed-in account here, as `Identity` `accounts` with scope `machine` lists them.
pub(super) async fn accounts(state: &AppState, _caller: &RemoteCaller, _params: Value) -> RemoteResult {
    let result = episodes::identity_accounts(state.clone()).await.map_err(failed)?;
    serde_json::to_value(result).map_err(|e| failed(e.to_string()))
}

/// Move an agent here to another account. A state change: the transport has already required a
/// certificate from a trusted install for it.
pub(super) async fn switch(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let p: SwitchParams = serde_json::from_value(params).map_err(|e| RemoteRefusal::new("bad_params", format!("identity.switch: {e}")))?;
    let agent = p.agent_id.trim();
    if agent.is_empty() || p.account.trim().is_empty() {
        return Err(RemoteRefusal::new("bad_params", "identity.switch: say which agent and which account"));
    }
    let actor = caller.actor();
    if episodes::is_all_limited(agent) {
        let req = AgentAccountSwitchReq { agent_id: None, account: p.account, when: p.when };
        let all = episodes::switch_all_limited(state.clone(), actor.clone(), req).await.map_err(failed)?;
        tracing::info!(actor = %actor, agents = all.agents.len(), "remote identity.switch: moved every limited agent");
        return serde_json::to_value(all).map_err(|e| failed(e.to_string()));
    }
    let req = AgentAccountSwitchReq { agent_id: Some(agent.to_string()), account: p.account, when: p.when };
    let done = episodes::switch_account(state.clone(), actor.clone(), req).await.map_err(failed)?;
    tracing::info!(actor = %actor, agent = %agent, status = ?done.status, "remote identity.switch: switched an agent's account");
    Ok(json!(done))
}

#[cfg(test)]
#[path = "identity_ops_tests.rs"]
mod tests;

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! `agent.account.switch`: change which account an agent runs on
//! (`docs/specs/SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md` §3.1, §3.2, §3.4).
//!
//! The one door for the composer's login email, the failure row, and agents
//! (`Identity` `switch`). It does what `bindAccountToAgent` did in the window
//! (replace the link, clear the old account's limits, restart the panes with
//! `--resume`), in srv, so every caller gets the same result and the same audit
//! row. An agent in a turn is not restarted under it: the choice waits on its
//! panes (`agent:pending_account`) and is applied when none of them is in a turn.

use serde::{Deserialize, Serialize};

use crate::backend::agent_resolve::resolve_agent_id;
use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block};
use crate::backend::storage::identities::{IdentityAccount, SecretRef};
use crate::server::app_api::agent_spawn::account_label_in;
use crate::server::AppState;

use super::moves::{busy, relink, restart, signed_in};
use super::{agent_name, canonical, tab_blocks};

/// A pane's pending switch: the account id the agent moves to once none of its
/// panes is in a turn. The pane's meta carries it, so the composer chip shows it
/// live and it survives a restart.
pub const META_PENDING_ACCOUNT: &str = "agent:pending_account";
/// Who asked for it, for the audit row written when it is applied.
const META_PENDING_BY: &str = "agent:pending_account_by";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum AccountSwitchWhen {
    /// Restart the agent's panes now, ending a turn in progress.
    Now,
    /// Apply when none of its panes is in a turn (at once when none is).
    #[default]
    AfterTurn,
}

/// `agent.account.switch`.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentAccountSwitchReq {
    /// The agent (slug or definition id). An agent connection may leave it out for itself.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The account: its id, or the label the pane shows (`email ·2`).
    pub account: String,
    #[serde(default)]
    pub when: AccountSwitchWhen,
}

/// What `agent.account.switch` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum AccountSwitchStatus {
    /// Linked, and the open panes restarted on it.
    Switched,
    /// An agent's pane is in a turn: the switch waits for the turn to end.
    Queued,
    /// The agent is already on that account (a queued switch is cancelled).
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentAccountSwitchResult {
    pub status: AccountSwitchStatus,
    pub account_id: String,
    pub account_label: String,
    /// Panes restarted on the new account.
    pub restarted: u32,
    /// Panes linked but not restarted, with why.
    pub failed: Vec<String>,
}

fn open_panes(state: &AppState, def_id: &str) -> Vec<Block> {
    tab_blocks(&state.mstore).into_iter().filter(|b| obj::meta_get_string(&b.meta, "agentId", "") == def_id).collect()
}

fn set_pending(state: &AppState, blocks: &[Block], pending: Option<(&str, &str)>) {
    for b in blocks {
        let has = b.meta.get(META_PENDING_ACCOUNT).is_some_and(|v| !v.is_null());
        if pending.is_none() && !has {
            continue;
        }
        let mut update = obj::MetaMapType::new();
        update.insert(META_PENDING_ACCOUNT.to_string(), pending.map_or(serde_json::Value::Null, |(a, _)| serde_json::json!(a)));
        update.insert(META_PENDING_BY.to_string(), pending.map_or(serde_json::Value::Null, |(_, by)| serde_json::json!(by)));
        match crate::server::service::update_object_meta(&state.mstore, &format!("block:{}", b.oid), &update) {
            Ok(()) => blockcontroller::core::broadcast_block_update(&state.mstore, &state.event_bus, &b.oid),
            Err(e) => tracing::warn!(block_id = %b.oid, error = %e, "agent.account.switch: pending switch not stored"),
        }
    }
}

/// Point the pane's own `cmd:env` config-dir variable at `to`. The spawn injects the linked
/// account over it, but the window's auth re-check and login recovery read `cmd:env` as it is.
fn refresh_auth_env(state: &AppState, block: &Block, to: &IdentityAccount, provider: &str) {
    let SecretRef::OAuthConfigDir { dir } = &to.secret_ref else { return };
    let Some(var) = crate::backend::providers::get_provider(provider).map(|p| p.auth_config_dir_env_var).filter(|v| !v.is_empty()) else { return };
    let mut env = blockcontroller::cmd_env_of(&block.meta);
    env.insert(var.to_string(), dir.clone());
    let mut update = obj::MetaMapType::new();
    update.insert("cmd:env".to_string(), serde_json::json!(env));
    match crate::server::service::update_object_meta(&state.mstore, &format!("block:{}", block.oid), &update) {
        Ok(()) => blockcontroller::core::broadcast_block_update(&state.mstore, &state.event_bus, &block.oid),
        Err(e) => tracing::warn!(block_id = %block.oid, error = %e, "agent.account.switch: the pane's auth env was not updated"),
    }
}

/// End the pane's turn by replacing its process. A forced resync of a persistent pane in a turn
/// only defers the restart to the turn's end, which is `after_turn`, not `now`.
fn stop_turn(block: &Block) {
    if let Some(ctrl) = blockcontroller::take_controller(&block.oid) {
        let _ = crate::sagas::agent_teardown::replace_now(ctrl.as_ref());
    }
}

/// Link `agent` to `to` and restart each of `blocks` on it. `actor` is recorded. `now` ends a
/// turn in progress; otherwise the caller has seen that no pane is in one.
fn apply(state: &AppState, actor: &str, def_id: &str, to: &IdentityAccount, provider: &str, blocks: &[Block], all: &[IdentityAccount], now: bool) -> AgentAccountSwitchResult {
    set_pending(state, blocks, None);
    let label = account_label_in(to, all);
    let mut result = AgentAccountSwitchResult { status: AccountSwitchStatus::Switched, account_id: to.id.clone(), account_label: label.clone(), restarted: 0, failed: Vec::new() };
    let linked = relink(state, def_id, &to.id, provider);
    let request_id = uuid::Uuid::new_v4().to_string();
    let reason = format!("switched to {label}");
    for block in blocks {
        let outcome = linked.clone().and_then(|()| {
            refresh_auth_env(state, block, to, provider);
            if now && busy(state, &block.oid) {
                stop_turn(block);
            }
            restart(state, block).map_err(|e| format!("linked, but its pane didn't restart ({e}); it uses the new account when it next starts"))
        });
        state.reactive_handler.log_fleet_action_audit(
            Some(actor),
            &agent_name(&state.mstore, block),
            &block.oid,
            "agent.account.switch",
            outcome.is_ok(),
            outcome.as_ref().err().map(String::as_str),
            &request_id,
            Some(&reason),
        );
        match outcome {
            Ok(()) => result.restarted += 1,
            Err(e) => result.failed.push(e),
        }
    }
    if blocks.is_empty() {
        if let Err(e) = linked {
            result.failed.push(e);
        }
    }
    tracing::info!(agent = %actor, def_id = %def_id, restarted = result.restarted, failed = result.failed.len(), "agent.account.switch: switched an agent's account");
    super::refresh_soon();
    result
}

/// `agent.account.switch`. `caller` is the connection's agent slug, empty for the window.
/// BLOCKING (store reads, link writes, restarts).
pub(super) fn switch(state: &AppState, caller: &str, req: &AgentAccountSwitchReq) -> Result<AgentAccountSwitchResult, String> {
    let named = req.agent_id.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or(caller);
    if named.is_empty() {
        return Err("agent.account.switch: say which agent".to_string());
    }
    let def_id = resolve_agent_id(&state.mstore, named).map_err(|e| format!("agent.account.switch: {e}"))?;
    let provider = state
        .mstore
        .agent_def_list()
        .map_err(|e| format!("agent.account.switch: {e}"))?
        .into_iter()
        .find(|d| d.id == def_id)
        // The provider its launch uses: its own, or its bundle's when it has none.
        .map(|d| canonical(&state.mstore.resolve_effective_provider_id(&d)))
        .ok_or("agent.account.switch: no such agent")?;
    let accounts: Vec<IdentityAccount> = state
        .id_store
        .identity_list(None)
        .map_err(|e| format!("agent.account.switch: accounts: {e}"))?
        .into_iter()
        .filter(|a| canonical(&a.provider) == provider)
        .collect();
    let wanted = req.account.trim();
    let Some(to) = accounts.iter().find(|a| crate::identity::account_numbering::names(a, wanted, &accounts)) else {
        let known: Vec<String> = accounts.iter().map(|a| account_label_in(a, &accounts)).collect();
        return Err(format!("UNKNOWN_ACCOUNT: no {provider} account {wanted:?}; the user's are: {}", known.join(", ")));
    };
    if !signed_in(&to.status) {
        return Err(format!("ACCOUNT_CANT_RUN: {} is not signed in; sign in to it first", account_label_in(to, &accounts)));
    }
    let blocks = open_panes(state, &def_id);
    let current = state
        .identity_store
        .agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("agent.account.switch: {e}"))?
        .into_iter()
        .find(|l| canonical(&l.provider) == provider)
        .map(|l| l.account_id);
    let label = account_label_in(to, &accounts);
    if current.as_deref() == Some(to.id.as_str()) {
        set_pending(state, &blocks, None);
        return Ok(AgentAccountSwitchResult { status: AccountSwitchStatus::Unchanged, account_id: to.id.clone(), account_label: label, restarted: 0, failed: Vec::new() });
    }
    let actor = if caller.is_empty() { "operator" } else { caller };
    if req.when == AccountSwitchWhen::AfterTurn && blocks.iter().any(|b| busy(state, &b.oid)) {
        set_pending(state, &blocks, Some((&to.id, actor)));
        return Ok(AgentAccountSwitchResult { status: AccountSwitchStatus::Queued, account_id: to.id.clone(), account_label: label, restarted: 0, failed: Vec::new() });
    }
    Ok(apply(state, actor, &def_id, to, &provider, &blocks, &accounts, req.when == AccountSwitchWhen::Now))
}

/// Apply each queued switch whose agent has no pane in a turn. Runs with every
/// episode refresh (a turn ending pokes it). BLOCKING.
pub(super) fn apply_pending(state: &AppState) {
    let panes: Vec<Block> = tab_blocks(&state.mstore).into_iter().filter(|b| b.meta.get(META_PENDING_ACCOUNT).is_some_and(|v| v.is_string())).collect();
    let mut done = std::collections::HashSet::new();
    for first in &panes {
        let def_id = obj::meta_get_string(&first.meta, "agentId", "");
        let account_id = obj::meta_get_string(&first.meta, META_PENDING_ACCOUNT, "");
        let by = obj::meta_get_string(&first.meta, META_PENDING_BY, "operator");
        if def_id.is_empty() || !done.insert(def_id.clone()) {
            continue;
        }
        let blocks = open_panes(state, &def_id);
        if blocks.iter().any(|b| busy(state, &b.oid)) {
            continue;
        }
        let accounts: Vec<IdentityAccount> = state.id_store.identity_list(None).unwrap_or_default();
        let Some(to) = accounts.iter().find(|a| a.id == account_id && signed_in(&a.status)) else {
            tracing::info!(def_id = %def_id, "agent.account.switch: a queued switch was dropped, its account is gone or signed out");
            set_pending(state, &blocks, None);
            continue;
        };
        let provider = canonical(&to.provider);
        let same_provider: Vec<IdentityAccount> = accounts.iter().filter(|a| canonical(&a.provider) == provider).cloned().collect();
        apply(state, &by, &def_id, to, &provider, &blocks, &same_provider, false);
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Automatic failover (`docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` §4.4, D4):
//! an agent whose pane hit a usage limit moves itself to another signed-in account of its
//! provider that has quota, once its turn is over, and says so in one note to the agent and one
//! to its lead.
//!
//! It is `agent.account.switch` (`switch::switch_to`, `after_turn`) chosen by srv, so the move
//! relinks, clears the old account's limit and restarts the pane the way every other switch does.
//! It never picks:
//! - an account the operator marked "never fail over to this" (`context.never_failover`);
//! - an account that is not signed in (`valid`/`ok`; an unchecked one is not trusted with an
//!   unattended move), or one a pane of another agent has just found out of quota;
//! - an account of another provider.
//!
//! `agent:accountfailover` (default on; `false` turns it all off) is the one setting.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::backend::agent_state::AgentState;
use crate::backend::obj::Block;
use crate::backend::storage::identities::IdentityAccount;
use crate::server::AppState;

use super::accounts::pick;
use super::machine::BlockedPane;
use super::moves::signed_in;
use super::switch::{open_panes, AccountSwitchWhen, META_PENDING_ACCOUNT};
use super::{agent_name, canonical};

/// `agent:accountfailover`: move an agent that hit its limit to an account with quota. On unless `false`.
pub const SETTING: &str = "agent:accountfailover";
/// Who the audit row names, and what tells [`notify`] a switch was a failover.
pub(super) const ACTOR: &str = "failover";
/// The same for the move off a login that expired (`login_expiry`).
pub(super) const ACTOR_EXPIRED: &str = "failover:expired";

/// Whether `actor` is one of the failovers (their moves are told to the agent and its lead).
pub(super) fn is_actor(actor: &str) -> bool {
    actor == ACTOR || actor == ACTOR_EXPIRED
}
/// The account's mark (in `context`): never move an agent onto it by itself.
pub const NEVER_FAILOVER: &str = "never_failover";

/// Whether the operator marked `a` "never fail over to this".
pub fn is_never(a: &IdentityAccount) -> bool {
    a.context.get(NEVER_FAILOVER).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// `identity.account.set_failover`.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountSetFailoverReq {
    pub account_id: String,
    /// `false` marks the account "never fail over to this"; `true` takes the mark off.
    pub allow: bool,
}

/// `identity.account.set_failover`: how the account stands now.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountSetFailoverResult {
    pub account_id: String,
    pub allow: bool,
}

/// `identity.account.set_failover`. BLOCKING (a store write).
pub(super) fn set_failover(state: &AppState, req: &IdentityAccountSetFailoverReq) -> Result<IdentityAccountSetFailoverResult, String> {
    let found = state.id_store.identity_set_never_failover(&req.account_id, !req.allow).map_err(|e| format!("identity.account.set_failover: {e}"))?;
    if !found {
        return Err(format!("identity.account.set_failover: no account {:?}", req.account_id));
    }
    super::refresh_soon();
    Ok(IdentityAccountSetFailoverResult { account_id: req.account_id.clone(), allow: req.allow })
}

/// Accounts an agent was moved off after a limit, and when that limit lifts (unix ms): the
/// move clears the stored failure, so without this the next failover could send the agent
/// straight back to an account no pane has re-tested yet.
static LEFT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> = std::sync::LazyLock::new(Default::default);
/// How long to avoid an account left over a limit with no known reset.
const UNKNOWN_RESET_MS: u64 = 60 * 60_000;

fn remember_left(pane: &BlockedPane, now_ms: u64) {
    let until = pane.resets_at_ms.unwrap_or(now_ms + UNKNOWN_RESET_MS);
    LEFT.lock().unwrap_or_else(|e| e.into_inner()).insert(pane.account_id.clone(), until);
}

fn recently_left(account_id: &str, now_ms: u64) -> bool {
    let mut left = LEFT.lock().unwrap_or_else(|e| e.into_inner());
    left.retain(|_, until| *until > now_ms);
    left.contains_key(account_id)
}

pub(super) fn enabled(state: &AppState) -> bool {
    state.config_watcher.extra_bool(SETTING).unwrap_or(true)
}

/// The account `pane`'s agent moves to: one of its provider's other accounts that is signed in,
/// has quota, isn't marked and wasn't just left over its own limit, else why not.
fn target<'a>(state: &AppState, accounts: &'a [IdentityAccount], pane: &BlockedPane, now_ms: u64) -> Result<&'a IdentityAccount, String> {
    let usable: Vec<IdentityAccount> =
        accounts.iter().filter(|a| canonical(&a.provider) == pane.provider && signed_in(&a.status) && !is_never(a) && !recently_left(&a.id, now_ms)).cloned().collect();
    let chosen = pick(state, &usable, "any with quota", std::slice::from_ref(&pane.account_id), &pane.provider)?;
    accounts.iter().find(|a| a.id == chosen.id).ok_or_else(|| "the account is gone".to_string())
}

/// Move each agent with a pane out of quota (a usage limit, not a refused sign-in) to an account
/// that can run. `panes` are the blocked panes of this refresh. An agent with a move already
/// queued is left to it. BLOCKING (store reads, link writes, restarts).
pub(super) fn run(state: &AppState, panes: &[BlockedPane]) {
    if !enabled(state) {
        return;
    }
    let mut by_agent: BTreeMap<&str, &BlockedPane> = BTreeMap::new();
    for pane in panes.iter().filter(|p| p.state == AgentState::Limited && !p.agent_id.is_empty()) {
        by_agent.entry(pane.agent_id.as_str()).or_insert(pane);
    }
    if by_agent.is_empty() {
        return;
    }
    let accounts = state.id_store.identity_list(None).unwrap_or_default();
    for (def_id, pane) in by_agent {
        if open_panes(state, def_id).iter().any(|b| b.meta.get(META_PENDING_ACCOUNT).is_some_and(|v| v.is_string())) {
            continue;
        }
        let now_ms = agentmux_common::time::now_ms_u64();
        let to = match target(state, &accounts, pane, now_ms) {
            Ok(to) => to,
            Err(why) => {
                tracing::debug!(agent = %pane.agent_name, "account failover: no account to move to: {why}");
                continue;
            }
        };
        let same_provider: Vec<IdentityAccount> = accounts.iter().filter(|a| canonical(&a.provider) == pane.provider).cloned().collect();
        match super::switch::switch_to(state, ACTOR, def_id, to, &pane.provider, &same_provider, AccountSwitchWhen::AfterTurn) {
            Ok(_) => remember_left(pane, now_ms),
            Err(e) => tracing::warn!(agent = %pane.agent_name, error = %e, "account failover: the move failed"),
        }
    }
}

/// The one-line notes for a failover that landed: to the agent's panes, and to its lead's first
/// open pane. Held for each one's next turn: neither is woken for it, and the agent's pane is not
/// asked to run before its account is in place. `expired` says the login ended, not the quota;
/// the panes in `woken` get their own note that starts a turn (`login_expiry`), so not this one.
pub(super) fn notify(state: &AppState, def_id: &str, to_label: &str, blocks: &[Block], expired: bool, woken: &[String]) {
    let Some(first) = blocks.first() else { return };
    let name = agent_name(&state.mstore, first);
    let to = crate::backend::reactive::sanitize::quote_in_system_note(to_label);
    let key = format!("failover:{def_id}");
    let why = if expired { "Your login expired" } else { "Your account hit its usage limit" };
    let own = format!("[AgentMux] {why}, so AgentMux moved you to {to}. Your conversation goes on there.");
    for block in blocks.iter().filter(|b| !woken.contains(&b.oid)) {
        crate::backend::blockcontroller::system_note_into_turn_or_hold(&block.oid, &key, &own);
    }
    let Some(lead) = crate::server::app_api::agent_spawn::worker_of(&state.mstore, def_id) else { return };
    let Some(lead_pane) = open_panes(state, &lead).into_iter().next() else { return };
    let who = crate::backend::reactive::sanitize::quote_in_system_note(&name);
    let text = if expired {
        format!("[AgentMux] {who}'s login expired, so AgentMux moved it to {to}.")
    } else {
        format!("[AgentMux] {who} hit its usage limit, so AgentMux moved it to {to}.")
    };
    crate::backend::blockcontroller::system_note_into_turn_or_hold(&lead_pane.oid, &key, &text);
}

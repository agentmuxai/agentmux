// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Move these agents to another signed-in account"
//! (`docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §6.3): `agent.account.switch`
//! (`switch::switch_to`) for each agent of the episode, so an agent in a turn is queued.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::backend::agent_state::{self, AgentState};
use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block};
use crate::backend::storage::identities::IdentityAccount;
use crate::server::app_api::agent_spawn::{account_blocks, account_label_in};
use crate::server::AppState;

use super::{canonical, AccountEpisodeAgent, AccountMoveCandidate};

/// `identity.account.move_agents`: move these panes' agents from one account
/// to another of the same provider.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountMoveAgentsReq {
    pub from: String,
    pub to: String,
    pub block_ids: Vec<String>,
}

/// A pane whose agent couldn't be moved, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountMoveFailure {
    pub block_id: String,
    pub agent_name: String,
    pub error: String,
}

/// `identity.account.move_agents`: what moved, what was left because it is
/// in a turn, and what failed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountMoveAgentsResult {
    pub moved: Vec<AccountEpisodeAgent>,
    /// In a turn (working, or waiting on a question): its move is queued and
    /// happens when the turn ends (`agent:pending_account`).
    pub busy: Vec<AccountEpisodeAgent>,
    pub failed: Vec<AccountMoveFailure>,
}

pub(super) fn signed_in(status: &str) -> bool {
    matches!(status, "valid" | "ok")
}

/// Whether a queued move may still land on the account: what the strip offered (signed in, or not
/// checked yet), so a move it reported as queued isn't dropped when the turn ends.
pub(super) fn can_receive(status: &str) -> bool {
    signed_in(status) || status == "unknown"
}

/// Where `from`'s agents can go (spec §6.3): the channel's other accounts of
/// the same canonical provider that nothing blocks (`account_blocks`: signed
/// out, or a pane on it out of quota). Signed-in ones first, so the picker's
/// first entry is the one to preselect (spec §13, decision 2).
pub(super) fn candidates(state: &AppState, from: &str, provider: &str) -> Vec<AccountMoveCandidate> {
    // Labels are numbered against every account of the provider, the source included.
    let every: Vec<IdentityAccount> =
        state.id_store.identity_list(None).unwrap_or_default().into_iter().filter(|a| canonical(&a.provider) == provider).collect();
    let accounts: Vec<IdentityAccount> = every.iter().filter(|a| a.id != from).cloned().collect();
    let blocked = account_blocks(state, &accounts);
    let mut out: Vec<AccountMoveCandidate> = accounts
        .iter()
        .filter(|a| !blocked.contains_key(&a.id))
        .map(|a| AccountMoveCandidate { account_id: a.id.clone(), label: account_label_in(a, &every), status: a.status.clone() })
        .collect();
    out.sort_by_key(|c| !signed_in(&c.status));
    out
}

/// Link `agent_id` to `to` for `provider`: unlink its rows stored under an
/// alias of the provider first (one would otherwise silently win at the next
/// spawn), then link, then clear the failures its old account caused.
pub(super) fn relink(state: &AppState, agent_id: &str, to: &str, provider: &str) -> Result<(), String> {
    let before = super::failed_panes(&state.mstore, &state.identity_store, agent_id, provider);
    let links = state.identity_store.agent_identity_list_for_agent(agent_id).map_err(|e| e.to_string())?;
    for l in links.iter().filter(|l| l.provider != provider && canonical(&l.provider) == provider) {
        state.identity_store.agent_identity_unlink(agent_id, &l.provider).map_err(|e| e.to_string())?;
    }
    state.identity_store.agent_identity_link(agent_id, to, provider).map_err(|e| e.to_string())?;
    state.broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(agent_id, &(), vec![], 0);
    if let Some(watcher) = crate::backend::subagent_watcher::global() {
        watcher.recheck_all_watched_agents();
    }
    super::after_rebind(&state.mstore, &state.event_bus, before, to);
    Ok(())
}

/// Restart `block`'s process on the new link, keeping its conversation
/// (`ControllerResync` with `forcerestart`). A parked block, one in no tab, restarts with no tab
/// (an empty id, as everywhere a block is parked): its agent runs with no pane showing it.
pub(super) fn restart(state: &AppState, block: &Block) -> Result<(), String> {
    let tab_id = block.parentoref.strip_prefix("tab:").unwrap_or("");
    blockcontroller::resync_controller(
        block,
        tab_id,
        None,
        true,
        true,
        Some(state.broker.clone()),
        Some(state.event_bus.clone()),
        Some(state.mstore.clone()),
        Some(state.filestore.clone()),
        Some(state.id_store.clone()),
        Some(state.identity_store.clone()),
        state.mstore.shared_agent_registry(),
        state.boot_id.clone(),
        &state.auth_key,
        Some(state.config_watcher.clone()),
    )
}

/// Whether the pane is in a turn: it isn't moved under it.
pub(super) fn busy(state: &AppState, block_id: &str) -> bool {
    matches!(agent_state::current_state(&state.mstore, block_id), Some(AgentState::Working | AgentState::Waiting))
}

/// `identity.account.move_agents`. BLOCKING (store reads, link writes, restarts).
pub(super) fn move_agents(state: &AppState, req: &IdentityAccountMoveAgentsReq) -> Result<IdentityAccountMoveAgentsResult, String> {
    let accounts = state.id_store.identity_list(None).map_err(|e| format!("identity.account.move_agents: accounts: {e}"))?;
    let to = accounts.iter().find(|a| a.id == req.to).ok_or("identity.account.move_agents: no such account to move to")?;
    if req.to == req.from {
        return Err("identity.account.move_agents: that is the account they are on".to_string());
    }
    let provider = canonical(&to.provider);
    if let Some(from) = accounts.iter().find(|a| a.id == req.from) {
        if canonical(&from.provider) != provider {
            return Err(format!("identity.account.move_agents: {} is a {} account; agents never move across providers", account_label_in(to, &accounts), to.provider));
        }
    }
    if let Some(why) = account_blocks(state, std::slice::from_ref(to)).get(&to.id) {
        return Err(format!("identity.account.move_agents: {} {why}", account_label_in(to, &accounts)));
    }

    let mut result = IdentityAccountMoveAgentsResult::default();
    let mut by_agent: BTreeMap<String, Vec<Block>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for block_id in req.block_ids.iter().filter(|b| seen.insert(b.as_str())) {
        let block = state.mstore.get::<Block>(block_id).ok().flatten();
        let agent_id = block.as_ref().map(|b| obj::meta_get_string(&b.meta, "agentId", "")).unwrap_or_default();
        match block {
            Some(b) if !agent_id.is_empty() => by_agent.entry(agent_id).or_default().push(b),
            _ => result.failed.push(AccountMoveFailure {
                block_id: block_id.clone(),
                agent_name: String::new(),
                error: "not an open agent pane".to_string(),
            }),
        }
    }

    let same_provider: Vec<IdentityAccount> = accounts.iter().filter(|a| canonical(&a.provider) == provider).cloned().collect();
    let open = super::tab_blocks(&state.mstore);
    for (agent_id, asked) in by_agent {
        // The link is the agent's, for every pane of it, not only the ones asked for.
        let mut blocks: Vec<Block> = open.iter().filter(|b| obj::meta_get_string(&b.meta, "agentId", "") == agent_id).cloned().collect();
        for b in asked {
            if !blocks.iter().any(|o| o.oid == b.oid) {
                blocks.push(b);
            }
        }
        let agents: Vec<AccountEpisodeAgent> = blocks
            .iter()
            .map(|b| AccountEpisodeAgent { block_id: b.oid.clone(), agent_id: agent_id.clone(), agent_name: super::agent_name(&state.mstore, b) })
            .collect();
        // The one door the composer and agents use: an agent in a turn is queued, not left.
        match super::switch::switch_to(state, "operator", &agent_id, to, &provider, &same_provider, super::switch::AccountSwitchWhen::AfterTurn) {
            Ok(r) if r.status == super::switch::AccountSwitchStatus::Queued => result.busy.extend(agents),
            Ok(r) if r.failed.is_empty() => result.moved.extend(agents),
            // Linked, but some panes didn't restart. The failures don't say which pane, and the
            // strip names the agent, so the counts are what matters: one failure per pane that
            // didn't restart, the rest moved.
            Ok(r) => {
                let mut panes = agents.into_iter();
                for error in &r.failed {
                    if let Some(a) = panes.next() {
                        result.failed.push(AccountMoveFailure { block_id: a.block_id, agent_name: a.agent_name, error: error.clone() });
                    }
                }
                result.moved.extend(panes);
            }
            Err(error) => {
                result.failed.extend(agents.into_iter().map(|a| AccountMoveFailure { block_id: a.block_id, agent_name: a.agent_name, error: error.clone() }))
            }
        }
    }
    tracing::info!(
        provider = %provider,
        moved = result.moved.len(),
        busy = result.busy.len(),
        failed = result.failed.len(),
        "identity.account.move_agents: moved an episode's agents to another account"
    );
    if !result.moved.is_empty() {
        crate::backend::reactive::held_jekt_wake().notify_one();
    }
    Ok(result)
}

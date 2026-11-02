// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A login that expired: its agents move to a valid login by themselves, and the one that was
//! mid-turn carries on.
//!
//! An account whose sign-in status is `expired`, `needs_reauth` or `invalid` can't run an agent. The
//! agents on it:
//! - move to another valid login of the same account first (the same stable id, so the same plan
//!   and quota), else to another signed-in account of the provider that can run (never one marked
//!   "never fail over to this"; `agent:accountfailover` turns it all off), as `agent.account.switch`
//!   does, once their turn is over;
//! - get one note that says so; one whose turn died on the expired login gets, as the note, a turn
//!   of its own: "your last step may not have finished: check git status and your PR first", so it
//!   carries on instead of sitting idle (a compaction that failed on the login is a turn that
//!   compacts again on the new one);
//! - are the person's business once: one notification, and a row in the account strip with the
//!   sign-in action, until the login is signed in again.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use crate::agents::failure::FailureClass;
use crate::backend::agent_state::AgentState;
use crate::backend::agent_state::failure_of_block;
use crate::backend::obj::Block;
use crate::backend::storage::identities::IdentityAccount;
use crate::identity::stable_account::stable_account_id;
use crate::server::app_api::agent_spawn::{account_blocks, account_label_in};
use crate::server::AppState;

use super::failover::{self, ACTOR_EXPIRED};
use super::machine::BlockedPane;
use super::moves::signed_in;
use super::switch::{open_panes, AccountSwitchStatus, AccountSwitchWhen, META_PENDING_ACCOUNT};
use super::{canonical, AccountAvailability, AccountAvailabilityEvent};

/// The logins whose agents were moved off after they expired, with when: the person is told once
/// for each, and the strip shows it until it is signed in again.
pub(super) static TOLD: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

/// Whether a sign-in status means the login can't run an agent.
pub(super) fn expired(status: &str) -> bool {
    matches!(status, "expired" | "needs_reauth" | "invalid")
}

/// Whether the login is lost, not merely out of date. `needs_reauth` and `invalid` say the provider
/// refused it. `expired` can be an access token the CLI refreshes by itself (a spawn stores it so), so
/// it counts only once a pane's run on it was refused: `refused` are the accounts of those panes.
fn lost(a: &IdentityAccount, refused: &HashSet<&str>) -> bool {
    match a.status.as_str() {
        "needs_reauth" | "invalid" => true,
        "expired" => refused.contains(a.id.as_str()),
        _ => false,
    }
}

/// Of `blocks`, the ones whose turn died on a refused sign-in: read before the move clears it.
pub(super) fn interrupted(blocks: &[Block]) -> Vec<String> {
    blocks.iter().filter(|b| failure_of_block(b).is_some_and(|f| f.code == FailureClass::Auth)).map(|b| b.oid.clone()).collect()
}

/// Where the agents of `from` go: a signed-in login of the same account (stable id) that nothing
/// blocks, else another signed-in account of its provider that can run. `blocked` is
/// `account_blocks` (an account a pane found out of quota or signed out); `left` says an account an
/// agent was just moved off over its limit, which no pane has re-tested.
pub(super) fn choose<'a>(
    accounts: &'a [IdentityAccount],
    blocked: &HashMap<String, String>,
    left: &dyn Fn(&str) -> bool,
    from: &IdentityAccount,
) -> Option<&'a IdentityAccount> {
    let provider = canonical(&from.provider);
    let usable = |a: &&IdentityAccount| {
        a.id != from.id && canonical(&a.provider) == provider && signed_in(&a.status) && !failover::is_never(a) && !blocked.contains_key(&a.id) && !left(&a.id)
    };
    let oldest = |a: &&IdentityAccount| (a.created_at, a.id.clone());
    let same_account = stable_account_id(from);
    same_account
        .as_ref()
        .and_then(|sid| accounts.iter().filter(usable).filter(|a| stable_account_id(a).as_ref() == Some(sid)).min_by_key(oldest))
        .or_else(|| accounts.iter().filter(usable).min_by_key(oldest))
}

/// What the agent whose turn was cut off is told, as the turn it starts.
pub(super) fn resume_note(to_label: &str) -> String {
    let to = crate::backend::reactive::sanitize::quote_in_system_note(to_label);
    format!("[AgentMux] Your login expired, so AgentMux moved you to {to}. Your last step may not have finished: check git status and your PR first, then carry on.")
}

/// Start the turn of each of `block_ids` (panes just restarted on `to_label`) with [`resume_note`],
/// once its agent has registered.
pub(super) fn resume(block_ids: &[String], to_label: &str) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    for block_id in block_ids {
        let (block_id, text) = (block_id.clone(), resume_note(to_label));
        runtime.spawn(async move {
            if crate::backend::limit_wake::tell_when_registered(&block_id, &text).await {
                tracing::info!(block_id = %block_id, "login expiry: told the agent to carry on");
            } else {
                // Held for the agent's next turn: the move still gets said, though nothing started a turn.
                tracing::info!(block_id = %block_id, "login expiry: the agent didn't start, so the note is held for its next turn");
                crate::backend::blockcontroller::system_note_into_turn_or_hold(&block_id, "failover:resume", &text);
            }
        });
    }
}

/// The open agents on each account, by the links their spawn resolves through (their own, or the
/// seeded template's an agent inherits).
fn agents_by_account(state: &AppState) -> BTreeMap<String, BTreeSet<String>> {
    let defs: HashMap<String, crate::backend::storage::store::AgentDefinition> =
        state.mstore.agent_def_list().unwrap_or_default().into_iter().map(|d| (d.id.clone(), d)).collect();
    let open: BTreeSet<String> = super::tab_blocks(&state.mstore)
        .iter()
        .map(|b| crate::backend::obj::meta_get_string(&b.meta, "agentId", ""))
        .filter(|id| !id.is_empty())
        .collect();
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for def_id in open {
        let Some(def) = defs.get(&def_id) else { continue };
        let links = crate::identity::resolver::effective_identity_links(&state.mstore, &state.identity_store, def).unwrap_or_default();
        for link in links {
            out.entry(link.account_id).or_default().insert(def_id.clone());
        }
    }
    out
}

/// Move the agents of each lost login. Runs with every episode refresh; `panes` are its blocked panes.
/// BLOCKING.
pub(super) fn run(state: &AppState, panes: &[BlockedPane]) {
    if !failover::enabled(state) {
        return;
    }
    let accounts = state.id_store.identity_list(None).unwrap_or_default();
    forget_signed_in(state, &accounts);
    let refused: HashSet<&str> = panes.iter().filter(|p| p.state == AgentState::SignedOut).map(|p| p.account_id.as_str()).collect();
    let gone: Vec<&IdentityAccount> =
        accounts.iter().filter(|a| lost(a, &refused) && crate::backend::providers::get_provider(&canonical(&a.provider)).is_some()).collect();
    if gone.is_empty() {
        return;
    }
    let by_account = agents_by_account(state);
    let blocked = account_blocks(state, &accounts);
    let now_ms = agentmux_common::time::now_ms_u64();
    let left = |id: &str| failover::recently_left(id, now_ms);
    for from in gone {
        let provider = canonical(&from.provider);
        let same_provider: Vec<IdentityAccount> = accounts.iter().filter(|a| canonical(&a.provider) == provider).cloned().collect();
        let mut moved = 0usize;
        for def_id in by_account.get(&from.id).into_iter().flatten() {
            let open = open_panes(state, def_id);
            // Nothing runs, or a move is queued already.
            if open.is_empty() || open.iter().any(|b| b.meta.get(META_PENDING_ACCOUNT).is_some_and(|v| v.is_string())) {
                continue;
            }
            let Some(to) = choose(&accounts, &blocked, &left, from) else {
                tracing::debug!(def_id = %def_id, "login expiry: no valid login to move to");
                continue;
            };
            match super::switch::switch_to(state, ACTOR_EXPIRED, def_id, to, &provider, &same_provider, AccountSwitchWhen::AfterTurn) {
                Ok(r) if matches!(r.status, AccountSwitchStatus::Switched | AccountSwitchStatus::Queued) => moved += 1,
                Ok(_) => {}
                Err(e) => tracing::warn!(def_id = %def_id, error = %e, "login expiry: the move failed"),
            }
        }
        if moved > 0 {
            tell_person(state, from, moved);
        }
    }
}

/// The notice: once per login until it is signed in again, never naming it (a notification can show on
/// a locked screen); the strip row names it and has the sign-in action.
fn tell_person(state: &AppState, from: &IdentityAccount, moved: usize) {
    let first = TOLD.lock().unwrap_or_else(|e| e.into_inner()).insert(from.id.clone(), agentmux_common::time::now_ms_u64()).is_none();
    if !first {
        return;
    }
    tracing::info!(provider = %from.provider, moved, "login expiry: a login expired and its agents moved");
    super::publish(state, event_of(from, "signed_out"));
    if let Some(router) = crate::backend::notify::router::get(&state.broker) {
        let agents = if moved == 1 { "1 agent".to_string() } else { format!("{moved} agents") };
        router.account_unavailable(&notice_key(&from.id), format!("A login expired, so {agents} moved to another login. Sign in again to use it."));
    }
}

fn notice_key(account_id: &str) -> String {
    format!("login:{}", crate::backend::notify::policy::tag_for(account_id))
}

fn event_of(a: &IdentityAccount, state: &str) -> AccountAvailabilityEvent {
    AccountAvailabilityEvent { account_id: a.id.clone(), provider: canonical(&a.provider), state: state.to_string(), resets_at_ms: None, agents: Vec::new(), tasks: Vec::new() }
}

/// A login told about that is signed in again (or gone): its notice comes down and the strip row ends.
fn forget_signed_in(state: &AppState, accounts: &[IdentityAccount]) {
    let ended: Vec<String> = {
        let told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
        told.keys().filter(|id| !accounts.iter().any(|a| &a.id == *id && expired(&a.status))).cloned().collect()
    };
    for id in ended {
        TOLD.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(router) = crate::backend::notify::router::get(&state.broker) {
            router.resolve(&notice_key(&id), crate::backend::notify::policy::Family::Account);
        }
        if let Some(a) = accounts.iter().find(|a| a.id == id) {
            super::publish(state, event_of(a, "ok"));
        }
    }
}

/// The strip's rows for the logins told about that still need signing in: no agents (they moved), so
/// the row offers the sign-in, not a move. Not for a login an open episode shows already.
pub(super) fn rows(state: &AppState, shown: &[String]) -> Vec<AccountAvailability> {
    let told: Vec<(String, u64)> = TOLD.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(id, at)| (id.clone(), *at)).collect();
    let accounts = state.id_store.identity_list(None).unwrap_or_default();
    let mut out: Vec<AccountAvailability> = told
        .into_iter()
        .filter(|(id, _)| !shown.contains(id))
        .filter_map(|(id, since_ms)| {
            let a = accounts.iter().find(|a| a.id == id && expired(&a.status))?;
            let same_provider: Vec<IdentityAccount> = accounts.iter().filter(|o| canonical(&o.provider) == canonical(&a.provider)).cloned().collect();
            Some(AccountAvailability {
                account_id: a.id.clone(),
                account_label: account_label_in(a, &same_provider),
                provider: canonical(&a.provider),
                state: "signed_out".to_string(),
                resets_at_ms: None,
                limit_window: None,
                since_ms,
                agents: Vec::new(),
                tasks: Vec::new(),
                candidates: Vec::new(),
                agent_ids: Vec::new(),
                block_ids: Vec::new(),
            })
        })
        .collect();
    out.sort_by(|a, b| (a.since_ms, &a.account_id).cmp(&(b.since_ms, &b.account_id)));
    out
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `identity.accounts`: the signed-in accounts on this machine as an agent sees
//! them, and how `identity.switch` names one
//! (`docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md` §4.3, §4.4).
//!
//! The same view answers a local call and, later, a remote one, so it holds
//! only what a person reads off the Connectors page: the stable id, provider,
//! email, label, whether it is signed in, whether it has quota and when that
//! returns, and which agents run on it. Never a credential, a secret reference
//! or a config directory.

use std::collections::HashMap;

use serde::Serialize;

use crate::backend::agent_state::AgentState;
use crate::backend::storage::identities::IdentityAccount;
use crate::identity::stable_account::stable_account_id;
use crate::server::app_api::agent_spawn::{account_blocks, account_label_in};
use crate::server::AppState;

use super::moves::{can_receive, signed_in};
use super::{canonical, scan};

/// An agent that runs on an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountAgent {
    pub agent_id: String,
    pub name: String,
}

/// Whether an account can take work now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountQuota {
    #[ts(type = r#""ok" | "limited" | "signed_out""#)]
    pub state: String,
    /// When the last of its limits lifts (unix ms), when known.
    #[ts(type = "number | null")]
    pub resets_at_ms: Option<u64>,
    /// The provider's name for that limit's window (`five_hour`, `seven_day`, …).
    pub limit_window: Option<String>,
}

impl AccountQuota {
    fn of(state: &str) -> Self {
        AccountQuota { state: state.to_string(), resets_at_ms: None, limit_window: None }
    }
}

/// One account, as `identity.accounts` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountView {
    /// The id every machine agrees on (`identity::stable_account`); absent for an
    /// account with neither provider ids nor an email.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stable_id: Option<String>,
    /// This machine's id for it, which `identity.switch` also takes.
    pub account_id: String,
    pub provider: String,
    pub email: String,
    /// What the pane shows: the name or email, numbered (`email ·2`) when several share it.
    pub label: String,
    /// `valid`, `ok`, `unknown`, `expired`, `needs_reauth` or `invalid`.
    pub sign_in: String,
    pub quota: AccountQuota,
    pub agents: Vec<AccountAgent>,
}

/// `identity.accounts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountsResult {
    pub accounts: Vec<AccountView>,
}

/// The quota of each account that a pane's stored failure puts out of work now,
/// by local account id. An account missing from the map has quota as far as
/// this machine knows: it learns of a limit when a pane hits it.
fn quotas(state: &AppState, now_ms: u64) -> HashMap<String, AccountQuota> {
    let mut out: HashMap<String, AccountQuota> = HashMap::new();
    for pane in scan(state, now_ms) {
        let entry = out.entry(pane.account_id.clone()).or_insert_with(|| AccountQuota::of("limited"));
        if pane.state != AgentState::Limited {
            // Signed out wins over limited, as it does in an episode.
            *entry = AccountQuota::of("signed_out");
        } else if entry.state == "limited" && (entry.resets_at_ms.is_none() || pane.resets_at_ms > entry.resets_at_ms) {
            entry.resets_at_ms = pane.resets_at_ms;
            entry.limit_window = pane.limit_window.clone();
        }
    }
    out
}

fn email_of(a: &IdentityAccount) -> String {
    a.context.get("email").and_then(|e| e.as_str()).unwrap_or("").trim().to_string()
}

/// Every account of an agent provider on this machine. BLOCKING (store reads).
pub(super) fn accounts(state: &AppState, now_ms: u64) -> Result<IdentityAccountsResult, String> {
    let all: Vec<IdentityAccount> = state.id_store.identity_list(None).map_err(|e| format!("identity.accounts: {e}"))?;
    let quotas = quotas(state, now_ms);
    let names: HashMap<String, String> =
        state.mstore.agent_def_list().map_err(|e| format!("identity.accounts: {e}"))?.into_iter().map(|d| (d.id, d.name)).collect();
    let mut users: HashMap<String, Vec<AccountAgent>> = HashMap::new();
    for link in state.identity_store.agent_identity_list_all().map_err(|e| format!("identity.accounts: {e}"))? {
        let Some(name) = names.get(&link.agent_id) else { continue };
        let agents = users.entry(link.account_id).or_default();
        if !agents.iter().any(|a| a.agent_id == link.agent_id) {
            agents.push(AccountAgent { agent_id: link.agent_id, name: name.clone() });
        }
    }
    let mut out = Vec::new();
    for a in all.iter().filter(|a| crate::backend::providers::get_provider(&canonical(&a.provider)).is_some()) {
        let provider = canonical(&a.provider);
        let same_provider: Vec<IdentityAccount> = all.iter().filter(|o| canonical(&o.provider) == provider).cloned().collect();
        let mut agents = users.remove(&a.id).unwrap_or_default();
        agents.sort_by(|x, y| x.name.cmp(&y.name));
        let quota = match quotas.get(&a.id) {
            Some(q) => q.clone(),
            None if matches!(a.status.as_str(), "expired" | "needs_reauth" | "invalid") => AccountQuota::of("signed_out"),
            None => AccountQuota::of("ok"),
        };
        out.push(AccountView {
            stable_id: stable_account_id(a),
            account_id: a.id.clone(),
            provider,
            email: email_of(a),
            label: account_label_in(a, &same_provider),
            sign_in: a.status.clone(),
            quota,
            agents,
        });
    }
    out.sort_by(|x, y| (&x.provider, &x.label, &x.account_id).cmp(&(&y.provider, &y.label, &y.account_id)));
    Ok(IdentityAccountsResult { accounts: out })
}

/// Whether an account can take an agent now: signed in (or not checked yet) and
/// not blocked by a pane's limit or refused sign-in.
fn has_quota(a: &IdentityAccount, blocked: &HashMap<String, String>) -> bool {
    can_receive(&a.status) && !blocked.contains_key(&a.id)
}

/// `"any with quota"`, in the words an agent may type.
pub(super) fn is_any_with_quota(wanted: &str) -> bool {
    let w = wanted.trim().to_lowercase().replace(['_', '-'], " ");
    w == "any with quota" || w == "any"
}

/// Name one account among `accounts` (one provider's): by stable id, by local
/// id or label, by email when exactly one account of that email has quota, or
/// `"any with quota"`. `skip` holds the accounts an agent is already on, which
/// `any with quota` does not pick. The refusal lists what the caller could use.
pub(super) fn pick<'a>(
    state: &AppState,
    accounts: &'a [IdentityAccount],
    wanted: &str,
    skip: &[String],
    provider: &str,
) -> Result<&'a IdentityAccount, String> {
    let wanted = wanted.trim();
    let blocked = account_blocks(state, accounts);
    let list = |only_quota: bool| -> String {
        let shown: Vec<String> = accounts
            .iter()
            .filter(|a| !only_quota || has_quota(a, &blocked))
            .map(|a| match stable_account_id(a) {
                Some(id) => format!("{} ({id})", account_label_in(a, accounts)),
                None => account_label_in(a, accounts),
            })
            .collect();
        if shown.is_empty() {
            "none".to_string()
        } else {
            shown.join(", ")
        }
    };
    if is_any_with_quota(wanted) {
        // The signed-in ones before the unchecked, then the oldest: a stable choice.
        return accounts
            .iter()
            .filter(|a| has_quota(a, &blocked) && !skip.contains(&a.id))
            .min_by_key(|a| (!signed_in(&a.status), a.created_at, a.id.clone()))
            .ok_or_else(|| format!("NO_ACCOUNT_WITH_QUOTA: no {provider} account has quota now; the accounts are: {}", list(false)));
    }
    // Sign-ins of one provider account share the id and its quota: the one that can run wins.
    if let Some(a) = accounts
        .iter()
        .filter(|a| stable_account_id(a).is_some_and(|s| s.eq_ignore_ascii_case(wanted)))
        .min_by_key(|a| (!has_quota(a, &blocked), !signed_in(&a.status), a.created_at, a.id.clone()))
    {
        return Ok(a);
    }
    // A bare email names the one account of it that has quota, even when several sign-ins share it.
    let same_email: Vec<&IdentityAccount> = accounts.iter().filter(|a| !email_of(a).is_empty() && email_of(a).eq_ignore_ascii_case(wanted)).collect();
    if !same_email.is_empty() {
        let with_quota: Vec<&IdentityAccount> = same_email.iter().copied().filter(|a| has_quota(a, &blocked)).collect();
        // Already on one of them: that is the answer, and the switch reports it unchanged.
        if let Some(own) = with_quota.iter().copied().find(|a| skip.contains(&a.id)) {
            return Ok(own);
        }
        return match with_quota.as_slice() {
            [one] => Ok(one),
            [] => Err(format!("NO_ACCOUNT_WITH_QUOTA: no {provider} account of {wanted} has quota now; the accounts are: {}", list(false))),
            _ => Err(format!("AMBIGUOUS_ACCOUNT: {} {provider} accounts of {wanted} have quota; name one: {}", with_quota.len(), list(true))),
        };
    }
    if let Some(a) = accounts.iter().find(|a| crate::identity::account_numbering::names(a, wanted, accounts)) {
        return Ok(a);
    }
    Err(format!("UNKNOWN_ACCOUNT: no {provider} account {wanted:?}; the accounts are: {}", list(false)))
}

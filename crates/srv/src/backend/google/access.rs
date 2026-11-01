// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Which Google account an agent uses for a service, and at what level, or a
//! sentence saying why it can't and who can change that (D4).
//!
//! The accounts are those the agent holds a grant on for the service (it may hold
//! several, and names one by email when it does); the level is that grant, off
//! unless a person or an agent set it.

use std::sync::Arc;

use crate::backend::storage::google_grants::GrantLevel;
use crate::backend::storage::store::{AgentDefinition, IdentityAccount, Store};

use super::Service;

/// What an agent may use.
#[derive(Debug, Clone)]
pub struct Access {
    pub account: IdentityAccount,
    pub level: GrantLevel,
}

impl Access {
    pub fn email(&self) -> String {
        email_of(&self.account)
    }

    /// The scopes the account's token was granted.
    pub fn granted_scopes(&self) -> Vec<String> {
        self.account
            .context
            .get("granted_scopes")
            .and_then(serde_json::Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
}

/// The account's email, else its name.
pub fn email_of(account: &IdentityAccount) -> String {
    account
        .context
        .get("email")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| if account.display_name.is_empty() { account.name.clone() } else { account.display_name.clone() })
}

/// The Google account bound to `def`, if any.
pub fn bound_account(
    mstore: &Store,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    def: &AgentDefinition,
) -> Result<Option<IdentityAccount>, String> {
    let links = crate::identity::resolver::effective_identity_links(mstore, identity_store, def)
        .map_err(|e| format!("store: {e}"))?;
    let Some(link) = links.into_iter().find(|l| l.provider == "google") else {
        return Ok(None);
    };
    crate::identity::resolver::resolve_account(id_store, identity_store, &link.account_id)
        .map(|found| found.map(|(a, _)| a))
        .map_err(|e| format!("store: {e}"))
}

/// Every Google account `def` holds a grant on for `service`, by email. Several are allowed
/// (the spec's D7): a grant is all an agent needs, so it can read more than one mailbox.
pub fn granted_accounts(
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    def: &AgentDefinition,
    service: Service,
) -> Result<Vec<Access>, String> {
    let grants = identity_store.google_grants_for_agent(&def.id).map_err(|e| format!("store: {e}"))?;
    let mut held = Vec::new();
    for g in grants.into_iter().filter(|g| g.service == service.as_str()) {
        if let Ok(Some((account, _))) = crate::identity::resolver::resolve_account(id_store, identity_store, &g.account_id) {
            if account.provider == "google" {
                held.push(Access { account, level: g.level });
            }
        }
    }
    held.sort_by_key(|a| a.email().to_ascii_lowercase());
    Ok(held)
}

/// The account and level `def` may use `service` with. `account` is the
/// email the agent named, if it named one; without one, the single account it holds
/// the service on.
pub fn access_for(
    _mstore: &Store,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    def: &AgentDefinition,
    service: Service,
    account: Option<&str>,
) -> Result<Access, String> {
    let name = service_name(service);
    let mut held = granted_accounts(id_store, identity_store, def, service)?;
    let list = || held.iter().map(Access::email).collect::<Vec<_>>().join(", ");
    if let Some(asked) = account.map(str::trim).filter(|s| !s.is_empty()) {
        return match held.iter().find(|a| a.email().eq_ignore_ascii_case(asked)) {
            Some(found) => Ok(found.clone()),
            None if held.is_empty() => Err(format!(
                "{asked}'s {name} is off for you. Google grant turns it on for you (Google status lists the connected accounts)"
            )),
            None => Err(format!("you don't hold {name} on {asked}. You hold it on: {}", list())),
        };
    }
    match held.len() {
        0 => Err(format!(
            "no Google account has {name} turned on for you. Google status lists the connected accounts; Google grant turns {name} on for you"
        )),
        1 => Ok(held.remove(0)),
        _ => Err(format!("you hold {name} on several Google accounts: {}. Name one with account", list())),
    }
}

fn service_name(service: Service) -> &'static str {
    match service {
        Service::Calendar => "Calendar",
        Service::Drive => "Drive",
        Service::Gmail => "Gmail",
    }
}

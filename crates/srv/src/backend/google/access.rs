// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Which Google account an agent uses for a service, and at what level, or a
//! sentence saying why it can't and who can change that (D4).
//!
//! The account is the one bound to the agent ("Bind to agent", one Google
//! account per agent); the level is the agent's grant on that account's
//! service, off unless the person set it.

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

/// The account and level `def` may use `service` with. `account` is the
/// email the agent named, if it named one.
pub fn access_for(
    mstore: &Store,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    def: &AgentDefinition,
    service: Service,
    account: Option<&str>,
) -> Result<Access, String> {
    let name = service_name(service);
    let Some(acct) = bound_account(mstore, id_store, identity_store, def)? else {
        return Err(format!(
            "no Google account is bound to you. The person can bind one in Connectors > Accounts (Bind to agent) and turn {name} on for you there"
        ));
    };
    let email = email_of(&acct);
    if let Some(asked) = account.map(str::trim).filter(|s| !s.is_empty()) {
        if !asked.eq_ignore_ascii_case(&email) {
            return Err(format!("you are bound to the Google account {email}, not {asked}"));
        }
    }
    match identity_store.google_grant_get(&def.id, &acct.id, service.as_str()).map_err(|e| format!("store: {e}"))? {
        Some(level) => Ok(Access { account: acct, level }),
        None => Err(format!(
            "{email}'s {name} is off for you. The person can turn it on for you in Connectors > Accounts > {email} > {name}"
        )),
    }
}

fn service_name(service: Service) -> &'static str {
    match service {
        Service::Calendar => "Calendar",
        Service::Drive => "Drive",
        Service::Gmail => "Gmail",
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! One agent tool call to Google, end to end: whose account and grant
//! ([`super::access`]), whether the token covers the action, a fresh token
//! (srv refreshes it; the agent never sees it), the request, the trimmed
//! answer, and a line in the account's log (D7) however it ends.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::backend::storage::google_grants::{GoogleCall, GrantLevel};
use crate::backend::storage::store::{AgentDefinition, Store};
use crate::identity::oauth_tokens::{access_token_for, OAuthHttp, TokenSecrets};

use super::access::{access_for, Access};
use super::calendar::{self, CalendarArgs};
use super::{google_error, GoogleHttp, Service};

/// What a call needs from srv.
pub struct CallDeps<'a, G: GoogleHttp, O: OAuthHttp, S: TokenSecrets> {
    pub google: &'a G,
    pub oauth: &'a O,
    pub secrets: &'a S,
    pub mstore: &'a Store,
    pub id_store: &'a Arc<Store>,
    pub identity_store: &'a Arc<Store>,
}

/// A `Calendar` read action for agent `def`. The answer names the account
/// and carries the trimmed result.
pub async fn calendar_read<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    def: &AgentDefinition,
    action: &str,
    args: &CalendarArgs,
    account: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    if !calendar::READ_ACTIONS.contains(&action) {
        return Err(format!("Calendar has no action \"{action}\"; one of {}", calendar::READ_ACTIONS.join(", ")));
    }
    let access = access_for(deps.mstore, deps.id_store, deps.identity_store, def, Service::Calendar, account)?;
    let ids = calendar::logged_ids(action, args);
    let result = calendar_read_with(deps, &access, action, args, now).await;
    let outcome = match &result {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("error: {}", super::clip(e, 200)),
    };
    let line = GoogleCall {
        at: now.timestamp_millis(),
        agent_id: def.id.clone(),
        agent_name: def.name.clone(),
        service: Service::Calendar.as_str().to_string(),
        action: action.to_string(),
        ids,
        outcome,
    };
    if let Err(e) = deps.identity_store.google_call_log(&access.account.id, &line) {
        tracing::warn!(target: "google", account_id = %access.account.id, error = %e, "google: couldn't write the call log");
    }
    result.map(|r| json!({ "account": access.email(), "result": r }))
}

async fn calendar_read_with<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    access: &Access,
    action: &str,
    args: &CalendarArgs,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    debug_assert!(access.level >= GrantLevel::Read);
    if !calendar::granted_for(action, &access.granted_scopes()) {
        return Err(format!(
            "{} hasn't granted the Calendar permission {action} needs (it was only partly granted). The person can turn Calendar on again for the account in Connectors",
            access.email()
        ));
    }
    let req = calendar::plan(action, args, now)?;
    let token = access_token_for(deps.oauth, deps.secrets, deps.id_store, deps.identity_store, &access.account, None).await?;
    let reply = deps.google.send(&req, &token).await?;
    if !(200..300).contains(&reply.status) {
        return Err(google_error(&reply));
    }
    Ok(calendar::shape(action, &reply.body))
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;

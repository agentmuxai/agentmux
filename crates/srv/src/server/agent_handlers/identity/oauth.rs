// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The service-OAuth RPCs (`account.oauth.start`, `.poll`, `.cancel`, `.client.get`, `.client.clear`),
//! moved out of `identity::register` unchanged as named functions.

use std::sync::Arc;

use super::{
    builtin_client_of, byo_from_req, oauth_status_wire, panel_start_options, reads_saved_client, AccountOAuthClientClearResult, AccountOAuthClientInfo,
    AccountOAuthStartResult, OAuthClientReq, OAuthSessionReq, OAuthStartReq,
};
use crate::backend::rpc_types::AccountOAuthCancelResult;
use crate::backend::storage::store::Store;
use crate::identity::oauth_client::FlowTarget;

/// Resolve config and client (gating on "not configured"), spawn the flow, return the session id and its
/// first status. Poll and cancel drive the rest. With `accountId` it authorizes that account again (more
/// scopes, or a reconnect) instead of creating one: spec G0 of
/// docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md.
pub(super) async fn start_oauth(
    mstore: Arc<Store>,
    identity_store: Arc<Store>,
    data: serde_json::Value,
) -> Result<AccountOAuthStartResult, String> {
    // NB: `req.client_secret` is a secret, so never log `req`.
    let req: OAuthStartReq = serde_json::from_value(data).map_err(|e| format!("account.oauth.start: {e}"))?;
    let byo = match byo_from_req(&req) {
        Ok(b) => b,
        Err(e) => return Ok(start_failed(e)),
    };
    let target = match flow_target(&mstore, &req) {
        Ok(t) => t,
        Err(e) => return Ok(start_failed(e)),
    };
    let saved = if reads_saved_client(&req, &byo) { saved_client(&req.provider).await } else { None };
    let cloud_signed_in = cloud_signed_in(&mstore, &req.provider).await;
    let opts = panel_start_options(&req, target, byo, saved, cloud_signed_in);
    match crate::identity::oauth_client::start(&req.provider, opts, mstore, identity_store) {
        Ok((session_id, status)) => {
            Ok(AccountOAuthStartResult { session_id: Some(session_id), status: Some(oauth_status_wire(&status)), error: None })
        }
        // "not configured" / unknown provider surface as a clean error field, not an RPC failure, so the UI can show it.
        Err(e) => Ok(start_failed(e)),
    }
}

fn start_failed(error: String) -> AccountOAuthStartResult {
    AccountOAuthStartResult { session_id: None, status: None, error: Some(error) }
}

/// A new account, or the existing one the request names.
fn flow_target(mstore: &Store, req: &OAuthStartReq) -> Result<FlowTarget, String> {
    match req.account_id.as_deref().filter(|s| !s.is_empty()) {
        None => Ok(FlowTarget::New { name: req.name.clone() }),
        Some(id) => match mstore.identity_get(id) {
            Ok(Some(acct)) => Ok(FlowTarget::Existing(acct)),
            Ok(None) => Err(format!("account {id} not found")),
            Err(e) => Err(format!("account {id} could not be read: {e}")),
        },
    }
}

/// The user's saved client, when none came with the request. A keychain that can't be read means none: the
/// panel asks again.
async fn saved_client(provider: &str) -> Option<crate::identity::oauth_tokens::SavedClient> {
    let owned = provider.to_string();
    let loaded = tokio::task::spawn_blocking(move || crate::identity::oauth_tokens::load_saved_client(&owned))
        .await
        .map_err(|e| format!("keychain task: {e}"))
        .and_then(|r| r);
    match loaded {
        Ok(saved) => saved,
        Err(e) => {
            tracing::warn!(target: "identity", provider = %provider, error = %e, "account.oauth.start: the saved OAuth client could not be read");
            None
        }
    }
}

/// GitHub connects through the broker when signed in to AgentMux Cloud.
async fn cloud_signed_in(mstore: &Arc<Store>, provider: &str) -> bool {
    if provider != crate::identity::github_connect::PROVIDER {
        return false;
    }
    let store = mstore.clone();
    tokio::task::spawn_blocking(move || store.muxbus_load().ok().flatten().is_some()).await.unwrap_or(false)
}

/// The user's own OAuth client saved for a provider (D8): whether there is one. Accounts made with it keep
/// refreshing: each keeps its own copy of what a refresh needs.
pub(super) async fn oauth_client_info(req: OAuthClientReq) -> Result<AccountOAuthClientInfo, String> {
    let (builtin_available, offered_services) = builtin_client_of(&req.provider)?;
    let provider = req.provider.clone();
    let saved = tokio::task::spawn_blocking(move || crate::identity::oauth_tokens::load_saved_client(&provider))
        .await
        .map_err(|e| format!("account.oauth.client.get: keychain task: {e}"))?
        .map_err(|e| format!("account.oauth.client.get: {e}"))?;
    Ok(AccountOAuthClientInfo {
        saved: saved.is_some(),
        client_id: saved.as_ref().map(|c| c.client_id.clone()),
        has_secret: saved.as_ref().is_some_and(|c| c.client_secret.is_some()),
        mode: saved.as_ref().map(|c| c.mode.as_str().to_string()),
        builtin_available,
        offered_services,
    })
}

/// Forgets the user's saved OAuth client for a provider.
pub(super) async fn clear_oauth_client(req: OAuthClientReq) -> Result<AccountOAuthClientClearResult, String> {
    if crate::identity::oauth_client::config_for(&req.provider).is_none() {
        return Err(format!("account.oauth.client.clear: unknown OAuth provider: {}", req.provider));
    }
    let provider = req.provider.clone();
    let cleared = tokio::task::spawn_blocking(move || crate::identity::oauth_tokens::clear_saved_client(&provider))
        .await
        .map_err(|e| format!("account.oauth.client.clear: keychain task: {e}"))?
        .map_err(|e| format!("account.oauth.client.clear: {e}"))?;
    tracing::info!(target: "identity", provider = %req.provider, cleared, "account.oauth.client.clear: saved OAuth client forgotten");
    Ok(AccountOAuthClientClearResult { cleared })
}

/// The response stays `Value`: `oauth_status_wire` builds it, and the frontend models it as `OAuthFlowStatus`
/// (generated-manifest.json `returns`).
pub(super) fn poll_oauth(req: &OAuthSessionReq) -> Result<serde_json::Value, String> {
    match crate::identity::oauth_client::manager().poll(&req.session_id) {
        Some(s) => Ok(oauth_status_wire(&s)),
        None => Err(format!("account.oauth.poll: unknown session {}", req.session_id)),
    }
}

pub(super) fn cancel_oauth(req: &OAuthSessionReq) -> AccountOAuthCancelResult {
    AccountOAuthCancelResult { cancelled: crate::identity::oauth_client::manager().cancel(&req.session_id) }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn req(extra: serde_json::Value) -> OAuthStartReq {
        let mut base = json!({ "provider": "google", "name": "Mail" });
        base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn a_start_without_an_account_makes_a_new_one_named_as_asked() {
        let store = Store::open_in_memory().unwrap();
        for empty in [json!({}), json!({ "accountId": "" })] {
            match flow_target(&store, &req(empty)).unwrap() {
                FlowTarget::New { name } => assert_eq!(name, "Mail"),
                FlowTarget::Existing(_) => panic!("an empty account id means a new account"),
            }
        }
    }

    #[test]
    fn a_start_for_an_account_the_store_does_not_hold_says_which() {
        let store = Store::open_in_memory().unwrap();
        let err = flow_target(&store, &req(json!({ "accountId": "ghost" }))).err().unwrap();
        assert_eq!(err, "account ghost not found");
    }

    #[test]
    fn a_start_that_fails_answers_with_only_the_error() {
        let failed = start_failed("not configured".to_string());
        assert_eq!((failed.session_id, failed.status, failed.error.as_deref()), (None, None, Some("not configured")));
    }
}

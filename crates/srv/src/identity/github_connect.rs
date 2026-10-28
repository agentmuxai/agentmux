// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Connect with GitHub, desktop side.
//!
//! The person's grant to the AgentMux App is made, refreshed and ended through the
//! broker (`muxbus::github_broker`), which holds the App's client secret. This module
//! is what the service-OAuth flow (`oauth_client`) and the token path (`oauth_tokens`)
//! lean on for a GitHub account:
//!
//! - a **brokered** account is one made through the broker; its context says so
//!   (`broker: true`) and carries the GitHub user's numeric id and login;
//! - signed out of AgentMux Cloud there is no broker, so a connect falls back to the
//!   device flow with the App's client id, and the account is not brokered: GitHub
//!   won't refresh its token without the secret, so it asks to be connected again
//!   when the 8 hours are up.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::storage::store::{IdentityAccount, Store};
use crate::identity::oauth_tokens::{RefreshError, TokenBlob, TokenSecrets};
use crate::muxbus::github_broker::{self, BrokerError, TokenGrant};

/// The provider whose grant is brokered.
pub const PROVIDER: &str = "github";

/// The AgentMux App's public client id, for a build that doesn't ship one yet (and
/// for a developer's own App). A shipped id replaces this when the App is registered.
pub fn client_id_override() -> Option<String> {
    std::env::var("AGENTMUX_GITHUB_APP_CLIENT_ID").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Whether `account` was connected through the broker.
pub fn is_brokered(account: &IdentityAccount) -> bool {
    account.provider == PROVIDER && account.context.get("broker").and_then(Value::as_bool) == Some(true)
}

/// A grant as a GitHub-shaped token response, so the flow's last step stores it
/// like any other: the expiry in seconds from now, the numeric id and login, and the
/// marker that the account is brokered. A grant already expired (or about to) is one second
/// from now, never 0: the blob reads 0 as "doesn't expire", so it would never be refreshed.
pub fn grant_json(grant: &TokenGrant, now_ms: i64) -> Value {
    json!({
        "access_token": grant.access_token,
        "refresh_token": grant.refresh_token,
        "expires_in": ((grant.expires_at_ms - now_ms) / 1000).max(1),
        "github_user_id": grant.github_user_id,
        "login": grant.login,
        "broker": true,
    })
}

/// The id and login in a token response made by [`grant_json`].
pub fn identity_in(json: &Value) -> Option<(u64, String)> {
    let id = json.get("github_user_id").and_then(Value::as_u64)?;
    let login = json.get("login").and_then(Value::as_str).filter(|l| !l.is_empty())?;
    Some((id, login.to_string()))
}

/// What the account's context records about who is connected, and how.
pub fn stamp_context(context: &mut Value, json: &Value) {
    if let Some((id, login)) = identity_in(json) {
        context["github_user_id"] = json!(id);
        context["github_login"] = json!(login);
    } else if let Some(c) = context.as_object_mut() {
        // A grant that couldn't say who it is: the previous user's id and login would be wrong
        // for it, so the account shows nobody rather than someone it isn't.
        c.remove("github_user_id");
        c.remove("github_login");
    }
    if json.get("broker").and_then(Value::as_bool) == Some(true) {
        context["broker"] = json!(true);
    } else if let Some(c) = context.as_object_mut() {
        // Connected again without the broker: no longer refreshed through it.
        c.remove("broker");
    }
}

/// The label an account shows for who is connected.
pub fn display_name_for(json: &Value) -> Option<String> {
    identity_in(json).map(|(_, login)| format!("@{login}"))
}

/// Who a GitHub token belongs to (`GET /user`), for a grant the device flow made.
pub async fn user_of_token(token: &str) -> Result<(u64, String), String> {
    let reply = crate::identity::oauth_client::http()
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("couldn't ask GitHub who this is: {}", e.without_url()))?;
    if !reply.status().is_success() {
        return Err(format!("GitHub answered {} when asked who this is", reply.status().as_u16()));
    }
    let body: Value = reply.json().await.map_err(|_| "GitHub's answer about who this is wasn't readable".to_string())?;
    let id = body.get("id").and_then(Value::as_u64).ok_or("GitHub's answer had no user id")?;
    let login = body.get("login").and_then(Value::as_str).ok_or("GitHub's answer had no login")?;
    Ok((id, login.to_string()))
}

/// What the desktop does about a broker failure during a refresh: the grant is gone
/// (connect again) or it was only a bad moment (try later).
pub fn refresh_error_for(e: BrokerError) -> RefreshError {
    match e {
        BrokerError::Reauthorize | BrokerError::Blocked | BrokerError::NotConnected | BrokerError::NotLive(_) => {
            RefreshError::Reauth(e.to_string())
        }
        other => RefreshError::Failed(other.to_string()),
    }
}

/// A new blob for a brokered account's refresh token. GitHub's refresh tokens are
/// single use, so the caller must store what this returns before anything else.
pub async fn refresh_blob(mstore: &Arc<Store>, blob: &TokenBlob, now: i64) -> Result<TokenBlob, RefreshError> {
    let Some(refresh) = blob.refresh_token.as_deref().filter(|r| !r.is_empty()) else {
        return Err(RefreshError::Reauth("the access token expired and there is no refresh token".to_string()));
    };
    let who = github_broker::caller(mstore).await.map_err(refresh_error_for)?;
    let grant = github_broker::refresh(crate::identity::oauth_client::http(), &who, refresh).await.map_err(refresh_error_for)?;
    if !grant.identity_checked {
        tracing::info!(target: "identity", "identity.github: refreshed; GitHub couldn't be asked who this is, so the recorded user stands");
    }
    TokenBlob::from_token_response(&grant_json(&grant, now * 1000), now, Some(refresh), None).map_err(RefreshError::Failed)
}

/// Before a brokered account is deleted: end its grant through the broker, so it
/// doesn't live on at GitHub. `{ outcome: "revoked" | "failed", detail? }`
/// for the delete's result, or `None` when this isn't a brokered account.
pub async fn revoke_before_delete<S: TokenSecrets>(mstore: &Arc<Store>, secrets: &S, account: &IdentityAccount) -> Option<Value> {
    if !is_brokered(account) {
        return None;
    }
    let raw = match secrets.get(&account.id).await {
        Ok(Some(raw)) => raw,
        Ok(None) => return None,
        Err(e) => return Some(json!({ "outcome": "failed", "detail": format!("couldn't read the stored token: {e}") })),
    };
    let blob = TokenBlob::parse(&raw)?;
    let who = match github_broker::caller(mstore).await {
        Ok(who) => who,
        Err(e) => return Some(json!({ "outcome": "failed", "detail": e.to_string() })),
    };
    match github_broker::revoke(crate::identity::oauth_client::http(), &who, Some(&blob.access_token), blob.refresh_token.as_deref()).await {
        Ok(done) => {
            tracing::info!(target: "identity", account_id = %account.id, revoked = done.revoked, disconnected = done.disconnected, "identity.delete: GitHub grant ended through the broker");
            Some(json!({ "outcome": "revoked" }))
        }
        // The broker has no grant for this account: nothing is left to end.
        Err(BrokerError::NotConnected) => Some(json!({ "outcome": "revoked" })),
        Err(e) => Some(json!({ "outcome": "failed", "detail": e.to_string() })),
    }
}

/// What the broker says about this AgentMux account's GitHub connection (`github.connection`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct GithubConnection {
    /// Signed in to AgentMux Cloud. Without it there is nothing to ask.
    pub signed_in: bool,
    /// Why the broker couldn't be asked, when it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The broker was asked and answered with a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub connected: bool,
    /// `none`, `active`, `revoked` or `blocked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// GitHub's numeric user id, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

/// The broker's word on this account's GitHub connection, for Connectors.
pub async fn connection_of(mstore: &Arc<Store>) -> GithubConnection {
    let who = match github_broker::caller(mstore).await {
        Ok(who) => who,
        Err(e) => return GithubConnection { signed_in: false, reason: Some(e.to_string()), ..Default::default() },
    };
    match github_broker::connection(crate::identity::oauth_client::http(), &who).await {
        Ok(c) => GithubConnection {
            signed_in: true,
            connected: c.connected,
            status: Some(c.status),
            github_user_id: c.github_user_id.map(|id| id.to_string()),
            login: c.login,
            ..Default::default()
        },
        Err(e) => GithubConnection { signed_in: true, error: Some(e.to_string()), ..Default::default() },
    }
}

/// Whether a connection answer says the grant is gone: revoked on GitHub, blocked, or never made
/// (or ended from another install). Only a clear answer counts; a failure to ask says nothing.
pub fn grant_is_gone(c: &GithubConnection) -> bool {
    c.signed_in && c.error.is_none() && matches!(c.status.as_deref(), Some("revoked" | "blocked" | "none"))
}

/// [`connection_of`], and the brokered accounts whose grant is gone ask to be connected again
/// (the account's card shows "Reconnect needed"): a grant revoked on GitHub shows as revoked.
/// Nothing is cleared here: an account that works again is made valid by the
/// refresh that proves it.
pub async fn sync_connection(mstore: &Arc<Store>, identity_store: &Arc<Store>) -> GithubConnection {
    let answer = connection_of(mstore).await;
    if grant_is_gone(&answer) {
        for account in mstore.identity_list(Some(PROVIDER)).unwrap_or_default().into_iter().filter(is_brokered) {
            if account.status != crate::identity::oauth_tokens::STATUS_NEEDS_REAUTH {
                tracing::info!(target: "identity", account_id = %account.id, status = ?answer.status, "identity.github: the grant is gone, the account asks to be connected again");
                crate::identity::oauth_tokens::set_status(mstore, identity_store, &account.id, crate::identity::oauth_tokens::STATUS_NEEDS_REAUTH);
            }
        }
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> TokenGrant {
        serde_json::from_value(json!({
            "access_token": "ghu_A", "refresh_token": "ghr_B",
            "expires_at_ms": 1_000_000 + 8 * 3600 * 1000, "refresh_expires_at_ms": 1_000_000 + 180 * 86400 * 1000_i64,
            "github_user_id": 42, "login": "octo",
        }))
        .unwrap()
    }

    #[test]
    fn a_grant_becomes_a_token_response_the_flow_can_store() {
        let j = grant_json(&grant(), 1_000_000);
        assert_eq!(j["expires_in"], 8 * 3600);
        let blob = TokenBlob::from_token_response(&j, 1_000, None, None).unwrap();
        assert_eq!((blob.access_token.as_str(), blob.refresh_token.as_deref()), ("ghu_A", Some("ghr_B")));
        assert_eq!(blob.expires_at, 1_000 + 8 * 3600);
        assert_eq!(identity_in(&j), Some((42, "octo".to_string())));
        assert_eq!(display_name_for(&j).as_deref(), Some("@octo"));
    }

    #[test]
    fn a_grant_already_expired_asks_for_a_refresh_at_once() {
        let j = grant_json(&grant(), 1_000_000 + 9 * 3600 * 1000);
        // Not 0: the blob reads that as "never expires" and would never refresh it.
        let blob = TokenBlob::from_token_response(&j, 5_000, None, None).unwrap();
        assert!(blob.expires_at > 0 && blob.needs_refresh(5_000), "{blob:?}");
    }

    #[test]
    fn a_grant_that_cannot_say_who_it_is_does_not_keep_the_previous_user() {
        let mut c = json!({ "oauth": true, "github_user_id": 7, "github_login": "old" });
        stamp_context(&mut c, &json!({ "access_token": "x" }));
        assert!(c.get("github_user_id").is_none() && c.get("github_login").is_none(), "{c}");
        assert_eq!(c["oauth"], true);
    }

    #[test]
    fn the_context_records_who_is_connected_and_whether_through_the_broker() {
        let mut c = json!({ "oauth": true });
        stamp_context(&mut c, &grant_json(&grant(), 0));
        assert_eq!((c["github_user_id"].clone(), c["github_login"].clone(), c["broker"].clone()), (json!(42), json!("octo"), json!(true)));
        // The same account connected again by the device flow: still who it is, no longer brokered.
        let device = json!({ "access_token": "x", "github_user_id": 42, "login": "octo" });
        stamp_context(&mut c, &device);
        assert!(c.get("broker").is_none());
        assert_eq!(c["github_login"], "octo");
    }

    #[test]
    fn only_a_clear_answer_says_the_grant_is_gone() {
        let c = |signed_in, error: Option<&str>, status: Option<&str>| GithubConnection {
            signed_in,
            error: error.map(String::from),
            status: status.map(String::from),
            ..Default::default()
        };
        for gone in ["revoked", "blocked", "none"] {
            assert!(grant_is_gone(&c(true, None, Some(gone))), "{gone}");
        }
        assert!(!grant_is_gone(&c(true, None, Some("active"))));
        assert!(!grant_is_gone(&c(true, Some("unavailable"), None)), "a failure to ask says nothing");
        assert!(!grant_is_gone(&c(false, None, Some("revoked"))), "signed out: nothing was asked");
    }

    #[test]
    fn only_a_brokered_github_account_is_brokered() {
        let mut a = crate::identity::oauth_tokens::tests::oauth_account("a", "github", "cid");
        assert!(!is_brokered(&a));
        a.context["broker"] = json!(true);
        assert!(is_brokered(&a));
        a.provider = "google".to_string();
        assert!(!is_brokered(&a));
    }

    #[test]
    fn a_dropped_grant_asks_to_connect_again_and_a_bad_moment_does_not() {
        for e in [BrokerError::Reauthorize, BrokerError::Blocked, BrokerError::NotConnected, BrokerError::NotLive("revoked".into())] {
            assert!(matches!(refresh_error_for(e), RefreshError::Reauth(_)));
        }
        for e in [BrokerError::Unavailable("503".into()), BrokerError::NotSignedIn("no".into()), BrokerError::Other { status: 500, error: "x".into() }] {
            assert!(matches!(refresh_error_for(e), RefreshError::Failed(_)));
        }
    }
}

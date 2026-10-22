// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for `oauth_tokens`. Nothing here reaches a provider: [`StubHttp`]
//! answers every call and [`MemSecrets`] stands in for the keychain. Both are
//! shared with `oauth_client`'s tests.

use super::*;
use std::sync::Mutex as StdMutex;

/// An in-memory [`TokenSecrets`].
#[derive(Default)]
pub(crate) struct MemSecrets {
    pub map: StdMutex<HashMap<String, String>>,
}

impl MemSecrets {
    pub fn with(account_id: &str, value: &str) -> Self {
        let s = MemSecrets::default();
        s.map.lock().unwrap().insert(account_id.to_string(), value.to_string());
        s
    }
    pub fn value(&self, account_id: &str) -> Option<String> {
        self.map.lock().unwrap().get(account_id).cloned()
    }
}

impl TokenSecrets for MemSecrets {
    async fn get(&self, account_id: &str) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self.value(account_id).map(Zeroizing::new))
    }
    async fn put(&self, account_id: &str, value: &str) -> Result<(), String> {
        self.map.lock().unwrap().insert(account_id.to_string(), value.to_string());
        Ok(())
    }
}

/// One call [`StubHttp`] saw.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub url: String,
    pub params: Vec<(String, String)>,
}

impl Call {
    pub fn param(&self, k: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

/// An [`OAuthHttp`] that answers from a queue and records each call.
#[derive(Default)]
pub(crate) struct StubHttp {
    pub replies: StdMutex<Vec<Result<HttpReply, String>>>,
    pub calls: StdMutex<Vec<Call>>,
}

impl StubHttp {
    pub fn answering(replies: Vec<Result<HttpReply, String>>) -> Self {
        StubHttp { replies: StdMutex::new(replies), calls: Default::default() }
    }
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn next(&self, call: Call) -> Result<HttpReply, String> {
        self.calls.lock().unwrap().push(call);
        let mut r = self.replies.lock().unwrap();
        assert!(!r.is_empty(), "an unexpected HTTP call");
        r.remove(0)
    }
}

impl OAuthHttp for StubHttp {
    async fn post_form(&self, url: &str, params: &[(&str, String)]) -> Result<HttpReply, String> {
        let params = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        self.next(Call { url: url.to_string(), params })
    }
    async fn get_bearer(&self, url: &str, _token: &str) -> Result<HttpReply, String> {
        self.next(Call { url: url.to_string(), params: vec![] })
    }
}

pub(crate) fn reply(status: u16, body: serde_json::Value) -> Result<HttpReply, String> {
    Ok(HttpReply { status, body })
}

/// An unsigned JWT with `claims` as its payload.
pub(crate) fn jwt(claims: serde_json::Value) -> String {
    let enc = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).unwrap());
    format!("{}.{}.sig", enc(&serde_json::json!({ "alg": "RS256" })), enc(&claims))
}

pub(crate) fn oauth_account(id: &str, provider: &str, client_id: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: format!("{provider}-test"),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::Keychain {
            service: crate::identity::secret_store::SERVICE.to_string(),
            account: crate::identity::secret_store::account_key(id),
        },
        context: serde_json::json!({ "oauth": true, "oauth_client": { "mode": "byo", "client_id": client_id } }),
        status: "valid".to_string(),
        created_at: 1,
        updated_at: 1,
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn blob(access: &str, refresh: Option<&str>, expires_at: i64) -> String {
    serde_json::json!({ "access_token": access, "refresh_token": refresh, "expires_at": expires_at }).to_string()
}

const GOOGLE: &[&str] = &["https://accounts.google.com", "accounts.google.com"];

// ── The blob ────────────────────────────────────────────────────────────────

#[test]
fn a_plain_secret_is_not_a_blob_and_is_used_as_it_is() {
    assert!(TokenBlob::parse("ghp_abc123").is_none());
    assert!(TokenBlob::parse(r#"{"other":"x"}"#).is_none());
    assert!(TokenBlob::parse(r#"{"access_token":""}"#).is_none());
    assert_eq!(bearer_value("ghp_abc123".to_string()), "ghp_abc123");
    assert_eq!(bearer_value("sk-ant-xyz".to_string()), "sk-ant-xyz");
}

#[test]
fn an_oauth_github_blob_yields_its_access_token() {
    // What an OAuth-connected GitHub account keeps in the keychain (spec §2.3).
    let stored = r#"{"access_token":"gho_live","refresh_token":null,"expires_at":0}"#;
    assert_eq!(bearer_value(stored.to_string()), "gho_live");
    assert_eq!(bearer_value(format!("  {stored}")), "gho_live");
}

#[test]
fn a_token_response_becomes_a_blob_and_keeps_the_old_refresh_token() {
    let t = TokenBlob::from_token_response(
        &serde_json::json!({ "access_token": "a1", "expires_in": 3599 }),
        1000,
        Some("r-old"),
        Some("cs"),
    )
    .unwrap();
    assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref(), t.expires_at), ("a1", Some("r-old"), 4599));
    assert_eq!(t.client_secret.as_deref(), Some("cs"));
    let t = TokenBlob::from_token_response(
        &serde_json::json!({ "access_token": "a2", "refresh_token": "r-new" }),
        1000,
        Some("r-old"),
        None,
    )
    .unwrap();
    assert_eq!((t.refresh_token.as_deref(), t.expires_at), (Some("r-new"), 0));
    assert!(TokenBlob::from_token_response(&serde_json::json!({ "error": "x" }), 0, None, None).is_err());
    // Round trip; the secret field is left out when there is none.
    let json = t.to_json();
    assert!(!json.contains("client_secret"));
    assert_eq!(TokenBlob::parse(&json).unwrap().refresh_token.as_deref(), Some("r-new"));
}

#[test]
fn a_token_is_refreshed_within_a_minute_of_expiry() {
    let at = |expires_at| TokenBlob { access_token: "a".into(), refresh_token: None, expires_at, client_secret: None };
    assert!(!at(0).needs_refresh(1_000_000), "no lifetime: never refreshed");
    assert!(!at(1000 + 61).needs_refresh(1000));
    assert!(at(1000 + 60).needs_refresh(1000));
    assert!(at(900).needs_refresh(1000));
}

#[test]
fn debug_output_never_shows_a_secret() {
    let t = TokenBlob {
        access_token: "AT-SECRET".into(),
        refresh_token: Some("RT-SECRET".into()),
        expires_at: 5,
        client_secret: Some("CS-SECRET".into()),
    };
    let s = SavedClient { client_id: "cid".into(), client_secret: Some("CS-SECRET".into()), mode: ClientMode::Byo };
    let out = format!("{t:?} {s:?}");
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.contains("cid"));
}

// ── Scopes ──────────────────────────────────────────────────────────────────

#[test]
fn granted_scopes_are_read_back_from_the_token_response() {
    let req: Vec<String> = ["openid", "email"].map(String::from).to_vec();
    let google = serde_json::json!({ "scope": "openid https://www.googleapis.com/auth/userinfo.email openid" });
    assert_eq!(granted_scopes(&google, &req), ["openid", "https://www.googleapis.com/auth/userinfo.email"]);
    // GitHub separates with commas.
    assert_eq!(granted_scopes(&serde_json::json!({ "scope": "repo,read:org" }), &req), ["repo", "read:org"]);
    // No `scope`: exactly what was asked (RFC 6749 §5.1).
    assert_eq!(granted_scopes(&serde_json::json!({}), &req), req);
}

#[test]
fn a_partial_grant_names_what_was_unticked() {
    let cal = "https://www.googleapis.com/auth/calendar.events.readonly";
    let list = "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
    let requested: Vec<String> = ["openid", "email", "profile", cal, list].map(String::from).to_vec();
    let granted: Vec<String> = [
        "openid",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
        cal,
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(missing_scopes(&requested, &granted), [list]);
    assert!(missing_scopes(&requested[..3], &granted).is_empty(), "email/profile match their full names");
}

// ── The ID token ────────────────────────────────────────────────────────────

#[test]
fn the_id_token_gives_the_email_and_user_id() {
    let t = jwt(serde_json::json!({
        "iss": "https://accounts.google.com", "aud": "cid", "sub": "1234",
        "email": "me@example.com", "email_verified": true, "exp": now() + 3600,
    }));
    let c = id_token_claims(&t, GOOGLE, "cid", now()).unwrap();
    assert_eq!(c, IdTokenClaims { email: Some("me@example.com".into()), email_verified: true, subject: Some("1234".into()) });
    // The bare-host issuer Google also uses.
    let t = jwt(serde_json::json!({ "iss": "accounts.google.com", "aud": ["cid", "x"], "azp": "cid", "sub": "1" }));
    assert_eq!(id_token_claims(&t, GOOGLE, "cid", now()).unwrap().subject.as_deref(), Some("1"));
}

#[test]
fn an_id_token_for_another_issuer_client_or_time_is_refused() {
    let check = |claims: serde_json::Value| id_token_claims(&jwt(claims), GOOGLE, "cid", now());
    let e = check(serde_json::json!({ "iss": "https://evil.example", "aud": "cid" })).unwrap_err();
    assert!(e.contains("issuer"), "{e}");
    let e = check(serde_json::json!({ "iss": "accounts.google.com", "aud": "other" })).unwrap_err();
    assert!(e.contains("another client"), "{e}");
    // Several audiences without `azp` naming us.
    assert!(check(serde_json::json!({ "iss": "accounts.google.com", "aud": ["cid", "x"], "azp": "x" })).is_err());
    let e = check(serde_json::json!({ "iss": "accounts.google.com", "aud": "cid", "exp": now() - 3600 })).unwrap_err();
    assert!(e.contains("expired"), "{e}");
    let garbage = "not-a-jwt-SECRET";
    let e = id_token_claims(garbage, GOOGLE, "cid", now()).unwrap_err();
    assert!(!e.contains("SECRET"), "an error never quotes the token: {e}");
    assert!(id_token_claims("a.%%%.c", GOOGLE, "cid", now()).is_err());
}

// ── Refresh ─────────────────────────────────────────────────────────────────

fn google() -> ServiceOAuthConfig {
    crate::identity::oauth_client::config_for("google").unwrap()
}

#[tokio::test]
async fn refresh_sends_the_refresh_token_and_client_and_keeps_the_old_refresh_token() {
    let http = StubHttp::answering(vec![reply(200, serde_json::json!({ "access_token": "a2", "expires_in": 3600 }))]);
    let old = TokenBlob { access_token: "a1".into(), refresh_token: Some("r1".into()), expires_at: 1, client_secret: Some("cs".into()) };
    let fresh = refresh_blob(&http, &google(), "cid", &old, 1000).await.unwrap();
    assert_eq!((fresh.access_token.as_str(), fresh.refresh_token.as_deref(), fresh.expires_at), ("a2", Some("r1"), 4600));
    assert_eq!(fresh.client_secret.as_deref(), Some("cs"), "the next refresh uses the same client");
    let call = &http.calls()[0];
    assert_eq!(call.url, "https://oauth2.googleapis.com/token");
    assert_eq!(call.param("grant_type"), Some("refresh_token"));
    assert_eq!(call.param("refresh_token"), Some("r1"));
    assert_eq!(call.param("client_id"), Some("cid"));
    assert_eq!(call.param("client_secret"), Some("cs"));
}

#[tokio::test]
async fn refresh_without_a_client_secret_sends_none() {
    let http = StubHttp::answering(vec![reply(200, serde_json::json!({ "access_token": "a2", "refresh_token": "r2" }))]);
    let old = TokenBlob { access_token: "a1".into(), refresh_token: Some("r1".into()), expires_at: 1, client_secret: None };
    let fresh = refresh_blob(&http, &google(), "cid", &old, 0).await.unwrap();
    assert_eq!(fresh.refresh_token.as_deref(), Some("r2"));
    assert_eq!(http.calls()[0].param("client_secret"), None);
}

#[tokio::test]
async fn invalid_grant_or_no_refresh_token_means_reconnect() {
    let old = TokenBlob { access_token: "a1".into(), refresh_token: Some("r1".into()), expires_at: 1, client_secret: None };
    let http = StubHttp::answering(vec![reply(400, serde_json::json!({ "error": "invalid_grant", "error_description": "Token has been expired or revoked." }))]);
    assert!(matches!(refresh_blob(&http, &google(), "cid", &old, 0).await, Err(RefreshError::Reauth(_))));
    let none = TokenBlob { access_token: "a1".into(), refresh_token: None, expires_at: 1, client_secret: None };
    let http = StubHttp::default();
    assert!(matches!(refresh_blob(&http, &google(), "cid", &none, 0).await, Err(RefreshError::Reauth(_))));
    assert!(http.calls().is_empty());
    // Anything else is a plain failure, with the code and no body text.
    let http = StubHttp::answering(vec![reply(401, serde_json::json!({ "error": "invalid_client", "error_description": "SECRET-ish" }))]);
    let Err(RefreshError::Failed(e)) = refresh_blob(&http, &google(), "cid", &old, 0).await else { panic!() };
    assert!(e.contains("401") && e.contains("invalid_client") && !e.contains("SECRET"), "{e}");
    let http = StubHttp::answering(vec![Err("request failed: offline".into())]);
    assert!(matches!(refresh_blob(&http, &google(), "cid", &old, 0).await, Err(RefreshError::Failed(_))));
}

fn stores() -> (Store, Store) {
    (Store::open_in_memory().unwrap(), Store::open_in_memory().unwrap())
}

#[tokio::test]
async fn a_token_far_from_expiry_is_used_without_a_call() {
    let (a, b) = stores();
    let acct = oauth_account("acc-1", "google", "cid");
    let secrets = MemSecrets::with("acc-1", &blob("a-live", Some("r"), now() + 3600));
    let http = StubHttp::default();
    let t = access_token_for(&http, &secrets, &a, &b, &acct).await.unwrap();
    assert_eq!(t.as_str(), "a-live");
    assert!(http.calls().is_empty());
}

#[tokio::test]
async fn a_token_near_expiry_is_refreshed_and_written_back() {
    let (a, b) = stores();
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.status = STATUS_NEEDS_REAUTH.to_string();
    a.identity_upsert(&acct).unwrap();
    b.identity_upsert(&acct).unwrap();
    let secrets = MemSecrets::with("acc-1", &blob("a-old", Some("r1"), now() + 30));
    let http = StubHttp::answering(vec![reply(200, serde_json::json!({ "access_token": "a-new", "expires_in": 3600 }))]);
    let t = access_token_for(&http, &secrets, &a, &b, &acct).await.unwrap();
    assert_eq!(t.as_str(), "a-new");
    let stored = TokenBlob::parse(&secrets.value("acc-1").unwrap()).unwrap();
    assert_eq!((stored.access_token.as_str(), stored.refresh_token.as_deref()), ("a-new", Some("r1")));
    assert!(stored.expires_at > now() + 3000);
    // A refresh that works means the account works again.
    assert_eq!(a.identity_get("acc-1").unwrap().unwrap().status, "valid");
    assert_eq!(b.identity_get("acc-1").unwrap().unwrap().status, "valid");
}

#[tokio::test]
async fn invalid_grant_marks_the_account_and_is_not_retried() {
    let (a, b) = stores();
    let acct = oauth_account("acc-1", "google", "cid");
    a.identity_upsert(&acct).unwrap();
    b.identity_upsert(&acct).unwrap();
    let before = blob("a-old", Some("r1"), now() - 10);
    let secrets = MemSecrets::with("acc-1", &before);
    let http = StubHttp::answering(vec![reply(400, serde_json::json!({ "error": "invalid_grant" }))]);
    let e = access_token_for(&http, &secrets, &a, &b, &acct).await.unwrap_err();
    assert!(e.starts_with(RECONNECT_NEEDED), "{e}");
    assert!(!e.contains("r1") && !e.contains("a-old"), "no token in the error: {e}");
    assert_eq!(http.calls().len(), 1, "never retried");
    assert_eq!(a.identity_get("acc-1").unwrap().unwrap().status, STATUS_NEEDS_REAUTH);
    assert_eq!(b.identity_get("acc-1").unwrap().unwrap().status, STATUS_NEEDS_REAUTH);
    assert_eq!(secrets.value("acc-1").unwrap(), before, "the stored tokens are left as they were");
}

#[tokio::test]
async fn an_account_with_no_recorded_client_needs_reconnecting() {
    let (a, b) = stores();
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.context = serde_json::json!({ "oauth": true });
    a.identity_upsert(&acct).unwrap();
    let secrets = MemSecrets::with("acc-1", &blob("a", Some("r"), now() - 10));
    let e = access_token_for(&StubHttp::default(), &secrets, &a, &b, &acct).await.unwrap_err();
    assert!(e.starts_with(RECONNECT_NEEDED), "{e}");
    assert_eq!(a.identity_get("acc-1").unwrap().unwrap().status, STATUS_NEEDS_REAUTH);
}

#[tokio::test]
async fn a_plain_secret_is_returned_as_it_is() {
    let (a, b) = stores();
    let mut acct = oauth_account("acc-1", "github", "cid");
    acct.kind = "api_key".into();
    let secrets = MemSecrets::with("acc-1", "ghp_plain");
    let t = access_token_for(&StubHttp::default(), &secrets, &a, &b, &acct).await.unwrap();
    assert_eq!(t.as_str(), "ghp_plain");
    // And an account with nothing stored says so.
    let e = access_token_for(&StubHttp::default(), &MemSecrets::default(), &a, &b, &acct).await.unwrap_err();
    assert!(e.contains("no stored token"), "{e}");
}

// ── Revoke ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn revoke_sends_the_refresh_token_to_googles_endpoint() {
    let http = StubHttp::answering(vec![reply(200, serde_json::json!({}))]);
    let acct = oauth_account("acc-1", "google", "cid");
    let secrets = MemSecrets::with("acc-1", &blob("a", Some("r1"), 0));
    let out = revoke_before_delete(&http, &secrets, &acct).await;
    assert_eq!(out, Some(serde_json::json!({ "outcome": "revoked" })));
    let call = &http.calls()[0];
    assert_eq!(call.url, "https://oauth2.googleapis.com/revoke");
    assert_eq!(call.param("token"), Some("r1"));
}

#[tokio::test]
async fn revoke_falls_back_to_the_access_token_and_reports_failure() {
    let tok = TokenBlob { access_token: "a1".into(), refresh_token: None, expires_at: 0, client_secret: None };
    let http = StubHttp::answering(vec![reply(400, serde_json::json!({ "error": "invalid_token" }))]);
    // Only an access token, and it's not accepted: the grant may live on.
    let e = revoke_blob(&http, "https://oauth2.googleapis.com/revoke", &tok).await.unwrap_err();
    assert!(e.contains("400") && e.contains("invalid_token"), "{e}");
    assert_eq!(http.calls()[0].param("token"), Some("a1"));

    // A refresh token Google no longer knows: the grant is already gone.
    let tok = TokenBlob { access_token: "a1".into(), refresh_token: Some("r1".into()), expires_at: 0, client_secret: None };
    let http = StubHttp::answering(vec![reply(400, serde_json::json!({ "error": "invalid_token" }))]);
    assert_eq!(revoke_blob(&http, "u", &tok).await, Ok(()));

    // Offline: the delete reports it, so the UI can offer a retry.
    let acct = oauth_account("acc-1", "google", "cid");
    let secrets = MemSecrets::with("acc-1", &blob("a", Some("r1"), 0));
    let http = StubHttp::answering(vec![Err("request failed: dns error".into())]);
    let out = revoke_before_delete(&http, &secrets, &acct).await.unwrap();
    assert_eq!(out["outcome"], "failed");
    assert!(!out["detail"].as_str().unwrap().contains("r1"));
}

#[tokio::test]
async fn nothing_is_revoked_for_other_accounts() {
    let http = StubHttp::default();
    let secrets = MemSecrets::with("acc-1", &blob("a", Some("r1"), 0));
    // GitHub has no revoke endpoint a device-flow client can use.
    assert_eq!(revoke_before_delete(&http, &secrets, &oauth_account("acc-1", "github", "cid")).await, None);
    // An API key account.
    let mut key = oauth_account("acc-1", "google", "cid");
    key.kind = "api_key".into();
    assert_eq!(revoke_before_delete(&http, &secrets, &key).await, None);
    // Nothing stored.
    let none = revoke_before_delete(&http, &MemSecrets::default(), &oauth_account("acc-1", "google", "cid")).await;
    assert_eq!(none, None);
    assert!(http.calls().is_empty());
}

// ── Clients ─────────────────────────────────────────────────────────────────

#[test]
fn the_client_an_account_was_made_with_is_read_from_its_context() {
    let acct = oauth_account("acc-1", "google", "cid");
    assert_eq!(account_client(&acct), Some((ClientMode::Byo, "cid".to_string())));
    let mut old = acct.clone();
    old.context = serde_json::json!({ "oauth": true });
    assert_eq!(account_client(&old), None);
    assert_eq!(ClientMode::parse_user(None), Ok(ClientMode::Byo));
    assert_eq!(ClientMode::parse_user(Some("workspace")), Ok(ClientMode::Workspace));
    assert!(ClientMode::parse_user(Some("builtin")).is_err(), "a request can't claim the built-in client");
    assert_eq!(saved_client_key("google"), "oauth-client:google");
}

#[test]
fn a_saved_client_round_trips_as_json() {
    let s = SavedClient { client_id: "cid".into(), client_secret: None, mode: ClientMode::Workspace };
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, r#"{"client_id":"cid","client_secret":null,"mode":"workspace"}"#);
    let back: SavedClient = serde_json::from_str(&json).unwrap();
    assert_eq!((back.client_id.as_str(), back.mode), ("cid", ClientMode::Workspace));
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for `oauth_client`. The flow's network legs need a browser and a
//! provider; what they feed into (`build_auth_url`, `finish`) is tested here
//! against in-memory stores and secrets.

use super::*;
use crate::identity::oauth_tokens::tests::{jwt, oauth_account, MemSecrets};

#[test]
fn pkce_challenge_is_s256_of_verifier() {
    let p = pkce_pair();
    // Recompute the challenge independently and compare.
    let mut h = Sha256::new();
    h.update(p.verifier.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(h.finalize());
    assert_eq!(p.challenge, expected);
    // base64url, no padding.
    assert!(!p.challenge.contains('='));
    assert!(!p.challenge.contains('+'));
    assert!(!p.challenge.contains('/'));
    // 43–128 chars per RFC 7636 §4.1.
    assert!(p.verifier.len() >= 43 && p.verifier.len() <= 128);
}

#[test]
fn verifiers_are_unique() {
    assert_ne!(pkce_pair().verifier, pkce_pair().verifier);
    assert_ne!(random_state(), random_state());
}

#[test]
fn catalog_flow_choices() {
    assert_eq!(config_for("github").unwrap().flow, OAuthFlow::Device);
    assert_eq!(config_for("google").unwrap().flow, OAuthFlow::AuthCodePkce);
    assert!(config_for("slack").unwrap().requires_secret);
    assert!(!config_for("google").unwrap().requires_secret);
    assert!(config_for("nope").is_none());
    assert!(config_for("microsoft").is_some());
    // Only Google is revoked on delete and has its ID token read.
    assert_eq!(config_for("google").unwrap().revoke_url, Some("https://oauth2.googleapis.com/revoke"));
    for p in ["github", "microsoft", "slack"] {
        let cfg = config_for(p).unwrap();
        assert!(cfg.revoke_url.is_none() && cfg.id_token_issuers.is_empty() && cfg.extra_auth_params.is_empty(), "{p}");
    }
}

fn byo(id: &str, secret: Option<&str>) -> ByoCredentials {
    ByoCredentials { client_id: id.into(), client_secret: secret.map(String::from), mode: ClientMode::Byo }
}

#[test]
fn unconfigured_provider_is_gated() {
    let cfg = config_for("google").unwrap();
    // No built-in client id, no BYO and nothing saved → clear "not configured" error.
    assert!(resolve_client(&cfg, None, None).is_err());
    // BYO client id unblocks it.
    assert_eq!(resolve_client(&cfg, Some(&byo("abc", None)), None).unwrap().client_id, "abc");
}

#[test]
fn the_client_is_the_requests_then_the_saved_one_then_the_built_in_one() {
    let mut cfg = config_for("google").unwrap();
    cfg.client_id = Some("builtin-id");
    let saved = SavedClient { client_id: "saved-id".into(), client_secret: Some("s".into()), mode: ClientMode::Workspace };
    let c = resolve_client(&cfg, Some(&byo("req-id", None)), Some(&saved)).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode, c.client_secret), ("req-id", ClientMode::Byo, None));
    let c = resolve_client(&cfg, None, Some(&saved)).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode, c.client_secret.as_deref()), ("saved-id", ClientMode::Workspace, Some("s")));
    let c = resolve_client(&cfg, None, None).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode), ("builtin-id", ClientMode::Builtin));
    assert!(!format!("{c:?} {:?}", resolve_client(&cfg, None, Some(&saved)).unwrap()).contains("\"s\""));
}

#[test]
fn secret_mandatory_provider_requires_byo_secret() {
    let cfg = config_for("slack").unwrap();
    assert!(resolve_client(&cfg, Some(&byo("abc", None)), None).is_err());
    assert!(resolve_client(&cfg, Some(&byo("abc", Some("shh"))), None).is_ok());
}

/// The query parameters of `url`, decoded.
fn query(url: &str) -> Vec<(String, String)> {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap();
            (k.to_string(), percent_decode(v))
        })
        .collect()
}

#[test]
fn googles_auth_url_asks_for_offline_access_consent_and_incremental_scopes() {
    let cfg = config_for("google").unwrap();
    let scopes = requested_scopes(&cfg, &["https://www.googleapis.com/auth/calendar.events.readonly".to_string()]).unwrap();
    let url = build_auth_url(&cfg, "cid.apps.googleusercontent.com", "http://127.0.0.1:5555/callback", &scopes, "st", "ch");
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    let q = query(&url);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("response_type"), Some("code"));
    assert_eq!(get("client_id"), Some("cid.apps.googleusercontent.com"));
    assert_eq!(get("redirect_uri"), Some("http://127.0.0.1:5555/callback"));
    assert_eq!(get("scope"), Some("openid email profile https://www.googleapis.com/auth/calendar.events.readonly"));
    assert_eq!(get("state"), Some("st"));
    assert_eq!(get("code_challenge"), Some("ch"));
    assert_eq!(get("code_challenge_method"), Some("S256"));
    assert_eq!(get("access_type"), Some("offline"));
    assert_eq!(get("prompt"), Some("consent"));
    assert_eq!(get("include_granted_scopes"), Some("true"));
}

#[test]
fn other_providers_auth_urls_carry_none_of_googles_parameters() {
    let cfg = config_for("microsoft").unwrap();
    let url = build_auth_url(&cfg, "cid", "http://127.0.0.1:1/callback", &requested_scopes(&cfg, &[]).unwrap(), "s", "c");
    for k in ["access_type", "prompt", "include_granted_scopes"] {
        assert!(!query(&url).iter().any(|(n, _)| n == k), "{k} in {url}");
    }
}

#[test]
fn extra_scopes_are_added_once_and_must_be_scope_tokens() {
    let cfg = config_for("google").unwrap();
    let cal = "https://www.googleapis.com/auth/calendar.events".to_string();
    let got = requested_scopes(&cfg, &[cal.clone(), "email".into(), cal.clone()]).unwrap();
    assert_eq!(got, ["openid", "email", "profile", cal.as_str()]);
    for bad in ["", "two words", "quote\"d", "back\\slash", "ta\tb"] {
        assert!(requested_scopes(&cfg, &[bad.to_string()]).is_err(), "{bad:?}");
    }
}

#[test]
fn parses_code_and_state_from_callback() {
    let req = "GET /callback?code=abc123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let cb = parse_callback(req);
    assert_eq!(cb.code.as_deref(), Some("abc123"));
    assert_eq!(cb.state.as_deref(), Some("xyz"));
    assert_eq!(cb.error, None);
}

#[test]
fn percent_decodes_callback_values() {
    let req = "GET /callback?code=a%2Fb%2Bc&state=s HTTP/1.1\r\n\r\n";
    assert_eq!(parse_callback(req).code.as_deref(), Some("a/b+c"));
}

#[test]
fn a_callback_error_is_read_and_said_plainly() {
    let cb = parse_callback("GET /callback?error=admin_policy_enforced&state=s HTTP/1.1\r\n\r\n");
    assert_eq!(cb.error.as_deref(), Some("admin_policy_enforced"));
    assert!(callback_error_message("google", "admin_policy_enforced").contains("blocked by your organization"));
    assert!(callback_error_message("google", "access_denied").contains("access denied"));
    // Only [a-z_] of a provider's error reaches a message.
    let cb = parse_callback("GET /callback?error=%3Cscript%3Ebad%20thing&state=s HTTP/1.1\r\n\r\n");
    assert_eq!(cb.error.as_deref(), Some("scriptbadthing"));
}

#[test]
fn only_a_keychain_oauth_account_of_the_same_provider_can_be_reauthorized() {
    let acct = oauth_account("acc-1", "google", "cid");
    assert!(check_reauth_target(&acct, "google").is_ok());
    assert!(check_reauth_target(&acct, "github").is_err());
    let mut key = acct.clone();
    key.kind = "api_key".into();
    assert!(check_reauth_target(&key, "google").is_err());
    let mut dir = acct.clone();
    dir.secret_ref = SecretRef::OAuthConfigDir { dir: "/tmp/x".into() };
    assert!(check_reauth_target(&dir, "google").is_err());
}

#[test]
fn a_reconnect_as_another_user_is_refused() {
    let had = serde_json::json!({ "email": "me@example.com", "subject": "111" });
    let claims = |email: &str, sub: &str| IdTokenClaims { email: Some(email.into()), email_verified: true, subject: Some(sub.into()) };
    assert!(check_same_user(&had, Some(&claims("me@example.com", "111"))).is_ok());
    // The same user whose address changed is still the same user.
    assert!(check_same_user(&had, Some(&claims("new@example.com", "111"))).is_ok());
    let e = check_same_user(&had, Some(&claims("other@example.com", "222"))).unwrap_err();
    assert!(e.contains("other@example.com") && e.contains("me@example.com"), "{e}");
    // No user id recorded: the email decides.
    let email_only = serde_json::json!({ "email": "Me@Example.com" });
    assert!(check_same_user(&email_only, Some(&claims("me@example.com", "1"))).is_ok());
    assert!(check_same_user(&email_only, Some(&claims("x@example.com", "1"))).is_err());
    assert!(check_same_user(&email_only, None).is_ok());
}

fn ctx(target: FlowTarget, store: &Arc<Store>, extra: &[&str]) -> FlowCtx {
    let cfg = config_for("google").unwrap();
    let extra: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
    FlowCtx {
        requested: requested_scopes(&cfg, &extra).unwrap(),
        cfg,
        client: ResolvedClient { client_id: "cid".into(), client_secret: Some("csec".into()), mode: ClientMode::Byo },
        target,
        save_client: false,
        mstore: store.clone(),
        identity_store: store.clone(),
    }
}

fn token_response(email: &str, sub: &str, scope: &str, refresh: Option<&str>) -> serde_json::Value {
    let id_token = jwt(serde_json::json!({
        "iss": "https://accounts.google.com", "aud": "cid", "sub": sub,
        "email": email, "email_verified": true, "exp": chrono::Utc::now().timestamp() + 3600,
    }));
    let mut r = serde_json::json!({ "access_token": "at-1", "expires_in": 3599, "scope": scope, "id_token": id_token, "token_type": "Bearer" });
    if let Some(rt) = refresh {
        r["refresh_token"] = serde_json::json!(rt);
    }
    r
}

const BASE_GRANT: &str = "openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";

#[tokio::test]
async fn a_new_google_account_gets_its_email_scopes_and_client() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = MemSecrets::default();
    let c = ctx(FlowTarget::New { name: "Work".into() }, &store, &[]);
    let status = finish(&c, &secrets, &token_response("me@example.com", "111", BASE_GRANT, Some("rt-1"))).await.unwrap();
    let OAuthStatus::Success { account_id, granted_scopes, missing_scopes, email } = status else { panic!() };
    assert_eq!(email.as_deref(), Some("me@example.com"));
    assert!(missing_scopes.is_empty(), "{missing_scopes:?}");
    assert_eq!(granted_scopes.len(), 3);

    let acct = store.identity_get(&account_id).unwrap().unwrap();
    assert_eq!((acct.name.as_str(), acct.provider.as_str(), acct.kind.as_str()), ("Work", "google", "oauth"));
    assert_eq!((acct.display_name.as_str(), acct.status.as_str()), ("me@example.com", "valid"));
    assert_eq!(acct.context["email"], "me@example.com");
    assert_eq!(acct.context["subject"], "111");
    assert_eq!(acct.context["granted_scopes"][0], "openid");
    assert_eq!(acct.context["oauth_client"], serde_json::json!({ "mode": "byo", "client_id": "cid" }));
    let ctx_text = acct.context.to_string();
    assert!(!ctx_text.contains("at-1") && !ctx_text.contains("rt-1") && !ctx_text.contains("csec"), "no secret in the row");

    let blob = TokenBlob::parse(&secrets.value(&account_id).unwrap()).unwrap();
    assert_eq!((blob.access_token.as_str(), blob.refresh_token.as_deref()), ("at-1", Some("rt-1")));
    assert_eq!(blob.client_secret.as_deref(), Some("csec"), "a refresh uses the same client");
}

#[tokio::test]
async fn turning_a_service_on_updates_the_same_account_and_reports_a_partial_grant() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.context["email"] = serde_json::json!("me@example.com");
    acct.context["subject"] = serde_json::json!("111");
    acct.context["note"] = serde_json::json!("kept");
    acct.status = "needs_reauth".into();
    store.identity_upsert(&acct).unwrap();
    let secrets = MemSecrets::with("acc-1", r#"{"access_token":"old","refresh_token":"rt-old","expires_at":1}"#);

    let cal = "https://www.googleapis.com/auth/calendar.events.readonly";
    let list = "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
    let c = ctx(FlowTarget::Existing(acct.clone()), &store, &[cal, list]);
    // The user unticked the calendar list; Google returned no new refresh token.
    let resp = token_response("me@example.com", "111", &format!("{BASE_GRANT} {cal}"), None);
    let OAuthStatus::Success { account_id, missing_scopes, .. } = finish(&c, &secrets, &resp).await.unwrap() else { panic!() };
    assert_eq!(account_id, "acc-1", "no new account");
    assert_eq!(missing_scopes, [list]);
    assert_eq!(store.identity_list(None).unwrap().len(), 1);

    let got = store.identity_get("acc-1").unwrap().unwrap();
    assert_eq!((got.status.as_str(), got.name.as_str(), got.created_at), ("valid", acct.name.as_str(), 1));
    assert_eq!(got.context["note"], "kept");
    assert_eq!(got.context["granted_scopes"].as_array().unwrap().len(), 4);
    let blob = TokenBlob::parse(&secrets.value("acc-1").unwrap()).unwrap();
    assert_eq!((blob.access_token.as_str(), blob.refresh_token.as_deref()), ("at-1", Some("rt-old")));
}

#[tokio::test]
async fn a_reconnect_as_someone_else_changes_nothing() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.context["email"] = serde_json::json!("me@example.com");
    acct.context["subject"] = serde_json::json!("111");
    store.identity_upsert(&acct).unwrap();
    let before = r#"{"access_token":"old","refresh_token":"rt-old","expires_at":1}"#;
    let secrets = MemSecrets::with("acc-1", before);
    let c = ctx(FlowTarget::Existing(acct), &store, &[]);
    let e = finish(&c, &secrets, &token_response("other@example.com", "222", BASE_GRANT, Some("rt-2"))).await.unwrap_err();
    assert!(e.contains("other@example.com"), "{e}");
    assert_eq!(secrets.value("acc-1").unwrap(), before);
    assert_eq!(store.identity_get("acc-1").unwrap().unwrap().context["email"], "me@example.com");
}

#[tokio::test]
async fn an_id_token_for_another_client_fails_the_flow_and_stores_nothing() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = MemSecrets::default();
    let mut c = ctx(FlowTarget::New { name: String::new() }, &store, &[]);
    c.client.client_id = "a-different-client".into();
    let e = finish(&c, &secrets, &token_response("me@example.com", "1", BASE_GRANT, Some("rt"))).await.unwrap_err();
    assert!(e.starts_with("sign-in rejected"), "{e}");
    assert!(secrets.map.lock().unwrap().is_empty());
    assert!(store.identity_list(None).unwrap().is_empty());
}

#[tokio::test]
async fn a_provider_without_an_id_token_still_connects() {
    // GitHub's device flow: no ID token, comma-separated scopes.
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = MemSecrets::default();
    let cfg = config_for("github").unwrap();
    let c = FlowCtx {
        requested: requested_scopes(&cfg, &[]).unwrap(),
        cfg,
        client: ResolvedClient { client_id: "gh-cid".into(), client_secret: None, mode: ClientMode::Byo },
        target: FlowTarget::New { name: String::new() },
        save_client: false,
        mstore: store.clone(),
        identity_store: store.clone(),
    };
    let resp = serde_json::json!({ "access_token": "gho_x", "token_type": "bearer", "scope": "repo,read:org" });
    let OAuthStatus::Success { account_id, email, missing_scopes, .. } = finish(&c, &secrets, &resp).await.unwrap() else { panic!() };
    assert_eq!(email, None);
    assert_eq!(missing_scopes, ["user:email"]);
    let acct = store.identity_get(&account_id).unwrap().unwrap();
    assert_eq!(acct.name, "github", "an empty name falls back to the provider");
    // What the agent's GH_TOKEN becomes for this account (spec §2.3).
    assert_eq!(oauth_tokens::bearer_value(secrets.value(&account_id).unwrap()), "gho_x");
}

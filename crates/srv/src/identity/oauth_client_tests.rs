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
    assert!(config_for("google").unwrap().requires_secret);
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
    let mut cfg = config_for("google").unwrap();
    cfg.client_id = None;
    // No built-in client id, no BYO and nothing saved → clear "not configured" error.
    assert!(resolve_client_with(&cfg, None, None, Some("s".into())).is_err());
    // A BYO Desktop client needs its secret too: Google's token endpoint refuses one without.
    assert!(resolve_client(&cfg, Some(&byo("abc", None)), None).is_err());
    assert_eq!(resolve_client(&cfg, Some(&byo("abc", Some("sec"))), None).unwrap().client_id, "abc");
}

#[test]
fn the_client_is_the_requests_then_the_saved_one_then_the_built_in_one() {
    let mut cfg = config_for("google").unwrap();
    cfg.client_id = Some("builtin-id");
    let saved = SavedClient { client_id: "saved-id".into(), client_secret: Some("s".into()), mode: ClientMode::Workspace };
    let c = resolve_client(&cfg, Some(&byo("req-id", Some("r"))), Some(&saved)).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode, c.client_secret.as_deref()), ("req-id", ClientMode::Byo, Some("r")));
    let c = resolve_client(&cfg, None, Some(&saved)).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode, c.client_secret.as_deref()), ("saved-id", ClientMode::Workspace, Some("s")));
    let c = resolve_client_with(&cfg, None, None, Some("builtin-secret".into())).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode), ("builtin-id", ClientMode::Builtin));
    assert!(!format!("{c:?} {:?}", resolve_client(&cfg, None, Some(&saved)).unwrap()).contains("\"s\""));
}

#[test]
fn googles_built_in_client_is_agentmuxs_and_needs_the_builds_secret() {
    let cfg = config_for("google").unwrap();
    assert_eq!(cfg.client_id, Some(GOOGLE_CLIENT_ID));
    assert!(GOOGLE_CLIENT_ID.ends_with(".apps.googleusercontent.com"));
    // With the secret the build carries, the built-in client is what a connect runs as.
    let c = resolve_client_with(&cfg, None, None, Some("the-secret".into())).unwrap();
    assert_eq!((c.client_id.as_str(), c.mode, c.client_secret.as_deref()), (GOOGLE_CLIENT_ID, ClientMode::Builtin, Some("the-secret")));
    assert!(!format!("{c:?}").contains("the-secret"), "the secret never reaches a log line");
    // Without it Google refuses the exchange ("client_secret is missing"), so the build says so
    // up front instead of sending the person to a consent screen that ends in an error.
    let e = resolve_client_with(&cfg, None, None, None).unwrap_err();
    assert!(e.contains("isn't available in this build"), "{e}");
    // The person's own saved client still works in such a build.
    let saved = SavedClient { client_id: "mine".into(), client_secret: Some("ms".into()), mode: ClientMode::Byo };
    assert_eq!(resolve_client_with(&cfg, None, Some(&saved), None).unwrap().client_id, "mine");
}

#[test]
fn the_run_time_secret_wins_over_the_compiled_one_and_blanks_count_as_none() {
    assert_eq!(pick_secret(Some("run".into()), Some("built")).as_deref(), Some("run"));
    assert_eq!(pick_secret(None, Some("built")).as_deref(), Some("built"));
    assert_eq!(pick_secret(Some("  ".into()), Some(" built \n")).as_deref(), Some("built"));
    assert_eq!(pick_secret(Some(" run\r\n".into()), None).as_deref(), Some("run"));
    assert_eq!(pick_secret(None, None), None);
    assert_eq!(pick_secret(Some(String::new()), Some("")), None);
}

#[test]
fn only_an_account_made_with_agentmuxs_client_refreshes_with_its_secret() {
    let cfg = config_for("google").unwrap();
    // Another client's id never gets AgentMux's secret.
    assert_eq!(builtin_secret_for(&cfg, "someone-elses-client"), None);
    // Other providers have no built-in secret.
    let gh = config_for("github").unwrap();
    assert_eq!(builtin_secret_for(&gh, "anything"), None);
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

#[tokio::test]
async fn a_brokered_github_grant_names_who_is_connected_and_is_marked_brokered() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let secrets = MemSecrets::default();
    let cfg = config_for("github").unwrap();
    let c = FlowCtx {
        requested: requested_scopes(&cfg, &[]).unwrap(),
        cfg,
        client: ResolvedClient { client_id: "gh-app".into(), client_secret: None, mode: ClientMode::Builtin },
        target: FlowTarget::New { name: String::new() },
        save_client: false,
        mstore: store.clone(),
        identity_store: store.clone(),
    };
    let resp = serde_json::json!({
        "access_token": "ghu_x", "refresh_token": "ghr_y", "expires_in": 28800,
        "github_user_id": 42, "login": "octo", "broker": true,
    });
    let OAuthStatus::Success { account_id, .. } = finish(&c, &secrets, &resp).await.unwrap() else { panic!() };
    let acct = store.identity_get(&account_id).unwrap().unwrap();
    assert_eq!(acct.display_name, "@octo", "the account reads as who is connected");
    assert_eq!((acct.context["github_user_id"].clone(), acct.context["github_login"].clone()), (serde_json::json!(42), serde_json::json!("octo")));
    assert_eq!(acct.context["broker"], true);
    let stored = oauth_tokens::TokenBlob::parse(&secrets.value(&account_id).unwrap()).unwrap();
    assert_eq!(stored.refresh_token.as_deref(), Some("ghr_y"));
    assert!(stored.expires_at > chrono::Utc::now().timestamp() + 28000);
}

#[test]
fn a_user_mismatch_ends_the_flow_so_the_person_can_be_asked() {
    let s = OAuthStatus::ConfirmUser { current_login: "old".into(), current_id: 1, new_login: "new".into(), new_id: 2 };
    assert!(s.is_terminal());
}

#[test]
fn only_agentmuxs_own_github_app_runs_through_the_broker() {
    let client = |mode| ResolvedClient { client_id: "id".into(), client_secret: None, mode };
    assert!(runs_through_broker("github", true, &client(ClientMode::Builtin)));
    // The broker exchanges the code with AgentMux's App secret: the person's own app can't use it.
    assert!(!runs_through_broker("github", true, &client(ClientMode::Byo)));
    assert!(!runs_through_broker("github", true, &client(ClientMode::Workspace)));
    // Signed out of AgentMux Cloud: no broker; other providers never use it.
    assert!(!runs_through_broker("github", false, &client(ClientMode::Builtin)));
    assert!(!runs_through_broker("google", true, &client(ClientMode::Builtin)));
}


#[test]
fn the_pickers_url_asks_for_drive_file_alone_and_opens_the_picker() {
    let cfg = config_for("google").unwrap();
    let url = build_picker_url(&cfg, "cid", "http://127.0.0.1:5555/callback", "st", "ch", Some("me@example.com"));
    let q = query(&url);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    assert_eq!(get("scope"), Some("https://www.googleapis.com/auth/drive.file"), "no other scope may ride along");
    assert_eq!(get("trigger_onepick"), Some("true"));
    assert_eq!(get("prompt"), Some("consent"));
    assert_eq!(get("allow_multiple"), Some("true"));
    assert_eq!(get("login_hint"), Some("me@example.com"), "the account's own Google user is offered first");
    assert_eq!(get("state"), Some("st"));
    assert_eq!(get("code_challenge_method"), Some("S256"));
    for k in ["include_granted_scopes", "access_type"] {
        assert_eq!(get(k), None, "{k} would combine the picker with other grants");
    }
}

#[test]
fn the_pickers_files_come_from_the_callback_and_only_real_ids_survive() {
    let cb = parse_callback("GET /callback?code=c&state=s&picked_file_ids=1AbC_d-9%2Cfile2,../x,%3Cscript%3E,, HTTP/1.1\r\n\r\n");
    assert_eq!(picked_ids(cb.picked_file_ids.as_deref()), ["1AbC_d-9", "file2"]);
    assert!(picked_ids(None).is_empty());
    let many: String = (0..150).map(|i| format!("id{i}")).collect::<Vec<_>>().join(",");
    assert_eq!(picked_ids(Some(&many)).len(), MAX_PICKED);
    // The picker's callback with no files (closed without choosing) is not an error.
    assert!(picked_ids(parse_callback("GET /callback?code=c&state=s HTTP/1.1\r\n\r\n").picked_file_ids.as_deref()).is_empty());
}

#[test]
fn the_picker_needs_drive_turned_on_and_the_account_s_own_client() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.apply_identity_schema_for_tests().unwrap();
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.context["granted_scopes"] = serde_json::json!(["https://www.googleapis.com/auth/calendar.events.readonly"]);
    let client = ResolvedClient { client_id: "cid".into(), client_secret: Some("csec".into()), mode: ClientMode::Byo };
    let e = start_picker(acct, client, store.clone(), store).unwrap_err();
    assert!(e.contains("turn Drive on"), "{e}");
}

#[tokio::test]
async fn the_picker_runs_as_the_client_the_account_was_made_with() {
    let mut acct = oauth_account("acc-1", "google", "cid-9");
    acct.context["oauth_client"] = serde_json::json!({ "mode": "byo", "client_id": "cid-9" });
    let secrets = MemSecrets::with("acc-1", &serde_json::json!({ "access_token": "a", "refresh_token": "r", "client_secret": "csec-9" }).to_string());
    let c = picker_client(&secrets, &acct).await.unwrap();
    assert_eq!((c.client_id.as_str(), c.client_secret.as_deref(), c.mode), ("cid-9", Some("csec-9"), ClientMode::Byo));
    acct.context.as_object_mut().unwrap().remove("oauth_client");
    assert!(picker_client(&secrets, &acct).await.unwrap_err().contains("connect it again"));
}


#[test]
fn a_picker_used_as_another_google_user_is_refused_and_an_unknown_user_is_not() {
    let e = picker_user_mismatch(Some("me@example.com"), Some("other@example.com")).unwrap();
    assert!(e.contains("other@example.com") && e.contains("me@example.com") && e.contains("pick again"), "{e}");
    assert!(picker_user_mismatch(Some("Me@Example.com"), Some("me@example.com")).is_none());
    assert!(picker_user_mismatch(Some("me@example.com"), None).is_none(), "Google didn't say: the check can't run");
    assert!(picker_user_mismatch(None, Some("x@example.com")).is_none());
    let hintless = build_picker_url(&config_for("google").unwrap(), "cid", "http://127.0.0.1:1/callback", "s", "c", None);
    assert!(!hintless.contains("login_hint"));
}

#[tokio::test]
async fn a_callback_with_a_hundred_picked_files_is_read_whole() {
    use tokio::io::AsyncWriteExt;
    let ids: Vec<String> = (0..100).map(|i| format!("{:0>44}", i)).collect();
    let line = format!("GET /callback?picked_file_ids={}&code=the-code&state=the-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", ids.join(","));
    assert!(line.len() > 4096, "longer than the old single read");
    let (mut tx, mut rx) = tokio::io::duplex(512);
    let sent = line.clone();
    let writer = tokio::spawn(async move {
        for part in sent.as_bytes().chunks(700) {
            tx.write_all(part).await.unwrap();
        }
    });
    let req = read_request_line(&mut rx).await;
    writer.await.unwrap();
    let cb = parse_callback(&req);
    assert_eq!(cb.code.as_deref(), Some("the-code"));
    assert_eq!(cb.state.as_deref(), Some("the-state"));
    assert_eq!(picked_ids(cb.picked_file_ids.as_deref()).len(), 100);
}

#[tokio::test]
async fn a_request_that_never_ends_its_line_is_cut_at_the_limit() {
    let endless = vec![b'a'; MAX_REQUEST_LINE * 2];
    let mut r = &endless[..];
    let got = read_request_line(&mut r).await;
    assert!(got.len() >= MAX_REQUEST_LINE && got.len() < MAX_REQUEST_LINE + 4096);
}

// ── Sessions: cancel is final, and per flow ─────────────────────────────────

fn success() -> OAuthStatus {
    OAuthStatus::Success {
        account_id: "acc-1".into(),
        granted_scopes: Vec::new(),
        missing_scopes: Vec::new(),
        email: None,
    }
}

fn is_cancelled(s: Option<OAuthStatus>) -> bool {
    matches!(s, Some(OAuthStatus::Failed { error }) if error == "cancelled")
}

#[test]
fn a_cancelled_flow_cannot_later_report_success() {
    let m = OAuthSessionManager::default();
    let (id, _stop) = m.new_session(OAuthStatus::Pending);
    assert!(m.cancel(&id));
    // The flow's task finishes after the cancel and tries to record its result.
    assert!(!m.set_status(&id, success()), "a terminal status is final");
    assert!(is_cancelled(m.poll(&id)), "the poll still says cancelled");
}

#[test]
fn a_flow_that_was_cancelled_may_not_begin_to_write_the_account() {
    let m = OAuthSessionManager::default();
    let (id, _stop) = m.new_session(OAuthStatus::Pending);
    assert!(m.cancel(&id));
    assert!(!m.begin_commit(&id), "the cancel came first: the flow writes nothing");
}

#[test]
fn a_cancel_after_the_flow_began_to_write_is_refused() {
    let m = OAuthSessionManager::default();
    let (id, _stop) = m.new_session(OAuthStatus::Pending);
    assert!(m.begin_commit(&id));
    assert!(!m.cancel(&id), "too late to cancel");
    assert!(m.set_status(&id, success()));
    assert!(matches!(m.poll(&id), Some(OAuthStatus::Success { .. })));
}

#[test]
fn cancel_of_an_unknown_or_finished_session() {
    let m = OAuthSessionManager::default();
    assert!(!m.cancel("oauth-nope"), "unknown");
    let (id, _stop) = m.new_session(OAuthStatus::Pending);
    assert!(m.set_status(&id, OAuthStatus::Failed { error: "access denied".into() }));
    assert!(m.cancel(&id), "already over: nothing to cancel");
    assert!(
        matches!(m.poll(&id), Some(OAuthStatus::Failed { error }) if error == "access denied"),
        "a cancel does not rewrite a finished flow's result"
    );
}

#[tokio::test]
async fn a_cancel_stops_that_flow_only() {
    let m = OAuthSessionManager::default();
    let (a, mut stop_a) = m.new_session(OAuthStatus::Pending);
    let (b, mut stop_b) = m.new_session(OAuthStatus::Pending);
    assert!(m.cancel(&a));
    tokio::time::timeout(Duration::from_secs(1), stop_a.stopped()).await.expect("the cancelled flow is told to stop");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), stop_b.stopped()).await.is_err(),
        "the other flow keeps running"
    );
    assert!(matches!(m.poll(&b), Some(OAuthStatus::Pending)));
    assert!(m.begin_commit(&b));
}

#[tokio::test]
async fn the_timeout_sweep_is_final_and_stops_the_flow() {
    let m = OAuthSessionManager::default();
    let (id, mut stop) = m.new_session(OAuthStatus::Pending);
    {
        let mut guard = m.sessions.lock().unwrap();
        let s = guard.get_mut(&id).unwrap();
        s.started_at = Instant::now().checked_sub(SESSION_TIMEOUT + Duration::from_secs(1)).expect("uptime");
    }
    assert!(matches!(m.poll(&id), Some(OAuthStatus::Failed { error }) if error.contains("timed out")));
    tokio::time::timeout(Duration::from_secs(1), stop.stopped()).await.expect("a timed-out flow is told to stop");
    assert!(!m.set_status(&id, success()), "a flow that wakes up late cannot report success");
    assert!(!m.begin_commit(&id));
}

#[test]
fn only_a_missing_sign_in_is_answered_with_the_sign_in_prompt() {
    use crate::muxbus::github_broker::{BrokerError, NO_VALID_SIGN_IN};
    assert!(BrokerError::NotSignedIn(NO_VALID_SIGN_IN.into()).needs_sign_in());
    // Not running yet, or a key that can't be read: signing in changes nothing.
    assert!(!BrokerError::NotSignedIn("AgentMux Cloud isn't running yet".into()).needs_sign_in());
    assert!(!BrokerError::NotSignedIn("this install's key can't be read: x".into()).needs_sign_in());
    assert!(!BrokerError::NotConnected.needs_sign_in());
}

#[test]
fn github_through_agentmuxs_own_app_needs_a_cloud_sign_in() {
    let client = |mode| ResolvedClient { client_id: "id".into(), client_secret: None, mode };
    // No sign-in: AgentMux's own App stops with the code the UI turns into a sign-in prompt.
    assert!(needs_cloud_sign_in("github", false, &client(ClientMode::Builtin)));
    assert!(!needs_cloud_sign_in("github", true, &client(ClientMode::Builtin)));
    // The person's own app costs AgentMux nothing, and other providers are not GitHub Connect.
    assert!(!needs_cloud_sign_in("github", false, &client(ClientMode::Byo)));
    assert!(!needs_cloud_sign_in("github", false, &client(ClientMode::Workspace)));
    assert!(!needs_cloud_sign_in("google", false, &client(ClientMode::Builtin)));
    assert_eq!(NEEDS_CLOUD_SIGN_IN, "needs-cloud-sign-in");
}

#[test]
fn gmail_is_asked_only_on_a_client_the_person_brought() {
    let client = |mode| ResolvedClient { client_id: "id".into(), client_secret: None, mode };
    let gmail = vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()];
    let why = restricted_scope_needs_own_client("google", &gmail, &client(ClientMode::Builtin)).unwrap();
    assert!(why.contains("your own Google client"), "{why}");
    assert!(restricted_scope_needs_own_client("google", &gmail, &client(ClientMode::Byo)).is_none());
    assert!(restricted_scope_needs_own_client("google", &gmail, &client(ClientMode::Workspace)).is_none());
    // Full mail access is restricted too; Calendar and Drive's file scope are not.
    let full = vec!["https://mail.google.com/".to_string()];
    assert!(restricted_scope_needs_own_client("google", &full, &client(ClientMode::Builtin)).is_some());
    let others = vec!["https://www.googleapis.com/auth/calendar.events.readonly".to_string(), "https://www.googleapis.com/auth/drive.file".to_string()];
    assert!(restricted_scope_needs_own_client("google", &others, &client(ClientMode::Builtin)).is_none());
    assert!(restricted_scope_needs_own_client("github", &gmail, &client(ClientMode::Builtin)).is_none());
}

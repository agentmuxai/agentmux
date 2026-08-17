// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Service-account OAuth 2.0 client for the Armory.
//!
//! Distinct from the CLI-provider OAuth in `auth_session.rs` (which scrapes a
//! spawned CLI's stdout): this drives the flow itself — opens the system
//! browser, runs a transient loopback listener (or polls a device endpoint),
//! exchanges the code, and stores the resulting tokens in the OS keychain.
//!
//! Security posture follows RFC 8252 (OAuth for Native Apps), RFC 7636
//! (PKCE, S256), RFC 8628 (Device Grant), and RFC 9700 (Security BCP):
//!   - Authorization Code + PKCE(S256) over a loopback `127.0.0.1:<ephemeral>`
//!     redirect for code-flow providers (Google, Microsoft).
//!   - Device Authorization Grant for GitHub (no client secret needed).
//!   - High-entropy `state` generated and verified on the callback (CSRF).
//!   - Public clients carry no secret; secret-mandatory providers (Slack) use
//!     BYO credentials (the user supplies their own client_id/secret).
//!   - Tokens are stored in the OS keychain (never the DB, never logged).
//!
//! **Scaffold status:** the per-provider `client_id`s are not yet provisioned
//! (`client_id: None` below). Until a client id is supplied — either baked
//! into the catalog or passed as BYO — `start` returns a clear
//! "not configured" error and no flow runs. Dropping in the ids (and wiring
//! the frontend Connect button) activates this end to end. See
//! specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §4.2/§12.1.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// How a provider's OAuth flow is driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlow {
    /// Authorization Code + PKCE over a loopback redirect (Google, Microsoft).
    AuthCodePkce,
    /// Device Authorization Grant — RFC 8628 (GitHub, headless).
    Device,
}

/// Static per-service OAuth configuration. `client_id` is `None` in the
/// shipped scaffold — supply it (or BYO) to activate the provider.
#[derive(Debug, Clone)]
pub struct ServiceOAuthConfig {
    pub provider: &'static str,
    pub flow: OAuthFlow,
    /// Public client id. `None` ⇒ provider not configured (BYO required).
    pub client_id: Option<&'static str>,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    /// Device authorization endpoint (Device flow only).
    pub device_url: Option<&'static str>,
    pub scopes: &'static [&'static str],
    /// True for providers whose token exchange mandates a client_secret
    /// (Slack). Such providers require BYO credentials on a desktop client.
    pub requires_secret: bool,
}

/// Per-provider catalog. Endpoints + flow choice follow each provider's
/// native-app guidance (see SPEC §12.1). `client_id` intentionally `None`
/// until provisioned.
pub fn config_for(provider: &str) -> Option<ServiceOAuthConfig> {
    match provider {
        "google" => Some(ServiceOAuthConfig {
            provider: "google",
            flow: OAuthFlow::AuthCodePkce,
            client_id: None, // TODO: provision public client id
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            device_url: None,
            scopes: &["openid", "email", "profile"],
            requires_secret: false,
        }),
        "microsoft" => Some(ServiceOAuthConfig {
            provider: "microsoft",
            flow: OAuthFlow::AuthCodePkce,
            client_id: None, // TODO
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            device_url: None,
            scopes: &["openid", "profile", "email", "offline_access", "User.Read"],
            requires_secret: false,
        }),
        "github" => Some(ServiceOAuthConfig {
            provider: "github",
            flow: OAuthFlow::Device, // no client secret needed
            client_id: None, // TODO
            auth_url: "https://github.com/login/oauth/authorize",
            token_url: "https://github.com/login/oauth/access_token",
            device_url: Some("https://github.com/login/device/code"),
            scopes: &["repo", "read:org", "user:email"],
            requires_secret: false,
        }),
        "slack" => Some(ServiceOAuthConfig {
            provider: "slack",
            flow: OAuthFlow::AuthCodePkce,
            client_id: None, // BYO only
            auth_url: "https://slack.com/oauth/v2/authorize",
            token_url: "https://slack.com/api/oauth.v2.access",
            device_url: None,
            scopes: &["users:read"],
            requires_secret: true,
        }),
        _ => None,
    }
}

// ── PKCE + state helpers (RFC 7636 §4, RFC 8252 §8.9) ───────────────────────

/// A PKCE verifier/challenge pair. `challenge = BASE64URL(SHA256(verifier))`.
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

/// Generate a PKCE pair. The verifier is 256 bits of CSPRNG entropy (two v4
/// UUIDs, as `uuid` draws from the OS RNG) base64url-encoded — well within the
/// 43–128 char range, charset-safe. Challenge uses S256 (never `plain`).
pub fn pkce_pair() -> PkcePair {
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    let mut raw = [0u8; 32];
    raw[..16].copy_from_slice(a.as_bytes());
    raw[16..].copy_from_slice(b.as_bytes());
    let verifier = URL_SAFE_NO_PAD.encode(raw);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    PkcePair { verifier, challenge }
}

/// High-entropy CSRF `state` value. Verified byte-for-byte on the callback.
pub fn random_state() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ── Session manager ─────────────────────────────────────────────────────────

const SESSION_TIMEOUT: Duration = Duration::from_secs(300);

/// Terminal/transient status surfaced to the frontend poll.
#[derive(Debug, Clone)]
pub enum OAuthStatus {
    /// Seeded the instant a session is created, before the spawned task has
    /// emitted its first real status. Non-terminal so an early poll doesn't
    /// see a terminal value and abort the flow.
    Pending,
    /// Code flow: browser opened to `auth_url`; awaiting the loopback callback.
    UrlAvailable { auth_url: String },
    /// Device flow: show `user_code` + `verification_uri`; polling in progress.
    CodeEmitted { user_code: String, verification_uri: String },
    /// Done — the account row + keychain token are persisted.
    Success { account_id: String },
    Failed { error: String },
}

impl OAuthStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, OAuthStatus::Success { .. } | OAuthStatus::Failed { .. })
    }
}

struct OAuthSession {
    status: OAuthStatus,
    started_at: Instant,
}

#[derive(Default)]
pub struct OAuthSessionManager {
    sessions: Mutex<HashMap<String, OAuthSession>>,
}

/// How long a session entry is retained before it's pruned. Generous margin
/// over the 5-min flow timeout so a slow frontend poll still finds its result.
const SESSION_RETENTION: Duration = Duration::from_secs(900);

impl OAuthSessionManager {
    fn new_session(&self, status: OAuthStatus) -> String {
        let id = format!("oauth-{}", uuid::Uuid::new_v4());
        let mut guard = self.sessions.lock().unwrap();
        // Prune stale entries so the global map can't grow unbounded over the
        // process lifetime (terminal + timed-out sessions are GC'd here).
        guard.retain(|_, s| s.started_at.elapsed() < SESSION_RETENTION);
        guard.insert(id.clone(), OAuthSession { status, started_at: Instant::now() });
        id
    }

    fn set_status(&self, id: &str, status: OAuthStatus) {
        if let Some(s) = self.sessions.lock().unwrap().get_mut(id) {
            s.status = status;
        }
    }

    /// Current status, sweeping the 5-minute timeout to `Failed`.
    pub fn poll(&self, id: &str) -> Option<OAuthStatus> {
        let mut guard = self.sessions.lock().unwrap();
        let s = guard.get_mut(id)?;
        if !s.status.is_terminal() && s.started_at.elapsed() > SESSION_TIMEOUT {
            s.status = OAuthStatus::Failed { error: "authorization timed out".into() };
        }
        Some(s.status.clone())
    }

    pub fn cancel(&self, id: &str) -> bool {
        let mut guard = self.sessions.lock().unwrap();
        if let Some(s) = guard.get_mut(id) {
            if !s.status.is_terminal() {
                s.status = OAuthStatus::Failed { error: "cancelled".into() };
            }
            return true;
        }
        false
    }
}

/// Process-global manager. A dedicated singleton (rather than an AppState
/// field) keeps this scaffold's blast radius small; service-OAuth sessions are
/// short-lived and window-agnostic.
pub fn manager() -> &'static OAuthSessionManager {
    static M: OnceLock<OAuthSessionManager> = OnceLock::new();
    M.get_or_init(OAuthSessionManager::default)
}

/// BYO OAuth-app credentials, supplied by the user for secret-mandatory
/// providers (Slack) or to override the built-in public client.
#[derive(Debug, Clone)]
pub struct ByoCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Resolve the effective client id: BYO overrides the built-in. Returns an
/// error string when neither is available (the "not configured" gate) or when
/// a secret-mandatory provider is missing its BYO secret.
pub fn resolve_client(
    cfg: &ServiceOAuthConfig,
    byo: Option<&ByoCredentials>,
) -> Result<(String, Option<String>), String> {
    let client_id = byo
        .map(|b| b.client_id.clone())
        .or_else(|| cfg.client_id.map(|s| s.to_string()))
        .ok_or_else(|| {
            format!(
                "{} OAuth is not configured yet — no client id available. \
                 Supply your own OAuth app credentials (BYO) to continue.",
                cfg.provider
            )
        })?;
    if cfg.requires_secret && byo.and_then(|b| b.client_secret.as_ref()).is_none() {
        return Err(format!(
            "{} requires a client secret — register your own OAuth app and \
             supply its client id + secret (BYO).",
            cfg.provider
        ));
    }
    Ok((client_id, byo.and_then(|b| b.client_secret.clone())))
}

// ── Flow execution ──────────────────────────────────────────────────────────

use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::util::open_browser;

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("AgentMux")
            .build()
            .expect("reqwest client build failed")
    })
}

/// Tokens returned by an exchange. Stored as a keychain JSON blob — never the
/// DB, never logged.
fn token_blob(json: &serde_json::Value, now: i64) -> serde_json::Value {
    let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(0);
    serde_json::json!({
        "access_token": json.get("access_token").and_then(|v| v.as_str()).unwrap_or(""),
        "refresh_token": json.get("refresh_token").and_then(|v| v.as_str()),
        "expires_at": if expires_in > 0 { now + expires_in } else { 0 },
    })
}

/// Persist the OAuth account: token blob → keychain, pointer + non-secret
/// context → DB. Returns the account id.
fn persist_oauth_account(
    wstore: &Arc<Store>,
    identity_store: &Arc<Store>,
    provider: &str,
    name: &str,
    tokens: &serde_json::Value,
) -> Result<String, String> {
    let account_id = uuid::Uuid::new_v4().to_string();
    let blob = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    crate::identity::secret_store::put(&account_id, &blob)?;
    let now = chrono::Utc::now().timestamp_millis();
    let account = IdentityAccount {
        id: account_id.clone(),
        name: name.to_string(),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::Keychain {
            service: crate::identity::secret_store::SERVICE.to_string(),
            account: crate::identity::secret_store::account_key(&account_id),
        },
        context: serde_json::json!({ "oauth": true }),
        status: "valid".to_string(),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = wstore.identity_upsert_with_mirror(identity_store, &account) {
        let _ = crate::identity::secret_store::delete(&account_id);
        return Err(format!("persist failed: {e}"));
    }
    Ok(account_id)
}

/// URL-encode a query component (RFC 3986 unreserved kept).
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Start an OAuth flow. Resolves config + client (gating on "not configured"),
/// creates a session, and spawns the background task that drives it. Returns
/// `(session_id, initial_status)`. The frontend then polls `manager().poll`.
pub fn start(
    provider: &str,
    name: String,
    byo: Option<ByoCredentials>,
    wstore: Arc<Store>,
    identity_store: Arc<Store>,
) -> Result<(String, OAuthStatus), String> {
    let cfg = config_for(provider).ok_or_else(|| format!("unknown OAuth provider: {provider}"))?;
    let (client_id, client_secret) = resolve_client(&cfg, byo.as_ref())?;

    match cfg.flow {
        OAuthFlow::AuthCodePkce => start_code_flow(cfg, client_id, client_secret, name, wstore, identity_store),
        OAuthFlow::Device => start_device_flow(cfg, client_id, name, wstore, identity_store),
    }
}

fn start_code_flow(
    cfg: ServiceOAuthConfig,
    client_id: String,
    client_secret: Option<String>,
    name: String,
    wstore: Arc<Store>,
    identity_store: Arc<Store>,
) -> Result<(String, OAuthStatus), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let pkce = pkce_pair();
    let state = random_state();
    let scopes = cfg.scopes.join(" ");

    let session_id = manager().new_session(OAuthStatus::Pending);
    let sid = session_id.clone();

    tokio::spawn(async move {
        // Transient loopback listener on an OS-assigned ephemeral port. Use the
        // 127.0.0.1 literal (not "localhost") per RFC 8252 §7.3.
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) => {
                manager().set_status(&sid, OAuthStatus::Failed { error: format!("bind failed: {e}") });
                return;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");

        let auth_url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            cfg.auth_url,
            enc(&client_id),
            enc(&redirect_uri),
            enc(&scopes),
            enc(&state),
            enc(&pkce.challenge),
        );
        manager().set_status(&sid, OAuthStatus::UrlAvailable { auth_url: auth_url.clone() });
        open_browser(&auth_url);

        // Await the single inbound callback (the session timeout sweep bounds
        // total wait; this accept has its own guard too).
        let accept = tokio::time::timeout(SESSION_TIMEOUT, listener.accept()).await;
        let mut stream = match accept {
            Ok(Ok((s, _))) => s,
            _ => {
                manager().set_status(&sid, OAuthStatus::Failed { error: "no callback received".into() });
                return;
            }
        };
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let (code, got_state) = parse_callback(&req);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body>You can close this tab and return to AgentMux.</body></html>")
            .await;

        // CSRF: reject a callback whose state doesn't match (RFC 8252 §8.9).
        if got_state.as_deref() != Some(state.as_str()) {
            manager().set_status(&sid, OAuthStatus::Failed { error: "state mismatch — possible CSRF, aborted".into() });
            return;
        }
        let code = match code {
            Some(c) => c,
            None => {
                manager().set_status(&sid, OAuthStatus::Failed { error: "no authorization code in callback".into() });
                return;
            }
        };

        // Exchange the code (+ PKCE verifier) for tokens.
        let mut params = vec![
            ("grant_type", "authorization_code".to_string()),
            ("client_id", client_id.clone()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", pkce.verifier.clone()),
        ];
        if let Some(secret) = client_secret.as_ref() {
            params.push(("client_secret", secret.clone()));
        }
        match exchange(cfg.token_url, &params).await {
            Ok(json) => {
                let now = chrono::Utc::now().timestamp();
                let tokens = token_blob(&json, now);
                match persist_oauth_account(&wstore, &identity_store, cfg.provider, &name, &tokens) {
                    Ok(account_id) => manager().set_status(&sid, OAuthStatus::Success { account_id }),
                    Err(e) => manager().set_status(&sid, OAuthStatus::Failed { error: e }),
                }
            }
            Err(e) => manager().set_status(&sid, OAuthStatus::Failed { error: e }),
        }
    });

    Ok((session_id, OAuthStatus::UrlAvailable { auth_url: String::new() }))
}

fn start_device_flow(
    cfg: ServiceOAuthConfig,
    client_id: String,
    name: String,
    wstore: Arc<Store>,
    identity_store: Arc<Store>,
) -> Result<(String, OAuthStatus), String> {
    let device_url = cfg.device_url.ok_or("provider has no device endpoint")?.to_string();
    let scopes = cfg.scopes.join(" ");
    let session_id = manager().new_session(OAuthStatus::Pending);
    let sid = session_id.clone();

    tokio::spawn(async move {
        // 1. Device authorization request.
        let params = [("client_id", client_id.clone()), ("scope", scopes)];
        let resp = match http().post(&device_url).header("Accept", "application/json").form(&params).send().await {
            Ok(r) => r,
            Err(e) => {
                manager().set_status(&sid, OAuthStatus::Failed { error: format!("device request failed: {e}") });
                return;
            }
        };
        let json: serde_json::Value = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        let device_code = json.get("device_code").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let user_code = json.get("user_code").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let verification_uri = json
            .get("verification_uri")
            .and_then(|v| v.as_str())
            .unwrap_or("https://github.com/login/device")
            .to_string();
        let mut interval = json.get("interval").and_then(|v| v.as_u64()).unwrap_or(5);
        if device_code.is_empty() {
            manager().set_status(&sid, OAuthStatus::Failed { error: "no device_code in response".into() });
            return;
        }
        manager().set_status(&sid, OAuthStatus::CodeEmitted { user_code, verification_uri });

        // 2. Poll the token endpoint (RFC 8628 §3.4–3.5).
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            // Stop if the session was cancelled / timed out by the manager.
            match manager().poll(&sid) {
                Some(s) if s.is_terminal() => return,
                None => return,
                _ => {}
            }
            let p = [
                ("client_id", client_id.clone()),
                ("device_code", device_code.clone()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code".to_string()),
            ];
            let r = match http().post(cfg.token_url).header("Accept", "application/json").form(&p).send().await {
                Ok(r) => r,
                Err(e) => {
                    manager().set_status(&sid, OAuthStatus::Failed { error: format!("poll failed: {e}") });
                    return;
                }
            };
            let j: serde_json::Value = r.json().await.unwrap_or_else(|_| serde_json::json!({}));
            if j.get("access_token").is_some() {
                let now = chrono::Utc::now().timestamp();
                let tokens = token_blob(&j, now);
                match persist_oauth_account(&wstore, &identity_store, cfg.provider, &name, &tokens) {
                    Ok(account_id) => manager().set_status(&sid, OAuthStatus::Success { account_id }),
                    Err(e) => manager().set_status(&sid, OAuthStatus::Failed { error: e }),
                }
                return;
            }
            match j.get("error").and_then(|v| v.as_str()) {
                Some("authorization_pending") => {}
                Some("slow_down") => interval += 5, // RFC 8628 §3.5
                Some("access_denied") => {
                    manager().set_status(&sid, OAuthStatus::Failed { error: "access denied".into() });
                    return;
                }
                Some("expired_token") | Some(_) => {
                    manager().set_status(&sid, OAuthStatus::Failed { error: "device code expired".into() });
                    return;
                }
                None => {}
            }
        }
    });

    Ok((session_id, OAuthStatus::CodeEmitted { user_code: String::new(), verification_uri: String::new() }))
}

async fn exchange(token_url: &str, params: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let resp = http()
        .post(token_url)
        .header("Accept", "application/json")
        .form(params)
        .send()
        .await
        .map_err(|e| format!("token exchange failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("token endpoint returned {}", resp.status()));
    }
    resp.json().await.map_err(|e| format!("token response parse failed: {e}"))
}

/// Extract `code` and `state` from a raw HTTP request line
/// `GET /callback?code=...&state=... HTTP/1.1`.
fn parse_callback(req: &str) -> (Option<String>, Option<String>) {
    let first = req.lines().next().unwrap_or("");
    let path = first.split_whitespace().nth(1).unwrap_or("");
    let query = path.split('?').nth(1).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("code"), Some(v)) => code = Some(percent_decode(v)),
            (Some("state"), Some(v)) => state = Some(percent_decode(v)),
            _ => {}
        }
    }
    (code, state)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn unconfigured_provider_is_gated() {
        let cfg = config_for("google").unwrap();
        // No built-in client id and no BYO → clear "not configured" error.
        assert!(resolve_client(&cfg, None).is_err());
        // BYO client id unblocks it.
        let byo = ByoCredentials { client_id: "abc".into(), client_secret: None };
        assert_eq!(resolve_client(&cfg, Some(&byo)).unwrap().0, "abc");
    }

    #[test]
    fn parses_code_and_state_from_callback() {
        let req = "GET /callback?code=abc123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let (code, state) = parse_callback(req);
        assert_eq!(code.as_deref(), Some("abc123"));
        assert_eq!(state.as_deref(), Some("xyz"));
    }

    #[test]
    fn percent_decodes_callback_values() {
        let req = "GET /callback?code=a%2Fb%2Bc&state=s HTTP/1.1\r\n\r\n";
        let (code, _) = parse_callback(req);
        assert_eq!(code.as_deref(), Some("a/b+c"));
    }

    #[test]
    fn secret_mandatory_provider_requires_byo_secret() {
        let cfg = config_for("slack").unwrap();
        let id_only = ByoCredentials { client_id: "abc".into(), client_secret: None };
        assert!(resolve_client(&cfg, Some(&id_only)).is_err());
        let with_secret = ByoCredentials {
            client_id: "abc".into(),
            client_secret: Some("shh".into()),
        };
        assert!(resolve_client(&cfg, Some(&with_secret)).is_ok());
    }
}

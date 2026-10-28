// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What srv does with a service-OAuth account's tokens once it has them:
//! read the keychain blob, take the account's email from the ID token,
//! refresh near expiry, revoke on delete, and keep the user's own OAuth
//! client (BYO) in the keychain.
//! docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md G0
//! (§2.1's gaps, §3.1, D2, D8).
//!
//! The flow that obtains the tokens is `oauth_client`. Every network call
//! here goes through [`OAuthHttp`] and every keychain call through
//! [`TokenSecrets`], so tests stub both and never reach Google.
//!
//! No token, refresh token, client secret or ID token is ever logged, put in
//! an error string, or returned over RPC: errors carry an HTTP status and the
//! provider's `error` code, nothing from the request.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::identity::oauth_client::ServiceOAuthConfig;

/// Refresh an access token this many seconds before it expires.
pub const REFRESH_MARGIN_SECS: i64 = 60;

/// The account status a refresh answered with `invalid_grant` leaves behind.
/// The Connectors card shows it as "Reconnect needed" (identity-manager.tsx).
pub const STATUS_NEEDS_REAUTH: &str = "needs_reauth";

/// The start of every error that means "reconnect, don't retry".
pub const RECONNECT_NEEDED: &str = "Reconnect needed";

// ── The keychain blob ───────────────────────────────────────────────────────

/// An OAuth account's keychain entry: `{access_token, refresh_token,
/// expires_at}` as JSON, plus the client secret of a BYO client that has one,
/// so a refresh uses the same client the account was made with (D8) even
/// after the saved BYO client is cleared or replaced.
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenBlob {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds; 0 when the provider gave no lifetime (never refreshed).
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for TokenBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBlob")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl Drop for TokenBlob {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(r) = self.refresh_token.as_mut() {
            r.zeroize();
        }
        if let Some(s) = self.client_secret.as_mut() {
            s.zeroize();
        }
    }
}

impl TokenBlob {
    /// The blob in a stored secret, or `None` for a plain secret (an API key
    /// or PAT), which is not JSON or has no `access_token`.
    pub fn parse(secret: &str) -> Option<TokenBlob> {
        let trimmed = secret.trim_start();
        if !trimmed.starts_with('{') {
            return None;
        }
        let blob: TokenBlob = serde_json::from_str(trimmed).ok()?;
        (!blob.access_token.is_empty()).then_some(blob)
    }

    /// The blob for a token endpoint's answer. A refresh that returns no
    /// `refresh_token` keeps `previous_refresh` (RFC 6749 §6).
    pub fn from_token_response(
        json: &serde_json::Value,
        now: i64,
        previous_refresh: Option<&str>,
        client_secret: Option<&str>,
    ) -> Result<TokenBlob, String> {
        let access_token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or("the token response carried no access token")?;
        let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(0);
        let refresh_token = json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or(previous_refresh)
            .map(str::to_string);
        Ok(TokenBlob {
            access_token: access_token.to_string(),
            refresh_token,
            expires_at: if expires_in > 0 { now + expires_in } else { 0 },
            client_secret: client_secret.map(str::to_string),
        })
    }

    pub fn to_json(&self) -> Zeroizing<String> {
        Zeroizing::new(serde_json::to_string(self).unwrap_or_default())
    }

    /// Whether the access token expires within [`REFRESH_MARGIN_SECS`].
    pub fn needs_refresh(&self, now: i64) -> bool {
        self.expires_at > 0 && now >= self.expires_at - REFRESH_MARGIN_SECS
    }
}

/// The value to use as a token for a stored secret: the `access_token` of an
/// OAuth blob, or the secret itself when it is a plain key. An OAuth-connected
/// GitHub account used to reach agents as `GH_TOKEN='{"access_token":…}'`
/// (spec §2.3).
pub fn bearer_value(mut secret: String) -> String {
    match TokenBlob::parse(&secret) {
        Some(blob) => {
            secret.zeroize();
            blob.access_token.clone()
        }
        None => secret,
    }
}

// ── Scopes ──────────────────────────────────────────────────────────────────

/// The scopes a token response granted. Google, Microsoft and Slack separate
/// them with spaces, GitHub with commas. RFC 6749 §5.1 lets a server omit
/// `scope` when it granted exactly what was asked, so then it is `requested`.
pub fn granted_scopes(token_response: &serde_json::Value, requested: &[String]) -> Vec<String> {
    let Some(raw) = token_response.get("scope").and_then(|v| v.as_str()) else {
        return requested.to_vec();
    };
    let mut out: Vec<String> = Vec::new();
    for s in raw.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// The full name Google answers with for a short scope it accepts.
fn canonical_scope(scope: &str) -> &str {
    match scope {
        "email" => "https://www.googleapis.com/auth/userinfo.email",
        "profile" => "https://www.googleapis.com/auth/userinfo.profile",
        other => other,
    }
}

/// What was asked for and not granted: the user can untick any scope on
/// Google's consent screen (granular consent, spec §3.1). `email` and
/// `profile` match the full names Google returns for them.
pub fn missing_scopes(requested: &[String], granted: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter(|r| !granted.iter().any(|g| canonical_scope(g) == canonical_scope(r)))
        .cloned()
        .collect()
}

// ── The ID token ────────────────────────────────────────────────────────────

/// What srv keeps from an ID token. Not secret.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdTokenClaims {
    pub email: Option<String>,
    pub email_verified: bool,
    /// The provider's stable user id (`sub`).
    pub subject: Option<String>,
}

/// Read an ID token's claims. It came straight from the token endpoint over
/// TLS, so per OIDC Core §3.1.3.7 (6) the TLS server check stands in for the
/// signature; the issuer, audience and expiry are still checked. Errors never
/// quote the token.
pub fn id_token_claims(
    id_token: &str,
    issuers: &[&str],
    client_id: &str,
    now: i64,
) -> Result<IdTokenClaims, String> {
    let mut parts = id_token.split('.');
    let (Some(_header), Some(payload), Some(_sig), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("the ID token is not a JWT".to_string());
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "the ID token's payload is not base64url".to_string())?;
    let claims: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "the ID token's payload is not JSON".to_string())?;

    let iss = claims.get("iss").and_then(|v| v.as_str()).unwrap_or("");
    if !issuers.contains(&iss) {
        return Err(format!("the ID token's issuer is not one of {issuers:?}"));
    }
    let aud_ok = match claims.get("aud") {
        Some(serde_json::Value::String(a)) => a == client_id,
        Some(serde_json::Value::Array(a)) => {
            // Several audiences: ours must be one, and `azp` must name us.
            a.iter().any(|v| v.as_str() == Some(client_id))
                && claims.get("azp").and_then(|v| v.as_str()) == Some(client_id)
        }
        _ => false,
    };
    if !aud_ok {
        return Err("the ID token was issued for another client".to_string());
    }
    // Five minutes of clock skew.
    if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
        if now > exp + 300 {
            return Err("the ID token has expired".to_string());
        }
    }
    let text = |k: &str| claims.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let email_verified = match claims.get("email_verified") {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    };
    Ok(IdTokenClaims { email: text("email"), email_verified, subject: text("sub") })
}

// ── HTTP and keychain seams ─────────────────────────────────────────────────

/// A provider's HTTP answer: the status and the JSON body (`{}` when the body
/// isn't JSON). A token response's body holds tokens, so `Debug` shows only
/// the status and the OAuth error code.
#[derive(Clone)]
pub struct HttpReply {
    pub status: u16,
    pub body: serde_json::Value,
}

impl std::fmt::Debug for HttpReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpReply").field("status", &self.status).field("error", &self.error_code()).finish_non_exhaustive()
    }
}

impl HttpReply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// The OAuth `error` code (RFC 6749 §5.2), kept to `[a-z_]` so nothing
    /// else from a body reaches a message.
    pub fn error_code(&self) -> Option<String> {
        let code = self.body.get("error")?.as_str()?;
        let clean: String = code.chars().filter(|c| c.is_ascii_lowercase() || *c == '_').take(64).collect();
        (!clean.is_empty()).then_some(clean)
    }
}

/// The calls srv makes to a provider's OAuth endpoints.
pub trait OAuthHttp: Sync {
    fn post_form(&self, url: &str, params: &[(&str, String)]) -> impl Future<Output = Result<HttpReply, String>> + Send;
    fn get_bearer(&self, url: &str, token: &str) -> impl Future<Output = Result<HttpReply, String>> + Send;
}

/// [`OAuthHttp`] over the network.
pub struct LiveHttp;

impl OAuthHttp for LiveHttp {
    async fn post_form(&self, url: &str, params: &[(&str, String)]) -> Result<HttpReply, String> {
        let resp = crate::identity::oauth_client::http()
            .post(url)
            .header("Accept", "application/json")
            .form(params)
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e.without_url()))?;
        let status = resp.status().as_u16();
        let body = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        Ok(HttpReply { status, body })
    }

    async fn get_bearer(&self, url: &str, token: &str) -> Result<HttpReply, String> {
        let resp = crate::identity::oauth_client::http()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e.without_url()))?;
        let status = resp.status().as_u16();
        let body = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        Ok(HttpReply { status, body })
    }
}

/// Reads and writes of an account's keychain entry.
pub trait TokenSecrets: Sync {
    fn get(&self, account_id: &str) -> impl Future<Output = Result<Option<Zeroizing<String>>, String>> + Send;
    fn put(&self, account_id: &str, value: &str) -> impl Future<Output = Result<(), String>> + Send;
}

/// [`TokenSecrets`] over `secret_store`, off the async workers (the keyring
/// calls block).
pub struct KeychainSecrets;

impl TokenSecrets for KeychainSecrets {
    async fn get(&self, account_id: &str) -> Result<Option<Zeroizing<String>>, String> {
        let id = account_id.to_string();
        tokio::task::spawn_blocking(move || crate::identity::secret_store::get_optional(&id))
            .await
            .map_err(|e| format!("keychain task: {e}"))?
    }

    async fn put(&self, account_id: &str, value: &str) -> Result<(), String> {
        let (id, value) = (account_id.to_string(), Zeroizing::new(value.to_string()));
        tokio::task::spawn_blocking(move || crate::identity::secret_store::put(&id, &value))
            .await
            .map_err(|e| format!("keychain task: {e}"))?
    }
}

// ── Which client an account was made with (D8) ──────────────────────────────

/// Where an account's OAuth client came from: AgentMux's own, the user's own,
/// or the user's own created as an "Internal" Workspace app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientMode {
    #[default]
    Builtin,
    Byo,
    Workspace,
}

impl ClientMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ClientMode::Builtin => "builtin",
            ClientMode::Byo => "byo",
            ClientMode::Workspace => "workspace",
        }
    }

    /// A user-supplied client's mode from the RPC's `clientMode`.
    pub fn parse_user(s: Option<&str>) -> Result<ClientMode, String> {
        match s.unwrap_or("byo") {
            "" | "byo" => Ok(ClientMode::Byo),
            "workspace" => Ok(ClientMode::Workspace),
            other => Err(format!("unknown client mode '{other}' (byo or workspace)")),
        }
    }
}

/// The client an account was made with, as recorded in its context
/// (`context.oauth_client`). The client id isn't secret.
pub fn account_client(account: &IdentityAccount) -> Option<(ClientMode, String)> {
    let c = account.context.get("oauth_client")?;
    let id = c.get("client_id")?.as_str().filter(|s| !s.is_empty())?;
    let mode = c
        .get("mode")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default();
    Some((mode, id.to_string()))
}

// ── The user's own client, saved in the keychain (D8) ───────────────────────

/// A BYO OAuth client saved for a provider, reused by later connects.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedClient {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub mode: ClientMode,
}

impl std::fmt::Debug for SavedClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<redacted>"))
            .field("mode", &self.mode)
            .finish()
    }
}

impl Drop for SavedClient {
    fn drop(&mut self) {
        if let Some(s) = self.client_secret.as_mut() {
            s.zeroize();
        }
    }
}

/// The keychain entry a provider's saved client lives in.
pub fn saved_client_key(provider: &str) -> String {
    format!("oauth-client:{provider}")
}

/// The client saved for `provider`, if any. Blocking (keychain).
pub fn load_saved_client(provider: &str) -> Result<Option<SavedClient>, String> {
    let Some(raw) = crate::identity::secret_store::get_optional(&saved_client_key(provider))? else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<SavedClient>(&raw).ok().filter(|c| !c.client_id.is_empty()))
}

/// Save `client` for `provider`, replacing any saved before. Blocking.
pub fn save_client(provider: &str, client: &SavedClient) -> Result<(), String> {
    let json = Zeroizing::new(serde_json::to_string(client).map_err(|e| e.to_string())?);
    crate::identity::secret_store::put(&saved_client_key(provider), &json)
}

/// Forget the client saved for `provider`. Accounts made with it keep working:
/// each keeps its own copy of what a refresh needs. Returns whether one was
/// saved. Blocking.
pub fn clear_saved_client(provider: &str) -> Result<bool, String> {
    let existed = crate::identity::secret_store::get_optional(&saved_client_key(provider))?.is_some();
    crate::identity::secret_store::delete(&saved_client_key(provider))?;
    Ok(existed)
}

// ── Refresh ─────────────────────────────────────────────────────────────────

/// Why a refresh produced no token.
#[derive(Debug, PartialEq, Eq)]
pub enum RefreshError {
    /// The grant is gone (`invalid_grant`, or no refresh token): the user has
    /// to reconnect. Never retried.
    Reauth(String),
    /// Anything else (offline, a 5xx, a misconfigured client).
    Failed(String),
}

/// Exchange `blob`'s refresh token for a new access token with the client the
/// account was made with. Pure apart from `http`.
pub async fn refresh_blob<H: OAuthHttp>(
    http: &H,
    cfg: &ServiceOAuthConfig,
    client_id: &str,
    blob: &TokenBlob,
    now: i64,
) -> Result<TokenBlob, RefreshError> {
    let Some(refresh) = blob.refresh_token.as_deref().filter(|r| !r.is_empty()) else {
        return Err(RefreshError::Reauth("the access token expired and there is no refresh token".to_string()));
    };
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh.to_string()),
        ("client_id", client_id.to_string()),
    ];
    if let Some(secret) = blob.client_secret.as_deref() {
        params.push(("client_secret", secret.to_string()));
    }
    let reply = http
        .post_form(cfg.token_url, &params)
        .await
        .map_err(|e| RefreshError::Failed(format!("token refresh failed: {e}")))?;
    for (_, v) in params.iter_mut() {
        v.zeroize();
    }
    if reply.ok() {
        return TokenBlob::from_token_response(&reply.body, now, Some(refresh), blob.client_secret.as_deref())
            .map_err(RefreshError::Failed);
    }
    match reply.error_code().as_deref() {
        Some("invalid_grant") => Err(RefreshError::Reauth(
            "the provider no longer accepts this account's grant (invalid_grant)".to_string(),
        )),
        Some(code) => Err(RefreshError::Failed(format!("the token endpoint returned {} ({code})", reply.status))),
        None => Err(RefreshError::Failed(format!("the token endpoint returned {}", reply.status))),
    }
}

/// One lock per account, so two callers needing a token at once make one
/// refresh, not two.
fn refresh_lock(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    map.entry(account_id.to_string()).or_default().clone()
}

/// Set an account's status in the account store and its global mirror.
fn set_status(id_store: &Store, identity_store: &Store, account_id: &str, status: &str) {
    for store in [id_store, identity_store] {
        if let Err(e) = store.identity_set_status(account_id, status) {
            tracing::warn!(target: "identity", account_id = %account_id, error = %e, "identity.oauth: status write failed");
        }
    }
}

/// A valid access token for `account`: the stored one, or a refreshed one
/// when it expires within [`REFRESH_MARGIN_SECS`] (written back to the
/// keychain). A plain (non-OAuth) secret is returned as it is. A grant the
/// provider no longer accepts marks the account [`STATUS_NEEDS_REAUTH`] and
/// fails with [`RECONNECT_NEEDED`]; it is never retried.
///
/// `broker` is the store behind AgentMux Cloud's sign-in and this install's key: the
/// way a GitHub grant made through the broker is refreshed (spec §4.1 item 4).
pub async fn access_token_for<H: OAuthHttp, S: TokenSecrets>(
    http: &H,
    secrets: &S,
    id_store: &Store,
    identity_store: &Store,
    account: &IdentityAccount,
    broker: Option<&Arc<Store>>,
) -> Result<Zeroizing<String>, String> {
    if !matches!(account.secret_ref, SecretRef::Keychain { .. }) {
        return Err("this account keeps no token in the keychain".to_string());
    }
    let read = || async {
        secrets
            .get(&account.id)
            .await?
            .ok_or_else(|| "no stored token for this account".to_string())
    };
    let raw = read().await?;
    let Some(blob) = TokenBlob::parse(&raw) else {
        return Ok(raw);
    };
    let now = chrono::Utc::now().timestamp();
    if !blob.needs_refresh(now) {
        return Ok(Zeroizing::new(blob.access_token.clone()));
    }

    let lock = refresh_lock(&account.id);
    let _guard = lock.lock().await;
    // Another caller may have refreshed while this one waited.
    let raw = read().await?;
    let blob = TokenBlob::parse(&raw).ok_or("the stored token is no longer an OAuth token")?;
    let now = chrono::Utc::now().timestamp();
    if !blob.needs_refresh(now) {
        return Ok(Zeroizing::new(blob.access_token.clone()));
    }

    let reauth = |why: String| {
        tracing::warn!(
            target: "identity",
            account_id = %account.id,
            provider = %account.provider,
            reason = %why,
            "identity.oauth: refresh refused, account needs reconnecting"
        );
        set_status(id_store, identity_store, &account.id, STATUS_NEEDS_REAUTH);
        format!("{RECONNECT_NEEDED}: {why}. Connect the {} account again in Connectors.", account.provider)
    };
    let cfg = crate::identity::oauth_client::config_for(&account.provider)
        .ok_or_else(|| format!("unknown OAuth provider: {}", account.provider))?;
    // The refresh token is single use at GitHub: what the broker returns is stored below
    // before anything else can fail.
    let refreshed = if crate::identity::github_connect::is_brokered(account) {
        match broker {
            Some(mstore) => crate::identity::github_connect::refresh_blob(mstore, &blob, now).await,
            None => Err(RefreshError::Failed("no AgentMux Cloud connection to refresh the GitHub grant through".to_string())),
        }
    } else {
        let Some((_, client_id)) = account_client(account) else {
            return Err(reauth("the account doesn't record the OAuth client it was made with".to_string()));
        };
        refresh_blob(http, &cfg, &client_id, &blob, now).await
    };
    match refreshed {
        Ok(fresh) => {
            secrets.put(&account.id, &fresh.to_json()).await?;
            if account.status != "valid" {
                set_status(id_store, identity_store, &account.id, "valid");
            }
            tracing::info!(target: "identity", account_id = %account.id, provider = %account.provider, "identity.oauth: access token refreshed");
            Ok(Zeroizing::new(fresh.access_token.clone()))
        }
        Err(RefreshError::Reauth(why)) => Err(reauth(why)),
        Err(RefreshError::Failed(why)) => Err(why),
    }
}

// ── Revoke ──────────────────────────────────────────────────────────────────

/// Revoke `blob`'s grant at `revoke_url` (RFC 7009). The refresh token is
/// sent when there is one: revoking it ends the whole grant, where an
/// access token may already have expired.
pub async fn revoke_blob<H: OAuthHttp>(http: &H, revoke_url: &str, blob: &TokenBlob) -> Result<(), String> {
    let (token, is_refresh) = match blob.refresh_token.as_deref().filter(|r| !r.is_empty()) {
        Some(r) => (r, true),
        None => (blob.access_token.as_str(), false),
    };
    let mut params = [("token", token.to_string())];
    let reply = http.post_form(revoke_url, &params).await;
    params[0].1.zeroize();
    let reply = reply.map_err(|e| format!("couldn't reach the revoke endpoint: {e}"))?;
    if reply.ok() {
        return Ok(());
    }
    // A refresh token the provider no longer knows means the grant is
    // already gone (revoked by the user, or expired).
    if is_refresh && reply.error_code().as_deref() == Some("invalid_token") {
        return Ok(());
    }
    Err(match reply.error_code() {
        Some(code) => format!("the revoke endpoint returned {} ({code})", reply.status),
        None => format!("the revoke endpoint returned {}", reply.status),
    })
}

/// Before an OAuth account is deleted, revoke its grant at the provider, so
/// it doesn't live on there (spec §7 "Disconnect"). `None` when there's
/// nothing to revoke: not an OAuth account, a provider with no revoke
/// endpoint, or no stored token. Otherwise
/// `{ outcome: "revoked" | "failed", detail? }`, for the delete's result.
pub async fn revoke_before_delete<H: OAuthHttp, S: TokenSecrets>(
    http: &H,
    secrets: &S,
    account: &IdentityAccount,
) -> Option<serde_json::Value> {
    if account.kind != "oauth" || !matches!(account.secret_ref, SecretRef::Keychain { .. }) {
        return None;
    }
    let revoke_url = crate::identity::oauth_client::config_for(&account.provider)?.revoke_url?;
    let raw = match secrets.get(&account.id).await {
        Ok(Some(raw)) => raw,
        Ok(None) => return None,
        Err(e) => return Some(revoke_failed(account, format!("couldn't read the stored token: {e}"))),
    };
    let blob = TokenBlob::parse(&raw)?;
    match revoke_blob(http, revoke_url, &blob).await {
        Ok(()) => {
            tracing::info!(target: "identity", account_id = %account.id, provider = %account.provider, "identity.delete: grant revoked at the provider");
            Some(serde_json::json!({ "outcome": "revoked" }))
        }
        Err(e) => Some(revoke_failed(account, e)),
    }
}

fn revoke_failed(account: &IdentityAccount, detail: String) -> serde_json::Value {
    tracing::warn!(target: "identity", account_id = %account.id, provider = %account.provider, error = %detail, "identity.delete: revoking the grant failed");
    serde_json::json!({ "outcome": "failed", "detail": detail })
}

#[cfg(test)]
#[path = "oauth_tokens_tests.rs"]
pub(crate) mod tests;

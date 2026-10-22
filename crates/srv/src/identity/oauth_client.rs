// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Service-account OAuth 2.0 client for Connectors → Accounts.
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
//! **Client status:** the per-provider built-in `client_id`s are not yet
//! provisioned (`client_id: None` below), so a flow runs with the user's own
//! client (BYO), which is saved in the keychain after the first successful
//! connect and reused (`oauth_tokens::SavedClient`, spec D8). With neither,
//! `start` returns a clear "not configured" error and no flow runs.
//!
//! What happens to the tokens afterwards (refresh, revoke, the ID token) is
//! `oauth_tokens`. docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md
//! G0; the original design is docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §4.2/§12.1.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::identity::oauth_tokens::{
    self, ClientMode, IdTokenClaims, OAuthHttp, SavedClient, TokenBlob, TokenSecrets,
};

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
    /// The base scopes every connect asks for. A connect can add more
    /// (`StartOptions::extra_scopes`).
    pub scopes: &'static [&'static str],
    /// True for providers whose token exchange mandates a client_secret
    /// (Slack). Such providers require BYO credentials on a desktop client.
    pub requires_secret: bool,
    /// Extra query parameters on the authorization URL, after the standard
    /// ones.
    pub extra_auth_params: &'static [(&'static str, &'static str)],
    /// RFC 7009 revocation endpoint, called before the account is deleted.
    pub revoke_url: Option<&'static str>,
    /// Accepted `iss` values of the ID token. Empty ⇒ the ID token isn't read.
    pub id_token_issuers: &'static [&'static str],
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
            // Google's token endpoint refuses a Desktop client's code exchange
            // and refresh without its secret. That secret isn't confidential
            // (RFC 8252 §8.5; spec §3.1), but it's required, so AgentMux's own
            // client must ship its secret too once it's provisioned.
            requires_secret: true,
            // A refresh token is returned only with `access_type=offline`,
            // and again on a later consent only with `prompt=consent`;
            // `include_granted_scopes` makes each new token cover every scope
            // granted so far (incremental authorization). Spec §3.1.
            extra_auth_params: &[
                ("access_type", "offline"),
                ("prompt", "consent"),
                ("include_granted_scopes", "true"),
            ],
            revoke_url: Some("https://oauth2.googleapis.com/revoke"),
            id_token_issuers: &["https://accounts.google.com", "accounts.google.com"],
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
            extra_auth_params: &[],
            revoke_url: None,
            // `common` issues per-tenant issuers; not read until Microsoft
            // gets its own stage.
            id_token_issuers: &[],
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
            extra_auth_params: &[],
            // GitHub revokes only with the client secret (Basic auth), which
            // a device-flow client doesn't have.
            revoke_url: None,
            id_token_issuers: &[],
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
            extra_auth_params: &[],
            revoke_url: None,
            id_token_issuers: &[],
        }),
        _ => None,
    }
}

/// The scopes a connect asks for: the provider's base scopes, then `extra`
/// (a service being turned on), without repeats. Each must be an RFC 6749
/// §3.3 scope token.
pub fn requested_scopes(cfg: &ServiceOAuthConfig, extra: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = cfg.scopes.iter().map(|s| s.to_string()).collect();
    for s in extra {
        let s = s.trim();
        let valid = !s.is_empty()
            && s.len() <= 256
            && s.bytes().all(|b| b == 0x21 || (0x23..=0x5B).contains(&b) || (0x5D..=0x7E).contains(&b));
        if !valid {
            return Err(format!("'{s}' is not a valid OAuth scope"));
        }
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    Ok(out)
}

/// The authorization URL for a code flow: the standard parameters, PKCE
/// (S256) and `state`, then the provider's `extra_auth_params`.
pub fn build_auth_url(
    cfg: &ServiceOAuthConfig,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    code_challenge: &str,
) -> String {
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        cfg.auth_url,
        enc(client_id),
        enc(redirect_uri),
        enc(&scopes.join(" ")),
        enc(state),
        enc(code_challenge),
    );
    for (k, v) in cfg.extra_auth_params {
        url.push_str(&format!("&{}={}", enc(k), enc(v)));
    }
    url
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
    /// Done — the account row + keychain token are persisted. `granted_scopes`
    /// is what the provider granted; `missing_scopes` what was asked for and
    /// unticked on the consent screen (a partial grant).
    Success {
        account_id: String,
        granted_scopes: Vec<String>,
        missing_scopes: Vec<String>,
        email: Option<String>,
    },
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
#[derive(Clone)]
pub struct ByoCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
    /// `Byo`, or `Workspace` for a client created as an organization's
    /// "Internal" app (D8).
    pub mode: ClientMode,
}

/// The client a flow runs with, and where it came from.
#[derive(Clone)]
pub struct ResolvedClient {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub mode: ClientMode,
}

impl std::fmt::Debug for ResolvedClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<redacted>"))
            .field("mode", &self.mode)
            .finish()
    }
}

/// Resolve the client a flow runs with: credentials passed with the request,
/// else the user's client saved for this provider, else the built-in one.
/// Returns an error string when none is available (the "not configured"
/// gate) or when a secret-mandatory provider is missing its secret.
pub fn resolve_client(
    cfg: &ServiceOAuthConfig,
    byo: Option<&ByoCredentials>,
    saved: Option<&SavedClient>,
) -> Result<ResolvedClient, String> {
    let client = if let Some(b) = byo {
        ResolvedClient { client_id: b.client_id.clone(), client_secret: b.client_secret.clone(), mode: b.mode }
    } else if let Some(s) = saved {
        ResolvedClient { client_id: s.client_id.clone(), client_secret: s.client_secret.clone(), mode: s.mode }
    } else if let Some(id) = cfg.client_id {
        ResolvedClient { client_id: id.to_string(), client_secret: None, mode: ClientMode::Builtin }
    } else {
        return Err(format!(
            "{} OAuth is not configured yet — no client id available. \
             Supply your own OAuth app credentials (BYO) to continue.",
            cfg.provider
        ));
    };
    if cfg.requires_secret && client.client_secret.is_none() {
        return Err(format!(
            "{} requires a client secret — register your own OAuth app and \
             supply its client id + secret (BYO).",
            cfg.provider
        ));
    }
    Ok(client)
}

// ── Flow execution ──────────────────────────────────────────────────────────

use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::util::open_browser;

pub(crate) fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("AgentMux")
            .build()
            .expect("reqwest client build failed")
    })
}

/// What a flow connects: a new account, or an existing one authorized again
/// — a service turned on (more scopes) or a reconnect — whose keychain entry
/// is updated in place.
#[derive(Debug, Clone)]
pub enum FlowTarget {
    New { name: String },
    Existing(IdentityAccount),
}

/// What `start` needs besides the provider.
pub struct StartOptions {
    pub target: FlowTarget,
    /// Client credentials passed with the request.
    pub byo: Option<ByoCredentials>,
    /// The user's client saved for this provider, used when `byo` is `None`.
    pub saved: Option<SavedClient>,
    /// Scopes beyond the provider's base ones.
    pub extra_scopes: Vec<String>,
    /// Save `byo` for this provider once the flow succeeds.
    pub save_client: bool,
}

/// Refuse to re-authorize `account` as `provider` unless it is a keychain
/// OAuth account of that provider.
pub fn check_reauth_target(account: &IdentityAccount, provider: &str) -> Result<(), String> {
    if account.provider != provider {
        return Err(format!("account {} is a {} account, not {provider}", account.id, account.provider));
    }
    if account.kind != "oauth" || !matches!(account.secret_ref, SecretRef::Keychain { .. }) {
        return Err(format!("account {} was not connected with OAuth here; connect it again instead", account.id));
    }
    Ok(())
}

/// The account's non-secret context after a successful flow: what was there
/// before, plus the granted scopes, the client it was made with (D8), and
/// the email and user id from the ID token.
pub fn oauth_context(
    existing: Option<&serde_json::Value>,
    granted: &[String],
    claims: Option<&IdTokenClaims>,
    client: &ResolvedClient,
) -> serde_json::Value {
    let mut ctx = existing.filter(|c| c.is_object()).cloned().unwrap_or_else(|| serde_json::json!({}));
    ctx["oauth"] = serde_json::json!(true);
    ctx["granted_scopes"] = serde_json::json!(granted);
    ctx["oauth_client"] = serde_json::json!({ "mode": client.mode.as_str(), "client_id": client.client_id });
    if let Some(c) = claims {
        if let Some(email) = &c.email {
            ctx["email"] = serde_json::json!(email);
            ctx["email_verified"] = serde_json::json!(c.email_verified);
        }
        if let Some(sub) = &c.subject {
            ctx["subject"] = serde_json::json!(sub);
        }
    }
    ctx
}

/// A reconnect must sign in as the same user the account already is:
/// otherwise one account would silently become another's.
fn check_same_user(existing: &serde_json::Value, claims: Option<&IdTokenClaims>) -> Result<(), String> {
    let Some(c) = claims else { return Ok(()) };
    let had = |k: &str| existing.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    if let (Some(old), Some(new)) = (had("subject"), c.subject.as_deref()) {
        if old != new {
            let who = c.email.as_deref().unwrap_or("another user");
            let was = had("email").unwrap_or("another user");
            return Err(format!(
                "you signed in as {who}, but this account is {was}. Sign in as {was}, or connect {who} as a new account."
            ));
        }
        return Ok(());
    }
    if let (Some(old), Some(new)) = (had("email"), c.email.as_deref()) {
        if !old.eq_ignore_ascii_case(new) {
            return Err(format!(
                "you signed in as {new}, but this account is {old}. Sign in as {old}, or connect {new} as a new account."
            ));
        }
    }
    Ok(())
}

/// Everything the flow's last step needs.
struct FlowCtx {
    cfg: ServiceOAuthConfig,
    client: ResolvedClient,
    target: FlowTarget,
    requested: Vec<String>,
    save_client: bool,
    mstore: Arc<Store>,
    identity_store: Arc<Store>,
}

/// The last step of a flow: read the token response (granted scopes, the ID
/// token), write the tokens to the keychain and the account row, and answer
/// the frontend's poll. A new account's keychain entry is removed again if its
/// row can't be written.
async fn finish<S: TokenSecrets>(ctx: &FlowCtx, secrets: &S, json: &serde_json::Value) -> Result<OAuthStatus, String> {
    let now = chrono::Utc::now().timestamp();
    let granted = oauth_tokens::granted_scopes(json, &ctx.requested);
    let missing = oauth_tokens::missing_scopes(&ctx.requested, &granted);
    let claims = match (ctx.cfg.id_token_issuers.is_empty(), json.get("id_token").and_then(|v| v.as_str())) {
        (false, Some(id_token)) => Some(
            oauth_tokens::id_token_claims(id_token, ctx.cfg.id_token_issuers, &ctx.client.client_id, now)
                .map_err(|e| format!("sign-in rejected: {e}"))?,
        ),
        _ => None,
    };

    let (account_id, existing) = match &ctx.target {
        FlowTarget::New { .. } => (uuid::Uuid::new_v4().to_string(), None),
        FlowTarget::Existing(acct) => {
            check_same_user(&acct.context, claims.as_ref())?;
            (acct.id.clone(), Some(acct))
        }
    };
    // A reconnect that gets no new refresh token keeps the one it had.
    let previous = match existing {
        Some(acct) => secrets.get(&acct.id).await.ok().flatten().and_then(|raw| TokenBlob::parse(&raw)),
        None => None,
    };
    let blob = TokenBlob::from_token_response(
        json,
        now,
        previous.as_ref().and_then(|b| b.refresh_token.as_deref()),
        ctx.client.client_secret.as_deref(),
    )?;
    secrets.put(&account_id, &blob.to_json()).await?;

    let context = oauth_context(existing.map(|a| &a.context), &granted, claims.as_ref(), &ctx.client);
    let email = claims.as_ref().and_then(|c| c.email.clone());
    let now_ms = chrono::Utc::now().timestamp_millis();
    let account = match &ctx.target {
        FlowTarget::New { name } => IdentityAccount {
            id: account_id.clone(),
            name: if name.trim().is_empty() { ctx.cfg.provider.to_string() } else { name.clone() },
            provider: ctx.cfg.provider.to_string(),
            kind: "oauth".to_string(),
            display_name: email.clone().unwrap_or_default(),
            secret_ref: SecretRef::Keychain {
                service: crate::identity::secret_store::SERVICE.to_string(),
                account: crate::identity::secret_store::account_key(&account_id),
            },
            context,
            status: "valid".to_string(),
            created_at: now_ms,
            updated_at: now_ms,
        },
        FlowTarget::Existing(acct) => IdentityAccount {
            display_name: email.clone().unwrap_or_else(|| acct.display_name.clone()),
            context,
            status: "valid".to_string(),
            updated_at: now_ms,
            ..acct.clone()
        },
    };
    if let Err(e) = ctx.mstore.identity_upsert_with_mirror(&ctx.identity_store, &account) {
        if existing.is_none() {
            let id = account_id.clone();
            let _ = tokio::task::spawn_blocking(move || crate::identity::secret_store::delete(&id)).await;
        }
        return Err(format!("persist failed: {e}"));
    }
    Ok(OAuthStatus::Success { account_id, granted_scopes: granted, missing_scopes: missing, email })
}

/// [`finish`] against the keychain, then save a BYO client passed with the
/// request (D8) so the next connect doesn't ask for it again. Saving is
/// best-effort: the account is already connected.
async fn complete(ctx: &FlowCtx, json: &serde_json::Value) -> OAuthStatus {
    let status = match finish(ctx, &oauth_tokens::KeychainSecrets, json).await {
        Ok(s) => s,
        Err(error) => return OAuthStatus::Failed { error },
    };
    if ctx.save_client && ctx.client.mode != ClientMode::Builtin {
        let provider = ctx.cfg.provider;
        let saved = SavedClient {
            client_id: ctx.client.client_id.clone(),
            client_secret: ctx.client.client_secret.clone(),
            mode: ctx.client.mode,
        };
        let res = tokio::task::spawn_blocking(move || oauth_tokens::save_client(provider, &saved)).await;
        if let Err(e) = res.map_err(|e| e.to_string()).and_then(|r| r) {
            tracing::warn!(target: "identity", provider = %provider, error = %e, "identity.oauth: saving the OAuth client failed");
        }
    }
    status
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
    opts: StartOptions,
    mstore: Arc<Store>,
    identity_store: Arc<Store>,
) -> Result<(String, OAuthStatus), String> {
    let cfg = config_for(provider).ok_or_else(|| format!("unknown OAuth provider: {provider}"))?;
    if let FlowTarget::Existing(acct) = &opts.target {
        check_reauth_target(acct, provider)?;
    }
    let client = resolve_client(&cfg, opts.byo.as_ref(), opts.saved.as_ref())?;
    let requested = requested_scopes(&cfg, &opts.extra_scopes)?;
    let ctx = FlowCtx {
        cfg,
        client,
        target: opts.target,
        requested,
        save_client: opts.save_client && opts.byo.is_some(),
        mstore,
        identity_store,
    };
    match ctx.cfg.flow {
        OAuthFlow::AuthCodePkce => start_code_flow(ctx),
        OAuthFlow::Device => start_device_flow(ctx),
    }
}

fn start_code_flow(ctx: FlowCtx) -> Result<(String, OAuthStatus), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let pkce = pkce_pair();
    let state = random_state();

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

        let auth_url =
            build_auth_url(&ctx.cfg, &ctx.client.client_id, &redirect_uri, &ctx.requested, &state, &pkce.challenge);
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
        let callback = parse_callback(&req);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body>You can close this tab and return to AgentMux.</body></html>")
            .await;

        // CSRF: reject a callback whose state doesn't match (RFC 8252 §8.9).
        if callback.state.as_deref() != Some(state.as_str()) {
            manager().set_status(&sid, OAuthStatus::Failed { error: "state mismatch — possible CSRF, aborted".into() });
            return;
        }
        if let Some(code) = callback.error.as_deref() {
            manager().set_status(&sid, OAuthStatus::Failed { error: callback_error_message(ctx.cfg.provider, code) });
            return;
        }
        let code = match callback.code {
            Some(c) => c,
            None => {
                manager().set_status(&sid, OAuthStatus::Failed { error: "no authorization code in callback".into() });
                return;
            }
        };

        // Exchange the code (+ PKCE verifier) for tokens.
        let mut params = vec![
            ("grant_type", "authorization_code".to_string()),
            ("client_id", ctx.client.client_id.clone()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", pkce.verifier.clone()),
        ];
        if let Some(secret) = ctx.client.client_secret.as_ref() {
            params.push(("client_secret", secret.clone()));
        }
        let status = match exchange(ctx.cfg.token_url, &params).await {
            Ok(json) => complete(&ctx, &json).await,
            Err(error) => OAuthStatus::Failed { error },
        };
        manager().set_status(&sid, status);
    });

    Ok((session_id, OAuthStatus::UrlAvailable { auth_url: String::new() }))
}

fn start_device_flow(ctx: FlowCtx) -> Result<(String, OAuthStatus), String> {
    let device_url = ctx.cfg.device_url.ok_or("provider has no device endpoint")?.to_string();
    let scopes = ctx.requested.join(" ");
    let session_id = manager().new_session(OAuthStatus::Pending);
    let sid = session_id.clone();
    let client_id = ctx.client.client_id.clone();

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
            let r = match http().post(ctx.cfg.token_url).header("Accept", "application/json").form(&p).send().await {
                Ok(r) => r,
                Err(e) => {
                    manager().set_status(&sid, OAuthStatus::Failed { error: format!("poll failed: {e}") });
                    return;
                }
            };
            let j: serde_json::Value = r.json().await.unwrap_or_else(|_| serde_json::json!({}));
            if j.get("access_token").is_some() {
                manager().set_status(&sid, complete(&ctx, &j).await);
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

/// The code exchange. A failure names the status and the OAuth `error` code,
/// never anything from the request.
async fn exchange(token_url: &str, params: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let reply = oauth_tokens::LiveHttp
        .post_form(token_url, params)
        .await
        .map_err(|e| format!("token exchange failed: {e}"))?;
    if !(200..300).contains(&reply.status) {
        return Err(match reply.error_code() {
            Some(code) => format!("token endpoint returned {} ({code})", reply.status),
            None => format!("token endpoint returned {}", reply.status),
        });
    }
    Ok(reply.body)
}

/// What the provider sent back to the loopback listener.
#[derive(Debug, Default, PartialEq, Eq)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    /// The OAuth `error` (RFC 6749 §4.1.2.1), kept to `[a-z_]`.
    error: Option<String>,
}

/// Extract `code`, `state` and `error` from a raw HTTP request line
/// `GET /callback?code=...&state=... HTTP/1.1`.
fn parse_callback(req: &str) -> Callback {
    let first = req.lines().next().unwrap_or("");
    let path = first.split_whitespace().nth(1).unwrap_or("");
    let query = path.split('?').nth(1).unwrap_or("");
    let mut out = Callback::default();
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("code"), Some(v)) => out.code = Some(percent_decode(v)),
            (Some("state"), Some(v)) => out.state = Some(percent_decode(v)),
            (Some("error"), Some(v)) => {
                let clean: String = percent_decode(v)
                    .chars()
                    .filter(|c| c.is_ascii_lowercase() || *c == '_')
                    .take(64)
                    .collect();
                out.error = Some(if clean.is_empty() { "unknown_error".to_string() } else { clean });
            }
            _ => {}
        }
    }
    out
}

/// The sentence a flow fails with when the provider answered with an error.
fn callback_error_message(provider: &str, code: &str) -> String {
    match code {
        "access_denied" => "access denied: the consent screen was cancelled or declined".to_string(),
        "admin_policy_enforced" => format!(
            "blocked by your organization: a {provider} Workspace administrator doesn't allow this app \
             (admin_policy_enforced). Ask the administrator to allow it, or use a Workspace internal client."
        ),
        other => format!("{provider} refused the sign-in ({other})"),
    }
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
#[path = "oauth_client_tests.rs"]
mod tests;

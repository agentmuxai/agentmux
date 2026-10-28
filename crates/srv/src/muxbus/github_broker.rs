// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The desktop side of AgentMux Cloud's GitHub broker (`/github/v1/*`).
//!
//! The broker holds the AgentMux App's client secret, so a connect, a refresh
//! and a disconnect all go through it. Every call carries two proofs:
//!
//! - the account's AgentMux Cloud sign-in, as a bearer token;
//! - an **install signature** (the Ed25519 instance key of this install, the one
//!   the WAN jekts use) over the request, so a stolen sign-in alone can't call
//!   the broker from another machine.
//!
//! The signature covers, joined by U+0001: `amx-github-broker-v1`, the HTTP
//! method in upper case, the path and query as sent, the timestamp header's
//! value (epoch milliseconds), and the lower-case hex SHA-256 of the exact body
//! bytes (of the empty string when there is none). It travels as
//! `x-amx-instance`, `x-amx-ts` and `x-amx-sig` (standard base64). The golden
//! vectors in the tests are the broker's own (generated independently of this
//! code), so a drift on either side fails a test.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::Signer;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The signature's domain separator.
pub const SIGNATURE_DOMAIN: &str = "amx-github-broker-v1";
const SEP: char = '\u{1}';
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// What the install signs for one request.
pub fn signature_material(method: &str, path_and_query: &str, ts_ms: &str, body: &[u8]) -> String {
    let body_hash = hex::encode(Sha256::digest(body));
    [SIGNATURE_DOMAIN, &method.to_ascii_uppercase(), path_and_query, ts_ms, &body_hash].join(&SEP.to_string())
}

/// The base64 signature of `material` under the instance's 32-byte seed.
pub fn sign(seed: &[u8; 32], material: &str) -> String {
    let key = ed25519_dalek::SigningKey::from_bytes(seed);
    BASE64.encode(key.sign(material.as_bytes()).to_bytes())
}

/// Who is calling, and as what: everything a call needs besides its request.
pub struct Caller {
    /// The AgentMux Cloud base URL (`https://…`, no trailing slash).
    pub base: String,
    /// The account's cloud access token.
    pub bearer: String,
    /// This install's instance id.
    pub instance_id: String,
    /// The instance key's seed. Never logged.
    pub seed: [u8; 32],
}

impl std::fmt::Debug for Caller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Caller").field("base", &self.base).field("instance_id", &self.instance_id).finish_non_exhaustive()
    }
}

/// Why a broker call gave no answer. The variants are what the desktop does
/// next, not what the broker said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    /// There is no AgentMux Cloud sign-in (or this install has no key yet).
    NotSignedIn(String),
    /// GitHub no longer accepts this grant: connect again.
    Reauthorize,
    /// An operator blocked this account's GitHub grant.
    Blocked,
    /// The broker doesn't know this install, or it isn't approved: sign in and approve it in the Console.
    InstallNotApproved(String),
    /// The grant was revoked or is otherwise not live (`status` is the broker's word for it).
    NotLive(String),
    /// This account has no GitHub grant.
    NotConnected,
    /// The account is connected as another GitHub user. Ask, then connect again with `replace`.
    UserMismatch { current: GithubUser, new: GithubUser },
    /// GitHub or the broker is briefly unavailable.
    Unavailable(String),
    /// The installation isn't linked to this account (or its link was revoked).
    NotLinked,
    /// The link wasn't verified lately: present a fresh user token to `links/revalidate`, then ask again.
    LinkStale,
    /// A requested repository is outside what this account can see in the installation.
    RepositoryNotVisible(String),
    /// A requested permission is above what the App may give.
    PermissionNotAllowed(String),
    /// The broker refused the request as malformed: a bug here, not something to retry.
    BadRequest(String),
    /// The broker isn't set up to mint yet.
    NotConfigured,
    /// Anything else: the status and the broker's error code.
    Other { status: u16, error: String },
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSignedIn(why) => write!(f, "sign in to AgentMux Cloud first ({why})"),
            Self::Reauthorize => write!(f, "GitHub no longer accepts this connection: connect GitHub again"),
            Self::Blocked => write!(f, "this account's GitHub connection was blocked by an operator"),
            Self::InstallNotApproved(code) => {
                write!(f, "AgentMux Cloud doesn't accept this install yet ({code}): sign in again and approve it in the Console")
            }
            Self::NotLive(status) => write!(f, "the GitHub connection is {status}: connect GitHub again"),
            Self::NotConnected => write!(f, "GitHub isn't connected on this AgentMux account"),
            Self::UserMismatch { current, new } => {
                write!(f, "this AgentMux account is connected as @{}, not @{}", current.login, new.login)
            }
            Self::Unavailable(why) => write!(f, "GitHub or AgentMux Cloud is unavailable right now ({why}); try again shortly"),
            Self::NotLinked => write!(f, "the AgentMux GitHub App isn't linked to this account for that owner: install it in Connectors"),
            Self::LinkStale => write!(f, "the link to that installation needs to be checked again"),
            Self::RepositoryNotVisible(repo) => {
                write!(f, "{repo} isn't a repository this account can reach through the AgentMux GitHub App: add it to the installation, or check you can see it on GitHub")
            }
            Self::PermissionNotAllowed(permission) => write!(f, "the AgentMux GitHub App can't give the {permission} permission"),
            Self::BadRequest(why) => write!(f, "AgentMux Cloud refused a malformed GitHub request ({why})"),
            Self::NotConfigured => write!(f, "AgentMux Cloud can't mint GitHub tokens yet"),
            Self::Other { status, error } => write!(f, "AgentMux Cloud refused the GitHub request ({status} {error})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GithubUser {
    pub github_user_id: u64,
    pub login: String,
}

/// `POST /github/v1/token` for a code exchange or a refresh.
#[derive(Clone, Deserialize)]
pub struct TokenGrant {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
    pub refresh_expires_at_ms: i64,
    pub github_user_id: u64,
    pub login: String,
    /// False when GitHub's `/user` couldn't be asked during a refresh: the id and
    /// login are the ones the broker had on record, not re-read.
    #[serde(default = "yes")]
    pub identity_checked: bool,
}

fn yes() -> bool {
    true
}

impl std::fmt::Debug for TokenGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenGrant")
            .field("github_user_id", &self.github_user_id)
            .field("login", &self.login)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("identity_checked", &self.identity_checked)
            .finish_non_exhaustive()
    }
}

/// `GET /github/v1/connection`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Connection {
    pub connected: bool,
    #[serde(default)]
    pub github_user_id: Option<u64>,
    #[serde(default)]
    pub login: Option<String>,
    /// `none`, `active`, `revoked` or `blocked`.
    pub status: String,
}

/// What a disconnect did.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Revoked {
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub disconnected: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// What `POST /github/v1/installation-token` asks for: a token for one installation, narrowed
/// to the repositories (bare names under the installation's account) and the permissions.
pub struct InstallationRequest<'a> {
    pub installation_id: u64,
    pub repositories: &'a [String],
    pub permissions: &'a std::collections::BTreeMap<String, String>,
    /// For the broker's audit line only; never what it trusts.
    pub agent_id: &'a str,
}

/// The App's bot: what its commits are attributed to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AppBot {
    pub slug: String,
    pub bot_user_id: u64,
}

/// A minted installation token.
#[derive(Clone, Deserialize)]
pub struct InstallationToken {
    pub token: String,
    /// RFC 3339.
    pub expires_at: String,
    #[serde(default)]
    pub permissions: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub repository_ids: Vec<u64>,
    pub app: AppBot,
}

impl std::fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallationToken")
            .field("expires_at", &self.expires_at)
            .field("permissions", &self.permissions)
            .field("repository_ids", &self.repository_ids)
            .field("app", &self.app)
            .finish_non_exhaustive()
    }
}

impl InstallationToken {
    /// When the token expires, in epoch seconds; `None` if the broker's timestamp isn't RFC 3339.
    pub fn expires_at_epoch(&self) -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(&self.expires_at).ok().map(|t| t.timestamp())
    }
}

/// The GitHub account an installation is on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InstallationAccount {
    pub id: u64,
    pub login: String,
}

/// One of the account's links to an installation, from `GET /github/v1/installations`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LinkedInstallation {
    pub installation_id: u64,
    pub github_account: InstallationAccount,
    /// `all` or `selected`.
    #[serde(default)]
    pub repository_selection: String,
    /// `active`, `suspended` or `deleted`.
    pub installation_status: String,
    /// `active` or `revoked`.
    pub link_status: String,
    #[serde(default)]
    pub verified_at_ms: i64,
}

impl LinkedInstallation {
    /// Whether a token can be asked for through this link.
    pub fn is_usable(&self) -> bool {
        self.installation_status == "active" && self.link_status == "active"
    }
}

/// What a revalidation did.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Revalidated {
    #[serde(default)]
    pub revalidated: Vec<u64>,
    #[serde(default)]
    pub revoked: Vec<u64>,
    #[serde(default)]
    pub remaining: u64,
}

/// The body of an authorization-code exchange.
pub fn code_body(code: &str, code_verifier: &str, redirect_uri: &str, replace: bool) -> Value {
    let mut body = json!({ "grant": "code", "code": code, "code_verifier": code_verifier, "redirect_uri": redirect_uri });
    if replace {
        body["replace"] = json!(true);
    }
    body
}

/// Maps a failed answer to what the desktop does about it.
pub fn error_for(status: u16, body: &Value) -> BrokerError {
    let code = body.get("error").and_then(Value::as_str).unwrap_or("").to_string();
    let user = |key: &str| -> Option<GithubUser> { serde_json::from_value(body.get(key)?.clone()).ok() };
    // A name the broker put in its answer, for a message: short and printable, never trusted further.
    let text = |key: &str| -> String {
        body.get(key).and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(100).collect()
    };
    match (status, code.as_str()) {
        (401, "reauthorize") => BrokerError::Reauthorize,
        (401, c) if c.starts_with("install_signature") => BrokerError::InstallNotApproved(code),
        (403, "install_unknown") | (403, "install_revoked") | (403, "install_not_approved") => BrokerError::InstallNotApproved(code),
        (403, "not_linked") | (404, "not_linked") => BrokerError::NotLinked,
        (403, "link_stale") => BrokerError::LinkStale,
        (403, "repository_not_visible") => BrokerError::RepositoryNotVisible(text("repository")),
        (403, "permission_not_allowed") => BrokerError::PermissionNotAllowed(text("permission")),
        (400, "bad_request") => BrokerError::BadRequest(text("message")),
        (503, "github_not_configured") => BrokerError::NotConfigured,
        (403, "blocked") => BrokerError::Blocked,
        (403, "not_connected") | (404, "not_connected") => BrokerError::NotConnected,
        (403, "revoked") => BrokerError::NotLive("revoked".to_string()),
        (409, "github_user_mismatch") => match (user("current"), user("new")) {
            (Some(current), Some(new)) => BrokerError::UserMismatch { current, new },
            // A refresh's mismatch names nobody: the refresh token isn't this account's user's.
            _ => BrokerError::Reauthorize,
        },
        (429, _) | (503, _) | (502, "github_unavailable") => BrokerError::Unavailable(if code.is_empty() { status.to_string() } else { code }),
        (_, _) => BrokerError::Other { status, error: code },
    }
}

/// One signed call. `body` is sent as given, so the bytes signed are the bytes sent.
async fn call(http: &reqwest::Client, who: &Caller, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value, BrokerError> {
    let bytes = match body {
        Some(v) => serde_json::to_vec(v).map_err(|e| BrokerError::Other { status: 0, error: e.to_string() })?,
        None => Vec::new(),
    };
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let material = signature_material(method.as_str(), path, &ts, &bytes);
    let mut request = http
        .request(method, format!("{}{}", who.base, path))
        .timeout(CALL_TIMEOUT)
        .header("User-Agent", crate::muxbus::relay::USER_AGENT)
        .header("Authorization", format!("Bearer {}", who.bearer))
        .header("x-amx-instance", &who.instance_id)
        .header("x-amx-ts", &ts)
        .header("x-amx-sig", sign(&who.seed, &material));
    if body.is_some() {
        request = request.header("Content-Type", "application/json").body(bytes);
    }
    let reply = request.send().await.map_err(|e| BrokerError::Unavailable(format!("couldn't reach AgentMux Cloud: {}", e.without_url())))?;
    let status = reply.status().as_u16();
    // Not `reply.json()`: an error page isn't JSON, and the status still says what to do.
    let text = reply.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return Ok(parsed);
    }
    Err(error_for(status, &parsed))
}

fn grant_of(value: Value) -> Result<TokenGrant, BrokerError> {
    serde_json::from_value(value).map_err(|_| BrokerError::Other { status: 200, error: "unreadable_answer".to_string() })
}

/// Trade an authorization code (with its PKCE verifier) for the user's grant.
pub async fn exchange_code(
    http: &reqwest::Client,
    who: &Caller,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    replace: bool,
) -> Result<TokenGrant, BrokerError> {
    let body = code_body(code, code_verifier, redirect_uri, replace);
    grant_of(call(http, who, reqwest::Method::POST, "/github/v1/token", Some(&body)).await?)
}

/// Trade a refresh token for a new grant. GitHub's refresh tokens are single use:
/// the answer's replaces the one sent.
pub async fn refresh(http: &reqwest::Client, who: &Caller, refresh_token: &str) -> Result<TokenGrant, BrokerError> {
    let body = json!({ "grant": "refresh", "refresh_token": refresh_token });
    grant_of(call(http, who, reqwest::Method::POST, "/github/v1/token", Some(&body)).await?)
}

/// End the grant on GitHub and at the broker. Both of the install's tokens are
/// sent, so a dead access token can fall back to the refresh token.
pub async fn revoke(http: &reqwest::Client, who: &Caller, access_token: Option<&str>, refresh_token: Option<&str>) -> Result<Revoked, BrokerError> {
    let mut body = json!({ "grant": "revoke" });
    if let Some(a) = access_token.filter(|a| !a.is_empty()) {
        body["access_token"] = json!(a);
    }
    if let Some(r) = refresh_token.filter(|r| !r.is_empty()) {
        body["refresh_token"] = json!(r);
    }
    let answer = call(http, who, reqwest::Method::POST, "/github/v1/token", Some(&body)).await?;
    serde_json::from_value(answer).map_err(|_| BrokerError::Other { status: 200, error: "unreadable_answer".to_string() })
}

/// Whether the account has a live GitHub grant, and as whom.
pub async fn connection(http: &reqwest::Client, who: &Caller) -> Result<Connection, BrokerError> {
    let answer = call(http, who, reqwest::Method::GET, "/github/v1/connection", None).await?;
    serde_json::from_value(answer).map_err(|_| BrokerError::Other { status: 200, error: "unreadable_answer".to_string() })
}

/// A token narrowed to the repositories and permissions asked for, on one linked installation.
pub async fn installation_token(http: &reqwest::Client, who: &Caller, request: &InstallationRequest<'_>) -> Result<InstallationToken, BrokerError> {
    let body = json!({
        "installation_id": request.installation_id,
        "repositories": request.repositories,
        "permissions": request.permissions,
        "agent_id": request.agent_id.chars().take(80).collect::<String>(),
    });
    let answer = call(http, who, reqwest::Method::POST, "/github/v1/installation-token", Some(&body)).await?;
    serde_json::from_value(answer).map_err(|_| BrokerError::Other { status: 200, error: "unreadable_answer".to_string() })
}

/// The installations this account has linked.
pub async fn installations(http: &reqwest::Client, who: &Caller) -> Result<Vec<LinkedInstallation>, BrokerError> {
    let answer = call(http, who, reqwest::Method::GET, "/github/v1/installations", None).await?;
    let list = answer.get("installations").cloned().unwrap_or(Value::Array(Vec::new()));
    serde_json::from_value(list).map_err(|_| BrokerError::Other { status: 200, error: "unreadable_answer".to_string() })
}

/// Present a fresh user token so the broker re-reads what this account can see; it keeps nothing of the token.
pub async fn revalidate(http: &reqwest::Client, who: &Caller, user_token: &str) -> Result<Revalidated, BrokerError> {
    let body = json!({ "access_token": user_token });
    let answer = call(http, who, reqwest::Method::POST, "/github/v1/links/revalidate", Some(&body)).await?;
    serde_json::from_value(answer).map_err(|_| BrokerError::Other { status: 200, error: "unreadable_answer".to_string() })
}

/// The caller for this install: the cloud sign-in and the instance key.
pub async fn caller(mstore: &std::sync::Arc<crate::backend::storage::store::Store>) -> Result<Caller, BrokerError> {
    let scheduler = crate::broker::get_global().ok_or_else(|| BrokerError::NotSignedIn("AgentMux Cloud isn't running yet".to_string()))?;
    let bearer = crate::muxbus::cloud_subscriber::load_valid_token(mstore, &scheduler)
        .await
        .ok_or_else(|| BrokerError::NotSignedIn("no valid sign-in".to_string()))?;
    // The install's identity is attached to the channel's own store, and installed process-wide;
    // `mstore` is often the shared account store, which doesn't carry it.
    let wan = mstore
        .wan_identity()
        .or_else(crate::backend::storage::wan_identity::global)
        .ok_or_else(|| BrokerError::NotSignedIn("this install has no identity yet".to_string()))?;
    let instance = wan
        .instance_ensure(&crate::backend::reactive::registry::local_host_label())
        .map_err(|e| BrokerError::NotSignedIn(format!("this install's key can't be read: {e}")))?;
    Ok(Caller {
        base: crate::muxbus::relay::rest_base_url().trim_end_matches('/').to_string(),
        bearer,
        instance_id: instance.instance_id,
        seed: instance.private_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The vectors are the broker's: made by `crypto.sign` (node) over the same
    // material, with a test key (a seed of 32 bytes of 0x07) that is used nowhere.
    const SEED: [u8; 32] = [7; 32];
    const TS: &str = "1793147000000";

    #[test]
    fn the_signature_matches_the_brokers_golden_vector_for_a_post() {
        let body = br#"{"grant":"refresh","refresh_token":"ghr_TESTREFRESHTOKEN0123456789"}"#;
        let material = signature_material("POST", "/github/v1/token", TS, body);
        assert_eq!(
            material,
            "amx-github-broker-v1\u{1}POST\u{1}/github/v1/token\u{1}1793147000000\u{1}6013f538617ee04e04a5006f92e3949bef75ac92b53711ae2bfc8dd80c3295dd"
        );
        assert_eq!(
            sign(&SEED, &material),
            "6C23T7rCDfmmdBCIzChNl2+XzIWBNqBGPzvz3Ltdr03IC5omlnL4TwQqZyu4REun+DVtWEa33oyHHQ+1KSr5Bw=="
        );
    }

    #[test]
    fn the_signature_matches_the_brokers_golden_vector_for_a_get_with_no_body() {
        let material = signature_material("get", "/github/v1/connection", TS, b"");
        assert!(material.ends_with("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
        assert!(material.contains("\u{1}GET\u{1}"), "the method is signed in upper case");
        assert_eq!(
            sign(&SEED, &material),
            "olMcFNqD771CqWvmPe8Gsv9xhy8pFalbnCSNfbooHHK6T4SalUe6Dt5yIcjmxtjLYYufLBTpFhBELJr9/gkrBQ=="
        );
    }

    #[test]
    fn a_failed_answer_becomes_what_the_desktop_does_next() {
        let body = |v: Value| v;
        assert_eq!(error_for(401, &body(json!({ "error": "reauthorize" }))), BrokerError::Reauthorize);
        assert_eq!(error_for(403, &body(json!({ "error": "blocked" }))), BrokerError::Blocked);
        assert_eq!(error_for(403, &body(json!({ "error": "not_connected" }))), BrokerError::NotConnected);
        assert_eq!(error_for(404, &body(json!({ "error": "not_connected" }))), BrokerError::NotConnected);
        assert_eq!(error_for(403, &body(json!({ "error": "revoked" }))), BrokerError::NotLive("revoked".into()));
        assert!(matches!(error_for(401, &body(json!({ "error": "install_signature_expired" }))), BrokerError::InstallNotApproved(c) if c == "install_signature_expired"));
        assert!(matches!(error_for(403, &body(json!({ "error": "install_unknown" }))), BrokerError::InstallNotApproved(_)));
        assert!(matches!(error_for(503, &body(json!({ "error": "github_unavailable" }))), BrokerError::Unavailable(_)));
        assert!(matches!(error_for(429, &Value::Null), BrokerError::Unavailable(_)));
        assert_eq!(error_for(418, &Value::Null), BrokerError::Other { status: 418, error: String::new() });
    }

    #[test]
    fn a_user_mismatch_names_both_users_and_a_refresh_s_means_reauthorize() {
        let answer = json!({
            "error": "github_user_mismatch",
            "current": { "github_user_id": 1, "login": "old" },
            "new": { "github_user_id": 2, "login": "new" },
        });
        assert_eq!(
            error_for(409, &answer),
            BrokerError::UserMismatch {
                current: GithubUser { github_user_id: 1, login: "old".into() },
                new: GithubUser { github_user_id: 2, login: "new".into() },
            }
        );
        // The refresh path answers with the bare code.
        assert_eq!(error_for(409, &json!({ "error": "github_user_mismatch" })), BrokerError::Reauthorize);
    }

    #[test]
    fn a_grant_reads_as_the_broker_sends_it_and_never_prints_a_token() {
        let g = grant_of(json!({
            "access_token": "ghu_SECRET", "refresh_token": "ghr_SECRET",
            "expires_at_ms": 5, "refresh_expires_at_ms": 9, "github_user_id": 42, "login": "octo",
        }))
        .unwrap();
        assert!(g.identity_checked, "absent means the identity was checked");
        assert_eq!((g.github_user_id, g.login.as_str()), (42, "octo"));
        assert!(!format!("{g:?}").contains("SECRET"));
        let unchecked = grant_of(json!({
            "access_token": "a", "refresh_token": "r", "expires_at_ms": 1, "refresh_expires_at_ms": 2,
            "github_user_id": 42, "login": "octo", "identity_checked": false,
        }))
        .unwrap();
        assert!(!unchecked.identity_checked);
    }

    #[test]
    fn a_code_body_carries_replace_only_when_the_user_agreed() {
        let plain = code_body("c", "v", "http://127.0.0.1:1/callback", false);
        assert!(plain.get("replace").is_none());
        assert_eq!(code_body("c", "v", "http://127.0.0.1:1/callback", true)["replace"], json!(true));
    }

    /// A one-shot stand-in for the broker: it checks the headers the way the
    /// broker does (the instance's public key over the material it rebuilds
    /// from what it received) and answers with `reply`.
    async fn stub(reply: &'static str) -> (String, tokio::task::JoinHandle<(String, String, String, String)>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                data.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&data).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if data.len() >= end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&data).to_string();
            let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
            let header = |name: &str| {
                head.lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                    })
                    .unwrap_or_default()
            };
            let line = head.lines().next().unwrap_or_default().to_string();
            let out = (line, header("x-amx-ts"), header("x-amx-sig"), body.to_string());
            stream
                .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply).as_bytes())
                .await
                .unwrap();
            out
        });
        (base, task)
    }

    fn caller_at(base: String) -> Caller {
        Caller { base, bearer: "cloud-token".into(), instance_id: "a".repeat(26), seed: SEED }
    }

    #[tokio::test]
    async fn a_refresh_signs_the_exact_bytes_it_sends() {
        let (base, task) = stub(r#"{"access_token":"a","refresh_token":"r2","expires_at_ms":1,"refresh_expires_at_ms":2,"github_user_id":42,"login":"octo"}"#).await;
        let grant = refresh(&reqwest::Client::new(), &caller_at(base), "ghr_OLD").await.unwrap();
        assert_eq!(grant.refresh_token, "r2");
        let (line, ts, sig, body) = task.await.unwrap();
        assert_eq!(line, "POST /github/v1/token HTTP/1.1");
        assert_eq!(body, r#"{"grant":"refresh","refresh_token":"ghr_OLD"}"#);
        assert_eq!(sig, sign(&SEED, &signature_material("POST", "/github/v1/token", &ts, body.as_bytes())));
    }

    #[test]
    fn the_new_refusals_map_to_what_the_desktop_does_next() {
        let e = |status, v: Value| error_for(status, &v);
        assert_eq!(e(403, json!({"error":"install_not_approved"})), BrokerError::InstallNotApproved("install_not_approved".into()));
        assert_eq!(e(403, json!({"error":"not_linked"})), BrokerError::NotLinked);
        assert_eq!(e(403, json!({"error":"link_stale"})), BrokerError::LinkStale);
        assert_eq!(e(403, json!({"error":"repository_not_visible","repository":"secret-repo"})), BrokerError::RepositoryNotVisible("secret-repo".into()));
        assert_eq!(e(403, json!({"error":"permission_not_allowed","permission":"workflows"})), BrokerError::PermissionNotAllowed("workflows".into()));
        assert_eq!(e(400, json!({"error":"bad_request","message":"x"})), BrokerError::BadRequest("x".into()));
        assert_eq!(e(503, json!({"error":"github_not_configured"})), BrokerError::NotConfigured);
        // Still a brief outage, not "not configured".
        assert!(matches!(e(503, json!({"error":"github_unavailable"})), BrokerError::Unavailable(_)));
        assert!(matches!(e(429, json!({"error":"rate_limited"})), BrokerError::Unavailable(_)));
        // What the broker puts in a name is cut and cleaned before it reaches a message.
        let long = format!("a\u{7}{}", "b".repeat(300));
        match e(403, json!({"error":"repository_not_visible","repository":long})) {
            BrokerError::RepositoryNotVisible(r) => assert!(r.len() <= 100 && !r.contains('\u{7}'), "{r}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_installation_token_is_a_signed_post_that_names_the_repositories_and_permissions() {
        let (base, task) = stub(r#"{"token":"ghs_SECRET","expires_at":"2026-10-28T10:00:00Z","permissions":{"contents":"write"},"repository_ids":[9],"app":{"slug":"muxagent","bot_user_id":77}}"#).await;
        let permissions = std::collections::BTreeMap::from([("contents".to_string(), "write".to_string())]);
        let repositories = vec!["repo".to_string()];
        let request = InstallationRequest { installation_id: 5, repositories: &repositories, permissions: &permissions, agent_id: "korp" };
        let minted = installation_token(&reqwest::Client::new(), &caller_at(base), &request).await.unwrap();
        assert_eq!((minted.app.slug.as_str(), minted.app.bot_user_id, minted.repository_ids.clone()), ("muxagent", 77, vec![9]));
        assert_eq!(minted.expires_at_epoch(), Some(1_793_181_600));
        assert!(!format!("{minted:?}").contains("ghs_SECRET"), "the token must stay out of logs");
        let (line, ts, sig, body) = task.await.unwrap();
        assert_eq!(line, "POST /github/v1/installation-token HTTP/1.1");
        let sent: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(sent["installation_id"], 5);
        assert_eq!(sent["repositories"], json!(["repo"]));
        assert_eq!(sent["permissions"], json!({"contents":"write"}));
        assert_eq!(sent["agent_id"], "korp");
        assert_eq!(sig, sign(&SEED, &signature_material("POST", "/github/v1/installation-token", &ts, body.as_bytes())));
    }

    #[tokio::test]
    async fn the_installations_list_reads_the_links() {
        let (base, task) = stub(r#"{"installations":[{"installation_id":5,"github_account":{"id":1,"login":"Octo-Org","type":"Organization"},"repository_selection":"selected","installation_status":"active","link_status":"active","visible_count":2,"visible_repository_ids":[1,2],"verified_at_ms":10,"linked_at_ms":5}]}"#).await;
        let list = installations(&reqwest::Client::new(), &caller_at(base)).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].is_usable());
        assert_eq!(list[0].github_account.login, "Octo-Org");
        let (line, _, _, body) = task.await.unwrap();
        assert_eq!((line.as_str(), body.as_str()), ("GET /github/v1/installations HTTP/1.1", ""));
    }

    #[tokio::test]
    async fn a_connection_check_is_a_signed_get_with_no_body() {
        let (base, task) = stub(r#"{"connected":true,"github_user_id":42,"login":"octo","status":"active"}"#).await;
        let c = connection(&reqwest::Client::new(), &caller_at(base)).await.unwrap();
        assert_eq!((c.connected, c.login.as_deref(), c.status.as_str()), (true, Some("octo"), "active"));
        let (line, ts, sig, body) = task.await.unwrap();
        assert_eq!(line, "GET /github/v1/connection HTTP/1.1");
        assert_eq!(body, "");
        assert_eq!(sig, sign(&SEED, &signature_material("GET", "/github/v1/connection", &ts, b"")));
    }
}

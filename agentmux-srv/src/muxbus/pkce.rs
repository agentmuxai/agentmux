// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PKCE Authorization Code flow for Cognito desktop login.
//!
//! Flow (cloud-relayed callback — no loopback listener; see
//! docs/specs/SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15.md):
//!   1. Generate code_verifier + code_challenge (S256).
//!   2. Register the flow's state with the muxbus login relay.
//!   3. Open browser to Cognito hosted UI; it redirects to the hosted
//!      /desktop-callback page, which posts {state, code} to the relay.
//!   4. Poll the relay for the code (single-read on the relay side).
//!   5. Exchange code for tokens via /oauth2/token (PKCE verifier never
//!      left this process).
//!   6. Decode email from id_token claims (no re-verification needed —
//!      we fetched the token directly from Cognito).

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::backend::storage::muxbus::MuxBusCredentials;
use crate::util::open_browser;

pub const LOGIN_TIMEOUT_SECS: u64 = 300;

#[derive(Debug)]
pub struct PkceResult {
    pub credentials: MuxBusCredentials,
}

/// Base URL of the muxbus REST API hosting the login relay. The env override
/// exists for tests and for running against a local muxbus server
/// (`http://localhost:3100`, registered in the dev Cognito app client).
fn relay_base_url() -> String {
    std::env::var("AGENTMUX_MUXBUS_REST_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| crate::muxbus::cloud_subscriber::MUXBUS_REST_URL.to_string())
}

// Only one login flow may exist per process. Two concurrent flows both bind
// the fixed callback port, and (on Windows, where SO_REUSEADDR permits binding
// a port another live socket already holds) the browser's redirect lands on
// whichever listener the OS picks — a 400 state-mismatch coin flip. The newest
// attempt is the one the user actually wants: abort the predecessor.
static ACTIVE_LOGIN: std::sync::Mutex<Option<tokio::task::AbortHandle>> =
    std::sync::Mutex::new(None);

// Set immediately before cancel_active_login() aborts the task, so the
// aborted flow's own `Err(e) if e.is_cancelled()` branch can tell "the user
// clicked Cancel" apart from "a newer muxbus.login call superseded this one"
// — same underlying tokio abort, different message the frontend needs to
// react to differently (see HostPopover.tsx's Cancel button).
static CANCELLED_BY_USER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Abort the in-flight login flow, if any. Returns false if there was none
/// (a harmless no-op — the UI can call this without checking state first).
pub fn cancel_active_login() -> bool {
    let mut guard = ACTIVE_LOGIN.lock().unwrap();
    match guard.take() {
        Some(handle) => {
            CANCELLED_BY_USER.store(true, std::sync::atomic::Ordering::SeqCst);
            handle.abort();
            true
        }
        None => false,
    }
}

pub async fn run_pkce_login(
    cognito_domain: &str,
    client_id: &str,
    http_client: &reqwest::Client,
) -> Result<PkceResult, String> {
    // Abort-predecessor and register-successor must be one critical section:
    // with separate lock acquisitions, two concurrent muxbus.login RPCs can
    // both take() None and neither aborts the other (reagent P1 on this PR).
    // tokio::spawn is synchronous and non-blocking, so holding the std mutex
    // across it is fine — the guard never spans an await.
    let task = {
        let mut guard = ACTIVE_LOGIN.lock().unwrap();
        if let Some(prev) = guard.take() {
            prev.abort();
        }
        let task = tokio::spawn(run_pkce_login_inner(
            cognito_domain.to_string(),
            client_id.to_string(),
            http_client.clone(),
        ));
        *guard = Some(task.abort_handle());
        task
    };
    match task.await {
        Ok(r) => r,
        Err(e) if e.is_cancelled() => {
            if CANCELLED_BY_USER.swap(false, std::sync::atomic::Ordering::SeqCst) {
                Err("sign-in cancelled".to_string())
            } else {
                Err("this login attempt was superseded by a newer one".to_string())
            }
        }
        Err(e) => Err(format!("login task failed: {e}")),
    }
}

async fn run_pkce_login_inner(
    cognito_domain: String,
    client_id: String,
    http_client: reqwest::Client,
) -> Result<PkceResult, String> {
    // 1. Generate code_verifier (43–128 chars of URL-safe chars)
    let v1 = uuid::Uuid::new_v4();
    let v2 = uuid::Uuid::new_v4();
    let mut raw = [0u8; 32];
    raw[..16].copy_from_slice(v1.as_bytes());
    raw[16..].copy_from_slice(v2.as_bytes());
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);

    // 2. code_challenge = BASE64URL(SHA-256(code_verifier))
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let digest = hasher.finalize();
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

    // 3. State (CSRF)
    let state = uuid::Uuid::new_v4().to_string();

    // 4. Register the flow with the cloud relay BEFORE opening the browser.
    //    The hosted /desktop-callback page (agentmux-cloud, login-relay.ts)
    //    posts {state, code} there; step 7 polls it back out. No loopback
    //    listener: the browser never touches 127.0.0.1, and concurrent
    //    instances can't collide — each flow polls its own state key.
    //    See docs/specs/SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15.md.
    let relay_base = relay_base_url();
    let redirect_uri = format!("{relay_base}/desktop-callback");
    let create_resp = http_client
        .post(format!("{relay_base}/api/login-relay"))
        .json(&serde_json::json!({ "state": state }))
        .send()
        .await
        .map_err(|e| format!("could not reach the login relay: {e}"))?;
    if !create_resp.status().is_success() {
        let status = create_resp.status();
        let body = create_resp.text().await.unwrap_or_default();
        return Err(format!("login relay refused the flow ({status}): {body}"));
    }

    // 5. Build auth URL
    let scopes = "openid+email+profile+https%3A%2F%2Fmuxbus.agentmux.ai%2Fread+https%3A%2F%2Fmuxbus.agentmux.ai%2Fwrite";
    let auth_url = format!(
        "{cognito_domain}/oauth2/authorize\
         ?response_type=code\
         &client_id={client_id}\
         &redirect_uri={redirect_uri_enc}\
         &scope={scopes}\
         &code_challenge={code_challenge}\
         &code_challenge_method=S256\
         &state={state}",
        redirect_uri_enc = percent_encode(&redirect_uri),
    );

    // 6. Open browser (suppressed under test — the supersede test drives
    //    real flows that would otherwise launch real browser windows)
    if !cfg!(test) {
        open_browser(&auth_url);
    }

    tracing::info!(
        cognito_domain = cognito_domain,
        relay = relay_base,
        "muxbus: PKCE login started, awaiting relayed browser callback"
    );

    // 7. Poll the relay until the code arrives (5-min overall timeout, matching
    //    the relay record's TTL). Transient relay/network errors are tolerated
    //    and re-polled; a 404 is fatal — the flow expired or was already
    //    consumed (single-read on the relay side).
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(LOGIN_TIMEOUT_SECS),
        async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let resp = match http_client
                    .get(format!("{relay_base}/api/login-relay/{state}"))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(_) => continue, // transient network problem — keep polling
                };
                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err(
                        "login attempt expired or was completed elsewhere — please try again"
                            .to_string(),
                    );
                }
                if !resp.status().is_success() {
                    continue; // transient server error — keep polling
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match body["status"].as_str() {
                    Some("ready") => match body["code"].as_str() {
                        Some(c) => return Ok(c.to_string()),
                        None => return Err("relay returned ready without a code".to_string()),
                    },
                    // Cognito denial forwarded by the hosted page — fail fast
                    // rather than polling out the rest of the 5 minutes.
                    Some("failed") => {
                        let err = body["error"].as_str().unwrap_or("unknown");
                        return Err(format!("Cognito returned error: {err}"));
                    }
                    // "pending" or anything unrecognized — keep polling until
                    // the overall timeout says otherwise.
                    _ => continue,
                }
            }
        },
    )
    .await
    .map_err(|_| "login timed out (5 min) — please try again".to_string())??;

    // 8. Exchange code for tokens
    let token_url = format!("{cognito_domain}/oauth2/token");
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id.as_str()),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("code_verifier", &code_verifier),
    ];

    let resp = http_client
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed: {body}"));
    }

    let token_json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("token response parse failed: {e}"))?;

    let access_token = token_json["access_token"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let refresh_token = token_json["refresh_token"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let id_token = token_json["id_token"].as_str().unwrap_or("").to_string();
    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64)
        + expires_in;

    // 9. Extract email + sub from id_token payload (no re-verification needed)
    let (user_email, user_sub) = extract_jwt_claims(&id_token);

    tracing::info!(
        email = user_email,
        "muxbus: PKCE login succeeded"
    );

    Ok(PkceResult {
        credentials: MuxBusCredentials {
            cognito_domain: cognito_domain.to_string(),
            client_id: client_id.to_string(),
            access_token,
            refresh_token,
            id_token,
            expires_at,
            user_email,
            user_sub,
        },
    })
}

/// Typed so callers (the credential broker's registered `refresh` closure)
/// can distinguish "worth retrying" from "this refresh_token is dead, only
/// a fresh login fixes it" without string-sniffing an error message.
/// `Rejected` specifically means the token endpoint responded but rejected
/// the request (Cognito's `invalid_grant`-class 4xx for a revoked/expired
/// refresh_token) — the credential itself is the problem. `Network`/
/// `ParseFailed` mean the ATTEMPT failed, not the credential.
#[derive(Debug)]
pub enum RefreshTokenError {
    NoRefreshToken,
    Network(String),
    Rejected { status: u16, body: String },
    ParseFailed(String),
}

impl std::fmt::Display for RefreshTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefreshTokenError::NoRefreshToken => write!(f, "no refresh token stored"),
            RefreshTokenError::Network(e) => write!(f, "refresh request failed: {e}"),
            RefreshTokenError::Rejected { status, body } => {
                write!(f, "token refresh failed ({status}): {body}")
            }
            RefreshTokenError::ParseFailed(e) => write!(f, "refresh response parse failed: {e}"),
        }
    }
}

pub async fn refresh_token(
    creds: &MuxBusCredentials,
    http_client: &reqwest::Client,
) -> Result<MuxBusCredentials, RefreshTokenError> {
    if creds.refresh_token.is_empty() {
        return Err(RefreshTokenError::NoRefreshToken);
    }
    let token_url = format!("{}/oauth2/token", creds.cognito_domain);
    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", creds.client_id.as_str()),
        ("refresh_token", creds.refresh_token.as_str()),
    ];
    let resp = http_client
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| RefreshTokenError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(RefreshTokenError::Rejected { status, body });
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| RefreshTokenError::ParseFailed(e.to_string()))?;

    let access_token = json["access_token"].as_str().unwrap_or("").to_string();
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64)
        + expires_in;
    // Cognito doesn't rotate refresh_token on refresh — keep existing
    let new_id_token = json["id_token"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| creds.id_token.clone());

    Ok(MuxBusCredentials {
        cognito_domain: creds.cognito_domain.clone(),
        client_id: creds.client_id.clone(),
        access_token,
        refresh_token: creds.refresh_token.clone(),
        id_token: new_id_token,
        expires_at,
        user_email: creds.user_email.clone(),
        user_sub: creds.user_sub.clone(),
    })
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

fn extract_jwt_claims(token: &str) -> (String, String) {
    let payload = token.splitn(3, '.').nth(1).unwrap_or("");
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload))
        .unwrap_or_default();
    let json: serde_json::Value =
        serde_json::from_slice(&decoded).unwrap_or_default();
    let email = json["email"].as_str().unwrap_or("").to_string();
    let sub = json["sub"].as_str().unwrap_or("").to_string();
    (email, sub)
}


#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Minimal fake login relay: 201 on POST /api/login-relay, an eternally
    /// "pending" JSON body on GET polls. Keeps real flows parked at the
    /// poll stage with zero external network dependency.
    async fn spawn_fake_relay() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let Ok(n) = stream.read(&mut buf).await else { return };
                    let req = std::str::from_utf8(&buf[..n]).unwrap_or("");
                    let body = if req.starts_with("POST /api/login-relay") {
                        r#"{"status":"pending","ttl_seconds":300}"#
                    } else {
                        r#"{"status":"pending"}"#
                    };
                    let resp = format!(
                        "HTTP/1.1 {} 
Content-Type: application/json
Content-Length: {}
Connection: close

{}",
                        if req.starts_with("POST") { "201 Created" } else { "200 OK" },
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    // ACTIVE_LOGIN / CANCELLED_BY_USER are process-global statics — by
    // design, only one login flow may exist per process. Cargo runs tests
    // in this file on parallel threads of the same process by default, so
    // any two tests that drive a real flow through those statics race each
    // other unless serialized here.
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // The superseded-login guarantee: starting flow B while flow A is pending
    // must abort A rather than leaving two flows both polling (and both able
    // to open browser tabs). Uses the real run_pkce_login entry so the
    // abort-registry path itself is exercised; the fake relay keeps both
    // flows parked at the poll stage.
    #[tokio::test(flavor = "multi_thread")]
    async fn second_login_supersedes_first() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let relay = spawn_fake_relay().await;
        std::env::set_var("AGENTMUX_MUXBUS_REST_URL", &relay);

        let http = reqwest::Client::new();
        let flow_a = tokio::spawn({
            let http = http.clone();
            async move { run_pkce_login("http://127.0.0.1:1", "test-client", &http).await }
        });
        // Give A time to register + start polling before B supersedes it.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let flow_b = tokio::spawn({
            let http = http.clone();
            async move { run_pkce_login("http://127.0.0.1:1", "test-client", &http).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let a = tokio::time::timeout(std::time::Duration::from_secs(2), flow_a)
            .await
            .expect("flow A should resolve promptly after being superseded")
            .expect("flow A task must not panic");
        assert!(
            a.as_ref().is_err_and(|e| e.contains("superseded")),
            "flow A should report being superseded, got: {a:?}"
        );
        assert!(!flow_b.is_finished(), "flow B should still be polling the relay");

        // Cleanup: abort B's inner flow via the registry.
        if let Some(h) = ACTIVE_LOGIN.lock().unwrap().take() {
            h.abort();
        }
        flow_b.abort();
        std::env::remove_var("AGENTMUX_MUXBUS_REST_URL");
    }

    // A user-initiated Cancel must resolve the pending flow immediately
    // (not wait out LOGIN_TIMEOUT_SECS) and report a "cancelled" error
    // distinct from the supersede case above — otherwise the UI has no way
    // to tell "you cancelled this" from "a newer login attempt replaced
    // this", which is what src/statusbar/HostPopover.tsx's Cancel button
    // needs to show a quiet, non-error reset instead of a scary banner.
    #[tokio::test(flavor = "multi_thread")]
    async fn manual_cancel_resolves_promptly_with_cancelled_error() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let relay = spawn_fake_relay().await;
        std::env::set_var("AGENTMUX_MUXBUS_REST_URL", &relay);

        let http = reqwest::Client::new();
        let flow = tokio::spawn({
            let http = http.clone();
            async move { run_pkce_login("http://127.0.0.1:1", "test-client", &http).await }
        });
        // Give the flow time to register + start polling before cancelling.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        assert!(cancel_active_login(), "expected an active login to cancel");

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), flow)
            .await
            .expect("cancel should resolve the flow promptly, not after the 5-min timeout")
            .expect("flow task must not panic");
        assert!(
            result.as_ref().is_err_and(|e| e.contains("cancelled")),
            "expected a cancellation error, got: {result:?}"
        );

        std::env::remove_var("AGENTMUX_MUXBUS_REST_URL");
    }

    // Cancelling with no active login is a harmless no-op the UI can call
    // defensively without checking state first.
    #[tokio::test]
    async fn cancel_with_no_active_login_is_a_noop() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!cancel_active_login());
    }
}

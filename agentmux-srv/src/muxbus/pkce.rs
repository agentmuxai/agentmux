// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PKCE Authorization Code flow for Cognito desktop login.
//!
//! Flow:
//!   1. Generate code_verifier + code_challenge (S256).
//!   2. Bind fixed port 9379 for the redirect_uri (Cognito does exact-string
//!      matching only — no wildcard port support despite RFC 8252 §8.3).
//!   3. Open browser to Cognito hosted UI.
//!   4. Await HTTP callback (code + state).
//!   5. Exchange code for tokens via /oauth2/token.
//!   6. Decode email from id_token claims (no re-verification needed —
//!      we fetched the token directly from Cognito).

use base64::Engine as _;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpSocket;

use crate::backend::storage::muxbus::MuxBusCredentials;

pub const LOGIN_TIMEOUT_SECS: u64 = 300;

#[derive(Debug)]
pub struct PkceResult {
    pub credentials: MuxBusCredentials,
}

pub async fn run_pkce_login(
    cognito_domain: &str,
    client_id: &str,
    http_client: &reqwest::Client,
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

    // 4. Bind fixed port 9379 — must match Cognito callbackUrls exactly.
    //    SO_REUSEADDR allows immediate re-login after a cancelled attempt whose
    //    socket is still in TIME_WAIT (would otherwise hit EADDRINUSE).
    const CALLBACK_PORT: u16 = 9379;
    let redirect_uri = format!("http://127.0.0.1:{CALLBACK_PORT}/callback");
    let tcp_sock = TcpSocket::new_v4()
        .map_err(|e| format!("failed to create callback socket: {e}"))?;
    tcp_sock
        .set_reuseaddr(true)
        .map_err(|e| format!("failed to set SO_REUSEADDR: {e}"))?;
    tcp_sock
        .bind(format!("127.0.0.1:{CALLBACK_PORT}").parse().unwrap())
        .map_err(|e| format!("failed to bind callback port {CALLBACK_PORT}: {e} — another process may be holding the port"))?;
    let listener = tcp_sock
        .listen(10)
        .map_err(|e| format!("failed to listen on callback port {CALLBACK_PORT}: {e}"))?;

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

    // 6. Open browser
    open_browser(&auth_url);

    tracing::info!(
        cognito_domain = cognito_domain,
        port = CALLBACK_PORT,
        "muxbus: PKCE login started, awaiting browser callback"
    );

    // 7. Accept connections until the valid Cognito callback arrives (5-min overall timeout).
    //    On a fixed well-known port, stray connections (browser prefetch, port probes) can
    //    arrive before Cognito's redirect. Each accepted connection is spawned into a task
    //    so slow/idle strays don't block accept() and delay the real callback.
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(LOGIN_TIMEOUT_SECS),
        async move {
            loop {
                tokio::select! {
                    biased;
                    result = result_rx.recv() => {
                        return result.unwrap_or_else(|| Err("callback channel closed".to_string()));
                    }
                    accept_res = listener.accept() => {
                        let (mut stream, _) = accept_res
                            .map_err(|e| format!("callback accept failed: {e}"))?;
                        let state_val = state.clone();
                        let tx = result_tx.clone();
                        tokio::spawn(async move {
                            let mut buf = [0u8; 4096];
                            let n = match tokio::time::timeout(
                                std::time::Duration::from_secs(2),
                                stream.read(&mut buf),
                            )
                            .await
                            {
                                Ok(Ok(n)) => n,
                                _ => return,
                            };
                            let request = std::str::from_utf8(&buf[..n]).unwrap_or("");
                            let first_line = request.lines().next().unwrap_or("");
                            let raw_path = first_line.split_whitespace().nth(1).unwrap_or("");
                            let (path_only, query) =
                                raw_path.split_once('?').unwrap_or((raw_path, ""));

                            if path_only != "/callback" {
                                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").await;
                                return;
                            }

                            let returned_state = query_param(query, "state").unwrap_or_default();
                            if returned_state != state_val {
                                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
                                return;
                            }

                            let html: &[u8] = if query_param(query, "code").is_some() {
                                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
                                  <html><body><h2>Connected to AgentMux Cloud.</h2>\
                                  <p>You can close this tab.</p></body></html>"
                            } else {
                                b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n\
                                  <html><body><h2>Login failed.</h2><p>Please retry from AgentMux.</p></body></html>"
                            };
                            let _ = stream.write_all(html).await;

                            let result = match query_param(query, "code") {
                                Some(c) => Ok(c),
                                None => {
                                    let err = query_param(query, "error")
                                        .unwrap_or_else(|| "unknown".to_string());
                                    Err(format!("Cognito returned error: {err}"))
                                }
                            };
                            let _ = tx.send(result).await;
                        });
                    }
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
        ("client_id", client_id),
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

pub async fn refresh_token(
    creds: &MuxBusCredentials,
    http_client: &reqwest::Client,
) -> Result<MuxBusCredentials, String> {
    if creds.refresh_token.is_empty() {
        return Err("no refresh token stored".to_string());
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
        .map_err(|e| format!("refresh request failed: {e}"))?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token refresh failed: {body}"));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("refresh response parse failed: {e}"))?;

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

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            Some(percent_decode(v))
        } else {
            None
        }
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

fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(hex);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

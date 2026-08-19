// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MuxBus cloud connectivity RPC handlers.
//!
//! Four commands:
//!   * `muxbus.login`        — PKCE browser flow (blocks until complete or timeout)
//!   * `muxbus.login.cancel` — abort an in-flight `muxbus.login` (e.g. user closed the browser)
//!   * `muxbus.status`       — current credential status
//!   * `muxbus.disconnect`   — clear stored credentials

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

pub const COMMAND_MUXBUS_LOGIN: &str = "muxbus.login";
pub const COMMAND_MUXBUS_LOGIN_CANCEL: &str = "muxbus.login.cancel";
pub const COMMAND_MUXBUS_STATUS: &str = "muxbus.status";
pub const COMMAND_MUXBUS_DISCONNECT: &str = "muxbus.disconnect";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MuxBusLoginReq {
    cognito_domain: String,
    client_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MuxBusLoginResp {
    success: bool,
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MuxBusLoginCancelResp {
    /// False when there was no in-flight login to cancel (already resolved,
    /// or never started) — not an error, just nothing to do.
    cancelled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MuxBusStatusResp {
    connected: bool,
    email: String,
    cognito_domain: String,
    expires_at: i64,
    valid: bool,
}

pub fn register_muxbus_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // muxbus.login — PKCE browser flow, returns when browser login completes
    let wstore_login = state.id_store.clone();
    let http_client_login = state.http_client.clone();
    engine.register_handler(
        COMMAND_MUXBUS_LOGIN,
        Box::new(move |data, _ctx| {
            let wstore = wstore_login.clone();
            let http = http_client_login.clone();
            Box::pin(async move {
                let req: MuxBusLoginReq = serde_json::from_value(data)
                    .map_err(|e| format!("muxbus.login: {e}"))?;

                match crate::muxbus::pkce::run_pkce_login(
                    &req.cognito_domain,
                    &req.client_id,
                    &http,
                )
                .await
                {
                    Ok(result) => {
                        // reagent P1: previously this only logged a warning on a
                        // save failure and still reported success — now that
                        // MuxBus tokens live in the OS keychain (which can
                        // genuinely fail: locked, no Secret Service daemon on
                        // headless Linux, permission denied), that meant the UI
                        // could show "logged in" for a credential that was never
                        // actually persisted anywhere. Report the real outcome.
                        // spawn_blocking — reagent P1 on #2260: muxbus_save
                        // does a synchronous OS-keychain write, which can
                        // hang on a slow/unresponsive Secret Service D-Bus
                        // daemon (headless Linux) and must not stall this
                        // tokio worker thread.
                        let email = result.credentials.user_email.clone();
                        let save_store = wstore.clone();
                        let save_result = tokio::task::spawn_blocking(move || {
                            save_store.muxbus_save(&result.credentials)
                        })
                        .await
                        .map_err(|e| format!("muxbus.login: save task: {e}"))?;
                        if let Err(e) = save_result {
                            tracing::warn!(error = %e, "muxbus.login: failed to save credentials");
                            let resp = MuxBusLoginResp {
                                success: false,
                                email: String::new(),
                                error: Some(format!("login succeeded but credentials couldn't be saved: {e}")),
                            };
                            return Ok(Some(serde_json::to_value(resp).unwrap()));
                        }
                        // Kick the cloud subscriber to open a WS with the new token
                        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                            sub.reload_token();
                        }
                        let resp = MuxBusLoginResp {
                            success: true,
                            email,
                            error: None,
                        };
                        Ok(Some(serde_json::to_value(resp).unwrap()))
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "muxbus.login: PKCE flow failed");
                        let resp = MuxBusLoginResp {
                            success: false,
                            email: String::new(),
                            error: Some(e),
                        };
                        Ok(Some(serde_json::to_value(resp).unwrap()))
                    }
                }
            })
        }),
    );

    // muxbus.login.cancel — abort an in-flight muxbus.login. The aborted
    // flow's own task.await (in run_pkce_login) is what actually resolves
    // the original muxbus.login RPC call with a "sign-in cancelled" error —
    // this handler just fires the abort and returns immediately, it does
    // not wait for that resolution.
    engine.register_handler(
        COMMAND_MUXBUS_LOGIN_CANCEL,
        Box::new(move |_data, _ctx| {
            Box::pin(async move {
                let cancelled = crate::muxbus::pkce::cancel_active_login();
                let resp = MuxBusLoginCancelResp { cancelled };
                Ok(Some(serde_json::to_value(resp).unwrap()))
            })
        }),
    );

    // muxbus.status — return current credential state
    let wstore_status = state.id_store.clone();
    engine.register_handler(
        COMMAND_MUXBUS_STATUS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_status.clone();
            Box::pin(async move {
                // spawn_blocking — reagent P1 on #2260: same
                // synchronous-keychain-read concern as muxbus.login's save.
                let load_store = wstore.clone();
                let load_result = tokio::task::spawn_blocking(move || load_store.muxbus_load())
                    .await
                    .map_err(|e| format!("muxbus.status: load task: {e}"))?;
                match load_result {
                    Ok(Some(creds)) => {
                        let valid = creds.is_valid();
                        let resp = MuxBusStatusResp {
                            connected: !creds.access_token.is_empty(),
                            email: creds.user_email,
                            cognito_domain: creds.cognito_domain,
                            expires_at: creds.expires_at,
                            valid,
                        };
                        Ok(Some(serde_json::to_value(resp).unwrap()))
                    }
                    Ok(None) => {
                        let resp = MuxBusStatusResp {
                            connected: false,
                            email: String::new(),
                            cognito_domain: String::new(),
                            expires_at: 0,
                            valid: false,
                        };
                        Ok(Some(serde_json::to_value(resp).unwrap()))
                    }
                    Err(e) => Err(format!("muxbus.status: {e}")),
                }
            })
        }),
    );

    // muxbus.disconnect — clear credentials
    let wstore_disconnect = state.id_store.clone();
    engine.register_handler(
        COMMAND_MUXBUS_DISCONNECT,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_disconnect.clone();
            Box::pin(async move {
                // spawn_blocking — reagent P1 on #2260: muxbus_clear does a
                // synchronous OS-keychain delete, same concern as every
                // other muxbus call site in this module.
                tokio::task::spawn_blocking(move || wstore.muxbus_clear())
                    .await
                    .map_err(|e| format!("muxbus.disconnect: task: {e}"))?
                    .map_err(|e| format!("muxbus.disconnect: {e}"))?;
                Ok(Some(serde_json::json!({})))
            })
        }),
    );
}

/// Inject MUXBUS_TOKEN into spawn env if credentials are stored and valid.
/// Token refresh is async — this path just injects whatever is currently stored.
/// Agents should re-spawn after the user refreshes via muxbus.login if the token expires.
///
/// `async` (not sync) and `spawn_blocking` internally — reagent P1 on #2260:
/// muxbus_load does a synchronous OS-keychain read, which can hang on a
/// slow/unresponsive Secret Service D-Bus daemon (headless Linux) and must
/// not stall the caller's tokio worker thread.
pub async fn inject_muxbus_env(
    wstore: &Arc<crate::backend::storage::store::Store>,
    env_vars: &mut std::collections::HashMap<String, String>,
) {
    let load_store = wstore.clone();
    let load_result = tokio::task::spawn_blocking(move || load_store.muxbus_load()).await;
    let creds = match load_result {
        Ok(Ok(Some(c))) => c,
        Ok(Ok(None)) => return,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "muxbus inject: failed to load credentials");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "muxbus inject: load task panicked");
            return;
        }
    };

    if creds.access_token.is_empty() {
        return;
    }

    if creds.is_valid() {
        env_vars.insert("MUXBUS_TOKEN".to_string(), creds.access_token.clone());
        env_vars.insert("MUXBUS_COGNITO_DOMAIN".to_string(), creds.cognito_domain.clone());
        tracing::debug!(email = creds.user_email, "muxbus: injected MUXBUS_TOKEN into spawn env");
    } else {
        tracing::warn!(
            email = creds.user_email,
            expires_at = creds.expires_at,
            "muxbus: token expired, skipping injection — user should reconnect via muxbus.login"
        );
    }
}

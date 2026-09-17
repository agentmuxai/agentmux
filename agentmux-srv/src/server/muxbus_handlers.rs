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
    let mstore_login = state.id_store.clone();
    let http_client_login = state.http_client.clone();
    engine.register_handler(
        COMMAND_MUXBUS_LOGIN,
        Box::new(move |data, _ctx| {
            let mstore = mstore_login.clone();
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
                        let save_store = mstore.clone();
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
                        // Kick the cloud subscriber to open a WS with the new
                        // token. On an isolated local-package channel
                        // (reagentx P1 on PR #3248, round 2), boot skipped
                        // `CloudSubscriber::init_global` entirely, so there is
                        // no subscriber to reload yet — an explicit, successful
                        // login is exactly the signal that should lazily bring
                        // one up now, rather than leaving `muxbus.login` report
                        // "success" while no WebSocket ever opens and agents
                        // silently never receive WAN injections. `init_global`
                        // itself is idempotent (`OnceLock::set`, no-ops if
                        // already initialized), so this is safe to call
                        // unconditionally rather than re-deriving "was this
                        // isolated at boot" here.
                        match crate::muxbus::cloud_subscriber::get_global_subscriber() {
                            Some(sub) => sub.reload_token(),
                            None => {
                                crate::muxbus::cloud_subscriber::CloudSubscriber::init_global(
                                    mstore.clone(),
                                );
                            }
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
    let mstore_status = state.id_store.clone();
    engine.register_handler(
        COMMAND_MUXBUS_STATUS,
        Box::new(move |_data, _ctx| {
            let mstore = mstore_status.clone();
            Box::pin(async move {
                // reagentx P0 on PR #3248, round 2: `frontend/app/statusbar/
                // HostPopover.tsx` mounts globally and polls this handler on
                // mount + every 60s, unconditionally — so an unguarded
                // `muxbus_load()` here hit Keychain within a minute of launch
                // on a fresh local build regardless of the boot-time and
                // agent-spawn fixes elsewhere in this PR, defeating the whole
                // point of it. Same subscriber-presence gate as
                // `inject_muxbus_env` (see that function's doc comment for
                // why presence, not the static channel flag, is the right
                // signal): before any MuxBus session has ever been
                // established on this process, report "not connected"
                // without touching the keychain at all — identical to the
                // real Ok(None) response below, just without the read.
                if crate::muxbus::cloud_subscriber::get_global_subscriber().is_none() {
                    let resp = MuxBusStatusResp {
                        connected: false,
                        email: String::new(),
                        cognito_domain: String::new(),
                        expires_at: 0,
                        valid: false,
                    };
                    return Ok(Some(serde_json::to_value(resp).unwrap()));
                }
                // spawn_blocking — reagent P1 on #2260: same
                // synchronous-keychain-read concern as muxbus.login's save.
                let load_store = mstore.clone();
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
    let mstore_disconnect = state.id_store.clone();
    engine.register_handler(
        COMMAND_MUXBUS_DISCONNECT,
        Box::new(move |_data, _ctx| {
            let mstore = mstore_disconnect.clone();
            Box::pin(async move {
                // spawn_blocking — reagent P1 on #2260: muxbus_clear does a
                // synchronous OS-keychain delete, same concern as every
                // other muxbus call site in this module.
                tokio::task::spawn_blocking(move || mstore.muxbus_clear())
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
///
/// Skipped when `muxbus::cloud_subscriber::get_global_subscriber()` is
/// `None` — i.e. before ANY MuxBus session has ever been established on
/// this process (reagentx P0/P1 on PR #3248, round 2). This call site
/// runs on EVERY agent spawn, not just MuxBus-related ones — without
/// this gate, the automatic `muxbus_load()` here would still prompt for
/// Keychain consent on a fresh local build the moment ANY agent is
/// opened, even after the startup-reconnect prompt was already fixed
/// (see docs/retro/retro-macos-0560-stale-cef-cache-launch-crash-2026-09-16.md).
///
/// Deliberately checks subscriber presence rather than re-deriving
/// `isolated_muxbus_reconnect_enabled()` here: on `stable`/`dev-*`
/// channels the subscriber is always initialized at boot, so this is
/// equivalent there — but on an isolated local-package channel, the
/// subscriber starts `None` and only becomes `Some` the moment the user
/// explicitly completes `muxbus.login` (which lazily initializes it,
/// see that handler). Gating on the channel flag directly would have
/// kept this injection dead for the rest of the process's life even
/// after a real, successful login — the user would need to fully
/// restart the app to see it take effect. Gating on subscriber presence
/// means injection starts working on the very next agent spawn after
/// login, same session, no restart.
pub async fn inject_muxbus_env(
    mstore: &Arc<crate::backend::storage::store::Store>,
    env_vars: &mut std::collections::HashMap<String, String>,
) {
    if crate::muxbus::cloud_subscriber::get_global_subscriber().is_none() {
        return;
    }
    let load_store = mstore.clone();
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

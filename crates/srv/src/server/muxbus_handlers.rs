// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MuxBus cloud connectivity RPC handlers.
//!
//! Five commands:
//!   * `muxbus.login`        — PKCE browser flow (blocks until complete or timeout)
//!   * `muxbus.login.cancel` — abort an in-flight `muxbus.login` (e.g. user closed the browser)
//!   * `muxbus.status`       — current credential status
//!   * `muxbus.disconnect`   — clear stored credentials
//!   * `muxbus.cloudconfig`  — the cloud settings srv resolved, and where they came from

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

pub const COMMAND_MUXBUS_LOGIN: &str = "muxbus.login";
pub const COMMAND_MUXBUS_LOGIN_CANCEL: &str = "muxbus.login.cancel";
pub const COMMAND_MUXBUS_STATUS: &str = "muxbus.status";
pub const COMMAND_MUXBUS_DISCONNECT: &str = "muxbus.disconnect";
pub const COMMAND_MUXBUS_CLOUDCONFIG: &str = "muxbus.cloudconfig";

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MuxBusLoginReq {
    pub cognito_domain: String,
    pub client_id: String,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MuxBusLoginResp {
    pub success: bool,
    pub email: String,
    /// skip_serializing_if, so the key is OMITTED on success rather than
    /// carrying null -- `error?: string`, which is what the hand-written
    /// inline type already said.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MuxBusLoginCancelResp {
    /// False when there was no in-flight login to cancel (already resolved,
    /// or never started) — not an error, just nothing to do.
    pub cancelled: bool,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MuxBusStatusResp {
    pub connected: bool,
    pub email: String,
    pub cognito_domain: String,
    #[ts(type = "number")]
    pub expires_at: i64,
    pub valid: bool,
    /// The stored sign-in can't work anymore and only a new sign-in helps:
    /// the refresh was refused, or it was issued by a client the cloud no
    /// longer publishes (SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.3).
    /// The UI shows "Sign in again" with `email`.
    pub needs_reauth: bool,
    /// Whether cloud messages are reaching this channel's agents
    /// (`muxbus::delivery_status`); live changes arrive as the
    /// `muxbus:status` event. Always present from this srv.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub delivery: Option<crate::backend::rpc_types::MuxBusDeliveryStatus>,
}

/// Empty request shapes for `muxbus.login.cancel`, `muxbus.status` and
/// `muxbus.disconnect`. The handlers ignore their payload, but these must be
/// structs rather than `()`: the stub calls all three with `{}`, and serde
/// deserializes `()` only from JSON `null`, so a unit Req would reject every
/// real call at runtime while compiling and passing every CI gate -- the
/// `bookmarks.list` bug.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MuxBusLoginCancelReq {}

#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MuxBusStatusReq {}

#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MuxBusDisconnectReq {}

/// Result of `muxbus.disconnect`. Was an inline `json!({})`; the stub already
/// typed it `Record<string, never>`, which is exactly what an empty struct
/// generates.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MuxBusDisconnectResp {}

/// Empty request for `muxbus.cloudconfig`: a struct rather than `()` so the
/// stub's `{}` deserializes, like the request shapes above.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MuxBusCloudConfigReq {}

/// Where `muxbus.cloudconfig`'s settings came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum MuxBusCloudConfigSource {
    /// The relay's `/.well-known/agentmux-cloud.json` (the last good copy
    /// when a later fetch failed).
    Discovered,
    /// No document: srv's compiled relay URLs, and no sign-in settings.
    Default,
}

/// Result of `muxbus.cloudconfig`: the AgentMux Cloud settings srv resolved
/// (SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.2). A build compiled
/// without a Cognito client id asks this whether sign-in is possible anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct MuxBusCloudConfigResp {
    pub source: MuxBusCloudConfigSource,
    /// The relay REST base srv uses. Always `relay::rest_base_url()`, the
    /// root of trust the document is fetched from, whatever the document says.
    pub api: String,
    /// The relay WebSocket URL the cloud subscriber connects to.
    pub ws: String,
    /// The Cloud Console URL; empty when none is published.
    pub console: String,
    /// The Cognito hosted-UI domain and desktop client id `muxbus.login`
    /// prefers over the ones in its request. Empty with `source: "default"`:
    /// srv has no compiled sign-in settings, only the frontend build does.
    pub cognito_domain: String,
    pub client_id: String,
    pub region: String,
    pub user_pool_id: String,
}

/// What `muxbus.cloudconfig` reports for a discovery result.
fn cloud_config(discovered: Option<crate::muxbus::discovery::CloudSettings>, rest_base: String) -> MuxBusCloudConfigResp {
    match discovered {
        Some(s) => MuxBusCloudConfigResp {
            source: MuxBusCloudConfigSource::Discovered,
            api: rest_base,
            ws: s.ws,
            console: s.console,
            cognito_domain: s.cognito.domain,
            client_id: s.cognito.client_id,
            region: s.cognito.region,
            user_pool_id: s.cognito.user_pool_id,
        },
        None => MuxBusCloudConfigResp {
            source: MuxBusCloudConfigSource::Default,
            api: rest_base,
            ws: crate::muxbus::cloud_subscriber::MUXBUS_WS_URL.to_string(),
            console: String::new(),
            cognito_domain: String::new(),
            client_id: String::new(),
            region: String::new(),
            user_pool_id: String::new(),
        },
    }
}

pub fn register_muxbus_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // muxbus.login — PKCE browser flow, returns when browser login completes
    let mstore_login = state.id_store.clone();
    let http_client_login = state.http_client.clone();
    engine.register_typed(
        COMMAND_MUXBUS_LOGIN,
        move |req: MuxBusLoginReq, _ctx| {
            let mstore = mstore_login.clone();
            let http = http_client_login.clone();
            async move {
                // The cloud's published sign-in settings win over the build's
                // compiled ones in the request, so a new user pool reaches
                // installed builds (SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.2).
                let discovered = crate::muxbus::discovery::cloud_settings(&http).await;
                let (cognito_domain, client_id) = match crate::muxbus::discovery::resolve_login(
                    discovered.as_ref(),
                    &req.cognito_domain,
                    &req.client_id,
                ) {
                    Ok(pair) => pair,
                    // Same shape as a failed PKCE flow, so the UI shows it the same way.
                    Err(e) => {
                        return Ok(MuxBusLoginResp { success: false, email: String::new(), error: Some(e) });
                    }
                };

                match crate::muxbus::pkce::run_pkce_login(
                    &cognito_domain,
                    &client_id,
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
                            return Ok(resp);
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
                        // W3-S: a login may be an account switch — forget
                        // peer records fetched under the old account (§2.3),
                        // and publish any keys that were waiting on a login.
                        // A login to another relay or account is a directory
                        // that has never seen this install's keys; publications
                        // are recorded per directory, so the pass this nudge
                        // starts finds them all pending there.
                        crate::muxbus::delivery_status::fresh_sign_in(Some(email.clone()));
                        clear_wan_peer_cache();
                        crate::muxbus::wan_publish::nudge();
                        // And publish this install's presence now, whatever
                        // the presence publisher was waiting out.
                        crate::muxbus::wan_presence::sign_in_changed();
                        let resp = MuxBusLoginResp {
                            success: true,
                            email,
                            error: None,
                        };
                        Ok(resp)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "muxbus.login: PKCE flow failed");
                        let resp = MuxBusLoginResp {
                            success: false,
                            email: String::new(),
                            error: Some(e),
                        };
                        Ok(resp)
                    }
                }
            }
        },
    );

    // muxbus.login.cancel — abort an in-flight muxbus.login. The aborted
    // flow's own task.await (in run_pkce_login) is what actually resolves
    // the original muxbus.login RPC call with a "sign-in cancelled" error —
    // this handler just fires the abort and returns immediately, it does
    // not wait for that resolution.
    engine.register_typed(
        COMMAND_MUXBUS_LOGIN_CANCEL,
        move |_req: MuxBusLoginCancelReq, _ctx| {
            async move {
                let cancelled = crate::muxbus::pkce::cancel_active_login();
                let resp = MuxBusLoginCancelResp { cancelled };
                Ok(resp)
            }
        },
    );

    // muxbus.status — return current credential state
    let mstore_status = state.id_store.clone();
    let http_client_status = state.http_client.clone();
    engine.register_typed(
        COMMAND_MUXBUS_STATUS,
        move |_req: MuxBusStatusReq, _ctx| {
            let mstore = mstore_status.clone();
            let http = http_client_status.clone();
            async move {
                // reagentx P0 on PR #3248, round 2: `frontend/app/statusbar/
                // HostPopover.tsx` mounts globally and polls this handler on
                // mount + every 60s, unconditionally — so an unguarded
                // `muxbus_load()` here hit Keychain within a minute of launch
                // on a fresh local build regardless of the boot-time and
                // agent-spawn fixes elsewhere in this PR, defeating the whole
                // point of it. Gated on subscriber presence, not the static
                // channel flag: on an isolated channel the subscriber only
                // starts at an explicit `muxbus.login`, so presence is what
                // says a session exists. Before any MuxBus session has ever been
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
                        needs_reauth: false,
                        delivery: Some(crate::muxbus::delivery_status::status()),
                    };
                    return Ok(resp);
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
                        let needs_reauth = crate::muxbus::stale_sign_in(&creds, &http).await.is_some();
                        let resp = MuxBusStatusResp {
                            connected: !creds.access_token.is_empty(),
                            email: creds.user_email,
                            cognito_domain: creds.cognito_domain,
                            expires_at: creds.expires_at,
                            valid,
                            needs_reauth,
                            delivery: Some(crate::muxbus::delivery_status::status()),
                        };
                        Ok(resp)
                    }
                    Ok(None) => {
                        let resp = MuxBusStatusResp {
                            connected: false,
                            email: String::new(),
                            cognito_domain: String::new(),
                            expires_at: 0,
                            valid: false,
                            needs_reauth: false,
                            delivery: Some(crate::muxbus::delivery_status::status()),
                        };
                        Ok(resp)
                    }
                    // The UI already reads a failed status as disconnected;
                    // answering that shape keeps it so, and still carries
                    // `delivery`, which is what says the sign-in is unreadable.
                    Err(e) => {
                        tracing::warn!(error = %e, "muxbus.status: stored sign-in unreadable");
                        Ok(MuxBusStatusResp {
                            connected: false,
                            email: String::new(),
                            cognito_domain: String::new(),
                            expires_at: 0,
                            valid: false,
                            needs_reauth: false,
                            delivery: Some(crate::muxbus::delivery_status::status()),
                        })
                    }
                }
            }
        },
    );

    // muxbus.disconnect — clear credentials
    let mstore_disconnect = state.id_store.clone();
    engine.register_typed(
        COMMAND_MUXBUS_DISCONNECT,
        move |_req: MuxBusDisconnectReq, _ctx| {
            let mstore = mstore_disconnect.clone();
            async move {
                // spawn_blocking — reagent P1 on #2260: muxbus_clear does a
                // synchronous OS-keychain delete, same concern as every
                // other muxbus call site in this module.
                tokio::task::spawn_blocking(move || mstore.muxbus_clear())
                    .await
                    .map_err(|e| format!("muxbus.disconnect: task: {e}"))?
                    .map_err(|e| format!("muxbus.disconnect: {e}"))?;
                clear_wan_peer_cache();
                crate::muxbus::wan_presence::sign_in_changed();
                // A deliberate sign-out, not a lost sign-in: no notification,
                // no agent notes. And close the relay connection, which would
                // otherwise keep delivering on the old token until it ends.
                crate::muxbus::delivery_status::signed_out();
                if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                    sub.reload_token();
                }
                Ok(MuxBusDisconnectResp {})
            }
        },
    );

    // muxbus.cloudconfig — the resolved cloud settings. No credential or
    // keychain access; discovery itself is cached (`discovery::cloud_settings`).
    let http_client_cloudconfig = state.http_client.clone();
    engine.register_typed(
        COMMAND_MUXBUS_CLOUDCONFIG,
        move |_req: MuxBusCloudConfigReq, _ctx| {
            let http = http_client_cloudconfig.clone();
            async move {
                let discovered = crate::muxbus::discovery::cloud_settings(&http).await;
                Ok(cloud_config(discovered, crate::muxbus::relay::rest_base_url()))
            }
        },
    );
}

/// W3-S (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.3): cached peer
/// records belong to the account they were fetched under. Best-effort — the
/// records are self-certifying, so a stale one can't verify a forgery; this
/// only keeps one account's directory from answering for another's.
fn clear_wan_peer_cache() {
    if let Some(wan) = crate::backend::storage::wan_identity::global() {
        if let Err(e) = wan.peer_cache_clear() {
            tracing::warn!(error = %e, "wan identity: could not clear the peer-record cache");
        }
    }
}

// Request-shape tests for the `muxbus.*` commands.
//
// These types were private to this file until now, so the frontend's inline
// copies were hand-maintained against nothing.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    // The stub sends camelCase keys; the Rust fields are snake_case and rely
    // on `rename_all = "camelCase"`. If that attribute were ever dropped the
    // struct would still compile and the binding would still generate -- it
    // would just silently stop parsing real payloads. Pin it.
    #[test]
    fn login_accepts_the_camel_case_payload_the_stub_sends() {
        let r: MuxBusLoginReq =
            serde_json::from_value(json!({"cognitoDomain": "d", "clientId": "c"}))
                .expect("muxbus.login must accept camelCase keys");
        assert_eq!((r.cognito_domain.as_str(), r.client_id.as_str()), ("d", "c"));

        assert!(
            serde_json::from_value::<MuxBusLoginReq>(json!({"cognito_domain": "d", "client_id": "c"}))
                .is_err(),
            "snake_case keys are NOT the wire format here"
        );
    }

    // All three payload-ignoring commands are called with `{}`. A unit Req
    // would reject that (serde deserializes `()` only from null).
    #[test]
    fn the_payload_ignoring_commands_accept_the_empty_object() {
        serde_json::from_value::<MuxBusLoginCancelReq>(json!({})).expect("login.cancel");
        serde_json::from_value::<MuxBusStatusReq>(json!({})).expect("status");
        serde_json::from_value::<MuxBusDisconnectReq>(json!({})).expect("disconnect");
        serde_json::from_value::<MuxBusCloudConfigReq>(json!({})).expect("cloudconfig");
        assert!(serde_json::from_value::<()>(json!({})).is_err());
    }

    // `error` is skip_serializing_if, so a successful login OMITS the key
    // rather than sending null -- which is why the binding says `error?`.
    #[test]
    fn login_response_omits_error_on_success() {
        let ok = serde_json::to_value(MuxBusLoginResp {
            success: true,
            email: "a@b.c".to_string(),
            error: None,
        })
        .expect("serializable");
        assert_eq!(ok, json!({"success": true, "email": "a@b.c"}));
    }

    // disconnect answered with an inline `json!({})`; the stub already typed it
    // `Record<string, never>`, which is what an empty struct generates.
    #[test]
    fn disconnect_response_is_an_empty_object() {
        assert_eq!(serde_json::to_value(MuxBusDisconnectResp {}).unwrap(), json!({}));
    }

    fn published() -> crate::muxbus::discovery::CloudSettings {
        crate::muxbus::discovery::parse(
            r#"{"version":1,"api":"https://api.example.test","ws":"wss://ws.example.test","console":"https://console.example.test",
                "cognito":{"domain":"https://auth.example.test","clientId":"abc123","region":"us-east-1","userPoolId":"us-east-1_X"}}"#,
            "https://relay.example.test",
        )
        .unwrap()
    }

    // SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.2: a published document
    // supplies the sign-in settings and the WebSocket URL; REST stays srv's.
    #[test]
    fn cloudconfig_reports_a_published_document_as_discovered() {
        let r = cloud_config(Some(published()), "https://relay.example.test".to_string());
        assert_eq!(r.source, MuxBusCloudConfigSource::Discovered);
        assert_eq!(r.api, "https://relay.example.test", "REST is never taken from the document");
        assert_eq!(r.ws, "wss://ws.example.test");
        assert_eq!(r.console, "https://console.example.test");
        assert_eq!(r.cognito_domain, "https://auth.example.test");
        assert_eq!(r.client_id, "abc123");
        assert_eq!(r.region, "us-east-1");
        assert_eq!(r.user_pool_id, "us-east-1_X");
    }

    // No document (an older relay, or discovery failed with no earlier copy):
    // srv's compiled relay URLs, and no client id, so a build without its
    // own can't offer sign-in.
    #[test]
    fn cloudconfig_without_a_document_reports_the_defaults() {
        let r = cloud_config(None, "https://relay.example.test".to_string());
        assert_eq!(r.source, MuxBusCloudConfigSource::Default);
        assert_eq!(r.api, "https://relay.example.test");
        assert_eq!(r.ws, crate::muxbus::cloud_subscriber::MUXBUS_WS_URL);
        assert!(r.client_id.is_empty() && r.cognito_domain.is_empty() && r.console.is_empty());
    }

    // The wire shape the frontend reads: camelCase keys, lowercase source.
    #[test]
    fn cloudconfig_serializes_camel_case_with_a_lowercase_source() {
        let v = serde_json::to_value(cloud_config(Some(published()), "https://relay.example.test".to_string())).unwrap();
        assert_eq!(v["source"], "discovered");
        assert_eq!(v["clientId"], "abc123");
        assert_eq!(v["cognitoDomain"], "https://auth.example.test");
        assert_eq!(v["userPoolId"], "us-east-1_X");
        let v = serde_json::to_value(cloud_config(None, String::new())).unwrap();
        assert_eq!(v["source"], "default");
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pre-launch OAuth flow RPC handlers.
//!
//! Five commands per `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §7:
//!
//!   * `auth.start`           — `StartProviderAuth`
//!   * `auth.poll`            — `PollProviderAuth`
//!   * `auth.submitcallback`  — `SubmitAuthCallback`
//!   * `auth.cancel`          — `CancelProviderAuth`
//!   * `auth.submitapikey`    — `SubmitProviderApiKey`
//!
//! This module owns the RPC surface and dispatches into
//! `crate::identity::auth_session::AuthSessionManager`. The actual
//! provider-CLI spawn (which feeds stdout lines into the session
//! via `record_line`) lands in a follow-up commit on this branch —
//! these handlers ship the lifecycle plumbing and return clear
//! "spawn not yet wired" errors for the callback/api-key paths so
//! frontend (PR B) can integrate against the real shape.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

pub const COMMAND_AUTH_START: &str = "auth.start";
pub const COMMAND_AUTH_POLL: &str = "auth.poll";
pub const COMMAND_AUTH_SUBMIT_CALLBACK: &str = "auth.submitcallback";
pub const COMMAND_AUTH_CANCEL: &str = "auth.cancel";
pub const COMMAND_AUTH_SUBMIT_API_KEY: &str = "auth.submitapikey";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartProviderAuthReq {
    provider_id: String,
    /// Optional — when set, a successful auth adds an account to the
    /// existing bundle. When None, a fresh bundle is created.
    #[serde(default)]
    into_bundle_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollProviderAuthReq {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitAuthCallbackReq {
    session_id: String,
    /// The redirect URL the user pasted back (browser-didn't-open
    /// fallback). The handler writes it to the spawned CLI's stdin.
    callback_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelProviderAuthReq {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitProviderApiKeyReq {
    provider_id: String,
    /// Optional — same semantics as StartProviderAuth.into_bundle_id.
    #[serde(default)]
    into_bundle_id: Option<String>,
    /// The API key the user pasted. Will be validated by running the
    /// provider's authCheckCommand against it, then persisted as a
    /// `SecretRef::PlaintextDev` (PR 1 of the storage spec) or
    /// encrypted variant (PR 2).
    api_key: String,
    account_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AckResp {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn register_identity_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mgr = state.auth_session_manager.clone();
    engine.register_handler(
        COMMAND_AUTH_START,
        Box::new(move |data, _ctx| {
            let mgr = mgr.clone();
            Box::pin(async move {
                let req: StartProviderAuthReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.start: {e}"))?;
                tracing::info!(
                    provider_id = %req.provider_id,
                    into_bundle_id = ?req.into_bundle_id,
                    "auth.start"
                );
                let r = mgr.start_session(req.provider_id, req.into_bundle_id);
                Ok(Some(serde_json::to_value(&r).unwrap_or_default()))
            })
        }),
    );

    let mgr = state.auth_session_manager.clone();
    engine.register_handler(
        COMMAND_AUTH_POLL,
        Box::new(move |data, _ctx| {
            let mgr = mgr.clone();
            Box::pin(async move {
                let req: PollProviderAuthReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.poll: {e}"))?;
                let snap = mgr
                    .poll_session(&req.session_id)
                    .ok_or_else(|| format!("auth.poll: unknown session {}", req.session_id))?;
                Ok(Some(serde_json::to_value(&snap).unwrap_or_default()))
            })
        }),
    );

    let mgr = state.auth_session_manager.clone();
    engine.register_handler(
        COMMAND_AUTH_CANCEL,
        Box::new(move |data, _ctx| {
            let mgr = mgr.clone();
            Box::pin(async move {
                let req: CancelProviderAuthReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.cancel: {e}"))?;
                let ok = mgr.cancel_session(&req.session_id);
                Ok(Some(
                    serde_json::to_value(&AckResp {
                        success: ok,
                        error: if ok {
                            None
                        } else {
                            Some(format!("unknown or already-terminal session: {}", req.session_id))
                        },
                    })
                    .unwrap_or_default(),
                ))
            })
        }),
    );

    // The next-commit-on-this-branch CLI spawn integration replaces
    // these stubs. Returning a structured error lets frontend (PR B)
    // exercise the unimplemented path explicitly during integration
    // tests rather than getting an opaque server crash.
    engine.register_handler(
        COMMAND_AUTH_SUBMIT_CALLBACK,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let _req: SubmitAuthCallbackReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.submitcallback: {e}"))?;
                // TODO(PR A v2): write callback_url to spawned CLI's
                // stdin (browser-didn't-open path). Until then,
                // surface a clear "not yet wired" so frontend
                // integration tests can assert against it.
                Err::<Option<serde_json::Value>, String>(
                    "auth.submitcallback: stdin injection lands in a follow-up commit"
                        .to_string(),
                )
            })
        }),
    );

    engine.register_handler(
        COMMAND_AUTH_SUBMIT_API_KEY,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let _req: SubmitProviderApiKeyReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.submitapikey: {e}"))?;
                // TODO(PR A v2): run provider's authCheckCommand with
                // the api_key in the appropriate env var; on success,
                // persist via wstore as a new IdentityAccount row
                // with kind=api_key + SecretRef::PlaintextDev.
                Err::<Option<serde_json::Value>, String>(
                    "auth.submitapikey: provider validation lands in a follow-up commit"
                        .to_string(),
                )
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // Request shape parsing tests — verify the wire contract matches
    // what the frontend (PR B) will send. The end-to-end RPC-engine
    // invocation is covered at the integration level once the CLI
    // spawn lands; the underlying AuthSessionManager behaviour is
    // covered in `identity::auth_session::tests`.

    #[test]
    fn start_req_parses_minimal() {
        let v = serde_json::json!({ "providerId": "claude" });
        let r: StartProviderAuthReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.provider_id, "claude");
        assert!(r.into_bundle_id.is_none());
    }

    #[test]
    fn start_req_parses_with_bundle_id() {
        let v = serde_json::json!({ "providerId": "codex", "intoBundleId": "bundle-1" });
        let r: StartProviderAuthReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.provider_id, "codex");
        assert_eq!(r.into_bundle_id.as_deref(), Some("bundle-1"));
    }

    #[test]
    fn poll_req_round_trips() {
        let v = serde_json::json!({ "sessionId": "auth-xyz" });
        let r: PollProviderAuthReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.session_id, "auth-xyz");
    }

    #[test]
    fn submit_callback_req_round_trips() {
        let v = serde_json::json!({
            "sessionId": "auth-xyz",
            "callbackUrl": "https://example.com/cb?code=abc"
        });
        let r: SubmitAuthCallbackReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.session_id, "auth-xyz");
        assert!(r.callback_url.contains("code=abc"));
    }

    #[test]
    fn submit_api_key_req_round_trips() {
        let v = serde_json::json!({
            "providerId": "openclaw",
            "apiKey": "sk-test",
            "accountName": "my-key"
        });
        let r: SubmitProviderApiKeyReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.provider_id, "openclaw");
        assert_eq!(r.api_key, "sk-test");
        assert_eq!(r.account_name, "my-key");
        assert!(r.into_bundle_id.is_none());
    }

    #[test]
    fn ack_resp_omits_error_when_success() {
        let r = AckResp { success: true, error: None };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, serde_json::json!({ "success": true }));
    }

    #[test]
    fn ack_resp_includes_error_when_failure() {
        let r = AckResp { success: false, error: Some("boom".into()) };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, serde_json::json!({ "success": false, "error": "boom" }));
    }
}

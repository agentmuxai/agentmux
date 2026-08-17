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
//!
//! Split across four files (module-organization pass, see
//! `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`):
//! this file owns the RPC surface (wire types + `register_identity_handlers`);
//! `identity_auth_spawn` owns the subprocess spawn/drain engine
//! (`spawn_auth_cli`, `spawn_auth_cli_pty`, `confirm_authenticated`);
//! `identity_auth_dirs` owns filesystem/directory provisioning
//! (`compute_and_ensure_bundle_dir`, `compute_and_ensure_account_dir`);
//! `identity_auth_persist` owns `IdentityAccount` persistence on OAuth
//! success (`persist_oauth_direct_account`, `persist_oauth_success`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc::engine::WshRpcEngine;

use super::identity_auth_dirs::{compute_and_ensure_account_dir, compute_and_ensure_bundle_dir};
use super::identity_auth_spawn::spawn_auth_cli;
use super::AppState;

pub const COMMAND_AUTH_START: &str = "auth.start";
pub const COMMAND_AUTH_POLL: &str = "auth.poll";
pub const COMMAND_AUTH_SUBMIT_CALLBACK: &str = "auth.submitcallback";
pub const COMMAND_AUTH_CANCEL: &str = "auth.cancel";
pub const COMMAND_AUTH_SUBMIT_API_KEY: &str = "auth.submitapikey";
/// Mints (or reuses) a per-account isolated config dir without spawning any
/// CLI — a standalone entry point onto `compute_and_ensure_account_dir` for
/// callers that seed a credential file directly (`seed_provider_auth_from_global`)
/// instead of driving a fresh OAuth handshake through `auth.start`. See
/// `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` §7 —
/// "single point, not global": every credential-bearing dir a running agent
/// reads from must belong to a real `IdentityAccount` row, never the shared
/// default dir.
pub const COMMAND_ENSURE_ACCOUNT_DIR: &str = "identity.ensureaccountdir";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartProviderAuthReq {
    provider_id: String,
    /// Vestigial — bundle mode (a successful auth adding an account to
    /// an Identity bundle) was retired in Phase 4c of
    /// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md. Kept on the wire
    /// shape only; `direct_account` is always true in practice.
    #[serde(default)]
    into_bundle_id: Option<String>,
    /// Always `true` in practice — `AuthFlowController` (the sole
    /// frontend caller) hardcodes `directAccount: true`. A successful
    /// auth persists a standalone `IdentityAccount`; the actual
    /// agent<->account link is written later, once the agent exists, by
    /// the launch-flow reconcile.
    #[serde(default)]
    direct_account: bool,
    /// Direct-account reconnect: non-empty to refresh an already-
    /// linked account's tokens in place (same isolation dir, same
    /// account row updated). Empty mints a fresh account id. Ignored
    /// unless `direct_account` is set.
    #[serde(default)]
    existing_account_id: String,
    /// Resolved CLI path. The frontend calls `resolvecli` first to
    /// install / locate the provider's CLI; the resulting path is
    /// passed here. Keeps the provider table single-sourced in the
    /// frontend.
    cli_path: String,
    /// e.g. `["auth", "login"]` from the provider definition's
    /// `authLoginCommand` field.
    auth_login_args: Vec<String>,
    /// e.g. `["auth", "status", "--json"]` — used to confirm
    /// authentication after the CLI emits its success line.
    auth_check_args: Vec<String>,
    /// Env vars to inject at spawn time. Per-provider auth
    /// isolation env vars come here (CLAUDE_CONFIG_DIR, CODEX_HOME,
    /// GEMINI_CLI_HOME, etc.).
    #[serde(default)]
    auth_env: std::collections::HashMap<String, String>,
    /// Spawn the auth login subprocess under a PTY (instead of plain
    /// piped stdio). Required by providers whose auth subcommand
    /// checks `isatty()` and refuses to run otherwise (currently
    /// OpenClaw's `models auth login --provider <id>`). The flag is
    /// forwarded down to the CEF host's `run_cli_login` which picks
    /// the PTY branch when set.
    #[serde(default)]
    requires_tty: bool,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnsureAccountDirReq {
    provider_id: String,
    /// Non-empty to resolve an already-minted account's own dir
    /// (reconnect-in-place); empty mints a fresh account id.
    #[serde(default)]
    existing_account_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnsureAccountDirResp {
    account_id: String,
    /// `None` when the provider isn't oauth-class or the dir couldn't be
    /// resolved/created — same soft-failure contract as `auth.start`'s
    /// internal use of `compute_and_ensure_account_dir`.
    #[serde(skip_serializing_if = "Option::is_none")]
    dir: Option<String>,
}

pub fn register_identity_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mgr = state.auth_session_manager.clone();
    let wstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    let wstore_for_ensure_dir = wstore.clone();
    engine.register_handler(
        COMMAND_AUTH_START,
        Box::new(move |data, _ctx| {
            let mgr = mgr.clone();
            let wstore = wstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let req: StartProviderAuthReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.start: {e}"))?;
                tracing::info!(
                    provider_id = %req.provider_id,
                    cli_path = %req.cli_path,
                    into_bundle_id = ?req.into_bundle_id,
                    direct_account = req.direct_account,
                    "auth.start"
                );
                // `direct_account` is the only path actually exercised
                // (`AuthFlowController` hardcodes `directAccount: true`) —
                // mints/reuses an account id and resolves its own
                // isolation dir, mirroring the dir into `auth_env` under
                // the provider's `auth_config_dir_env_var` (e.g.
                // `CLAUDE_CONFIG_DIR`), overriding whatever the frontend
                // computed via the legacy ambient `ensureAuthDir` path.
                //
                // The `into_bundle_id`/`compute_and_ensure_bundle_dir`
                // branch below is vestigial — bundle mode was retired in
                // Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md
                // — kept only so this wire shape and its call sites don't
                // need touching.
                //
                // Errors (path resolve, mkdir) log + fall back to the
                // legacy env — never abort `auth.start` over a dir issue.
                // Mirrors the `inject_identity_env` pattern.
                let mut auth_env = req.auth_env;
                let (account_id, bundle_dir) = if req.direct_account {
                    let (account_id, dir) = compute_and_ensure_account_dir(
                        &wstore,
                        &req.existing_account_id,
                        &req.provider_id,
                        &mut auth_env,
                    );
                    (account_id, dir)
                } else {
                    let dir = compute_and_ensure_bundle_dir(
                        req.into_bundle_id.as_deref(),
                        &req.provider_id,
                        &mut auth_env,
                    );
                    (String::new(), dir)
                };
                let r = mgr.start_session(req.provider_id.clone(), req.into_bundle_id.clone());
                spawn_auth_cli(
                    mgr,
                    wstore,
                    identity_store,
                    broker,
                    r.session_id.clone(),
                    req.provider_id,
                    req.into_bundle_id,
                    bundle_dir,
                    req.direct_account,
                    account_id,
                    req.cli_path,
                    req.auth_login_args,
                    req.auth_check_args,
                    auth_env,
                    req.requires_tty,
                );
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

    let mgr = state.auth_session_manager.clone();
    engine.register_handler(
        COMMAND_AUTH_SUBMIT_CALLBACK,
        Box::new(move |data, _ctx| {
            let mgr = mgr.clone();
            Box::pin(async move {
                let req: SubmitAuthCallbackReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.submitcallback: {e}"))?;
                let delivered = mgr
                    .send_to_stdin(&req.session_id, req.callback_url)
                    .await;
                Ok(Some(
                    serde_json::to_value(&AckResp {
                        success: delivered,
                        error: if delivered {
                            None
                        } else {
                            Some(format!(
                                "no stdin sender for session {} (process exited or session unknown)",
                                req.session_id
                            ))
                        },
                    })
                    .unwrap_or_default(),
                ))
            })
        }),
    );

    engine.register_handler(
        COMMAND_AUTH_SUBMIT_API_KEY,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let _req: SubmitProviderApiKeyReq = serde_json::from_value(data)
                    .map_err(|e| format!("auth.submitapikey: {e}"))?;
                // API-key path validates by running the provider's
                // authCheckCommand with the key in the appropriate
                // env var, then persists via wstore. That persistence
                // is part of PR C (bundle auto-creation) per spec
                // §10 — the validate-and-stash logic is here but
                // wstore writes wait. For PR A we return an explicit
                // error so frontend (PR B) sees a clear "not yet"
                // signal while OAuth providers work end-to-end.
                Err::<Option<serde_json::Value>, String>(
                    "auth.submitapikey: bundle persistence lands in PR C"
                        .to_string(),
                )
            })
        }),
    );

    engine.register_handler(
        COMMAND_ENSURE_ACCOUNT_DIR,
        Box::new(move |data, _ctx| {
            let wstore = wstore_for_ensure_dir.clone();
            Box::pin(async move {
                let req: EnsureAccountDirReq = serde_json::from_value(data)
                    .map_err(|e| format!("identity.ensureaccountdir: {e}"))?;
                let mut auth_env = std::collections::HashMap::new();
                let (account_id, dir) = compute_and_ensure_account_dir(
                    &wstore,
                    &req.existing_account_id,
                    &req.provider_id,
                    &mut auth_env,
                );
                let resp = EnsureAccountDirResp { account_id, dir };
                Ok(Some(serde_json::to_value(&resp).unwrap_or_default()))
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
        let v = serde_json::json!({
            "providerId": "claude",
            "cliPath": "/usr/bin/claude",
            "authLoginArgs": ["login"],
            "authCheckArgs": ["whoami"]
        });
        let r: StartProviderAuthReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.provider_id, "claude");
        assert_eq!(r.cli_path, "/usr/bin/claude");
        assert_eq!(r.auth_login_args, vec!["login"]);
        assert_eq!(r.auth_check_args, vec!["whoami"]);
        assert!(r.into_bundle_id.is_none());
        assert!(r.auth_env.is_empty());
    }

    #[test]
    fn start_req_parses_with_bundle_id() {
        let v = serde_json::json!({
            "providerId": "codex",
            "cliPath": "/usr/bin/codex",
            "authLoginArgs": ["auth", "login"],
            "authCheckArgs": ["auth", "status"],
            "authEnv": { "FOO": "bar" },
            "intoBundleId": "bundle-1"
        });
        let r: StartProviderAuthReq = serde_json::from_value(v).unwrap();
        assert_eq!(r.provider_id, "codex");
        assert_eq!(r.into_bundle_id.as_deref(), Some("bundle-1"));
        assert_eq!(r.auth_env.get("FOO").map(String::as_str), Some("bar"));
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

    // ── Issue #1624 PR-C Part B: direct-account OAuth ─────────────

    #[test]
    fn start_req_parses_direct_account_fields() {
        let v = serde_json::json!({
            "providerId": "claude",
            "cliPath": "/usr/bin/claude",
            "authLoginArgs": ["login"],
            "authCheckArgs": ["whoami"],
            "directAccount": true,
            "existingAccountId": "acc-1"
        });
        let r: StartProviderAuthReq = serde_json::from_value(v).unwrap();
        assert!(r.direct_account);
        assert_eq!(r.existing_account_id, "acc-1");
    }

    #[test]
    fn start_req_direct_account_fields_default_when_absent() {
        // Same minimal payload as start_req_parses_minimal — must still
        // parse now that these two fields exist, defaulting to the
        // legacy bundle-mode shape.
        let v = serde_json::json!({
            "providerId": "claude",
            "cliPath": "/usr/bin/claude",
            "authLoginArgs": ["login"],
            "authCheckArgs": ["whoami"]
        });
        let r: StartProviderAuthReq = serde_json::from_value(v).unwrap();
        assert!(!r.direct_account);
        assert_eq!(r.existing_account_id, "");
    }
}

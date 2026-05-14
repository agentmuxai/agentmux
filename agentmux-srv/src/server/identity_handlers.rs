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
                    cli_path = %req.cli_path,
                    into_bundle_id = ?req.into_bundle_id,
                    "auth.start"
                );
                let r = mgr.start_session(req.provider_id.clone(), req.into_bundle_id);
                spawn_auth_cli(
                    mgr,
                    r.session_id.clone(),
                    req.provider_id,
                    req.cli_path,
                    req.auth_login_args,
                    req.auth_check_args,
                    req.auth_env,
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
}

/// Spawn the provider's auth-login CLI and drive the session through
/// to a terminal state. Background-only — returns immediately. The
/// drain task feeds stdout+stderr lines into
/// `AuthSessionManager::record_line`; on a login-success pattern OR
/// child exit, runs the provider's authCheckCommand to confirm and
/// transitions to Success or Failed.
fn spawn_auth_cli(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    session_id: String,
    provider_id: String,
    cli_path: String,
    auth_login_args: Vec<String>,
    auth_check_args: Vec<String>,
    auth_env: std::collections::HashMap<String, String>,
) {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;
    use tokio::sync::mpsc;

    // Channel for SubmitAuthCallback → CLI stdin forwarding.
    // Buffer of 4 is enough — only one URL per session in normal use.
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(4);

    let mgr_for_task = mgr.clone();
    let session_id_for_task = session_id.clone();
    let cli_path_for_check = cli_path.clone();
    let auth_check_args_for_check = auth_check_args.clone();
    let auth_env_for_check = auth_env.clone();

    let handle = tokio::spawn(async move {
        tracing::info!(
            session_id = %session_id_for_task,
            provider_id = %provider_id,
            cli_path = %cli_path,
            "auth.spawn: launching provider CLI"
        );

        // Spawn the CLI. kill_on_drop guarantees cleanup if our
        // task is aborted (cancel path).
        let mut child = match Command::new(&cli_path)
            .args(&auth_login_args)
            .envs(&auth_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                mgr_for_task.finish_failure(
                    &session_id_for_task,
                    format!("spawn `{cli_path}` failed: {e}"),
                );
                mgr_for_task.detach_process(&session_id_for_task);
                return;
            }
        };

        let stdout = child
            .stdout
            .take()
            .expect("piped stdout");
        let stderr = child
            .stderr
            .take()
            .expect("piped stderr");
        let mut stdin = child
            .stdin
            .take()
            .expect("piped stdin");

        // Stdin writer — forwards callback URLs (pasted back by the
        // user when browser auto-open failed) into the CLI process.
        let stdin_writer = tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        // Stdout drain — line-by-line into the session manager. When
        // we see a login-success pattern, confirm via authCheckCommand
        // and transition to Success. Don't break the loop — some CLIs
        // continue printing after the success line.
        let mgr_stdout = mgr_for_task.clone();
        let sid_stdout = session_id_for_task.clone();
        let cli_path_stdout = cli_path_for_check.clone();
        let check_args_stdout = auth_check_args_for_check.clone();
        let check_env_stdout = auth_env_for_check.clone();
        let stdout_drain = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut success_transitioned = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let m = mgr_stdout.record_line(&sid_stdout, &line);
                if !success_transitioned
                    && matches!(
                        m,
                        Some(crate::identity::auth_patterns::AuthPatternMatch::LoginSuccess { .. })
                    )
                {
                    if confirm_authenticated(
                        &cli_path_stdout,
                        &check_args_stdout,
                        &check_env_stdout,
                    )
                    .await
                    {
                        // PR A: synthetic bundle id — PR C wires real
                        // wstore-backed persistence. The frontend can
                        // detect this placeholder (prefix
                        // "pending-bundle-for-") and surface "saving…"
                        // UI in the interim.
                        let bundle_id =
                            format!("pending-bundle-for-{}", sid_stdout);
                        mgr_stdout.finish_success(&sid_stdout, bundle_id);
                        success_transitioned = true;
                    }
                }
            }
        });

        // Stderr drain — same matcher path. Some CLIs emit the OAuth
        // URL on stderr (verbose logging style).
        let mgr_stderr = mgr_for_task.clone();
        let sid_stderr = session_id_for_task.clone();
        let stderr_drain = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = mgr_stderr.record_line(&sid_stderr, &line);
            }
        });

        // Wait for the child to exit (or for our task to be aborted
        // by cancel_session — in which case kill_on_drop handles the
        // child).
        let exit = child.wait().await;

        // Let stdout/stderr drains catch any final lines.
        let _ = stdout_drain.await;
        let _ = stderr_drain.await;
        // Stdin writer exits when the tx side is dropped (after this
        // task ends) — abort defensively in case it's blocked on a
        // pending receive.
        stdin_writer.abort();

        // Final transition if we haven't already hit Success on the
        // login-pattern path. Re-run authCheck because some CLIs exit
        // cleanly without printing a "logged in as" line.
        match exit {
            Ok(s) if s.success() => {
                if confirm_authenticated(
                    &cli_path_for_check,
                    &auth_check_args_for_check,
                    &auth_env_for_check,
                )
                .await
                {
                    let bundle_id =
                        format!("pending-bundle-for-{}", session_id_for_task);
                    mgr_for_task.finish_success(&session_id_for_task, bundle_id);
                } else {
                    mgr_for_task.finish_failure(
                        &session_id_for_task,
                        "CLI exited 0 but authentication check failed".to_string(),
                    );
                }
            }
            Ok(s) => {
                mgr_for_task.finish_failure(
                    &session_id_for_task,
                    format!("CLI exited with status {s}"),
                );
            }
            Err(e) => {
                mgr_for_task.finish_failure(
                    &session_id_for_task,
                    format!("wait error: {e}"),
                );
            }
        }

        mgr_for_task.detach_process(&session_id_for_task);
    });

    mgr.attach_process(&session_id, handle, stdin_tx);
}

/// Run the provider's auth-check subcommand and return true if it
/// exits 0. Failure modes (binary missing, network error, etc.) are
/// all treated as "not authenticated" — the caller will then either
/// keep waiting (drain task loop) or transition to Failed (exit
/// fallback).
async fn confirm_authenticated(
    cli_path: &str,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
) -> bool {
    use std::process::Stdio;
    use tokio::process::Command;
    match Command::new(cli_path)
        .args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
    {
        Ok(s) => s.success(),
        Err(_) => false,
    }
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
}

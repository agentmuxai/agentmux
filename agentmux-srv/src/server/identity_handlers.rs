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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::backend::providers::get_provider;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::backend::wps::Broker;

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

pub fn register_identity_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mgr = state.auth_session_manager.clone();
    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_AUTH_START,
        Box::new(move |data, _ctx| {
            let mgr = mgr.clone();
            let wstore = wstore.clone();
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
}

/// Spawn the provider's auth-login CLI and drive the session through
/// to a terminal state. Background-only — returns immediately. The
/// drain task feeds stdout+stderr lines into
/// `AuthSessionManager::record_line`; on a login-success pattern OR
/// child exit, runs the provider's authCheckCommand to confirm and
/// transitions to Success or Failed.
///
/// On the success path (CLI exited cleanly + authCheckCommand passed),
/// persists a standalone `IdentityAccount` (`persist_oauth_direct_account`)
/// with `SecretRef::OAuthConfigDir` pointing at `bundle_dir` (the
/// account's own isolation dir — see `compute_and_ensure_account_dir`),
/// status `valid`. `account_id` is the id minted/reused for that account.
/// After this point, future launches of any agent linked to the account
/// resolve through `inject_identity_env`'s oauth-class dispatch and reuse
/// the same CLI-managed tokens.
///
/// `into_bundle_id`/`direct_account` are vestigial parameters kept only
/// so the wire request shape and call sites don't need touching — bundle
/// mode (binding into `db_identity_bundles`) was retired in Phase 4c of
/// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md; `persist_oauth_success`
/// always takes the direct-account path now.
#[allow(clippy::too_many_arguments)]
fn spawn_auth_cli(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    wstore: Arc<Store>,
    broker: Arc<Broker>,
    session_id: String,
    provider_id: String,
    into_bundle_id: Option<String>,
    bundle_dir: Option<String>,
    direct_account: bool,
    account_id: String,
    cli_path: String,
    auth_login_args: Vec<String>,
    auth_check_args: Vec<String>,
    auth_env: std::collections::HashMap<String, String>,
    requires_tty: bool,
) {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;
    use tokio::sync::mpsc;

    // Channel for SubmitAuthCallback → CLI stdin forwarding.
    // Buffer of 4 is enough — only one URL per session in normal use.
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(4);

    // PTY branch: providers like OpenClaw whose auth subcommand
    // refuses to run when `isatty()==0`. Spawn via portable_pty so the
    // child sees an interactive terminal, feed PTY output through the
    // same record_line matcher the pipes path uses. Stdout/stderr are
    // merged into the single PTY stream — record_line handles both as
    // identical input.
    if requires_tty {
        spawn_auth_cli_pty(
            mgr,
            wstore,
            broker,
            session_id,
            provider_id,
            into_bundle_id,
            bundle_dir,
            direct_account,
            account_id,
            cli_path,
            auth_login_args,
            auth_check_args,
            auth_env,
            stdin_tx,
            stdin_rx,
        );
        return;
    }

    let mgr_for_task = mgr.clone();
    let session_id_for_task = session_id.clone();
    let cli_path_for_check = cli_path.clone();
    let auth_check_args_for_check = auth_check_args.clone();
    let auth_env_for_check = auth_env.clone();
    let wstore_for_task = wstore.clone();
    let broker_for_task = broker.clone();
    let into_bundle_id_for_task = into_bundle_id.clone();
    let bundle_dir_for_task = bundle_dir.clone();
    let provider_id_for_task = provider_id.clone();
    let account_id_for_task = account_id.clone();

    let handle = tokio::spawn(async move {
        tracing::info!(
            session_id = %session_id_for_task,
            provider_id = %provider_id,
            cli_path = %cli_path,
            auth_login_args = ?auth_login_args,
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
            Ok(c) => {
                tracing::info!(
                    session_id = %session_id_for_task,
                    pid = ?c.id(),
                    "auth.spawn: child started"
                );
                c
            }
            Err(e) => {
                tracing::error!(
                    session_id = %session_id_for_task,
                    error = %e,
                    "auth.spawn: Command::spawn failed"
                );
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
        let wstore_stdout = wstore_for_task.clone();
        let broker_stdout = broker_for_task.clone();
        let into_bundle_id_stdout = into_bundle_id_for_task.clone();
        let bundle_dir_stdout = bundle_dir_for_task.clone();
        let provider_id_stdout = provider_id_for_task.clone();
        let account_id_stdout = account_id_for_task.clone();
        // Shared between drain + post-exit. The drain sets it after
        // persisting on a LoginSuccess match; the post-exit transition
        // block (below) checks it and skips its entire success path if
        // already transitioned — without this guard the drain's
        // persist + post-exit's persist both ran on every successful
        // OAuth, producing orphan IdentityAccount rows (each `Uuid::new_v4`)
        // and duplicate `identityaccounts:changed` publishes.
        // Reagent P1 on #981.
        let success_transitioned = Arc::new(AtomicBool::new(false));
        let success_transitioned_drain = Arc::clone(&success_transitioned);
        let stdout_drain = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // NOT logging the raw line — OAuth providers print
                // auth URLs / callback codes / device codes on stdout
                // and even debug-level logs would persist those into
                // ~/.agentmux/logs/. Length-only is enough for "the
                // CLI emitted something" diagnostics. Reagent P2 on #847.
                tracing::debug!(session_id = %sid_stdout, bytes = line.len(), "auth.spawn: stdout line");
                let m = mgr_stdout.record_line(&sid_stdout, &line);
                if !success_transitioned_drain.load(Ordering::Acquire)
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
                        // Persist the standalone OAuthConfigDir account
                        // (always direct-account mode now). If `bundle_dir`
                        // (the account's own isolation dir) failed to
                        // resolve at spawn, persist_oauth_success skips
                        // persistence and the session still succeeds.
                        let (bundle_id, account_id) = persist_oauth_success(
                            &wstore_stdout,
                            &broker_stdout,
                            direct_account,
                            &account_id_stdout,
                            into_bundle_id_stdout.as_deref(),
                            &provider_id_stdout,
                            bundle_dir_stdout.as_deref(),
                            &sid_stdout,
                        );
                        mgr_stdout.finish_success(&sid_stdout, bundle_id, account_id);
                        success_transitioned_drain.store(true, Ordering::Release);
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
                // Same redaction rationale as stdout above.
                tracing::debug!(session_id = %sid_stderr, bytes = line.len(), "auth.spawn: stderr line");
                let _ = mgr_stderr.record_line(&sid_stderr, &line);
            }
        });

        // Wait for the child to exit (or for our task to be aborted
        // by cancel_session — in which case kill_on_drop handles the
        // child).
        let exit = child.wait().await;
        tracing::info!(session_id = %session_id_for_task, exit = ?exit, "auth.spawn: child exited");

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
        //
        // Skip the entire block when the drain has already transitioned
        // (LoginSuccess pattern matched + confirm_authenticated + persist
        // ran). Without this guard, persist_oauth_success would fire a
        // second time on every successful OAuth — a fresh IdentityAccount
        // UUID would be upserted and the broker would re-publish the
        // accounts-changed event. Reagent P1 on #981.
        if !success_transitioned.load(Ordering::Acquire) {
            match exit {
                Ok(s) if s.success() => {
                    if confirm_authenticated(
                        &cli_path_for_check,
                        &auth_check_args_for_check,
                        &auth_env_for_check,
                    )
                    .await
                    {
                        let (bundle_id, account_id) = persist_oauth_success(
                            &wstore_for_task,
                            &broker_for_task,
                            direct_account,
                            &account_id_for_task,
                            into_bundle_id_for_task.as_deref(),
                            &provider_id_for_task,
                            bundle_dir_for_task.as_deref(),
                            &session_id_for_task,
                        );
                        mgr_for_task.finish_success(&session_id_for_task, bundle_id, account_id);
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
        }

        mgr_for_task.detach_process(&session_id_for_task);
    });

    mgr.attach_process(&session_id, handle, stdin_tx);
}

/// PTY-backed variant of spawn_auth_cli. Used for providers whose auth
/// subcommand bails on `isatty()==0` (currently OpenClaw). Mirrors the
/// pipes path's lifecycle: spawn → drain → confirm → finish_success or
/// finish_failure → detach. Stdout+stderr are merged on the PTY side;
/// `record_line` doesn't care which stream a line came from.
///
/// Same OAuth-success invariant as the pipes path — once `bundle_dir`
/// (the account's own isolation dir) resolves and `confirm_authenticated`
/// returns true, persists a standalone `SecretRef::OAuthConfigDir`
/// account before `finish_success` — see `spawn_auth_cli`'s doc comment.
#[allow(clippy::too_many_arguments)]
fn spawn_auth_cli_pty(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    wstore: Arc<Store>,
    broker: Arc<Broker>,
    session_id: String,
    provider_id: String,
    into_bundle_id: Option<String>,
    bundle_dir: Option<String>,
    direct_account: bool,
    account_id: String,
    cli_path: String,
    auth_login_args: Vec<String>,
    auth_check_args: Vec<String>,
    auth_env: std::collections::HashMap<String, String>,
    stdin_tx: tokio::sync::mpsc::Sender<String>,
    mut stdin_rx: tokio::sync::mpsc::Receiver<String>,
) {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let mgr_for_task = mgr.clone();
    let session_id_for_task = session_id.clone();
    let cli_path_for_check = cli_path.clone();
    let auth_check_args_for_check = auth_check_args.clone();
    let auth_env_for_check = auth_env.clone();
    let wstore_for_task = wstore.clone();
    let broker_for_task = broker.clone();
    let into_bundle_id_for_task = into_bundle_id.clone();
    let bundle_dir_for_task = bundle_dir.clone();
    let provider_id_for_task = provider_id.clone();
    let account_id_for_task = account_id.clone();

    tracing::info!(
        session_id = %session_id,
        provider_id = %provider_id,
        cli_path = %cli_path,
        auth_login_args = ?auth_login_args,
        "auth.spawn (PTY): launching provider CLI"
    );

    // Allocate the PTY, build the command, spawn the child, and
    // register the PID — all synchronously, BEFORE the async drain/
    // wait task and BEFORE `attach_process`. That way no
    // `cancel_session` racing with the spawn can find a registered
    // handle but a missing PID; either the PID is registered first
    // (cancel can kill), or the early-return path fired and there
    // is no child to kill.
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            // PTY allocation failure means the provider's auth
            // subprocess can't get the interactive TTY it requires
            // (openclaw `models auth login`). Surface as the typed
            // `AMX-AUTH-001` so the FailedBanner shows the friendly
            // recovery hint instead of "openpty failed: <e>".
            let mux = agentmux_common::AgentMuxError::AuthRequiresTty {
                provider: provider_id.clone(),
            };
            tracing::error!(
                target: "amx::error",
                session_id = %session_id,
                amx_code = %mux.code(),
                error = %e,
                "auth.spawn (PTY): openpty failed"
            );
            mgr.finish_failure(&session_id, mux.to_wire().to_string());
            mgr.detach_process(&session_id);
            return;
        }
    };

    let mut cmd = CommandBuilder::new(&cli_path);
    for a in &auth_login_args {
        cmd.arg(a);
    }
    for (k, v) in &auth_env {
        cmd.env(k, v);
    }
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "auth.spawn (PTY): spawn_command failed"
            );
            mgr.finish_failure(&session_id, format!("PTY spawn `{cli_path}` failed: {e}"));
            mgr.detach_process(&session_id);
            return;
        }
    };

    if let Some(pid) = child.process_id() {
        mgr.attach_pty_pid(&session_id, pid);
    }

    let reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "auth.spawn (PTY): try_clone_reader failed");
            mgr.finish_failure(&session_id, format!("PTY reader: {e}"));
            mgr.detach_process(&session_id);
            return;
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "auth.spawn (PTY): take_writer failed");
            mgr.finish_failure(&session_id, format!("PTY writer: {e}"));
            mgr.detach_process(&session_id);
            return;
        }
    };

    let handle = tokio::spawn(async move {

        // Stdin writer: forward callback URLs from the manager into
        // the child's PTY input. portable_pty's master writer is sync;
        // wrap each write in `block_in_place` so the blocking IO
        // doesn't starve the tokio reactor when the PTY input buffer
        // is full.
        let stdin_writer_handle = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(line) = stdin_rx.recv().await {
                let res = tokio::task::block_in_place(|| {
                    use std::io::Write;
                    writer
                        .write_all(line.as_bytes())
                        .and_then(|_| writer.write_all(b"\n"))
                        .and_then(|_| writer.flush())
                });
                if res.is_err() {
                    break;
                }
            }
        });

        // Drain: synchronous line reader from PTY master, feeds into
        // the same record_line matcher the pipes path uses. Runs in a
        // blocking thread; sends parsed events through a oneshot when
        // we hit a login-success pattern (so the async task can run
        // confirm_authenticated without crossing the blocking thread).
        let mgr_drain = mgr_for_task.clone();
        let sid_drain = session_id_for_task.clone();
        let cli_path_drain = cli_path_for_check.clone();
        let check_args_drain = auth_check_args_for_check.clone();
        let check_env_drain = auth_env_for_check.clone();
        let wstore_drain = wstore_for_task.clone();
        let broker_drain = broker_for_task.clone();
        let into_bundle_id_drain = into_bundle_id_for_task.clone();
        let bundle_dir_drain = bundle_dir_for_task.clone();
        let provider_id_drain = provider_id_for_task.clone();
        let account_id_drain = account_id_for_task.clone();
        // Shared with the post-exit fallback block below — same pattern
        // as the pipes path. Drain's detached `Handle::current().spawn`
        // task does the persist + finish_success; the drain sets this
        // atomic BEFORE returning so the outer's check sees it. The
        // detached task may still be in flight when the outer checks
        // (PTY drain returns on EOF, the persist task runs async) — that
        // is fine because the OUTER would only re-persist if the atomic
        // is false, which it never is once the drain matched the
        // LoginSuccess pattern. Reagent P1 on #981.
        let success_transitioned = Arc::new(AtomicBool::new(false));
        let success_transitioned_drain = Arc::clone(&success_transitioned);
        let drain_handle = tokio::task::spawn_blocking(move || {
            use std::io::BufRead;
            let mut reader = std::io::BufReader::new(reader);
            let mut line = String::new();
            // The detached confirm/persist task's JoinHandle, populated
            // when the drain matches LoginSuccess. Returned to outer so
            // it can await completion before its post-exit check —
            // without that wait, outer would race the detached and
            // either skip too soon (if it sets the atomic eagerly) or
            // double-persist. Reagent P1 follow-up on #981.
            let mut maybe_detached: Option<tokio::task::JoinHandle<()>> = None;
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        // Same redaction policy as the pipes path —
                        // OAuth URLs / codes can be in here.
                        tracing::debug!(session_id = %sid_drain, bytes = line.len(), "auth.spawn (PTY): line");
                        let m = mgr_drain.record_line(&sid_drain, &line);
                        if maybe_detached.is_none()
                            && matches!(
                                m,
                                Some(crate::identity::auth_patterns::AuthPatternMatch::LoginSuccess { .. })
                            )
                        {
                            // Hand off to async to call confirm_authenticated.
                            // The detached task OWNS the transition —
                            // it calls finish_success on confirm-OK and
                            // finish_failure on confirm-NOT-OK, AND
                            // sets the shared atomic in BOTH cases. The
                            // drain only captures the JoinHandle here
                            // and returns it; outer awaits both drain
                            // and the inner handle before its check, so
                            // the atomic is correctly settled by then.
                            let cli = cli_path_drain.clone();
                            let args = check_args_drain.clone();
                            let env = check_env_drain.clone();
                            let mgr2 = mgr_drain.clone();
                            let sid2 = sid_drain.clone();
                            let wstore2 = wstore_drain.clone();
                            let broker2 = broker_drain.clone();
                            let into_bundle_id2 = into_bundle_id_drain.clone();
                            let bundle_dir2 = bundle_dir_drain.clone();
                            let provider_id2 = provider_id_drain.clone();
                            let account_id2 = account_id_drain.clone();
                            let success_for_detached = Arc::clone(&success_transitioned_drain);
                            let handle = tokio::runtime::Handle::current().spawn(async move {
                                if confirm_authenticated(&cli, &args, &env).await {
                                    let (bundle_id, account_id) = persist_oauth_success(
                                        &wstore2,
                                        &broker2,
                                        direct_account,
                                        &account_id2,
                                        into_bundle_id2.as_deref(),
                                        &provider_id2,
                                        bundle_dir2.as_deref(),
                                        &sid2,
                                    );
                                    mgr2.finish_success(&sid2, bundle_id, account_id);
                                    // Atomic set ONLY on confirm-success
                                    // — mirrors the pipes path. On
                                    // confirm-miss the detached does
                                    // nothing (atomic stays false),
                                    // outer's post-exit fallback gets
                                    // another shot at confirm by the
                                    // time creds are fully written.
                                    // Outer awaits this handle first,
                                    // so the atomic is correctly
                                    // reflected by the time it checks.
                                    // codex P1 follow-up on #981.
                                    success_for_detached.store(true, Ordering::Release);
                                }
                            });
                            maybe_detached = Some(handle);
                        }
                    }
                    Err(_) => break,
                }
            }
            maybe_detached
        });

        // Wait for the child in a blocking task. pair (master + slave)
        // moves into the closure so its destructor runs AFTER
        // child.wait() — ConPTY contract on Windows.
        let wait_handle = tokio::task::spawn_blocking(move || {
            let mut child = child;
            let exit = child.wait();
            drop(pair);
            exit
        });

        let exit = wait_handle.await;
        tracing::info!(session_id = %session_id_for_task, exit = ?exit, "auth.spawn (PTY): child exited");

        // Await the drain itself, THEN its optional detached
        // confirm/persist task. Both have to complete before the
        // post-exit check — otherwise outer might see atomic=false
        // (detached still confirming), run its fallback persist, AND
        // the detached's persist also runs → the original double-
        // persist race re-opens. Reagent P1 follow-up on #981.
        let detached = drain_handle.await.ok().flatten();
        if let Some(h) = detached {
            let _ = h.await;
        }
        stdin_writer_handle.abort();

        // Final transition fallback — some CLIs exit cleanly without
        // emitting a login-success line that record_line recognizes.
        //
        // Skip the whole block when the drain already transitioned —
        // same double-persist guard as the pipes path. Reagent P1 on
        // #981.
        if !success_transitioned.load(Ordering::Acquire) {
            match exit {
                Ok(Ok(s)) if s.success() => {
                    if confirm_authenticated(
                        &cli_path_for_check,
                        &auth_check_args_for_check,
                        &auth_env_for_check,
                    )
                    .await
                    {
                        let (bundle_id, account_id) = persist_oauth_success(
                            &wstore_for_task,
                            &broker_for_task,
                            direct_account,
                            &account_id_for_task,
                            into_bundle_id_for_task.as_deref(),
                            &provider_id_for_task,
                            bundle_dir_for_task.as_deref(),
                            &session_id_for_task,
                        );
                        mgr_for_task.finish_success(&session_id_for_task, bundle_id, account_id);
                    } else {
                        mgr_for_task.finish_failure(
                            &session_id_for_task,
                            "CLI exited cleanly but auth-check still failed".to_string(),
                        );
                    }
                }
                Ok(Ok(s)) => {
                    mgr_for_task.finish_failure(
                        &session_id_for_task,
                        format!("CLI exited with code {:?}", s.exit_code()),
                    );
                }
                Ok(Err(e)) => {
                    mgr_for_task.finish_failure(
                        &session_id_for_task,
                        format!("PTY wait error: {e}"),
                    );
                }
                Err(e) => {
                    mgr_for_task.finish_failure(
                        &session_id_for_task,
                        format!("PTY wait task join error: {e}"),
                    );
                }
            }
        }

        mgr_for_task.detach_process(&session_id_for_task);
    });

    mgr.attach_process(&session_id, handle, stdin_tx);
}

/// Compute the per-bundle OAuth config dir + ensure it exists +
/// override the provider's `auth_config_dir_env_var` entry in `auth_env`.
///
/// Returns `Some(<absolute path string>)` when:
///   - `into_bundle_id` is `Some` and non-empty AND
///   - the provider is registered in the CLI provider registry AND
///   - the provider declares an `auth_config_dir_env_var` (oauth-class
///     providers — claude / codex / openclaw — per spec §4.3) AND
///   - `DataPaths::from_env()` resolves AND
///   - `create_dir_all` succeeds.
///
/// Otherwise returns `None` and leaves `auth_env` untouched. Callers
/// must continue without per-bundle isolation (legacy ambient path,
/// or skip the binding-persist step) — never abort the OAuth start.
///
/// Per `specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.5: the dir
/// (and the env-var key) come from the CLI provider registry
/// (`backend::providers::get_provider(id)`) so the resolver / spawn
/// path / OAuth-start handler never drift on which env var redirects
/// each CLI's config home. The single source of truth lives in
/// `agentmux-srv/src/backend/providers.rs`.
fn compute_and_ensure_bundle_dir(
    into_bundle_id: Option<&str>,
    provider_id: &str,
    auth_env: &mut std::collections::HashMap<String, String>,
) -> Option<String> {
    let bundle_id = into_bundle_id.filter(|s| !s.is_empty())?;
    // Gate on provider_class so api-key-class providers (which have a
    // registry entry with a non-empty `auth_config_dir_env_var` —
    // e.g. kimi's `KIMI_SHARE_DIR`) never go through the per-bundle
    // OAuth-dir path. Only claude / codex / openclaw — the spec §4.3
    // oauth-class providers — get the per-bundle override.
    match crate::identity::resolver::provider_class(provider_id) {
        Some(crate::identity::resolver::ProviderClass::OAuth { .. }) => {}
        _ => return None,
    }
    let provider = match get_provider(provider_id) {
        Some(p) => p,
        None => {
            // OAuth-class per provider_class but missing from the CLI
            // registry — should be impossible (resolver reads the env
            // var from the registry itself), but treat as a soft fail.
            tracing::warn!(
                target: "identity",
                provider_id,
                "auth.start: oauth-class provider not in registry — skipping per-bundle dir override"
            );
            return None;
        }
    };
    // Empty env-var name → no isolation possible (oauth-class providers
    // should never have this empty per spec, but belt-and-braces).
    if provider.auth_config_dir_env_var.is_empty() {
        return None;
    }
    let paths = match agentmux_common::DataPaths::from_env() {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                bundle_id,
                "auth.start: DataPaths::from_env() returned None — skipping per-bundle dir override"
            );
            return None;
        }
    };
    // identity_dir rejects unsafe path segments (empty / `.` / `..` /
    // segment with `/`, `\`, drive-letter, …). bundle_id comes from
    // the auth.start request body, so this guard prevents a crafted
    // id from escaping the identities root.
    let bundle_root = match paths.identity_dir(bundle_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                bundle_id,
                "auth.start: bundle_id is not a safe path segment — skipping per-bundle dir override"
            );
            return None;
        }
    };
    let dir = bundle_root.join(provider.auth_dir_name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "identity",
            provider_id,
            bundle_id,
            error = %e,
            path = %dir.display(),
            "auth.start: failed to create per-bundle config dir — skipping override"
        );
        return None;
    }
    let dir_str = dir.to_string_lossy().to_string();
    // Override (or insert) the provider's config-dir env var. The
    // frontend may have computed the legacy ambient dir via
    // `ensureAuthDir(providerId)` and put it here under the same key;
    // we replace it with the per-bundle dir so the OAuth CLI writes
    // its tokens inside the bundle, not in the ambient version-config
    // dir.
    auth_env.insert(
        provider.auth_config_dir_env_var.to_string(),
        dir_str.clone(),
    );
    tracing::info!(
        target: "identity",
        provider_id,
        bundle_id,
        env_var = provider.auth_config_dir_env_var,
        dir = %dir.display(),
        "auth.start: per-bundle OAuth config dir wired"
    );
    Some(dir_str)
}

/// Direct-account sibling of `compute_and_ensure_bundle_dir` (issue
/// #1624 PR-C Part B) — bypasses the bundle system entirely. Mints a
/// fresh `account_id` when `existing_account_id` is empty (first-time
/// OAuth connect from the launch modal); reuses it when non-empty
/// (Reconnect — refresh tokens in place, same isolation dir, same
/// account row updated in place).
///
/// Returns `(account_id, dir)` — `account_id` is always populated (even
/// on a dir-resolution failure, so the caller can still log/track it);
/// `dir` is `None` on the same gate/registry/fs failures
/// `compute_and_ensure_bundle_dir` treats as soft failures — never
/// abort `auth.start` over a config-dir issue.
fn compute_and_ensure_account_dir(
    existing_account_id: &str,
    provider_id: &str,
    auth_env: &mut std::collections::HashMap<String, String>,
) -> (String, Option<String>) {
    let account_id = if existing_account_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        existing_account_id.to_string()
    };

    // Same provider_class gate as the bundle path — only oauth-class
    // providers (claude/codex/openclaw) get a per-account isolation dir.
    match crate::identity::resolver::provider_class(provider_id) {
        Some(crate::identity::resolver::ProviderClass::OAuth { .. }) => {}
        _ => return (account_id, None),
    }
    let provider = match get_provider(provider_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                "auth.start (direct-account): oauth-class provider not in registry — skipping config dir"
            );
            return (account_id, None);
        }
    };
    if provider.auth_config_dir_env_var.is_empty() {
        return (account_id, None);
    }
    let paths = match agentmux_common::DataPaths::from_env() {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                "auth.start (direct-account): DataPaths::from_env() returned None — skipping config dir"
            );
            return (account_id, None);
        }
    };
    // identity_dir is already generic (not bundle-specific) — same
    // unsafe-path-segment rejection applies to account_id here.
    let account_root = match paths.identity_dir(&account_id) {
        Some(p) => p,
        None => {
            tracing::warn!(
                target: "identity",
                provider_id,
                account_id,
                "auth.start (direct-account): account_id is not a safe path segment — skipping config dir"
            );
            return (account_id, None);
        }
    };
    let dir = account_root.join(provider.auth_dir_name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "identity",
            provider_id,
            account_id,
            error = %e,
            path = %dir.display(),
            "auth.start (direct-account): failed to create config dir — skipping"
        );
        return (account_id, None);
    }
    let dir_str = dir.to_string_lossy().to_string();
    auth_env.insert(provider.auth_config_dir_env_var.to_string(), dir_str.clone());
    tracing::info!(
        target: "identity",
        provider_id,
        account_id,
        env_var = provider.auth_config_dir_env_var,
        dir = %dir.display(),
        "auth.start (direct-account): OAuth config dir wired"
    );
    (account_id, Some(dir_str))
}

/// Upserts the `IdentityAccount` (`SecretRef::OAuthConfigDir`, status
/// "valid") on a successful OAuth handshake (CLI exited 0 +
/// authCheckCommand confirmed). The actual `agent_identity_link` write
/// happens later, once the agent exists (the launch-flow write-through
/// reconcile) — this function only makes sure the account itself exists
/// and is ready to be linked.
///
/// Publishes `identityaccounts:changed` (the same broad event
/// `account.key.verify`/`upsertidentityaccount` already use) rather
/// than a bundle-scoped event, since there's no bundle id to scope to.
///
/// Returns `None` on any persistence failure (dir never resolved, or
/// the account upsert itself failed) — same "log + skip, session still
/// succeeds" contract as the bundle path, just without a synthetic
/// placeholder to fall back to (direct-account mode has no "ambient"
/// concept to fall back to; the caller surfaces `account_id: None` on
/// the wire and the frontend treats that as "nothing to select").
fn persist_oauth_direct_account(
    wstore: &Arc<Store>,
    broker: &Arc<Broker>,
    account_id: &str,
    provider_id: &str,
    dir: Option<&str>,
    _session_id: &str,
) -> Option<String> {
    let dir = match dir.filter(|s| !s.is_empty()) {
        Some(d) => d,
        None => {
            tracing::warn!(
                target: "identity",
                account_id,
                provider_id,
                "auth success (direct-account): dir unresolved — skipping account persist"
            );
            return None;
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let account = IdentityAccount {
        id: account_id.to_string(),
        name: format!("{provider_id}-oauth"),
        provider: provider_id.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string() },
        context: serde_json::json!({}),
        // Same rationale as the bundle path: a binding the user JUST
        // OAuth'd into is `valid` by definition.
        status: crate::identity::resolver::oauth_status::VALID.to_string(),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = wstore.identity_upsert(&account) {
        tracing::warn!(
            target: "identity",
            account_id,
            provider_id,
            error = %e,
            "auth success (direct-account): identity_upsert failed"
        );
        return None;
    }
    broker.publish(crate::backend::wps::WaveEvent {
        event: "identityaccounts:changed".to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
    tracing::info!(
        target: "identity",
        account_id,
        provider_id,
        dir,
        "auth success (direct-account): OAuth account persisted"
    );
    Some(account_id.to_string())
}

/// Shared by all 4 OAuth-success call sites (pipes drain/post-exit, PTY
/// drain/post-exit) — persists the account and builds the
/// `(bundle_id, account_id)` pair `AuthSessionManager::finish_success`
/// expects. `bundle_id` is always empty now: bundle mode (`db_identity_bundles`
/// binding) was retired in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md
/// — confirmed unreachable from the frontend (`AuthFlowController` hardcodes
/// `directAccount: true`). `_direct_account`/`_into_bundle_id` stay as
/// parameters so the wire request shape and the 4 call sites don't need
/// touching. `dir` is the account's own isolation dir, resolved once at
/// spawn time by `compute_and_ensure_*_dir` in the `auth.start` handler.
///
/// Guards on `account_id` being non-empty before persisting: `auth.start`
/// only populates a real account_id when `direct_account` is true (via
/// `compute_and_ensure_account_dir`, which always mints/reuses a real id);
/// when `direct_account` is false (the wire default, still reachable by
/// any caller other than the one production frontend path), `account_id`
/// is `""`. Without this guard an empty id would flow into
/// `persist_oauth_direct_account`'s `identity_upsert`, whose
/// `ON CONFLICT(id) DO UPDATE` would silently overwrite/corrupt any prior
/// row that happened to have `id=""`. Reagent P1.
#[allow(clippy::too_many_arguments)]
fn persist_oauth_success(
    wstore: &Arc<Store>,
    broker: &Arc<Broker>,
    _direct_account: bool,
    account_id: &str,
    _into_bundle_id: Option<&str>,
    provider_id: &str,
    dir: Option<&str>,
    session_id: &str,
) -> (String, Option<String>) {
    if account_id.is_empty() {
        return (String::new(), None);
    }
    let persisted = persist_oauth_direct_account(wstore, broker, account_id, provider_id, dir, session_id);
    (String::new(), persisted)
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
    let mut c = Command::new(cli_path);
    c.args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // CREATE_NO_WINDOW: this status probe is polled in a drain loop during an
    // auth flow — without the flag each poll flashes a console. See cli.rs.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    match c.status().await {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared across every test in this module that mutates process-
    // global env vars (`AGENTMUX_HOME_OVERRIDE` + `DataPaths::to_env_vars()`
    // entries) via `compute_and_ensure_bundle_dir`/`compute_and_ensure_account_dir`
    // round-trip tests. `cargo test` runs tests in this binary in
    // parallel by default — a `static` declared inside a TEST FUNCTION
    // body is a separate item per function, so per-function locks don't
    // actually serialize against each other, only against repeated
    // calls to the SAME test. One module-level lock, taken by every
    // such test before touching the environment, is what actually
    // prevents them from racing.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    #[test]
    fn compute_account_dir_mints_fresh_id_when_existing_is_empty() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }

        let mut env = std::collections::HashMap::new();
        let (account_id, dir) = compute_and_ensure_account_dir("", "claude", &mut env);
        assert!(!account_id.is_empty(), "must mint a fresh id when none supplied");
        let dir = dir.expect("oauth-class provider must yield a dir");
        let expected = paths.identity_dir(&account_id).unwrap().join("claude");
        assert_eq!(std::path::Path::new(&dir), expected);
        assert!(expected.is_dir());
        assert_eq!(env.get("CLAUDE_CONFIG_DIR").map(String::as_str), Some(dir.as_str()));

        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn compute_account_dir_reuses_existing_id() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }

        let mut env = std::collections::HashMap::new();
        let (account_id, _) = compute_and_ensure_account_dir("acc-reconnect", "claude", &mut env);
        assert_eq!(account_id, "acc-reconnect", "reconnect must reuse the supplied id, not mint a new one");

        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn compute_account_dir_skipped_for_api_key_provider() {
        // Same provider_class gate as the bundle path — dir is None,
        // but the account id is still returned (unlike bundle mode,
        // there's no "skip entirely" case here — the account always
        // gets minted/reused, only the isolation dir is conditional).
        let mut env = std::collections::HashMap::new();
        let (account_id, dir) = compute_and_ensure_account_dir("", "kimi", &mut env);
        assert!(!account_id.is_empty());
        assert!(dir.is_none(), "api-key provider class must skip the OAuth dir path");
        assert!(env.get("KIMI_SHARE_DIR").is_none());
    }

    #[test]
    fn persist_oauth_direct_account_round_trip() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let r = persist_oauth_direct_account(
            &wstore,
            &broker,
            "acc-1",
            "claude",
            Some("/some/account/dir"),
            "sess-z",
        );
        assert_eq!(r, Some("acc-1".to_string()));

        let acct = wstore.identity_get("acc-1").unwrap().expect("account row exists");
        assert_eq!(acct.provider, "claude");
        assert_eq!(acct.kind, "oauth");
        assert_eq!(acct.status, "valid");
        match acct.secret_ref {
            SecretRef::OAuthConfigDir { dir } => assert_eq!(dir, "/some/account/dir"),
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }
    }

    #[test]
    fn persist_oauth_direct_account_returns_none_when_dir_unresolved() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let r = persist_oauth_direct_account(&wstore, &broker, "acc-1", "claude", None, "sess-z");
        assert!(r.is_none());
        assert!(wstore.identity_get("acc-1").unwrap().is_none(), "nothing persisted when dir is unresolved");
    }

    #[test]
    fn persist_oauth_success_always_routes_direct_account_mode() {
        // Bundle mode was retired in Phase 4c of
        // SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md — persist_oauth_success
        // always persists a direct account now, regardless of the
        // (now-vestigial) direct_account/into_bundle_id parameters.
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let (bundle_id, account_id) = persist_oauth_success(
            &wstore,
            &broker,
            true,
            "acc-1",
            None,
            "claude",
            Some("/some/dir"),
            "sess-route",
        );
        assert_eq!(bundle_id, "", "bundle id is always empty now");
        assert_eq!(account_id, Some("acc-1".to_string()));
        assert!(wstore.identity_get("acc-1").unwrap().is_some());
    }

    #[test]
    fn persist_oauth_success_skips_persistence_when_account_id_is_empty() {
        // Reagent P1: `auth.start` sets account_id = "" whenever
        // `direct_account` is false (the wire default) — a caller other
        // than the one production frontend path (which always sends
        // `directAccount: true`) can still reach this. Without the
        // empty-id guard, persist_oauth_direct_account's identity_upsert
        // would silently write/overwrite a db_accounts row with id="".
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let (bundle_id, account_id) = persist_oauth_success(
            &wstore,
            &broker,
            false,
            "",
            None,
            "claude",
            Some("/some/dir"),
            "sess-empty",
        );
        assert_eq!(bundle_id, "");
        assert_eq!(account_id, None, "empty account_id must not be persisted");
        assert!(
            wstore.identity_get("").unwrap().is_none(),
            "no row with id=\"\" should ever be written"
        );
    }
}

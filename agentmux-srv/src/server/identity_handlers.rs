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
                    "auth.start"
                );
                // OAuth Bundles PR C invariant — when an OAuth flow runs
                // against a bundle, the CLI's OAuth tokens land INSIDE the
                // bundle's per-provider config dir, NOT ambient at
                // `~/.claude/`. Compute the dir from the registry
                // (single source of truth — `auth_dir_name` per provider)
                // and the per-bundle root from `DataPaths::identity_dir`.
                // Mirror the dir into `auth_env` under the provider's
                // `auth_config_dir_env_var` (e.g. `CLAUDE_CONFIG_DIR`).
                // This overrides whatever the frontend computed via the
                // legacy ambient `ensureAuthDir` path so a bundle-bound
                // launch never accidentally writes to the global dir.
                //
                // When `into_bundle_id` is empty (ambient launch — no
                // bundle context), keep the legacy `auth_env` exactly so
                // the existing ambient flow keeps working.
                //
                // Errors (path resolve, mkdir) log + fall back to the
                // legacy env — never abort `auth.start` over a per-bundle
                // dir issue. Mirrors the `inject_identity_env` pattern.
                let mut auth_env = req.auth_env;
                let bundle_dir = compute_and_ensure_bundle_dir(
                    req.into_bundle_id.as_deref(),
                    &req.provider_id,
                    &mut auth_env,
                );
                let r = mgr.start_session(req.provider_id.clone(), req.into_bundle_id.clone());
                spawn_auth_cli(
                    mgr,
                    wstore,
                    broker,
                    r.session_id.clone(),
                    req.provider_id,
                    req.into_bundle_id,
                    bundle_dir,
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
/// when `into_bundle_id` AND `bundle_dir` are both set, the function
/// persists the OAuth binding into the bundle: a `SecretRef::OAuthConfigDir`
/// account is upserted (status `valid`) and bound via
/// `bundle_identity_bind`. This is the §4.5 OAuth-success invariant —
/// after this point, future launches of any agent against the bundle
/// resolve through `inject_identity_env`'s oauth-class dispatch (PR B)
/// and reuse the same CLI-managed tokens inside `bundle_dir`.
///
/// On failure or when `into_bundle_id` is empty (ambient launch), the
/// per-bundle binding step is skipped. The bundle row (if any was
/// auto-created by the New Identity modal) stays — the user's next
/// attempt reuses it.
#[allow(clippy::too_many_arguments)]
fn spawn_auth_cli(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    wstore: Arc<Store>,
    broker: Arc<Broker>,
    session_id: String,
    provider_id: String,
    into_bundle_id: Option<String>,
    bundle_dir: Option<String>,
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
        // Shared between drain + post-exit. The drain sets it after
        // persisting on a LoginSuccess match; the post-exit transition
        // block (below) checks it and skips its entire success path if
        // already transitioned — without this guard the drain's
        // persist + post-exit's persist both ran on every successful
        // OAuth, producing orphan IdentityAccount rows (each `Uuid::new_v4`)
        // and duplicate `identitybundlebindings:changed:<id>` publishes.
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
                        // Persist the OAuthConfigDir binding into the
                        // bundle. When `into_bundle_id` is empty (no
                        // bundle context, e.g. ambient continuation) or
                        // `bundle_dir` failed to resolve at spawn, the
                        // helper skips persistence and the session still
                        // succeeds — the user just won't get bundle-
                        // backed token reuse next launch. Bundle id
                        // returned to the frontend is the real bundle id
                        // when persistence happened, or a synthetic
                        // placeholder otherwise so existing UI keeps
                        // its filter-on-prefix behaviour.
                        let bundle_id = persist_oauth_binding_or_synthetic(
                            &wstore_stdout,
                            &broker_stdout,
                            into_bundle_id_stdout.as_deref(),
                            &provider_id_stdout,
                            bundle_dir_stdout.as_deref(),
                            &sid_stdout,
                        );
                        mgr_stdout.finish_success(&sid_stdout, bundle_id);
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
        // ran). Without this guard, persist_oauth_binding_or_synthetic
        // would fire a second time on every successful OAuth — a fresh
        // IdentityAccount UUID would be upserted, `bundle_identity_bind`
        // would repoint to the new account (orphaning the first), and
        // the broker would re-publish the bindings-changed event.
        // Reagent P1 on #981.
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
                        let bundle_id = persist_oauth_binding_or_synthetic(
                            &wstore_for_task,
                            &broker_for_task,
                            into_bundle_id_for_task.as_deref(),
                            &provider_id_for_task,
                            bundle_dir_for_task.as_deref(),
                            &session_id_for_task,
                        );
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
/// Same OAuth-success invariant as the pipes path — when
/// `into_bundle_id` and `bundle_dir` are both set and `confirm_authenticated`
/// returns true, persists `SecretRef::OAuthConfigDir` + binding before
/// `finish_success`.
#[allow(clippy::too_many_arguments)]
fn spawn_auth_cli_pty(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    wstore: Arc<Store>,
    broker: Arc<Broker>,
    session_id: String,
    provider_id: String,
    into_bundle_id: Option<String>,
    bundle_dir: Option<String>,
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
                            let success_for_detached = Arc::clone(&success_transitioned_drain);
                            let handle = tokio::runtime::Handle::current().spawn(async move {
                                if confirm_authenticated(&cli, &args, &env).await {
                                    let bundle_id = persist_oauth_binding_or_synthetic(
                                        &wstore2,
                                        &broker2,
                                        into_bundle_id2.as_deref(),
                                        &provider_id2,
                                        bundle_dir2.as_deref(),
                                        &sid2,
                                    );
                                    mgr2.finish_success(&sid2, bundle_id);
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
                        let bundle_id = persist_oauth_binding_or_synthetic(
                            &wstore_for_task,
                            &broker_for_task,
                            into_bundle_id_for_task.as_deref(),
                            &provider_id_for_task,
                            bundle_dir_for_task.as_deref(),
                            &session_id_for_task,
                        );
                        mgr_for_task.finish_success(&session_id_for_task, bundle_id);
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
/// Per `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.5: the dir
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

/// On a successful OAuth handshake (CLI exited 0 + authCheckCommand
/// confirmed), persist the OAuth binding into the bundle and return
/// the real bundle id to use in the `Success` wire status.
///
/// "Persist the binding" =
///   1. Upsert an `IdentityAccount` with:
///        - provider = `<provider_id>`
///        - kind = "oauth"
///        - secret_ref = `SecretRef::OAuthConfigDir { dir }`
///        - status = "valid"
///   2. Bind it via `bundle_identity_bind(bundle_id, provider, account_id)`.
///   3. Publish `identitybundlebindings:changed:<bundle_id>` so the
///      Launch modal (and any other open Identity pane) re-fetches
///      the new binding and the launch button enables without a
///      manual refresh.
///
/// Per-binding errors (account upsert, bind, broker publish) are
/// logged + downgraded — the success transition still fires with the
/// real bundle id when possible, and falls back to the legacy
/// `pending-bundle-for-<sid>` synthetic when not. The OAuth CLI's
/// tokens are already on disk inside `bundle_dir` either way; the
/// resolver layer (PR B) just won't find a binding to point at them.
/// Same shape as `inject_identity_env`'s "log + skip, never abort".
///
/// When `into_bundle_id` is empty or `bundle_dir` is `None`, returns
/// the synthetic placeholder unchanged — the legacy ambient path
/// (PR A behaviour) is preserved.
fn persist_oauth_binding_or_synthetic(
    wstore: &Arc<Store>,
    broker: &Arc<Broker>,
    into_bundle_id: Option<&str>,
    provider_id: &str,
    bundle_dir: Option<&str>,
    session_id: &str,
) -> String {
    let synthetic = || format!("pending-bundle-for-{session_id}");
    let bundle_id = match into_bundle_id.filter(|s| !s.is_empty()) {
        Some(b) => b,
        None => return synthetic(),
    };
    let dir = match bundle_dir.filter(|s| !s.is_empty()) {
        Some(d) => d,
        None => {
            tracing::warn!(
                target: "identity",
                bundle_id,
                provider_id,
                "auth success: bundle_dir unresolved — skipping binding persist"
            );
            return synthetic();
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    // Re-OAuth (token expiry / Reconnect): load any existing binding
    // for this (bundle, provider) and reuse its `account_id` so the
    // upsert UPDATES the prior IdentityAccount in place instead of
    // creating a fresh UUID + orphaning the old row in
    // `db_identity_accounts`. Fresh UUID only on first bind. codex
    // P2 follow-up on #981.
    let account_id = wstore
        .bundle_identity_bindings(bundle_id)
        .ok()
        .into_iter()
        .flatten()
        .find(|b| b.provider == provider_id)
        .map(|b| b.account_id)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let account = IdentityAccount {
        id: account_id.clone(),
        name: format!("{provider_id}-oauth"),
        provider: provider_id.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string() },
        context: serde_json::json!({}),
        // Per spec §4.4: a binding the user JUST OAuth'd into is `valid`
        // by definition — the token file was written within the past
        // few seconds. The resolver's expiry probe (PR D) refines this
        // on every spawn.
        status: crate::identity::resolver::oauth_status::VALID.to_string(),
        created_at: now,
        updated_at: now,
    };
    if let Err(e) = wstore.identity_upsert(&account) {
        tracing::warn!(
            target: "identity",
            bundle_id,
            provider_id,
            error = %e,
            "auth success: identity_upsert failed — falling back to synthetic bundle id"
        );
        return synthetic();
    }
    if let Err(e) = wstore.bundle_identity_bind(bundle_id, provider_id, &account_id) {
        tracing::warn!(
            target: "identity",
            bundle_id,
            provider_id,
            account_id,
            error = %e,
            "auth success: bundle_identity_bind failed — account row persisted but no binding"
        );
        return synthetic();
    }
    // Broker push so the frontend's `identitybundlebindings:changed:<id>`
    // listener (AgentLaunchModal createEffect) refetches bindings and
    // flips `hasMatchingBinding` → true without a manual reload.
    broker.publish(crate::backend::wps::WaveEvent {
        event: format!("identitybundlebindings:changed:{bundle_id}"),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
    tracing::info!(
        target: "identity",
        bundle_id,
        provider_id,
        account_id,
        dir,
        "auth success: OAuth binding persisted"
    );
    bundle_id.to_string()
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
    use crate::backend::storage::store::Identity;

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

    // ── OAuth Bundles PR C invariant ──────────────────────────────
    //
    // Round-trip: spawn-dir computation → persist OAuth binding →
    // bundle row carries a `SecretRef::OAuthConfigDir` pointing at
    // exactly that dir, with status = "valid".

    #[test]
    fn persist_oauth_binding_round_trip() {
        // Per spec §4.5: on auth success against bundle <id> for
        // provider <P>, the handler must
        //   1. compute the dir = DataPaths::identity_dir(<id>).join(P.auth_dir_name)
        //   2. set the provider's auth_config_dir_env_var to that dir
        //      in the spawn env (so the CLI writes tokens there, not
        //      at ~/.<P>/),
        //   3. on confirm, upsert an `IdentityAccount` whose
        //      `secret_ref` is `SecretRef::OAuthConfigDir { dir }`
        //      and `status = "valid"`,
        //   4. bind it via `bundle_identity_bind`.
        //
        // Together those three facts let the next launch's
        // `inject_identity_env` find the OAuth account, read its
        // OAuthConfigDir pointer, and inject CLAUDE_CONFIG_DIR with
        // the same path — closing the bundle loop.

        // Use a tempdir as the agentmux home so DataPaths resolves
        // without depending on the user's real ~/.agentmux. Local
        // mutex so two env-var-touching tests in this module can't
        // race (agentmux-common's TEST_ENV_LOCK is `pub(crate)`,
        // not reachable from here).
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());
        // DataPaths::from_env() wants every AGENTMUX_*_DIR — easiest
        // to compute paths once and export them.
        let paths = agentmux_common::DataPaths::resolve(
            "0.0.0-test",
            &agentmux_common::RuntimeMode::Installed,
        )
        .unwrap();
        paths.ensure_dirs().unwrap();
        for (k, v) in paths.to_env_vars() {
            std::env::set_var(k, v);
        }

        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());

        // Seed the bundle that the OAuth flow targets (PR 1 #969
        // creates this row up-front when the user names the new
        // identity; we mirror that here).
        let bundle_id = "id-oauth-pr-c";
        let identity = Identity {
            id: bundle_id.to_string(),
            name: "Work".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        wstore.bundle_identity_upsert(&identity).unwrap();

        // Step 1+2: compute and ensure dir, inject env var.
        let mut auth_env: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // Simulate the frontend's legacy `ensureAuthDir` putting an
        // ambient dir in the env first — the function MUST override
        // it with the per-bundle dir.
        auth_env.insert("CLAUDE_CONFIG_DIR".to_string(), "/legacy/ambient/claude".to_string());
        let dir = compute_and_ensure_bundle_dir(
            Some(bundle_id),
            "claude",
            &mut auth_env,
        )
        .expect("oauth-class provider with bundle id must yield a dir");

        // Dir is `DataPaths::identity_dir(<id>).join(<auth_dir_name>)`
        // — claude's `auth_dir_name` is "claude" per the registry.
        let expected = paths
            .identity_dir(bundle_id)
            .expect("test bundle_id is a safe segment")
            .join("claude");
        assert_eq!(
            std::path::Path::new(&dir),
            expected,
            "per-bundle dir must equal DataPaths::identity_dir(id).join(auth_dir_name)"
        );
        // create_dir_all happened.
        assert!(expected.is_dir(), "dir should be created idempotently");
        // Env var was OVERRIDDEN with the per-bundle dir — registry-
        // sourced key (CLAUDE_CONFIG_DIR for claude).
        assert_eq!(
            auth_env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some(dir.as_str()),
            "auth_config_dir_env_var must point at the per-bundle dir, NOT the legacy ambient one",
        );

        // Step 3+4: simulate the post-confirm_authenticated() success
        // path. The function returns the REAL bundle id (not the
        // synthetic placeholder) and persists the account + binding.
        let returned = persist_oauth_binding_or_synthetic(
            &wstore,
            &broker,
            Some(bundle_id),
            "claude",
            Some(&dir),
            "auth-sess-test",
        );
        assert_eq!(
            returned, bundle_id,
            "success path must return the bundle id, not the synthetic placeholder"
        );

        // Binding row exists for (bundle, claude).
        let bindings = wstore.bundle_identity_bindings(bundle_id).unwrap();
        assert_eq!(bindings.len(), 1, "exactly one binding after one auth success");
        assert_eq!(bindings[0].identity_id, bundle_id);
        assert_eq!(bindings[0].provider, "claude");

        // Account row carries SecretRef::OAuthConfigDir { dir } —
        // the same dir we passed at spawn — and status = "valid".
        let acct = wstore
            .identity_get(&bindings[0].account_id)
            .unwrap()
            .expect("account row exists");
        assert_eq!(acct.provider, "claude");
        assert_eq!(acct.kind, "oauth");
        assert_eq!(acct.status, "valid");
        match acct.secret_ref {
            SecretRef::OAuthConfigDir { dir: persisted_dir } => {
                assert_eq!(
                    persisted_dir, dir,
                    "persisted OAuthConfigDir.dir must equal the spawn dir"
                );
            }
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }

        // Cleanup env so other tests don't inherit our overrides.
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        for (k, _) in paths.to_env_vars() {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn compute_dir_skipped_for_empty_bundle_id() {
        // Ambient launch (empty into_bundle_id) — no per-bundle dir
        // override, legacy auth_env left intact.
        let mut env = std::collections::HashMap::new();
        env.insert("CLAUDE_CONFIG_DIR".to_string(), "/legacy/ambient".to_string());
        let dir = compute_and_ensure_bundle_dir(None, "claude", &mut env);
        assert!(dir.is_none());
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/legacy/ambient"),
            "legacy ambient env must survive when no bundle id"
        );

        let mut env = std::collections::HashMap::new();
        let dir = compute_and_ensure_bundle_dir(Some(""), "claude", &mut env);
        assert!(dir.is_none(), "empty string bundle id is the same as None");
        assert!(env.is_empty());
    }

    #[test]
    fn compute_dir_skipped_for_api_key_provider() {
        // Api-key-class providers must NOT go through the OAuth
        // per-bundle dir override even when their registry entry
        // declares an `auth_config_dir_env_var` (kimi has
        // `KIMI_SHARE_DIR` so the registry test alone isn't enough —
        // the gate has to be provider_class).
        let mut env = std::collections::HashMap::new();
        let dir = compute_and_ensure_bundle_dir(Some("id-1"), "kimi", &mut env);
        assert!(dir.is_none(), "api-key provider class must skip the OAuth dir path");
        assert!(
            env.get("KIMI_SHARE_DIR").is_none(),
            "api-key providers must NEVER get their config-dir env var set by the OAuth-start path"
        );
        // Same for github (no registry entry at all).
        let mut env = std::collections::HashMap::new();
        let dir = compute_and_ensure_bundle_dir(Some("id-1"), "github", &mut env);
        assert!(dir.is_none());
        assert!(env.get("GITHUB_TOKEN").is_none());
    }

    #[test]
    fn persist_returns_synthetic_when_no_bundle_id() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let r = persist_oauth_binding_or_synthetic(
            &wstore,
            &broker,
            None,
            "claude",
            Some("/some/dir"),
            "sess-x",
        );
        assert_eq!(r, "pending-bundle-for-sess-x");
    }

    #[test]
    fn persist_returns_synthetic_when_no_bundle_dir() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let r = persist_oauth_binding_or_synthetic(
            &wstore,
            &broker,
            Some("id-1"),
            "claude",
            None,
            "sess-y",
        );
        assert_eq!(
            r, "pending-bundle-for-sess-y",
            "no bundle dir → no persistence → synthetic id"
        );
    }
}

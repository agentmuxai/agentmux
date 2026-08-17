// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Subprocess spawn/drain engine for the pre-launch OAuth flow.
//!
//! Split out of `identity_handlers.rs` (module-organization pass, see
//! `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`) —
//! owns `spawn_auth_cli` (plain piped-stdio spawn), `spawn_auth_cli_pty`
//! (PTY-backed spawn for providers whose auth subcommand requires an
//! interactive TTY), and `confirm_authenticated` (the shared
//! authCheckCommand probe both spawn paths use). Both spawn paths call
//! into `identity_auth_persist::persist_oauth_success` on the OAuth-
//! success path.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::backend::storage::store::Store;
use crate::backend::wps::Broker;

use super::identity_auth_persist::persist_oauth_success;

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
pub(crate) fn spawn_auth_cli(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    wstore: Arc<Store>,
    identity_store: Arc<Store>,
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
            identity_store,
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
    let identity_store_for_task = identity_store.clone();
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
        let mut cmd = Command::new(&cli_path);
        cmd.args(&auth_login_args)
            .envs(&auth_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // CREATE_NO_WINDOW: console-flash suppression, see agentmux-common/src/cli.rs
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = match cmd.spawn()
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
        let identity_store_stdout = identity_store_for_task.clone();
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
                            &identity_store_stdout,
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
                            &identity_store_for_task,
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
    identity_store: Arc<Store>,
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
    let identity_store_for_task = identity_store.clone();
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
        let identity_store_drain = identity_store_for_task.clone();
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
                            let identity_store2 = identity_store_drain.clone();
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
                                        &identity_store2,
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
                            &identity_store_for_task,
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

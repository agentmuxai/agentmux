// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `spawn_auth_cli`: one login session's state (`Login`), the piped-stdio run and the PTY run of the
//! provider CLI, and how each ends the session (success saved, failure, or ended meanwhile).

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc::{Receiver, Sender};

use super::*;
use crate::identity::auth_session::AuthSessionManager;
use crate::server::identity_auth_persist::persist_oauth_success;

/// One login session: where it is recorded, where it saves, and the CLI that runs it and checks it.
pub(super) struct Login {
    pub(super) mgr: Arc<AuthSessionManager>,
    pub(super) mstore: Arc<Store>,
    pub(super) identity_store: Arc<Store>,
    pub(super) broker: Arc<Broker>,
    pub(super) session_id: String,
    pub(super) provider_id: String,
    pub(super) into_bundle_id: Option<String>,
    pub(super) bundle_dir: Option<String>,
    pub(super) direct_account: bool,
    pub(super) account_id: String,
    pub(super) cli_path: String,
    pub(super) auth_login_args: Vec<String>,
    pub(super) auth_check_args: Vec<String>,
    pub(super) auth_env: HashMap<String, String>,
}

/// The session ended (replaced, cancelled, timed out) before its login could be saved.
struct Ended;

impl Login {
    /// Whether the provider's auth check passes now (`confirm_authenticated`).
    async fn confirmed(&self) -> bool {
        confirm_authenticated(&self.cli_path, &self.auth_check_args, &self.auth_env).await
    }

    /// Save the account and finish the session as a success. `false` when the session ended first: nothing is saved.
    fn save_success(&self) -> bool {
        // Persist the standalone OAuthConfigDir account
        // (always direct-account mode now). If `bundle_dir`
        // (the account's own isolation dir) failed to
        // resolve at spawn, persist_oauth_success skips
        // persistence and the session still succeeds.
        // Read the email BEFORE finish_success consumes the
        // session — see AuthSessionManager::captured_email.
        // Ended meanwhile (replaced, cancelled, timed out): a newer sign-in
        // may own the directory, so this login is not saved.
        if self.mgr.ended(&self.session_id) {
            tracing::info!(session_id = %self.session_id, "auth.spawn: session ended before its login was saved; not saving it");
            return false;
        }
        let login_email = self.mgr.captured_email(&self.session_id);
        let (bundle_id, account_id) = persist_oauth_success(
            &self.mstore,
            &self.identity_store,
            &self.broker,
            self.direct_account,
            &self.account_id,
            self.into_bundle_id.as_deref(),
            &self.provider_id,
            self.bundle_dir.as_deref(),
            &self.session_id,
            login_email.as_deref(),
        );
        self.mgr.finish_success(&self.session_id, bundle_id, account_id);
        true
    }
}

/// The piped-stdio run: the task is attached to the session so cancelling it aborts the CLI.
pub(super) fn spawn_piped(login: Arc<Login>, stdin_tx: Sender<String>, stdin_rx: Receiver<String>) {
    let handle = tokio::spawn(run_piped(login.clone(), stdin_rx));
    login.mgr.attach_process(&login.session_id, handle, stdin_tx);
}

async fn run_piped(login: Arc<Login>, stdin_rx: Receiver<String>) {
    let (mgr, session_id) = (&login.mgr, &login.session_id);
    tracing::info!(
        session_id = %session_id,
        provider_id = %login.provider_id,
        cli_path = %login.cli_path,
        auth_login_args = ?login.auth_login_args,
        "auth.spawn: launching provider CLI"
    );

    let Some(mut child) = spawn_piped_child(&login) else {
        return;
    };

    let stdout = child
        .stdout
        .take()
        .expect("piped stdout");
    let stderr = child
        .stderr
        .take()
        .expect("piped stderr");
    let stdin = child
        .stdin
        .take()
        .expect("piped stdin");

    let stdin_writer = forward_stdin(stdin, stdin_rx);

    // Shared between drain + post-exit. The drain sets it after
    // persisting on a LoginSuccess match; the post-exit transition
    // block (`settle_piped_exit`) checks it and skips its entire success path if
    // already transitioned — without this guard the drain's
    // persist + post-exit's persist both ran on every successful
    // OAuth, producing orphan IdentityAccount rows (each `Uuid::new_v4`)
    // and duplicate `identityaccounts:changed` publishes.
    // Reagent P1 on #981.
    let success_transitioned = Arc::new(AtomicBool::new(false));
    let stdout_drain = drain_stdout(login.clone(), stdout, Arc::clone(&success_transitioned));
    let stderr_drain = drain_stderr(login.clone(), stderr);

    // Wait for the child to exit (or for our task to be aborted
    // by cancel_session — in which case kill_on_drop handles the
    // child).
    let exit = child.wait().await;
    tracing::info!(session_id = %session_id, exit = ?exit, "auth.spawn: child exited");

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
    if !success_transitioned.load(Ordering::Acquire) && settle_piped_exit(&login, exit).await.is_err() {
        return;
    }

    mgr.detach_process(session_id);
}

/// Spawn the provider CLI with piped stdio. `None` (the session already failed) when it cannot be started.
fn spawn_piped_child(login: &Login) -> Option<tokio::process::Child> {
    use std::process::Stdio;
    use tokio::process::Command;
    let (mgr, session_id, cli_path) = (&login.mgr, &login.session_id, &login.cli_path);

    // Spawn the CLI. kill_on_drop guarantees cleanup if our
    // task is aborted (cancel path).
    let mut cmd = Command::new(cli_path);
    // A provider OAuth CLI is a third-party, network-connected binary.
    // Without this it inherits srv's whole AGENTMUX_* environment — channel,
    // data dir, cache dir, and this instance's API endpoint. Stricter than
    // the pane policy: not even the in-pane helper keep-set applies here.
    // (ReAgent P0, round 2, on PR #3326.)
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(&login.auth_login_args)
        .envs(&login.auth_env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // CREATE_NO_WINDOW: console-flash suppression, see crates/common/src/cli.rs
    #[cfg(windows)]
    {
        cmd.no_window();
    }
    let child = match cmd.spawn()
    {
        Ok(c) => {
            tracing::info!(
                session_id = %session_id,
                pid = ?c.id(),
                "auth.spawn: child started"
            );
            c
        }
        Err(e) => {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "auth.spawn: Command::spawn failed"
            );
            mgr.finish_failure(
                session_id,
                format!("spawn `{cli_path}` failed: {e}"),
            );
            mgr.detach_process(session_id);
            return None;
        }
    };
    Some(child)
}

/// Forward pasted callback URLs to the CLI's stdin until the sender is dropped.
fn forward_stdin(mut stdin: tokio::process::ChildStdin, mut stdin_rx: Receiver<String>) -> tokio::task::JoinHandle<()> {
    use tokio::io::AsyncWriteExt;
    // Stdin writer — forwards callback URLs (pasted back by the
    // user when browser auto-open failed) into the CLI process.
    tokio::spawn(async move {
        while let Some(line) = stdin_rx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if stdin.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    })
}

/// Feed stdout into the session; on a login-success line that the auth check confirms, save the account.
fn drain_stdout(login: Arc<Login>, stdout: tokio::process::ChildStdout, success_transitioned: Arc<AtomicBool>) -> tokio::task::JoinHandle<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    // Stdout drain — line-by-line into the session manager. When
    // we see a login-success pattern, confirm via authCheckCommand
    // and transition to Success. Don't break the loop — some CLIs
    // continue printing after the success line.
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // NOT logging the raw line — OAuth providers print
            // auth URLs / callback codes / device codes on stdout
            // and even debug-level logs would persist those into
            // ~/.agentmux/logs/. Length-only is enough for "the
            // CLI emitted something" diagnostics. Reagent P2 on #847.
            tracing::debug!(session_id = %login.session_id, bytes = line.len(), "auth.spawn: stdout line");
            let m = login.mgr.record_line(&login.session_id, &line);
            if !success_transitioned.load(Ordering::Acquire)
                && matches!(
                    m,
                    Some(crate::identity::auth_patterns::AuthPatternMatch::LoginSuccess { .. })
                )
                && login.confirmed().await
            {
                if !login.save_success() {
                    return;
                }
                success_transitioned.store(true, Ordering::Release);
            }
        }
    })
}

/// Feed stderr into the session: some CLIs print the OAuth URL there.
fn drain_stderr(login: Arc<Login>, stderr: tokio::process::ChildStderr) -> tokio::task::JoinHandle<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    // Stderr drain — same matcher path. Some CLIs emit the OAuth
    // URL on stderr (verbose logging style).
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // Same redaction rationale as `drain_stdout`.
            tracing::debug!(session_id = %login.session_id, bytes = line.len(), "auth.spawn: stderr line");
            let _ = login.mgr.record_line(&login.session_id, &line);
        }
    })
}

/// End the session by how the CLI exited, when its output did not already: `Err` when the session ended first.
async fn settle_piped_exit(login: &Login, exit: std::io::Result<std::process::ExitStatus>) -> Result<(), Ended> {
    let (mgr, session_id) = (&login.mgr, &login.session_id);
    match exit {
        Ok(s) if s.success() => {
            if login.confirmed().await {
                if !login.save_success() {
                    return Err(Ended);
                }
            } else {
                mgr.finish_failure(session_id, "CLI exited 0 but authentication check failed".to_string());
            }
        }
        Ok(s) => {
            mgr.finish_failure(session_id, format!("CLI exited with status {s}"));
        }
        Err(e) => {
            mgr.finish_failure(session_id, format!("wait error: {e}"));
        }
    }
    Ok(())
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
pub(super) fn spawn_auth_cli_pty(login: Arc<Login>, stdin_tx: Sender<String>, stdin_rx: Receiver<String>) {
    tracing::info!(
        session_id = %login.session_id,
        provider_id = %login.provider_id,
        cli_path = %login.cli_path,
        auth_login_args = ?login.auth_login_args,
        "auth.spawn (PTY): launching provider CLI"
    );

    let Some(started) = start_pty(&login) else {
        return;
    };
    let handle = tokio::spawn(run_pty(login.clone(), started, stdin_rx));
    login.mgr.attach_process(&login.session_id, handle, stdin_tx);
}

/// The PTY, the child on it, and the ends of the PTY the run reads and writes.
struct Started {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pair: portable_pty::PtyPair,
    reader: Box<dyn std::io::Read + Send>,
    writer: Arc<std::sync::Mutex<Box<dyn std::io::Write + Send>>>,
}

/// Allocate the PTY, spawn the provider CLI on it and register its PID. `None` (the session already failed) when
/// any of that cannot be done.
fn start_pty(login: &Login) -> Option<Started> {
    use portable_pty::CommandBuilder;
    let (mgr, session_id, cli_path) = (&login.mgr, &login.session_id, &login.cli_path);

    // Allocate the PTY, build the command, spawn the child, and
    // register the PID — all synchronously, BEFORE the async drain/
    // wait task and BEFORE `attach_process`. That way no
    // `cancel_session` racing with the spawn can find a registered
    // handle but a missing PID; either the PID is registered first
    // (cancel can kill), or the early-return path fired and there
    // is no child to kill.
    let pair = open_pty(login)?;

    // On Windows, resolve past the npm `.cmd`/`.bat` shim: CommandBuilder on the
    // raw shim path goes through `cmd.exe /c`, which hangs under a real ConPTY
    // (the desktop host's login PTY found this first). `None` means the shim
    // matched no known npm shape: fail rather than fall back to the same hang.
    #[cfg(windows)]
    let (spawn_program, spawn_prefix_args) = match agentmux_common::resolve_cli_spawn_target(&cli_path) {
        Some(target) => target,
        None => {
            mgr.finish_failure(session_id, format!("could not resolve .cmd/.bat shim for spawn: {cli_path}"));
            mgr.detach_process(session_id);
            return None;
        }
    };
    #[cfg(not(windows))]
    let (spawn_program, spawn_prefix_args) = (cli_path.clone(), Vec::<String>::new());
    let mut cmd = CommandBuilder::new(&spawn_program);
    // Same as the non-PTY path (`spawn_piped_child`): CommandBuilder seeds from vars_os(), so
    // without this the provider CLI inherits the full instance identity.
    crate::backend::pane_env::sanitize_external_pty_command(&mut cmd);
    for a in &spawn_prefix_args {
        cmd.arg(a);
    }
    for a in &login.auth_login_args {
        cmd.arg(a);
    }
    for (k, v) in &login.auth_env {
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
            mgr.finish_failure(session_id, format!("PTY spawn `{cli_path}` failed: {e}"));
            mgr.detach_process(session_id);
            return None;
        }
    };

    if let Some(pid) = child.process_id() {
        mgr.attach_pty_pid(session_id, pid);
    }

    let reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "auth.spawn (PTY): try_clone_reader failed");
            mgr.finish_failure(session_id, format!("PTY reader: {e}"));
            mgr.detach_process(session_id);
            return None;
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(w) => Arc::new(std::sync::Mutex::new(w)),
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "auth.spawn (PTY): take_writer failed");
            mgr.finish_failure(session_id, format!("PTY writer: {e}"));
            mgr.detach_process(session_id);
            return None;
        }
    };
    Some(Started { child, pair, reader, writer })
}

async fn run_pty(login: Arc<Login>, started: Started, mut stdin_rx: Receiver<String>) {
    let Started { child, pair, reader, writer } = started;
    // Stdin writer: forward callback URLs from the manager into
    // the child's PTY input. portable_pty's master writer is sync;
    // wrap each write in `block_in_place` so the blocking IO
    // doesn't starve the tokio reactor when the PTY input buffer
    // is full.
    // Answers the cursor-position query (`ESC[6n`) Claude blocks on (issue
    // #2429) through the same writer, from the drain's thread.
    let dsr_writer = Arc::clone(&writer);
    let answer_dsr = move || {
        use std::io::Write;
        if let Ok(mut w) = dsr_writer.lock() {
            let _ = w.write_all(agentmux_common::login_pty::DSR_CURSOR_POSITION_REPLY);
            let _ = w.flush();
        }
    };
    let stdin_writer_handle = tokio::spawn(async move {
        while let Some(line) = stdin_rx.recv().await {
            let res = tokio::task::block_in_place(|| {
                use std::io::Write;
                let Ok(mut writer) = writer.lock() else {
                    return Err(std::io::Error::other("PTY writer lock poisoned"));
                };
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
    let drain_handle = pty_drain(login.clone(), reader, answer_dsr, Arc::clone(&success_transitioned));

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
    tracing::info!(session_id = %login.session_id, exit = ?exit, "auth.spawn (PTY): child exited");

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
    if !success_transitioned.load(Ordering::Acquire) && settle_pty_exit(&login, exit).await.is_err() {
        return;
    }

    login.mgr.detach_process(&login.session_id);
}

/// Read the PTY line by line into the session. Returns the detached confirm-and-save task of a login-success line.
fn pty_drain(
    login: Arc<Login>,
    reader: Box<dyn std::io::Read + Send>,
    answer_dsr: impl FnMut() + Send + 'static,
    success_transitioned: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<Option<tokio::task::JoinHandle<()>>> {
    // Drain: synchronous line reader from PTY master, feeds into
    // the same record_line matcher the pipes path uses. Runs in a
    // blocking thread; sends parsed events through a oneshot when
    // we hit a login-success pattern (so the async task can run
    // confirm_authenticated without crossing the blocking thread).
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(agentmux_common::login_pty::DsrRespondingReader::new(
            reader, answer_dsr,
        ));
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
                    tracing::debug!(session_id = %login.session_id, bytes = line.len(), "auth.spawn (PTY): line");
                    let m = login.mgr.record_line(&login.session_id, &line);
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
                        let handle = tokio::runtime::Handle::current()
                            .spawn(save_when_confirmed(login.clone(), Arc::clone(&success_transitioned)));
                        maybe_detached = Some(handle);
                    }
                }
                Err(_) => break,
            }
        }
        maybe_detached
    })
}

/// The detached half of a PTY login-success line: confirm it, save the account, and mark the success transitioned.
async fn save_when_confirmed(login: Arc<Login>, success_for_detached: Arc<AtomicBool>) {
    if login.confirmed().await {
        if !login.save_success() {
            return;
        }
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
}

/// End the session by how the PTY child exited, when its output did not already: `Err` when the session ended first.
async fn settle_pty_exit(
    login: &Login,
    exit: Result<std::io::Result<portable_pty::ExitStatus>, tokio::task::JoinError>,
) -> Result<(), Ended> {
    let (mgr, session_id) = (&login.mgr, &login.session_id);
    match exit {
        Ok(Ok(s)) if s.success() => {
            if login.confirmed().await {
                if !login.save_success() {
                    return Err(Ended);
                }
            } else {
                mgr.finish_failure(session_id, "CLI exited cleanly but auth-check still failed".to_string());
            }
        }
        Ok(Ok(s)) => {
            mgr.finish_failure(session_id, format!("CLI exited with code {:?}", s.exit_code()));
        }
        Ok(Err(e)) => {
            mgr.finish_failure(session_id, format!("PTY wait error: {e}"));
        }
        Err(e) => {
            mgr.finish_failure(session_id, format!("PTY wait task join error: {e}"));
        }
    }
    Ok(())
}

/// Allocate the PTY. `None` (the session already failed) when the OS cannot give one.
fn open_pty(login: &Login) -> Option<portable_pty::PtyPair> {
    use portable_pty::{native_pty_system, PtySize};
    let (mgr, session_id, provider_id) = (&login.mgr, &login.session_id, &login.provider_id);
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: 24,
        // Wide enough that the CLI never wraps its OAuth URL (`LOGIN_PTY_COLS`).
        cols: agentmux_common::login_pty::LOGIN_PTY_COLS,
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
            mgr.finish_failure(session_id, mux.to_wire().to_string());
            mgr.detach_process(session_id);
            return None;
        }
    };
    Some(pair)
}

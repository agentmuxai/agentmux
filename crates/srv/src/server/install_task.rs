// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The background task an `install.start` runs: take the install lock, reuse or
//! clear what another instance left, run `npm install` (or a release download),
//! publish every line of its output as `install_chunk` events, and end with one
//! `done` event. Every way out releases the session and the provider.
//!
//! `install_handlers.rs` owns the RPC handlers and the lookups; this file owns the
//! run, one step per function, in the order the run takes them.

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::json;
use tokio::task::JoinHandle;

use super::install_handlers::{effective_pin, installed_bin_in, provider_install_dir, InstallChunkEvent, InstallSessionRegistry, InstallStream};
use crate::backend::cli_install;
use crate::backend::mps::Broker;

/// How many lines of npm output a failed install logs.
const INSTALL_LOG_TAIL_LINES: usize = 20;

type OutputTail = Arc<Mutex<std::collections::VecDeque<String>>>;

/// What one install session publishes to, and releases when it ends.
struct InstallRun {
    broker: Arc<Broker>,
    registry: Arc<InstallSessionRegistry>,
    session_id: String,
    provider_id: String,
    /// `install:<sessionId>`, the scope every event of the session carries.
    scope: String,
    /// The last lines npm printed, for the log when an install fails: the dialog
    /// shows the whole output, but nothing else kept it, so a failed install left
    /// no trace of why (only `install.start` was logged).
    tail: OutputTail,
}

/// What `install.start` asked for, as the registry resolved it.
struct InstallJob {
    cli_command: String,
    npm_package: String,
    pinned_version: String,
}

impl InstallRun {
    fn log_done(&self, ok: bool, error: Option<&str>) {
        if ok {
            tracing::info!(session_id = %self.session_id, provider_id = %self.provider_id, "install.done");
        } else {
            let tail = self.tail.lock().iter().cloned().collect::<Vec<_>>().join(" | ");
            tracing::warn!(
                session_id = %self.session_id,
                provider_id = %self.provider_id,
                error = error.unwrap_or(""),
                npm_tail = %tail,
                "install.failed"
            );
        }
    }

    fn emit_line(&self, line: String, stream: InstallStream) {
        self.broker.publish_event::<crate::backend::mps_events::InstallChunk>(&InstallChunkEvent::line(&self.session_id, line, stream), vec![self.scope.clone()], 1024);
    }

    /// The `done` event with a free-text `error`.
    fn emit_done(&self, ok: bool, error: Option<String>) {
        self.log_done(ok, error.as_deref());
        self.broker.publish_event::<crate::backend::mps_events::InstallChunk>(&InstallChunkEvent::done(&self.session_id, ok, json!(error)), vec![self.scope.clone()], 1024);
    }

    /// The `done` event with the wire-format `AgentMuxError`, so the frontend can
    /// render a friendly `<ErrorBanner />`.
    fn emit_done_typed(&self, err: agentmux_common::AgentMuxError) {
        self.log_done(false, Some(&err.to_string()));
        self.broker.publish_event::<crate::backend::mps_events::InstallChunk>(&InstallChunkEvent::done(&self.session_id, false, err.to_wire()), vec![self.scope.clone()], 1024);
    }

    /// Forget the session and let another one install this provider.
    fn end(&self) {
        self.registry.drop_session(&self.session_id);
        self.registry.release_provider(&self.provider_id);
    }

    /// A failed install: the `done` event, then release.
    fn fail(&self, error: String) {
        self.emit_done(false, Some(error));
        self.end();
    }

    /// Publish each line `reader` yields, and keep it in the tail for the log.
    fn pipe_lines<R>(&self, reader: R, stream: InstallStream) -> JoinHandle<()>
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let (broker, session_id, scope, tail) = (self.broker.clone(), self.session_id.clone(), self.scope.clone(), self.tail.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push_tail(&tail, &line);
                broker.publish_event::<crate::backend::mps_events::InstallChunk>(&InstallChunkEvent::line(&session_id, line, stream), vec![scope.clone()], 1024);
            }
        })
    }
}

/// Run an `install.start` in the background.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_install_task(
    broker: Arc<Broker>,
    registry: Arc<InstallSessionRegistry>,
    session_id: String,
    provider_id: String,
    cli_command: String,
    npm_package: String,
    pinned_version: String,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let scope = format!("install:{}", session_id);
    let run = InstallRun { broker, registry, session_id, provider_id, scope, tail: Arc::default() };
    let job = InstallJob { cli_command, npm_package, pinned_version };
    tokio::spawn(run_install(run, job, cancel_rx));
}

async fn run_install(run: InstallRun, job: InstallJob, mut cancel_rx: tokio::sync::oneshot::Receiver<()>) {
    // One pin decides both the dir and the package version, so an
    // install can never put version X in a dir named for version Y.
    let pin = effective_pin(&run.provider_id, &job.pinned_version);
    let Some(provider_dir) = provider_install_dir(&run.provider_id, &pin) else {
        run.fail("cannot determine home directory".into());
        return;
    };
    let shared_dir = agentmux_common::DataPaths::from_env().and_then(|p| cli_install::shared_cli_dir(&p, &run.provider_id, &pin));

    // Held until this task ends.
    let _install_guard = match acquire_install_lock(&run, &pin, &mut cancel_rx).await {
        Ok(guard) => guard,
        Err(error) => return run.fail(error),
    };
    if let Some(dir) = shared_dir.as_deref() {
        match reuse_or_clear(dir, &job.cli_command) {
            Ok(true) => {
                run.emit_line("already installed by another AgentMux instance".to_string(), InstallStream::Stdout);
                run.emit_done(true, None);
                return run.end();
            }
            Ok(false) => {}
            Err(error) => return run.fail(error),
        }
    }
    // A release download or a uv venv, not an npm package (`backend::cli_download`, `backend::cli_uv`).
    if crate::backend::cli_download::is_managed_install(&run.provider_id) {
        let res = run_download_install(&run.provider_id, &job.cli_command, &run.broker, (&run.session_id, &run.scope), &mut cancel_rx).await;
        run.emit_done(res.is_ok(), res.err());
        return run.end();
    }
    // Makes the folder too, with the public registry and an empty npm config (cli_install.rs).
    let isolation = match cli_install::npm_isolation_args(&provider_dir) {
        Ok(isolation) => isolation,
        Err(e) => {
            // Disk-full, permission-denied and path-not-found go to the typed catalog so the
            // frontend shows a friendly message, not the raw OS error; other kinds stay Legacy.
            run.emit_done_typed(agentmux_common::AgentMuxError::from_io_with_path(provider_dir.display().to_string(), e));
            return run.end();
        }
    };
    let npm_args = npm_install_args(&run.provider_id, &job, &pin, &provider_dir, isolation);
    run.emit_line(format!("$ npm {}", npm_args.join(" ")), InstallStream::Stdout);

    let mut child = match spawn_npm(&npm_args) {
        Ok(c) => c,
        Err(e) => return run.fail(format!("spawn npm: {e}")),
    };
    let stdout_task = run.pipe_lines(child.stdout.take().expect("piped"), InstallStream::Stdout);
    let stderr_task = run.pipe_lines(child.stderr.take().expect("piped"), InstallStream::Stderr);

    tokio::select! {
        wait = child.wait() => {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            conclude_npm_exit(&run, wait, &provider_dir, shared_dir.as_deref(), &job.cli_command, &pin);
        }
        _ = &mut cancel_rx => {
            tracing::info!(session_id = %run.session_id, "install.cancel: killing child");
            let _ = child.kill().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            remove_partial_install(&run, &provider_dir);
            run.emit_done(false, Some("cancelled".into()));
        }
    }
    run.end();
}

/// The shared dir is written by every AgentMux instance on this
/// machine: serialize on its install lock. Waiting for another instance's
/// install must stay cancellable, so poll the lock and race `install.cancel`
/// instead of blocking in it: a blocking wait would ignore cancel, keep the
/// provider claimed, and could later report success for a cancelled session
/// (#3927). The error is the message of the `done` event.
async fn acquire_install_lock(
    run: &InstallRun,
    pin: &str,
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<cli_install::InstallGuard, String> {
    loop {
        let lock_provider = run.provider_id.clone();
        let lock_pin = pin.to_string();
        let res = tokio::task::spawn_blocking(move || {
            let paths = agentmux_common::DataPaths::from_env().ok_or_else(|| std::io::Error::other("DataPaths::from_env() failed"))?;
            cli_install::try_lock_install(&paths, &lock_provider, &lock_pin)
        })
        .await;
        match res {
            Ok(Ok(Some(guard))) => return Ok(guard),
            Ok(Ok(None)) => {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => continue,
                    _ = &mut *cancel_rx => {
                        tracing::info!(session_id = %run.session_id, "install.cancel: cancelled while waiting for another instance's install");
                        return Err("cancelled".into());
                    }
                }
            }
            Ok(Err(e)) => return Err(format!("cannot take the CLI install lock: {e}")),
            Err(e) => return Err(format!("install lock task panicked: {e}")),
        }
    }
}

/// Reuse only a FINISHED install (completion marker, not just a shim npm may
/// have written early); clear a failed or interrupted one. `Ok(true)`: there is
/// a finished install to reuse.
fn reuse_or_clear(dir: &Path, cli_command: &str) -> Result<bool, String> {
    if cli_install::is_complete(dir) && installed_bin_in(dir, cli_command).is_some() {
        return Ok(true);
    }
    cli_install::clear_unless_valid(dir, cli_command)
        .map(|()| false)
        .map_err(|e| format!("cannot clear an incomplete install at {}: {e}", dir.display()))
}

/// The arguments of the one `npm install` run: the pinned package plus any
/// companions (pi-acp's `pi`) in one run, one dir, one completion marker, then
/// the isolation flags.
fn npm_install_args(provider_id: &str, job: &InstallJob, pin: &str, provider_dir: &Path, isolation: Vec<String>) -> Vec<String> {
    let pkg_args: Vec<String> = match crate::backend::providers::get_provider(provider_id) {
        Some(p) if p.npm_package == job.npm_package => p.npm_install_specs(pin),
        _ if pin.is_empty() => vec![job.npm_package.clone()],
        _ => vec![format!("{}@{}", job.npm_package, pin)],
    };
    // `--progress=false` is unconditional: npm only renders the
    // progress bar when both stdout and stderr are TTYs, and this
    // task pipes both, so leaving progress at the default would
    // produce no visible spinner anyway. `--loglevel=verbose` is
    // also unconditional: the user's only signal of progress
    // during long installs is the per-package fetch/extract
    // chatter, so we always pay for the noise to gain the signal.
    let mut args: Vec<String> = std::iter::once("install".to_string())
        .chain(pkg_args)
        .chain([
            "--prefix".to_string(),
            provider_dir.to_string_lossy().to_string(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
            "--progress=false".to_string(),
            "--loglevel=verbose".to_string(),
            // A user's legacy-peer-deps would skip a package's peers.
            "--legacy-peer-deps=false".to_string(),
        ])
        .collect();
    args.extend(isolation);
    args
}

fn spawn_npm(args: &[String]) -> std::io::Result<tokio::process::Child> {
    use std::process::Stdio;
    let mut cmd = tokio::process::Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
    // `npm install` runs arbitrary postinstall scripts — the textbook case for
    // not handing a process this instance's identity or API endpoint.
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(args);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.no_window(); // CREATE_NO_WINDOW
    }
    cmd.spawn()
}

/// npm ended on its own: publish the one `done` event for how.
fn conclude_npm_exit(
    run: &InstallRun,
    wait: std::io::Result<std::process::ExitStatus>,
    provider_dir: &Path,
    shared_dir: Option<&Path>,
    cli_command: &str,
    pin: &str,
) {
    match wait {
        Ok(s) if s.success() => {
            // npm exit 0 ≠ "binary is on disk". Verify the
            // expected bin shim exists so a package/bin
            // rename or provider-config mismatch surfaces
            // as an install failure rather than a phantom
            // launch with a non-existent cmd path.
            if installed_bin_in(provider_dir, cli_command).is_some() {
                // Still under the install lock: publish the shared
                // install to every reader only now that it's verified.
                let marked = match shared_dir {
                    Some(dir) => cli_install::mark_complete(dir, pin),
                    None => Ok(()),
                };
                match marked {
                    Ok(()) => run.emit_done(true, None),
                    Err(e) => run.emit_done(false, Some(format!("cannot mark the install complete: {e}"))),
                }
            } else {
                run.emit_done(
                    false,
                    Some(format!("npm install reported success but {} not found in {}/node_modules/.bin/", cli_command, provider_dir.display())),
                );
            }
        }
        Ok(s) => run.emit_done(false, Some(format!("npm exited {:?}", s.code()))),
        Err(e) => run.emit_done(false, Some(format!("wait: {e}"))),
    }
}

/// Wipe the partial install of a cancelled run so a retry doesn't inherit a
/// half-written package-lock.json. Best-effort — logging only on failure.
fn remove_partial_install(run: &InstallRun, provider_dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(provider_dir) {
        // Best-effort cleanup; tag the log with the typed
        // code so grouped support requests can grep by
        // `amx_code` rather than free-text matching.
        let mux = agentmux_common::AgentMuxError::from_io_with_path(provider_dir.display().to_string(), e);
        tracing::warn!(
            target: "amx::error",
            session_id = %run.session_id,
            amx_code = %mux.code(),
            provider_dir = %provider_dir.display(),
            error = %mux,
            "install.cancel: remove partial dir failed"
        );
    }
}

/// An `install.start` for a provider that is a release download: fetch the
/// pinned file, check its hash, unpack, publish. Output lines go to the
/// session; `install.cancel` stops the transfer and removes the partial dir.
async fn run_download_install(
    provider_id: &str,
    cli_command: &str,
    broker: &Arc<Broker>,
    (session_id, scope): (&str, &str),
    cancel_rx: &mut tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    let paths = agentmux_common::DataPaths::from_env().ok_or("cannot determine home directory")?;
    let on_line = |_: &'static str, line: &str| {
        broker.publish_event::<crate::backend::mps_events::InstallChunk>(
            &InstallChunkEvent::line(session_id, line.to_string(), InstallStream::Stdout),
            vec![scope.to_string()],
            1024,
        );
    };
    let stop = std::sync::atomic::AtomicBool::new(false);
    let run = crate::backend::cli_download::install_managed_into(&paths, provider_id, cli_command, &on_line, &stop);
    tokio::pin!(run);
    let outcome = tokio::select! {
        r = &mut run => r,
        _ = &mut *cancel_rx => {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            (&mut run).await
        }
    };
    outcome.map(|_| ()).map_err(|e| e.to_string())
}

fn push_tail(tail: &Mutex<std::collections::VecDeque<String>>, line: &str) {
    let mut tail = tail.lock();
    if tail.len() == INSTALL_LOG_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_only_the_last_lines_of_npm_output() {
        let tail = parking_lot::Mutex::new(std::collections::VecDeque::new());
        for i in 0..(INSTALL_LOG_TAIL_LINES + 5) {
            push_tail(&tail, &format!("line {i}"));
        }
        let tail = tail.lock();
        assert_eq!(tail.len(), INSTALL_LOG_TAIL_LINES);
        assert_eq!(tail.front().map(String::as_str), Some("line 5"));
        assert_eq!(tail.back().cloned(), Some(format!("line {}", INSTALL_LOG_TAIL_LINES + 4)));
    }

    #[test]
    fn the_npm_arguments_name_the_pinned_package_the_prefix_and_then_the_isolation_flags() {
        let job = InstallJob { cli_command: "tool".into(), npm_package: "@scope/not-a-provider".into(), pinned_version: String::new() };
        let args = npm_install_args("no-such-provider", &job, "1.2.3", Path::new("/p/dir"), vec!["--registry=x".into()]);
        assert_eq!(
            args,
            [
                "install",
                "@scope/not-a-provider@1.2.3",
                "--prefix",
                "/p/dir",
                "--no-audit",
                "--no-fund",
                "--progress=false",
                "--loglevel=verbose",
                "--legacy-peer-deps=false",
                "--registry=x"
            ]
        );
        let unpinned = npm_install_args("no-such-provider", &job, "", Path::new("/p/dir"), Vec::new());
        assert_eq!(unpinned[1], "@scope/not-a-provider");
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Phase 1 of the fast-startup redesign
//! (`docs/specs/SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`,
//! tracked by issue #3258) — the primitives a "quiesce srv, run pending
//! migrations, come back" upgrade action needs, built and tested in
//! isolation ahead of wiring them into a live caller.
//!
//! WHY THIS IS SPLIT FROM ITS EVENTUAL CALLER. The spec's §4.3 sequence
//! (maintenance state → quiesce → snapshot → migrate → relaunch) has two
//! halves with very different risk profiles: the actual quiesce/migrate
//! logic below is a self-contained, thoroughly-testable async function; the
//! "suspend the live supervisor loop's respawn/teardown handling" half
//! means editing `supervisor/windows.rs`'s and `supervisor/unix.rs`'s
//! `tokio::select!` state machines — ~700- and ~1200-line functions this
//! repo has no way to exercise end-to-end outside a real multi-process
//! launcher run (see the I1-I6 isolation invariants in
//! `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`;
//! this is exactly the surface they exist to protect). §4.2 also hasn't landed yet, which is what
//! decides WHEN this runs at all — an upgrade-only srv mode most likely
//! means srv is never handed to the normal supervised loop as a
//! respawn-on-exit child in the first place, so the "suspend the loop"
//! problem may look completely different once that exists. Wiring this into
//! today's loops now would be guessing at an integration shape that phase 2
//! gets to actually decide. This module is deliberately the callable,
//! tested half; the calling half is follow-up work once §4.2 lands.
//! ([`quiesce_srv`] itself is live: both supervisors stop srv with it on
//! quit, after their loops have ended, so no respawn handling is involved.)
//!
//! Constraint 2 from the spec — "migrations only run with no live srv
//! holding that data dir" (`341faa981`'s data-loss incident) — is what
//! [`quiesce_srv`] exists to guarantee: it returns only once the OS has
//! genuinely reaped the process (`Child::wait()`'s own contract), replacing
//! `crates/cef/src/commands/backend.rs`'s dev-mode-only 300ms sleep
//! heuristic with a real wait for real exit.
//!
//! No snapshot step lives here on purpose: `agentmux-srv migrate`'s own
//! `apply_pending` (shared with the in-process daemon path) already takes
//! its own backup under the same cross-process migration lock before
//! applying anything (`crates/srv/src/migrations/runner.rs`'s
//! `backup_stores` call) — a launcher-side snapshot would be redundant with,
//! not a replacement for, that one. The spec's §4.3 step 3 was written
//! assuming the launcher would re-implement what `bootstrap/stores.rs` does for
//! the in-process path; going through the CLI subcommand instead means that
//! safety property already exists on this path.

use std::path::Path;
use std::time::Duration;

use tokio::process::Child;

use crate::data_dir::DataPaths;
use crate::srv_spawner::{run_migrate, SrvSpawnError};
use crate::startup_events::StartupEventSink;

/// How long [`quiesce_srv`] waits for srv to exit on its own before
/// escalating to a hard kill: long enough for srv to close every agent
/// (`agent_teardown::app_exit`, capped at `SRV_APP_EXIT_CAP`).
pub const GRACEFUL_QUIESCE_TIMEOUT: Duration = agentmux_common::process::SRV_EXIT_WAIT;

/// How [`quiesce_srv`] actually stopped the process — logged, and useful in
/// tests, but callers don't need to branch on it: either variant means the
/// same thing (the process is genuinely gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuiesceOutcome {
    /// The child had already exited before this call (e.g. a crash that
    /// raced the upgrade request).
    AlreadyExited,
    /// Exited on its own within the graceful window, after being told to.
    ExitedGracefully,
    /// Didn't exit in time and was force-killed.
    ExitedAfterForceKill,
}

/// Stop srv and return only once it has genuinely exited — never a fixed
/// sleep. The launcher's one way of stopping srv: on every quit (both
/// supervisors) and before a migration ([`run_migration_upgrade`]).
///
/// Telling srv to stop is dropping `stdin`, its keepalive pipe: srv reads
/// EOF as "shut down" on every platform (`bootstrap::install_shutdown_handlers`)
/// and closes every agent gracefully on the way out
/// (`agent_teardown::app_exit`). Unix also gets SIGTERM, which does the same.
/// Only if srv hasn't exited after `graceful_timeout` is it force-killed —
/// on Windows, whatever it still runs then goes with J0.
pub async fn quiesce_srv(
    child: &mut Child,
    stdin: Option<tokio::process::ChildStdin>,
    graceful_timeout: Duration,
) -> std::io::Result<(std::process::ExitStatus, QuiesceOutcome)> {
    if let Ok(Some(status)) = child.try_wait() {
        return Ok((status, QuiesceOutcome::AlreadyExited));
    }
    drop(stdin);
    #[cfg(not(target_os = "windows"))]
    crate::host_spawn::terminate_child_gracefully(child);
    match tokio::time::timeout(graceful_timeout, child.wait()).await {
        Ok(status) => Ok((status?, QuiesceOutcome::ExitedGracefully)),
        Err(_elapsed) => {
            child.start_kill()?;
            let status = child.wait().await?;
            Ok((status, QuiesceOutcome::ExitedAfterForceKill))
        }
    }
}

/// Why [`run_migration_upgrade`] didn't complete.
#[derive(Debug)]
pub enum UpgradeError {
    /// Stopping the running srv failed at the OS level (not "srv refused to
    /// stop" — [`quiesce_srv`] always eventually forces that; this is e.g.
    /// a `wait()` syscall failure).
    Quiesce(std::io::Error),
    /// The migration subprocess itself failed — see [`SrvSpawnError`] for
    /// which case (spawn failure, a real `AGENTMUXSRV-MIGRATION-FAILED`,
    /// etc). The data is left exactly as `agentmux-srv migrate` left it:
    /// possibly partially migrated (no batch transaction — see the spec's
    /// §4.3 "partial application" note), with a pre-migration backup
    /// already written under `~/.agentmux/shared/backups/`.
    Migrate(SrvSpawnError),
}

impl std::fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quiesce(e) => write!(f, "failed to stop the running srv: {e}"),
            Self::Migrate(e) => write!(f, "migration failed: {e}"),
        }
    }
}

/// Quiesce the running srv, then run pending migrations via the
/// `agentmux-srv migrate` subprocess. Does NOT relaunch srv afterward and
/// does NOT install a staged app update first — both are the caller's
/// responsibility once one exists (§4.2's boot gate for the former;
/// `app-update-check.md`'s still-unimplemented `install_update` for the
/// latter, per the spec's §5.4 ordering — a staged update must be installed
/// BEFORE this runs, so its migrations are the ones that end up applied).
///
/// `srv_child` must be the handle for the ALREADY-RUNNING srv this upgrade
/// is replacing — this function does not spawn or discover it — and
/// `srv_stdin` its stdin keepalive (see [`quiesce_srv`]).
pub async fn run_migration_upgrade(
    launcher_exe_dir: &Path,
    paths: &DataPaths,
    srv_child: &mut Child,
    srv_stdin: Option<tokio::process::ChildStdin>,
    sink: &StartupEventSink,
) -> Result<(), UpgradeError> {
    let (status, outcome) = quiesce_srv(srv_child, srv_stdin, GRACEFUL_QUIESCE_TIMEOUT)
        .await
        .map_err(UpgradeError::Quiesce)?;
    crate::log(&format!(
        "upgrade: srv quiesced ({:?}, exit={:?}) — starting migration",
        outcome, status
    ));

    run_migrate(launcher_exe_dir, paths, sink)
        .await
        .map_err(UpgradeError::Migrate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "windows"))]
    use tokio::io::{AsyncBufReadExt, BufReader};

    /// Portable "runs for a while, does nothing" child: `sleep` on Unix,
    /// `ping` on Windows (`timeout.exe` refuses to run with no console —
    /// "Input redirection is not supported" — the standard Windows-
    /// automation workaround for a long-lived, headless-safe dummy process
    /// is a ping loop against loopback, which never needs a console).
    fn spawn_long_lived() -> Child {
        #[cfg(not(target_os = "windows"))]
        {
            tokio::process::Command::new("sleep")
                .arg("30")
                .kill_on_drop(true)
                .spawn()
                .expect("spawn sleep")
        }
        #[cfg(target_os = "windows")]
        {
            tokio::process::Command::new("cmd")
                .args(["/C", "ping", "-n", "30", "127.0.0.1"])
                .kill_on_drop(true)
                .spawn()
                .expect("spawn ping")
        }
    }

    /// A child that ignores SIGTERM, forcing `quiesce_srv`'s escalation
    /// path (it has no stdin to close). Unix only: Windows sends no signal
    /// to ignore, so its escalation path is covered by the live-child test.
    ///
    /// Blocks until the child confirms its trap is actually installed
    /// before returning — sending SIGTERM immediately after `spawn()` races
    /// the shell's own startup (parse `-c`, run `trap`, THEN start `sleep`),
    /// and on a loaded CI runner that race is real: a signal arriving
    /// before `trap ''` executes finds SIGTERM at its default disposition
    /// and the shell dies immediately, which is exactly the CI-only flake
    /// this fixes (`ExitedGracefully` instead of `ExitedAfterForceKill`,
    /// ubuntu-latest, PR #3352). Piping stdout and reading one line back is
    /// a real synchronization point, not a fixed delay that would just
    /// narrow the window instead of closing it.
    #[cfg(not(target_os = "windows"))]
    async fn spawn_sigterm_immune() -> Child {
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "trap '' TERM; echo ready; sleep 30"])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sigterm-immune shell");

        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = BufReader::new(stdout).lines();
        let line = lines
            .next_line()
            .await
            .expect("read readiness line")
            .expect("readiness line present");
        assert_eq!(line.trim(), "ready", "trap must be installed before the test signals this child");

        child
    }

    #[tokio::test]
    async fn already_exited_child_is_reported_without_signaling_anything() {
        let mut child = tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) { vec!["/C", "exit 0"] } else { vec![] })
            .kill_on_drop(true)
            .spawn()
            .expect("spawn a trivially-exiting child");
        // Let it actually exit and get reaped before quiescing.
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("child did not exit");

        let (status, outcome) = quiesce_srv(&mut child, None, Duration::from_secs(1))
            .await
            .expect("quiesce_srv must not error on an already-exited child");

        assert_eq!(outcome, QuiesceOutcome::AlreadyExited);
        assert!(status.success());
    }

    #[tokio::test]
    async fn a_live_child_is_actually_stopped_and_reaped() {
        // The core guarantee this function exists for: not a sleep, a real
        // wait for a real exit. A long-lived process that were merely
        // signaled-and-assumed-dead (the bug this replaces) would still be
        // running here; this asserts the OS has actually reaped it.
        let mut child = spawn_long_lived();
        let pid = child.id().expect("live child has a pid");

        // A short graceful window: on Windows nothing here tells the dummy
        // to stop, so it is force-killed when the window ends.
        let (_status, _outcome) = tokio::time::timeout(
            Duration::from_secs(15),
            quiesce_srv(&mut child, None, Duration::from_secs(1)),
        )
        .await
        .expect("quiesce_srv hung instead of returning once the child exited")
        .expect("quiesce_srv errored");

        // A second wait on the same handle must not hang or find a live
        // process — the child is genuinely gone, not merely signaled.
        assert!(
            tokio::time::timeout(Duration::from_millis(500), child.wait())
                .await
                .is_ok(),
            "pid {pid} should already be reaped after quiesce_srv returned"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn a_sigterm_ignoring_child_is_force_killed_after_the_timeout() {
        use std::os::unix::process::ExitStatusExt;

        let mut child = spawn_sigterm_immune().await;

        let (status, outcome) = tokio::time::timeout(
            Duration::from_secs(10),
            // Short graceful window so the test doesn't wait the full
            // production 10s to prove the escalation path.
            quiesce_srv(&mut child, None, Duration::from_millis(300)),
        )
        .await
        .expect("quiesce_srv hung past its own escalation timeout")
        .expect("quiesce_srv errored");

        assert_eq!(outcome, QuiesceOutcome::ExitedAfterForceKill);
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    /// Closing stdin is the stop signal on every platform (it is the only
    /// one on Windows): a child that ignores SIGTERM but exits on stdin EOF,
    /// as srv does, exits gracefully.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn closing_stdin_stops_a_child_that_ignores_sigterm() {
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "trap '' TERM; echo ready; cat >/dev/null"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn stdin-reading shell");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let line = BufReader::new(stdout).lines().next_line().await.expect("read").expect("line");
        assert_eq!(line.trim(), "ready");

        let (status, outcome) = tokio::time::timeout(
            Duration::from_secs(10),
            quiesce_srv(&mut child, stdin, Duration::from_secs(5)),
        )
        .await
        .expect("quiesce_srv hung")
        .expect("quiesce_srv errored");

        assert_eq!(outcome, QuiesceOutcome::ExitedGracefully);
        assert!(status.success());
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn a_child_that_honors_sigterm_exits_gracefully_not_via_force_kill() {
        // Plain `sleep` has no signal handler — the default SIGTERM action
        // is immediate termination, so this should resolve almost
        // instantly, well inside the graceful window, and never reach the
        // force-kill branch.
        let mut child = spawn_long_lived();

        let (_status, outcome) = tokio::time::timeout(
            Duration::from_secs(5),
            quiesce_srv(&mut child, None, GRACEFUL_QUIESCE_TIMEOUT),
        )
        .await
        .expect("quiesce_srv hung")
        .expect("quiesce_srv errored");

        assert_eq!(outcome, QuiesceOutcome::ExitedGracefully);
    }
}

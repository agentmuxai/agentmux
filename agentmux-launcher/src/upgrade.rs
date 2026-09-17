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
//! `tokio::select!` state machines — 1000+-line functions this repo has no
//! way to exercise end-to-end outside a real multi-process launcher run
//! (see CLAUDE.md's I1-I6 isolation invariants; this is exactly the surface
//! they exist to protect). §4.2 also hasn't landed yet, which is what
//! decides WHEN this runs at all — an upgrade-only srv mode most likely
//! means srv is never handed to the normal supervised loop as a
//! respawn-on-exit child in the first place, so the "suspend the loop"
//! problem may look completely different once that exists. Wiring this into
//! today's loops now would be guessing at an integration shape that phase 2
//! gets to actually decide. This module is deliberately the callable,
//! tested half; the calling half is follow-up work once §4.2 lands.
//!
//! Constraint 2 from the spec — "migrations only run with no live srv
//! holding that data dir" (`341faa981`'s data-loss incident) — is what
//! [`quiesce_srv`] exists to guarantee: it returns only once the OS has
//! genuinely reaped the process (`Child::wait()`'s own contract), replacing
//! `agentmux-cef/src/commands/backend.rs`'s dev-mode-only 300ms sleep
//! heuristic with a real wait for real exit.
//!
//! No snapshot step lives here on purpose: `agentmux-srv migrate`'s own
//! `apply_pending` (shared with the in-process daemon path) already takes
//! its own backup under the same cross-process migration lock before
//! applying anything (`agentmux-srv/src/migrations/runner.rs`'s
//! `backup_stores` call) — a launcher-side snapshot would be redundant with,
//! not a replacement for, that one. The spec's §4.3 step 3 was written
//! assuming the launcher would re-implement what `bootstrap.rs` does for
//! the in-process path; going through the CLI subcommand instead means that
//! safety property already exists on this path.

use std::path::Path;
use std::time::Duration;

use tokio::process::Child;

use crate::data_dir::DataPaths;
use crate::srv_spawner::{run_migrate, SrvSpawnError};
use crate::startup_events::StartupEventSink;

/// How long [`quiesce_srv`] waits for srv to exit on its own (Unix: after
/// SIGTERM) before escalating to a hard kill. Generous on purpose — this
/// runs during a deliberate, user-initiated upgrade action, not a crash
/// recovery path, so there's no reason to race a slow-but-clean WAL
/// checkpoint.
pub const GRACEFUL_QUIESCE_TIMEOUT: Duration = Duration::from_secs(10);

/// How [`quiesce_srv`] actually stopped the process — logged, and useful in
/// tests, but callers don't need to branch on it: either variant means the
/// same thing (the process is genuinely gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuiesceOutcome {
    /// The child had already exited before this call (e.g. a crash that
    /// raced the upgrade request).
    AlreadyExited,
    /// Exited on its own within [`GRACEFUL_QUIESCE_TIMEOUT`] (Unix: after
    /// SIGTERM; Windows: see the "no graceful signal" note on
    /// `quiesce_srv`'s Windows branch).
    ExitedGracefully,
    /// Didn't exit in time (Unix only — Windows always takes this path, see
    /// below) and was force-killed.
    ExitedAfterForceKill,
}

/// Stop `child` and return only once it has genuinely exited — never a
/// fixed sleep. This is the fix for the exact gap
/// `agentmux-cef/src/commands/backend.rs:223`'s `sleep(300ms)` heuristic
/// left open: that comment already says "so the OS releases file locks
/// before we open the DB," which a sleep can only approximate and a real
/// `.wait()` guarantees outright.
///
/// # Windows
/// There is no graceful shutdown signal available here today — every
/// existing srv-teardown call site in this launcher already uses
/// `start_kill()` (`supervisor/windows.rs:574,741,1141`), relying on srv's
/// SQLite WAL durability rather than a clean in-process shutdown. Building
/// a graceful path for Windows is out of scope for this change; this
/// function stays consistent with that existing convention rather than
/// introducing a new, untested shutdown mechanism for one caller.
pub async fn quiesce_srv(
    child: &mut Child,
    graceful_timeout: Duration,
) -> std::io::Result<(std::process::ExitStatus, QuiesceOutcome)> {
    if let Ok(Some(status)) = child.try_wait() {
        return Ok((status, QuiesceOutcome::AlreadyExited));
    }

    #[cfg(target_os = "windows")]
    {
        child.start_kill()?;
        let status = child.wait().await?;
        Ok((status, QuiesceOutcome::ExitedAfterForceKill))
    }

    #[cfg(not(target_os = "windows"))]
    {
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
/// is replacing — this function does not spawn or discover it.
pub async fn run_migration_upgrade(
    launcher_exe_dir: &Path,
    paths: &DataPaths,
    srv_child: &mut Child,
    sink: &StartupEventSink,
) -> Result<(), UpgradeError> {
    let (status, outcome) = quiesce_srv(srv_child, GRACEFUL_QUIESCE_TIMEOUT)
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
    /// path. Unix only — Windows has no graceful signal to ignore in the
    /// first place (see `quiesce_srv`'s own doc comment), so its escalation
    /// path is unconditional and already covered by every Windows test here.
    #[cfg(not(target_os = "windows"))]
    fn spawn_sigterm_immune() -> Child {
        tokio::process::Command::new("sh")
            .args(["-c", "trap '' TERM; sleep 30"])
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sigterm-immune shell")
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

        let (status, outcome) = quiesce_srv(&mut child, Duration::from_secs(1))
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

        let (_status, _outcome) = tokio::time::timeout(
            Duration::from_secs(15),
            quiesce_srv(&mut child, GRACEFUL_QUIESCE_TIMEOUT),
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

        let mut child = spawn_sigterm_immune();

        let (status, outcome) = tokio::time::timeout(
            Duration::from_secs(10),
            // Short graceful window so the test doesn't wait the full
            // production 10s to prove the escalation path.
            quiesce_srv(&mut child, Duration::from_millis(300)),
        )
        .await
        .expect("quiesce_srv hung past its own escalation timeout")
        .expect("quiesce_srv errored");

        assert_eq!(outcome, QuiesceOutcome::ExitedAfterForceKill);
        assert_eq!(status.signal(), Some(libc::SIGKILL));
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
            quiesce_srv(&mut child, GRACEFUL_QUIESCE_TIMEOUT),
        )
        .await
        .expect("quiesce_srv hung")
        .expect("quiesce_srv errored");

        assert_eq!(outcome, QuiesceOutcome::ExitedGracefully);
    }
}

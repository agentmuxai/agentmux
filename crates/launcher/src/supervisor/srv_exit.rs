// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the Unix supervisor does when srv exits: classify the exit, and start
//! srv again (SPEC_UNIX_SRV_RESTART_BUDGET_2026_10_24). The Windows supervisor
//! does the same inline in `run_windows`'s srv arm.

use crate::logging::log;
use crate::{data_dir, mem_supervisor, srv_spawner, startup_events};
use std::time::Instant;

/// What to do about an srv exit.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SrvExit {
    /// It exited on purpose (clean exit, or a group-shutdown signal): the app is stopping.
    Shutdown,
    /// The computer is out of memory: wait for it to recover, then start srv again.
    WaitForMemory,
    /// Start srv again and restart the window process.
    Respawn,
    /// It keeps stopping: end the app.
    GiveUp,
    /// The computer stayed out of memory too often: end the app.
    GiveUpLowMemory,
}

/// The signals that mean the whole group is being shut down, not that srv crashed.
fn is_group_shutdown(signal: Option<i32>) -> bool {
    matches!(signal, Some(sig) if sig == libc::SIGINT || sig == libc::SIGTERM || sig == libc::SIGHUP)
}

/// Classify an srv exit and charge it to the right budget: `srv_restarts` (at most
/// [`super::SRV_RESTART_BUDGET`] in [`super::SRV_RESTART_WINDOW`]) for a crash, `srv_oom_restarts`
/// for one that happened while the computer was out of memory. On Linux a kernel OOM kill arrives
/// as SIGKILL, so the low commit reading decides, as for the window process.
pub(super) fn classify_srv_exit(
    code: Option<i32>,
    signal: Option<i32>,
    commit_free_mb: u64,
    now: Instant,
    srv_restarts: &mut Vec<Instant>,
    srv_oom_restarts: &mut Vec<Instant>,
) -> SrvExit {
    if code == Some(0) || is_group_shutdown(signal) {
        return SrvExit::Shutdown;
    }
    let low_memory = matches!(
        mem_supervisor::classify_host_exit(code.unwrap_or(1), commit_free_mb),
        mem_supervisor::HostExitClass::SystemOom
    );
    if low_memory {
        if mem_supervisor::budget_exhausted(
            srv_oom_restarts,
            now,
            mem_supervisor::OOM_RESTART_WINDOW,
            mem_supervisor::OOM_RESTART_BUDGET,
        ) {
            return SrvExit::GiveUpLowMemory;
        }
        return SrvExit::WaitForMemory;
    }
    if mem_supervisor::budget_exhausted(srv_restarts, now, super::SRV_RESTART_WINDOW, super::SRV_RESTART_BUDGET) {
        return SrvExit::GiveUp;
    }
    SrvExit::Respawn
}

/// What the supervisor does next about an srv exit.
pub(super) enum SrvAction {
    /// End the launcher with this exit code (already logged, and recorded when it is a give-up).
    Stop(i32),
    /// End the launcher with this exit code because srv keeps crashing past its restart budget:
    /// as [`SrvAction::Stop`], and the version is one the user may want to go back from.
    GiveUp(i32),
    /// Start srv again; `wait_for_memory` first when the computer is out of memory.
    Restart { why: crate::incidents::SrvStop, wait_for_memory: bool },
}

/// Decide what to do about srv's exit: classify it, charge the budgets, log it, and record and
/// show the give-up when the budget is spent. The three places that see srv exit call this.
pub(super) fn srv_exit_action(
    status: std::io::Result<std::process::ExitStatus>,
    srv_restarts: &mut Vec<Instant>,
    srv_oom_restarts: &mut Vec<Instant>,
) -> SrvAction {
    use std::os::unix::process::ExitStatusExt;
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            log(&format!("FATAL: srv wait failed: {e}"));
            return SrvAction::Stop(1);
        }
    };
    let (code, signal) = (status.code(), status.signal());
    let text = super::unix::srv_exit_text(&status);
    let commit_free = mem_supervisor::commit_free_mb();
    match classify_srv_exit(code, signal, commit_free, Instant::now(), srv_restarts, srv_oom_restarts) {
        SrvExit::Shutdown => {
            log("srv exited as part of shutdown — shutting down");
            SrvAction::Stop(0)
        }
        SrvExit::GiveUpLowMemory => {
            log(&format!(
                "srv exited {text} while the computer was out of memory ({commit_free} MB commit-free); \
                 the memory restart budget is spent ({} in {}s) — giving up",
                mem_supervisor::OOM_RESTART_BUDGET,
                mem_supervisor::OOM_RESTART_WINDOW.as_secs()
            ));
            crate::incidents::record(&crate::incidents::memory_give_up());
            crate::show_fatal_dialog(mem_supervisor::OOM_GIVEUP_TITLE, mem_supervisor::OOM_GIVEUP_BODY);
            SrvAction::Stop(code.unwrap_or(1))
        }
        SrvExit::GiveUp => {
            log(&format!(
                "srv exited {text}; the srv restart budget is spent ({} in {}s) — terminating launcher",
                super::SRV_RESTART_BUDGET,
                super::SRV_RESTART_WINDOW.as_secs()
            ));
            crate::incidents::record(&crate::incidents::crash_give_up(
                crate::incidents::GaveUpOn::Service,
                code.unwrap_or(1),
                super::SRV_RESTART_BUDGET,
                super::SRV_RESTART_WINDOW.as_secs(),
            ));
            SrvAction::GiveUp(1)
        }
        SrvExit::WaitForMemory => {
            log(&format!(
                "srv exited {text} while the computer was out of memory ({commit_free} MB commit-free) — \
                 waiting for memory to recover before starting it again"
            ));
            SrvAction::Restart { why: crate::incidents::SrvStop::LowMemory, wait_for_memory: true }
        }
        SrvExit::Respawn => {
            log(&format!(
                "srv exited UNEXPECTEDLY {text} — starting it again (restart {}/{})",
                srv_restarts.len(),
                super::SRV_RESTART_BUDGET
            ));
            let why = match signal {
                Some(sig) => crate::incidents::SrvStop::Signal(sig, signal_name(sig)),
                None => crate::incidents::SrvStop::Exited(code.unwrap_or(1)),
            };
            SrvAction::Restart { why, wait_for_memory: false }
        }
    }
}

/// The name of a signal, or "" when it has none worth a word.
pub(super) fn signal_name(sig: i32) -> &'static str {
    match sig {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGILL => "SIGILL",
        libc::SIGTRAP => "SIGTRAP",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGKILL => "SIGKILL",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGPIPE => "SIGPIPE",
        libc::SIGTERM => "SIGTERM",
        _ => "",
    }
}

/// The srv the launcher runs, replaced as a whole when it is started again.
pub(super) struct Srv {
    pub result: srv_spawner::SrvSpawnResult,
    pub child: tokio::process::Child,
    /// srv's stdin write end, held so that its EOF (the launcher died) is the only way srv sees one.
    pub stdin: Option<tokio::process::ChildStdin>,
    /// srv's pid, taken at spawn: tokio's `Child::id()` is `None` once the child has been waited on.
    pub pid: Option<u32>,
}

impl Srv {
    pub(super) fn new(result: srv_spawner::SrvSpawnResult, mut child: tokio::process::Child) -> Self {
        let stdin = child.stdin.take();
        let pid = child.id();
        Srv { result, child, stdin, pid }
    }
}

/// srv died. End what it left running, then start it again and point the notification presenter
/// at it. The new srv resumes the same agent sessions, so their old CLIs must be gone first, or
/// two would run on one session. `Err` when the new srv could not start.
pub(super) async fn respawn_srv(
    launcher_exe_dir: &std::path::Path,
    paths: &data_dir::DataPaths,
    sink: &startup_events::StartupEventSink,
    why: crate::incidents::SrvStop,
    srv: &mut Srv,
) -> Result<(), srv_spawner::SrvSpawnError> {
    super::unix::end_agents_left_by_srv(&paths.data_dir, srv.pid);
    // What else srv left in its own group (a helper it started outside any agent). The group id
    // stays valid while a member lives, and `orphaned_group_alive` says no process has that pid.
    if let Some(pid) = srv.pid {
        if agentmux_common::process::orphaned_group_alive(pid) {
            let _ = agentmux_common::process::signal_process_group(pid, libc::SIGKILL);
        }
    }
    // Built before the spawn: the new srv is told its id and writes how many agents it found
    // interrupted against it.
    let incident = crate::incidents::srv_restarted(why);
    let (result, child) = srv_spawner::spawn_srv(launcher_exe_dir, paths, "", sink, Some(&incident.id)).await?;
    *srv = Srv::new(result, child);
    crate::notify::update_endpoint(&srv.result.ws_endpoint, &srv.result.auth_key);
    crate::incidents::record(&incident);
    log(&format!(
        "srv respawned (pid {}) — new endpoints ws={} web={}",
        srv.result.pid, srv.result.ws_endpoint, srv.result.web_endpoint
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const PLENTY: u64 = 100_000;

    fn classify(code: Option<i32>, signal: Option<i32>, commit: u64, now: Instant, b: &mut Vec<Instant>) -> SrvExit {
        classify_srv_exit(code, signal, commit, now, b, &mut Vec::new())
    }

    #[test]
    fn a_clean_exit_and_the_group_shutdown_signals_stop_the_app() {
        let (now, mut b) = (Instant::now(), Vec::new());
        assert_eq!(classify(Some(0), None, PLENTY, now, &mut b), SrvExit::Shutdown);
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            assert_eq!(classify(None, Some(sig), PLENTY, now, &mut b), SrvExit::Shutdown, "signal {sig}");
        }
        assert!(b.is_empty(), "a shutdown is not charged to the budget");
    }

    #[test]
    fn a_crash_signal_or_a_failure_code_starts_srv_again() {
        let (now, mut b) = (Instant::now(), Vec::new());
        for (code, sig) in [(None, Some(libc::SIGKILL)), (None, Some(libc::SIGSEGV)), (Some(1), None), (Some(101), None)] {
            assert_eq!(classify(code, sig, PLENTY, now, &mut b), SrvExit::Respawn, "{code:?} {sig:?}");
            b.clear();
        }
    }

    #[test]
    fn the_fourth_crash_within_the_window_gives_up() {
        let (t0, mut b) = (Instant::now(), Vec::new());
        for i in 0..super::super::SRV_RESTART_BUDGET {
            let now = t0 + Duration::from_secs(i as u64 * 10);
            assert_eq!(classify(None, Some(libc::SIGKILL), PLENTY, now, &mut b), SrvExit::Respawn);
        }
        let now = t0 + Duration::from_secs(40);
        assert_eq!(classify(None, Some(libc::SIGKILL), PLENTY, now, &mut b), SrvExit::GiveUp);
    }

    #[test]
    fn a_spent_budget_is_a_give_up_the_supervisor_can_tell_from_a_shutdown() {
        use std::os::unix::process::ExitStatusExt;
        let killed = || Ok(std::process::ExitStatus::from_raw(libc::SIGKILL));
        let mut b: Vec<Instant> = (0..super::super::SRV_RESTART_BUDGET).map(|_| Instant::now()).collect();
        assert!(matches!(srv_exit_action(killed(), &mut b, &mut Vec::new()), SrvAction::GiveUp(1)));
        // A clean exit is a shutdown, not a give-up.
        let clean = Ok(std::process::ExitStatus::from_raw(0));
        assert!(matches!(srv_exit_action(clean, &mut Vec::new(), &mut Vec::new()), SrvAction::Stop(0)));
    }

    #[test]
    fn the_budget_refills_once_the_window_has_passed() {
        let (t0, mut b) = (Instant::now(), Vec::new());
        for _ in 0..super::super::SRV_RESTART_BUDGET {
            classify(None, Some(libc::SIGKILL), PLENTY, t0, &mut b);
        }
        let later = t0 + super::super::SRV_RESTART_WINDOW + Duration::from_secs(1);
        assert_eq!(classify(None, Some(libc::SIGKILL), PLENTY, later, &mut b), SrvExit::Respawn);
    }

    #[test]
    fn low_memory_waits_and_has_a_budget_of_its_own() {
        let (t0, mut crashes, mut ooms) = (Instant::now(), Vec::new(), Vec::new());
        for _ in 0..mem_supervisor::OOM_RESTART_BUDGET {
            let a = classify_srv_exit(None, Some(libc::SIGKILL), 0, t0, &mut crashes, &mut ooms);
            assert_eq!(a, SrvExit::WaitForMemory);
        }
        assert_eq!(classify_srv_exit(None, Some(libc::SIGKILL), 0, t0, &mut crashes, &mut ooms), SrvExit::GiveUpLowMemory);
        assert!(crashes.is_empty(), "memory exits are not charged to the crash budget");
    }

    #[test]
    fn signals_have_names() {
        assert_eq!(signal_name(libc::SIGKILL), "SIGKILL");
        assert_eq!(signal_name(libc::SIGUSR2), "");
    }
}

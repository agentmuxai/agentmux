// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Memory-pressure-aware host supervision — P0 of
//! `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`.
//!
//! The launcher's host-relaunch ladder (`HOST_RESTART_BUDGET`) is otherwise
//! memory-blind: on a *system out-of-memory* host crash it relaunches straight
//! back into the same commit-starved condition, burns the budget in seconds, and
//! gives up — a silent vanish (see `docs/retro/retro-oom-crash-2026-06-16.md`).
//!
//! This module adds the discrimination: a system-OOM exit is waited out (commit-
//! gated, backed-off relaunch on a *separate*, longer OOM budget), while a
//! genuine host fault still trips the fast wedged-host budget + degraded ladder
//! unchanged. The decision logic here is pure and unit-tested; the platform
//! commit probe and the backoff wait are thin wrappers over the OS.

use std::time::{Duration, Instant};

/// Chromium's intentional out-of-memory abort code (`base::win::kOomExceptionCode`,
/// raised non-continuably via `KERNELBASE!RaiseException` when an allocation
/// fails). It surfaces as the host's exit code; `ExitStatus::code()` returns it
/// as an `i32`, so the unsigned `0xE000_0008` reads back as `-536_870_904`.
pub const CHROMIUM_OOM_EXIT_CODE: i32 = 0xE000_0008u32 as i32;

/// Minimum commit-free (available page file) headroom, in MB, before it is safe
/// to (re)launch a CEF host without it instantly re-OOMing. A fresh host +
/// renderer commits on the order of a few hundred MB; 512 leaves margin. Shared
/// starting point with the renderer spec's `RESUME_FLOOR` (SPEC §6).
pub const RESUME_FLOOR_MB: u64 = 512;

/// Restart budget for *system-OOM* host exits — larger and longer than the
/// wedged-host `HOST_RESTART_BUDGET` (3 / 60 s), because transient system
/// pressure is the OS's problem, not a host bug, and recovery can legitimately
/// take minutes (SPEC §5.B / §6).
pub const OOM_RESTART_BUDGET: usize = 5;
pub const OOM_RESTART_WINDOW: Duration = Duration::from_secs(600);

/// Commit-gated relaunch backoff: while commit-free is below `RESUME_FLOOR_MB`,
/// wait this long and re-probe, doubling each time up to the cap. Relaunching
/// into a starved system just re-OOMs, so waiting is the only thing that works.
pub const BACKOFF_START: Duration = Duration::from_secs(2);
pub const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// Hard ceiling on how long to wait for commit to recover before giving up (and
/// showing the honest "out of memory" dialog rather than waiting forever).
pub const OOM_RELAUNCH_DEADLINE: Duration = Duration::from_secs(300);

/// User-facing copy for the graceful give-up dialog (SPEC §5.C). Shown via the
/// launcher's `show_fatal_dialog` (a renderer-free Win32 `MessageBoxW`) so the
/// crash is never a silent vanish.
pub const OOM_GIVEUP_TITLE: &str = "AgentMux — out of memory";
pub const OOM_GIVEUP_BODY: &str = "AgentMux ran low on system memory and couldn't recover this window.\n\n\
Your panes, agents, and sign-ins are saved. Close some other apps or AgentMux windows to free memory, then reopen AgentMux to restore your session.";

/// Classification of an abnormal (non-zero, non-clean-shutdown) host exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostExitClass {
    /// The OS was out of commit — transient and unavoidable. Wait it out on the
    /// separate OOM budget; do NOT consume the wedged-host budget.
    SystemOom,
    /// A genuine host fault (bug, GPU-process cascade, …). Use the existing fast
    /// wedged-host budget + degraded ladder, unchanged.
    Abnormal,
}

/// Classify an abnormal host exit. OOM is identified two ways, deliberately
/// defensive — Chromium sometimes surfaces an OOM as a generic crash code rather
/// than the exact OOM exception (electron#40426):
///   1. the exact Chromium OOM exit code, OR
///   2. *any* abnormal exit taken while commit-free was already below the resume
///      floor — the OS was out of memory, so whatever died, died of it.
pub fn classify_host_exit(exit_code: i32, commit_free_mb: u64) -> HostExitClass {
    if exit_code == CHROMIUM_OOM_EXIT_CODE || commit_free_mb < RESUME_FLOOR_MB {
        HostExitClass::SystemOom
    } else {
        HostExitClass::Abnormal
    }
}

/// Next backoff duration: double `prev`, capped at `BACKOFF_CAP`.
pub fn next_backoff(prev: Duration) -> Duration {
    (prev * 2).min(BACKOFF_CAP)
}

/// Crash-budget bookkeeping, factored out so it is unit-testable. Retains only
/// restarts within `window` of `now`; if the surviving count is already at
/// `budget`, returns `true` (exhausted — caller gives up) WITHOUT recording.
/// Otherwise records `now` and returns `false`. Matches the existing inline
/// wedged-host budget semantics (check-before-push).
pub fn budget_exhausted(
    restarts: &mut Vec<Instant>,
    now: Instant,
    window: Duration,
    budget: usize,
) -> bool {
    restarts.retain(|t| now.duration_since(*t) < window);
    if restarts.len() >= budget {
        return true;
    }
    restarts.push(now);
    false
}

/// Available commit (page file) in MB. The OS commit pool is process-global, so
/// the launcher reads it directly — no shared memory with the host's heartbeat
/// is needed. Returns `u64::MAX` if the probe fails, so a probe failure can never
/// *gate* a relaunch (fail open).
#[cfg(target_os = "windows")]
pub fn commit_free_mb() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `s` is a correctly-sized, zeroed MEMORYSTATUSEX with dwLength set.
    if unsafe { GlobalMemoryStatusEx(&mut s) } == 0 {
        return u64::MAX;
    }
    s.ullAvailPageFile / (1024 * 1024)
}

/// Linux commit proxy: `MemAvailable` from `/proc/meminfo`. The kernel OOM-killer
/// (a SIGKILL) is the real OOM signal here; the abnormal-exit-plus-low-commit
/// reading together approximate it (SPEC §9.4 open question).
#[cfg(target_os = "linux")]
pub fn commit_free_mb() -> u64 {
    if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("MemAvailable:") {
                if let Some(kb) = rest.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()) {
                    return kb / 1024;
                }
            }
        }
    }
    u64::MAX
}

/// macOS has no cheap commit figure; fail open until a platform probe lands.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn commit_free_mb() -> u64 {
    u64::MAX
}

/// Wait for system commit to recover above `RESUME_FLOOR_MB`, probing with
/// exponential backoff (`BACKOFF_START` → `BACKOFF_CAP`). Returns `true` once
/// recovered, or `false` if `OOM_RELAUNCH_DEADLINE` elapses first (the caller
/// then gives up gracefully). `log` is the launcher's logger, threaded in so the
/// wait is observable in the launcher log.
pub async fn await_commit_recovery(log: impl Fn(&str)) -> bool {
    let start = Instant::now();
    let mut backoff = BACKOFF_START;
    loop {
        let free = commit_free_mb();
        if free >= RESUME_FLOOR_MB {
            log(&format!(
                "commit recovered: {} MB free (>= {} MB floor) — relaunching host",
                free, RESUME_FLOOR_MB
            ));
            return true;
        }
        if start.elapsed() >= OOM_RELAUNCH_DEADLINE {
            log(&format!(
                "commit still low ({} MB free) after {}s — giving up host relaunch",
                free,
                OOM_RELAUNCH_DEADLINE.as_secs()
            ));
            return false;
        }
        log(&format!(
            "system out of commit ({} MB free, need {} MB) — waiting {}s before re-check",
            free,
            RESUME_FLOOR_MB,
            backoff.as_secs()
        ));
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oom_exit_code_roundtrips_to_known_signed_value() {
        // Documents the i32 value the launcher will actually compare against.
        assert_eq!(CHROMIUM_OOM_EXIT_CODE, -536_870_904);
    }

    #[test]
    fn exact_oom_code_is_system_oom_even_with_headroom() {
        // The exact Chromium OOM code is OOM regardless of the commit reading,
        // which may already have recovered by the time we sample it.
        assert_eq!(
            classify_host_exit(CHROMIUM_OOM_EXIT_CODE, 8192),
            HostExitClass::SystemOom
        );
    }

    #[test]
    fn abnormal_code_with_low_commit_is_system_oom() {
        // OOM misreported as a generic crash code — the low commit reading still
        // catches it (the OS was out of memory).
        assert_eq!(
            classify_host_exit(1, RESUME_FLOOR_MB - 1),
            HostExitClass::SystemOom
        );
    }

    #[test]
    fn abnormal_code_with_headroom_is_a_host_bug() {
        // A genuine crash with memory available → existing fast wedged-host path.
        assert_eq!(classify_host_exit(1, RESUME_FLOOR_MB), HostExitClass::Abnormal);
        assert_eq!(classify_host_exit(-1, 16_384), HostExitClass::Abnormal);
    }

    #[test]
    fn backoff_doubles_then_caps() {
        assert_eq!(next_backoff(Duration::from_secs(2)), Duration::from_secs(4));
        assert_eq!(next_backoff(Duration::from_secs(4)), Duration::from_secs(8));
        // 16 → 32 capped to 30.
        assert_eq!(next_backoff(Duration::from_secs(16)), Duration::from_secs(30));
        // Stays at the cap.
        assert_eq!(next_backoff(Duration::from_secs(30)), Duration::from_secs(30));
    }

    #[test]
    fn budget_allows_up_to_n_then_exhausts() {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut r = Vec::new();
        assert!(!budget_exhausted(&mut r, now, window, 3));
        assert!(!budget_exhausted(&mut r, now, window, 3));
        assert!(!budget_exhausted(&mut r, now, window, 3));
        // Fourth within the window → exhausted, and it is NOT recorded.
        assert!(budget_exhausted(&mut r, now, window, 3));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn budget_forgets_restarts_outside_the_window() {
        let base = Instant::now();
        let window = Duration::from_secs(60);
        let mut r = Vec::new();
        for _ in 0..3 {
            let _ = budget_exhausted(&mut r, base, window, 3);
        }
        assert!(budget_exhausted(&mut r, base, window, 3)); // exhausted at base
        // A restart well past the window prunes the stale entries → room again.
        let later = base + Duration::from_secs(120);
        assert!(!budget_exhausted(&mut r, later, window, 3));
        assert_eq!(r.len(), 1);
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Integration test (not a `src/`-embedded unit test) so `CARGO_BIN_EXE_
//! agentmux-bashwrap` is set correctly and `cargo test` is guaranteed to
//! build the plain `agentmux-bashwrap.exe` binary before this runs.
//!
//! An earlier version of this test lived in `bash_wrap.rs`'s own test
//! module and derived the binary's path manually from
//! `std::env::current_exe()` (`CARGO_BIN_EXE_*` isn't set for unit tests
//! embedded in the same bin crate's own harness — only for genuine
//! integration-test targets like this file). That worked locally because
//! leftover `cargo build -p agentmux-bashwrap` artifacts from manual
//! testing happened to already be sitting at the expected path — but
//! failed in CI's clean checkout, where nothing had built that plain
//! binary yet. See docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md.

use std::process::Stdio;
use std::time::{Duration, Instant};

/// Windows-only: counts processes whose command line contains `marker`
/// (via WMI `CommandLine`, which — unlike `tasklist`'s image-name-only
/// filter — exposes full arguments). Used to prove `kill_process_tree`
/// actually reaches descendants, not just the direct PTY child.
///
/// Excludes `powershell.exe` itself: the query's own invocation argument
/// (`-like '*<marker>*'`) literally contains the marker text, so without
/// this exclusion the query matches its own process — a `ps aux | grep
/// foo` matching-itself bug that produced a false "1 survivor" positive
/// while developing this test (reagent P1 follow-up, PR #2156) before
/// being caught.
#[cfg(windows)]
fn count_processes_with_marker(marker: &str) -> usize {
    let ps_cmd = format!(
        "(Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{}*' -and $_.Name -ne 'powershell.exe' }} | Measure-Object).Count",
        marker
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .output()
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out.stdout).trim().parse::<usize>().ok())
        .unwrap_or(usize::MAX) // parse failure reads as "assume orphans survived" — never a false pass
}

/// End-to-end proof that the idle-kill mechanism actually fires AND
/// reaches the whole process tree, not just the direct PTY child.
///
/// Uses two backgrounded `sleep` grandchildren (`bash -c '{ sleep
/// <marker>1 & sleep <marker>2 & wait; } </dev/null'`) rather than a
/// single-process command: killing only the direct `bash` child (what
/// `ChildKiller::kill()` alone does — see `kill_process_tree`'s doc
/// comment in `bash_wrap.rs`) would leave these two running as orphans,
/// exactly reproducing the leak one process removed instead of fixing it
/// (reagent P1, PR #2156). The marker values are unique per test run
/// (derived from the current process's PID) so a leftover orphan from a
/// *previous* failed run of this same test, or an unrelated `sleep`
/// elsewhere on a shared dev machine, can't produce a false pass.
///
/// The `sleep <marker>` argument doubles as the zero-output condition
/// this exists to catch (a clean, portable stand-in for "any command
/// silently blocked forever," the same shape as the pager hang — see
/// docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md). Without the
/// fix, this test would hang until the outer `tokio::time::timeout`
/// fires and fails it.
///
/// Investigating reagent's P1 (`kill_process_tree`/`taskkill /T` alone
/// might not reliably reach every descendant) took a real detour worth
/// recording: an early manual check of `Win32_Process.ParentProcessId`
/// for a backgrounded `sleep` showed it pointing at a PID that had
/// *already exited* by the time it was queried — not `bash.exe` itself.
/// That looked like proof that Git-for-Windows' bash (MSYS2) doesn't
/// preserve a discoverable Win32 parent-child chain for forked children
/// (`&`, `|`, multiple statements), which would make any PID-tree API
/// (including `taskkill /T`) structurally unable to reach them. Two early
/// versions of *this exact test* then failed with "1 survivor," seeming
/// to confirm it.
///
/// That confirmation was wrong. The actual bug was in this test's own
/// `count_processes_with_marker` helper: its PowerShell query embeds the
/// marker text directly in its own `-like '*<marker>*'` invocation
/// argument, so the query matched **itself** — a `ps aux | grep foo`
/// matching-its-own-`grep` bug, not a real orphan. Once
/// `count_processes_with_marker` excluded `powershell.exe`, this test
/// passed consistently across repeated runs: `taskkill /T /F /PID
/// <bash_pid>`, run promptly while `bash.exe` is still alive (the
/// tree-kill-before-direct-kill ordering documented on
/// `kill_process_tree`'s call site in `bash_wrap.rs`), *does* reliably
/// reach these backgrounded children in practice, despite the
/// stale-`ParentProcessId` observation above being real. (Plausible
/// reconciliation, not independently re-verified: `taskkill`'s own
/// tree-walk runs at a different, earlier moment than the later manual
/// check was taken at, and/or walks a broader relationship than a bare
/// `ParentProcessId` snapshot exposes — not chased further once the
/// empirical result was unambiguous.)
#[tokio::test]
#[cfg(windows)]
async fn bashwrap_binary_idle_kill_cleans_up_full_process_tree() {
    let bin = env!("CARGO_BIN_EXE_agentmux-bashwrap");

    // `300` tag keeps this marker from overlapping with any other test's
    // marker substring (multiple tests can run concurrently within the
    // same process — `std::process::id()` alone isn't unique across
    // them). See the equivalent comment history in
    // docs/retro/RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md.
    let pid = std::process::id();
    let marker = format!("{pid}300");
    let command = format!("sleep {marker}.003 & sleep {marker}.004 & wait");
    let b64 = {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        URL_SAFE_NO_PAD.encode(command.as_bytes())
    };

    let start = Instant::now();
    let mut child = tokio::process::Command::new(bin)
        .args(["exec", "--tool-id=test-binary-idle-kill", &format!("--b64-cmd={b64}")])
        .env("AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the real agentmux-bashwrap binary");

    let status = tokio::time::timeout(Duration::from_secs(20), child.wait())
        .await
        .expect("the binary must exit within a bounded time, not hang forever")
        .expect("waiting on the spawned binary");

    assert!(
        !status.success(),
        "an idle-killed invocation must not report a clean success exit"
    );
    assert!(
        start.elapsed() < Duration::from_secs(15),
        "should exit well within the 1s idle timeout + grace periods, got {:?}",
        start.elapsed()
    );

    // The actual proof: once the binary we just waited on has fully
    // exited, no descendant (both backgrounded sleep grandchildren)
    // should remain. Short settle delay for WMI query latency after
    // process exit, not for the kill itself.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let survivors = count_processes_with_marker(&marker);
    assert_eq!(
        survivors, 0,
        "process tree should be fully cleaned up once the wrapper binary \
         exits — found {survivors} process(es) still matching marker {marker:?}"
    );
}

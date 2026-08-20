// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Integration test (not a `src/`-embedded unit test) for the same reason
//! `idle_kill_full_process_tree.rs` is one: `CARGO_BIN_EXE_agentmux-bashwrap`
//! is only set for genuine integration-test targets, and this needs to spawn
//! the real, built binary — `detach_declared_background_session`'s actual
//! effect (a real `setsid()` syscall) isn't something a same-process unit
//! test can safely exercise (calling `setsid()` on the TEST RUNNER's own
//! process would detach the whole test binary from its session, an
//! unacceptable side effect for a shared test process running many other
//! tests).
//!
//! Covers the Codex finding on PR #2683: without `setsid()`, a declared-
//! background invocation's session stays the same as its parent `claude`
//! process's — meaning the wrapped command (and bashwrap itself) receives
//! SIGHUP the moment that parent's session leader dies, via two independent
//! POSIX mechanisms (PTY hangup on master close, and session-leader-exit),
//! undermining `stop_for_replace`'s whole point regardless of how carefully
//! it avoids touching the process group/job on the srv side. See
//! `detach_declared_background_session`'s doc comment in `bash_wrap.rs` and
//! docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md.

#![cfg(unix)]

use std::process::Stdio;
use std::time::Duration;

fn b64(s: &str) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(s.as_bytes())
}

/// A `--declared-background` invocation detaches into its own session —
/// its session id no longer matches the test process's own (the session
/// bashwrap would otherwise have inherited as a plain child).
#[tokio::test]
async fn declared_background_invocation_detaches_into_its_own_session() {
    let bin = env!("CARGO_BIN_EXE_agentmux-bashwrap");
    let b64_cmd = b64("sleep 5");

    let mut child = tokio::process::Command::new(bin)
        .args([
            "exec",
            "--tool-id=test-session-detach-bg",
            &format!("--b64-cmd={b64_cmd}"),
            "--declared-background",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the real agentmux-bashwrap binary");

    let bashwrap_pid = child.id().expect("spawned child must have a pid") as libc::pid_t;

    // Give detach_declared_background_session (the very first thing run()
    // does) a moment to actually execute before we query.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // SAFETY: getsid() is a well-defined POSIX syscall; both pids are
    // valid live processes at this point (this test process itself, and
    // the child we just spawned and haven't waited on yet).
    let own_sid = unsafe { libc::getsid(0) };
    let bashwrap_sid = unsafe { libc::getsid(bashwrap_pid) };

    assert_ne!(
        bashwrap_sid, -1,
        "getsid on the spawned bashwrap process failed: {}",
        std::io::Error::last_os_error()
    );
    assert_ne!(
        bashwrap_sid, own_sid,
        "a --declared-background bashwrap invocation must detach into its own \
         session (setsid()) — found it still sharing this test process's \
         session ({own_sid}), meaning it (and anything it PTY-spawns) is still \
         exposed to SIGHUP if this test process's session leader were killed"
    );
    // Session id of a session leader equals its own pid — confirms setsid()
    // specifically (not just "some other session"), matching the exact
    // postcondition setsid() documents.
    assert_eq!(
        bashwrap_sid, bashwrap_pid,
        "bashwrap should be the LEADER of its new session (setsid()'s own \
         postcondition: sid == pid), not merely a member of some other one"
    );

    // Cleanup — declared-background means bashwrap won't exit on its own
    // within any bounded time relevant to this test (that's the whole
    // point); just reap it directly rather than waiting.
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Sanity check the other direction: an ORDINARY (non-declared-background)
/// invocation does NOT detach — it should stay in the same session as
/// this test process, exactly like any other plain child. Guards against
/// a fix that's too broad (e.g. calling setsid() unconditionally).
#[tokio::test]
async fn ordinary_invocation_does_not_detach_its_session() {
    let bin = env!("CARGO_BIN_EXE_agentmux-bashwrap");
    let b64_cmd = b64("sleep 5");

    let mut child = tokio::process::Command::new(bin)
        .args(["exec", "--tool-id=test-session-detach-fg", &format!("--b64-cmd={b64_cmd}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the real agentmux-bashwrap binary");

    let bashwrap_pid = child.id().expect("spawned child must have a pid") as libc::pid_t;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let own_sid = unsafe { libc::getsid(0) };
    let bashwrap_sid = unsafe { libc::getsid(bashwrap_pid) };

    assert_ne!(bashwrap_sid, -1, "getsid failed: {}", std::io::Error::last_os_error());
    assert_eq!(
        bashwrap_sid, own_sid,
        "an ordinary (non-backgrounded) invocation must NOT detach its session"
    );

    let _ = child.kill().await;
    let _ = child.wait().await;
}

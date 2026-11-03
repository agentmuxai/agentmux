// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's crash monitor (`--crash-monitor`) must not outlive its use, and monitors must not
//! disturb each other.
//!
//! Found 2026-11-03: a monitor whose srv never connected (srv waited 250 ms for it) ran for
//! ever, holding `agentmux-srv.exe` open so that the next build could not replace it, and all
//! monitors shared one socket file, which a new monitor took over from a live one and which an
//! exiting monitor deleted from under a newer one. These run the real binary.

#![cfg(windows)]

// This binary must hold this checkout's code (`agentmux_common::checkout`).
agentmux_common::checkout_guard!();

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn spawn_monitor(dir: &Path, srv_pid: u32, grace_ms: u64) -> Child {
    Command::new(env!("CARGO_BIN_EXE_agentmux-srv"))
        .args(["--crash-monitor", "--parent-pid", &srv_pid.to_string()])
        .env("AGENTMUX_CRASH_DIR", dir)
        .env("AGENTMUX_CRASH_MONITOR_GRACE_MS", grace_ms.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn the crash monitor")
}

fn wait_until(secs: u64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

#[test]
fn a_monitor_no_srv_connects_to_gives_up_and_removes_its_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("monitor-4242.sock");
    let mut monitor = spawn_monitor(dir.path(), 4242, 1500);
    assert!(wait_until(30, || socket.exists()), "the monitor bound its socket");
    assert!(
        wait_until(30, || monitor.try_wait().expect("try_wait").is_some()),
        "a monitor nobody connected to exits by itself"
    );
    assert!(!socket.exists(), "and takes its socket with it");
}

#[test]
fn a_monitor_exiting_leaves_another_monitors_socket_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("monitor-111.sock"), dir.path().join("monitor-222.sock"));
    let mut short_lived = spawn_monitor(dir.path(), 111, 1500);
    let mut long_lived = spawn_monitor(dir.path(), 222, 120_000);
    assert!(wait_until(30, || first.exists() && second.exists()), "both monitors bound a socket of their own");
    assert!(wait_until(30, || short_lived.try_wait().expect("try_wait").is_some()), "the first one gave up");
    assert!(second.exists(), "its exit did not delete the other monitor's socket");
    assert!(long_lived.try_wait().expect("try_wait").is_none(), "the other monitor is still running");
    let _ = long_lived.kill();
    let _ = long_lived.wait();
}

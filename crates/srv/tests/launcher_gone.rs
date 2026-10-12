// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv must shut down when its launcher goes away, even though its stderr,
//! which the launcher reads, is gone by then.
//!
//! On 2026-10-11 macOS quit an AgentMux launcher (System Settings' "Quit &
//! Reopen" after a permission grant). srv survived, holding its data-dir lock,
//! and the app could not be opened again. Both of srv's launcher-gone watchers
//! (stdin EOF and the parent-process watcher) logged with `eprintln!` before
//! acting. With the launcher's end of the stderr pipe closed, that write fails
//! with EPIPE and `eprintln!` panics, so the watcher thread died before it
//! could shut srv down.
//!
//! This test plays the dead launcher: it closes its end of srv's stderr, then
//! closes srv's stdin, and expects srv to exit.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn srv_exits_on_stdin_eof_after_its_stderr_reader_is_gone() {
    let data_dir = tempfile::tempdir().expect("temp data dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentmux-srv"))
        .env("AGENTMUX_AUTH_KEY", "launcher-gone-test-key")
        .env("AGENTMUX_DATA_DIR", data_dir.path())
        .env("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "1")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to spawn agentmux-srv");

    // Wait until srv is up, as the launcher does.
    let stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let started = lines
        .by_ref()
        .map_while(Result::ok)
        .any(|line| line.contains("AGENTMUXSRV-ESTART"));
    assert!(started, "srv never printed AGENTMUXSRV-ESTART");

    // The launcher dies: its end of srv's stderr closes, then its end of
    // srv's stdin.
    drop(lines);
    drop(child.stdin.take());

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            return;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("srv still running 20 s after stdin closed with its stderr reader gone");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for subprocess I/O: stdin write + flush, stdout read,
//! stderr capture, and exit code reporting.
//!
//! These tests spawn real child processes with real pipes on the host OS,
//! exercising the exact Tokio IOCP/epoll path used in production. They
//! catch platform-specific bugs (CREATE_NO_WINDOW, pipe buffering, flush
//! timing) that unit tests and `cargo check` cannot.
//!
//! Requires: `node` on PATH.

use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::{timeout, Duration};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Spawn node with a fixture script. Returns the child process.
fn spawn_node(script: &str, extra_args: &[&str]) -> tokio::process::Child {
    let script_path = fixtures_dir().join(script);
    let mut cmd = tokio::process::Command::new("node");
    cmd.arg(script_path);
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // SPEC_TEST_SRV_SPAWN_GUARDS_2026_07_11 §DoD 4 — a panic between spawn
    // and the test's own wait() must not leak the node child. tokio's
    // kill_on_drop is the one-line guard for tokio-spawned test subjects.
    cmd.kill_on_drop(true);

    // Match production: suppress console window on Windows
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.spawn().expect("failed to spawn node — is it on PATH?")
}

#[tokio::test]
async fn stdin_stdout_roundtrip() {
    let mut child = spawn_node("echo-stdin.js", &[]);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Write message to stdin and flush
    stdin.write_all(b"hello world\n").await.unwrap();
    stdin.flush().await.unwrap();
    drop(stdin); // EOF

    // Read stdout
    let mut reader = BufReader::new(stdout).lines();
    let line = timeout(Duration::from_secs(10), reader.next_line())
        .await
        .expect("stdout read timed out")
        .expect("stdout read error")
        .expect("stdout EOF before any output");

    let parsed: serde_json::Value = serde_json::from_str(&line).expect("invalid JSON");
    assert_eq!(parsed["echo"], "hello world");

    let status = child.wait().await.unwrap();
    assert_eq!(status.code(), Some(0));
}

#[tokio::test]
async fn stdin_flush_required() {
    // Verify that without flush, data may not arrive before stdin drop.
    // This test writes data, flushes, then asserts the echo arrives.
    // If flush is broken, this would timeout.
    let mut child = spawn_node("echo-stdin.js", &[]);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    stdin.write_all(b"flush test\n").await.unwrap();
    stdin.flush().await.unwrap();
    drop(stdin);

    let mut reader = BufReader::new(stdout).lines();
    let result = timeout(Duration::from_secs(5), reader.next_line()).await;
    assert!(result.is_ok(), "stdin data did not arrive — flush may be broken");

    let line = result.unwrap().unwrap().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed["echo"], "flush test");

    child.wait().await.unwrap();
}

#[tokio::test]
async fn stderr_captured() {
    let mut child = spawn_node("exit-code.js", &["0"]);
    let stderr = child.stderr.take().unwrap();

    let mut reader = BufReader::new(stderr).lines();
    let line = timeout(Duration::from_secs(5), reader.next_line())
        .await
        .expect("stderr read timed out")
        .expect("stderr read error")
        .expect("stderr EOF before any output");

    assert!(line.contains("intentional error output"));
    child.wait().await.unwrap();
}

#[tokio::test]
async fn exit_code_nonzero() {
    let mut child = spawn_node("exit-code.js", &["42"]);
    // Drain stderr so the pipe doesn't block
    drop(child.stderr.take());
    drop(child.stdin.take());

    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("wait timed out")
        .expect("wait error");

    assert_eq!(status.code(), Some(42));
}

#[tokio::test]
async fn stdout_eof_on_process_exit() {
    let mut child = spawn_node("exit-code.js", &["0"]);
    let stdout = child.stdout.take().unwrap();
    drop(child.stderr.take());
    drop(child.stdin.take());

    let mut reader = BufReader::new(stdout).lines();

    // Should get Ok(None) = EOF, not an error
    let result = timeout(Duration::from_secs(5), reader.next_line())
        .await
        .expect("stdout read timed out")
        .expect("stdout read error");

    assert!(result.is_none(), "expected EOF (None), got a line");
    child.wait().await.unwrap();
}

#[tokio::test]
async fn large_stdin_payload() {
    let mut child = spawn_node("echo-stdin.js", &[]);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Write 100KB payload
    let payload = "x".repeat(100_000);
    stdin.write_all(payload.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    drop(stdin);

    let mut reader = BufReader::new(stdout).lines();
    let line = timeout(Duration::from_secs(15), reader.next_line())
        .await
        .expect("stdout read timed out on large payload")
        .expect("stdout read error")
        .expect("stdout EOF before output");

    let parsed: serde_json::Value = serde_json::from_str(&line).expect("invalid JSON");
    assert_eq!(parsed["echo"].as_str().unwrap().len(), 100_000);

    let status = child.wait().await.unwrap();
    assert_eq!(status.code(), Some(0));
}

#[cfg(windows)]
#[tokio::test]
async fn create_no_window_flag_set() {
    // Verify the child process runs without a visible console window.
    // We can't directly check GetConsoleWindow() from the parent, but
    // we CAN verify that stdio piping works with CREATE_NO_WINDOW set
    // (the bug was that without this flag, stdout went to the console
    // instead of the pipe, producing zero output).
    //
    // Bounded retry instead of one long timeout: this test flaked on a slow
    // node.exe cold-start on GH-hosted Windows runners twice (2026-06-28,
    // 2026-08-13), both times "fixed" by widening a single timeout (5s then
    // 15s) that just moved the goalpost past the one observed data point.
    // A retry across fresh child processes distinguishes "occasionally
    // slow" from "actually broken": the real regression this test guards
    // against (stdout going to a console instead of the pipe) reproduces on
    // every attempt, a slow spawn doesn't. This also keeps the common-case
    // runtime unchanged (one fast attempt) instead of paying a doubled
    // worst-case timeout on every run. See
    // docs/specs/PLAN_WINDOWS_CI_SUBPROCESS_IO_FLAKE_FIX_2026_08_13.md.
    const ATTEMPTS: u32 = 3;
    const PER_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

    let mut last_elapsed = Duration::ZERO;
    for attempt in 1..=ATTEMPTS {
        let mut child = spawn_node("echo-stdin.js", &[]);
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        stdin.write_all(b"windows pipe test\n").await.unwrap();
        stdin.flush().await.unwrap();
        drop(stdin);

        let mut reader = BufReader::new(stdout).lines();
        let attempt_start = std::time::Instant::now();
        let result = timeout(PER_ATTEMPT_TIMEOUT, reader.next_line()).await;
        last_elapsed = attempt_start.elapsed();

        if let Ok(Ok(Some(line))) = result {
            assert!(line.contains("windows pipe test"));
            child.wait().await.unwrap();
            if attempt > 1 {
                eprintln!(
                    "create_no_window_flag_set: passed on attempt {attempt}/{ATTEMPTS} \
                     (took {last_elapsed:?}) — node.exe cold-start was slow, not broken"
                );
            }
            return;
        }

        // child is dropped here; kill_on_drop(true) (set in spawn_node) reaps it.
        if attempt < ATTEMPTS {
            eprintln!(
                "create_no_window_flag_set: attempt {attempt}/{ATTEMPTS} produced no stdout \
                 within {PER_ATTEMPT_TIMEOUT:?} (took {last_elapsed:?}) — retrying"
            );
        }
    }

    // All attempts failed identically — a genuinely slow-but-working spawn
    // would be expected to clear within a couple of 10s attempts, so this is
    // unlikely to be cold-start variance. Capture Defender status since
    // neither prior incident on this test was root-caused past "loaded
    // runner" — this is the data point to check that guess next time.
    let defender_status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-MpComputerStatus | Format-List",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("<failed to query Defender status: {e}>"));

    panic!(
        "CREATE_NO_WINDOW: stdout pipe produced no data after {ATTEMPTS} attempts \
         (last attempt took {last_elapsed:?}, per-attempt timeout {PER_ATTEMPT_TIMEOUT:?}) — \
         node.exe may be writing to a console instead of the pipe.\n\
         Windows Defender status at failure:\n{defender_status}"
    );
}

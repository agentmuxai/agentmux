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
    let mut child = spawn_node("echo-stdin.js", &[]);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    stdin.write_all(b"windows pipe test\n").await.unwrap();
    stdin.flush().await.unwrap();
    drop(stdin);

    let mut reader = BufReader::new(stdout).lines();
    let result = timeout(Duration::from_secs(5), reader.next_line()).await;

    assert!(
        result.is_ok(),
        "CREATE_NO_WINDOW: stdout pipe produced no data — \
         node.exe may be writing to a console instead of the pipe"
    );

    let line = result.unwrap().unwrap().unwrap();
    assert!(line.contains("windows pipe test"));
    child.wait().await.unwrap();
}

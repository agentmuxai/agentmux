// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What starting the persistent CLI does, pinned before `spawn_process` is split into phases: the argv it
//! gets (`--resume` only for a known session), the message written to its stdin, the session it captures,
//! the generation it bumps, the status and exit code it ends with, and what a CLI that cannot start leaves
//! behind. A stub shell script stands in for the CLI.

#![cfg(unix)]

use super::super::*;
use crate::backend::blockcontroller::STATUS_DONE;

/// Logs its argv and the first line it reads on stdin to `log`, answers with an init event carrying
/// `sid` and a result, and exits with `code`.
fn stub(dir: &std::path::Path, sid: &str, code: i32) -> std::path::PathBuf {
    let script = dir.join("stub.sh");
    let log = dir.join("log");
    std::fs::write(
        &script,
        format!(
            "echo \"args=$*\" >> '{log}'\n\
             echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{sid}\"}}'\n\
             read line\necho \"heard=$line\" >> '{log}'\n\
             echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"{sid}\"}}'\n\
             exit {code}\n",
            log = log.display()
        ),
    )
    .unwrap();
    script
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("agentmux-spawn-process-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn controller(block: &str) -> PersistentSubprocessController {
    PersistentSubprocessController::new("tab".into(), block.into(), None, None, None, None)
}

fn config(script: &std::path::Path, session_id: &str) -> PersistentSpawnConfig {
    PersistentSpawnConfig {
        cli_command: "/bin/sh".into(),
        cli_args: vec![script.to_string_lossy().into_owned()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".into(),
        resume_flag: "--resume".into(),
        session_id: session_id.into(),
        message_id: None,
    }
}

const MESSAGE: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi there"}]}}"#;

async fn wait_done(c: &PersistentSubprocessController) -> i32 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let status = c.get_status_snapshot();
        if status.shellprocstatus == STATUS_DONE {
            return status.shellprocexitcode;
        }
        assert!(std::time::Instant::now() < deadline, "the process never ended: {status:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn log_lines(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("log")).unwrap_or_default().lines().map(str::to_string).collect()
}

#[tokio::test]
async fn a_first_spawn_writes_the_message_captures_the_session_and_ends_done() {
    let dir = scratch("first");
    let c = controller("sp-first");
    c.send_message(MESSAGE.to_string(), config(&stub(&dir, "sid-first", 0), "")).unwrap();
    assert_eq!(wait_done(&c).await, 0);

    let log = log_lines(&dir);
    assert_eq!(log[0], "args=", "no resume flag on a first spawn: {log:?}");
    assert!(log[1].starts_with("heard=") && log[1].contains("hi there"), "the message reached the CLI's stdin: {log:?}");
    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.session_id.as_deref(), Some("sid-first"), "the init event's session id is captured");
    assert_eq!(inner.spawn_generation, 1, "one spawn");
    assert!(inner.current_pid.is_none(), "an ended process leaves no pid");
    drop(inner);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_known_session_is_resumed() {
    let dir = scratch("resume");
    let c = controller("sp-resume");
    c.send_message(MESSAGE.to_string(), config(&stub(&dir, "sid-new", 0), "sid-old")).unwrap();
    wait_done(&c).await;
    assert_eq!(log_lines(&dir)[0], "args=--resume sid-old", "{:?}", log_lines(&dir));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_nonzero_exit_is_recorded() {
    let dir = scratch("exit");
    let c = controller("sp-exit");
    c.send_message(MESSAGE.to_string(), config(&stub(&dir, "sid-exit", 4), "")).unwrap();
    assert_eq!(wait_done(&c).await, 4);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_second_message_after_the_process_ended_starts_another_resuming_the_captured_session() {
    let dir = scratch("again");
    let c = controller("sp-again");
    let script = stub(&dir, "sid-again", 0);
    c.send_message(MESSAGE.to_string(), config(&script, "")).unwrap();
    wait_done(&c).await;
    c.send_message(MESSAGE.to_string(), config(&script, "")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while log_lines(&dir).len() < 4 {
        assert!(std::time::Instant::now() < deadline, "the second spawn never ran: {:?}", log_lines(&dir));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(log_lines(&dir)[2], "args=--resume sid-again", "{:?}", log_lines(&dir));
    assert_eq!(c.inner.lock().unwrap().spawn_generation, 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_cli_that_cannot_start_ends_the_pane_done_and_not_running() {
    let c = controller("sp-nospawn");
    let mut cfg = config(std::path::Path::new("/nonexistent/script"), "");
    cfg.cli_command = "/nonexistent/agentmux-no-such-cli".into();
    let result = c.send_message(MESSAGE.to_string(), cfg);
    assert!(result.is_err(), "the caller is told the CLI did not start: {result:?}");
    let status = c.get_status_snapshot();
    assert_ne!(status.shellprocstatus, STATUS_RUNNING, "{status:?}");
    assert!(c.inner.lock().unwrap().current_pid.is_none());
}

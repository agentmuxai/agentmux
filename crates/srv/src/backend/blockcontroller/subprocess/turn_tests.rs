// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What one host turn does, pinned before `spawn_turn` is split into phases: the stdin it is given, the
//! session it captures, the status it passes through, the exit code it records, what a failed spawn leaves
//! behind, and the message queued behind it. A stub shell script stands in for the CLI.

#![cfg(unix)]

use super::*;
use crate::backend::blockcontroller::STATUS_DONE;

fn controller(block: &str) -> SubprocessController {
    SubprocessController::new("tab-1".into(), block.into(), None, None, None, None, None, Arc::from("test-boot"))
}

fn config(script: &std::path::Path, message: &str) -> SubprocessSpawnConfig {
    SubprocessSpawnConfig {
        cli_command: "/bin/sh".into(),
        cli_args: vec![script.to_string_lossy().into_owned()],
        message: message.into(),
        resume_flag: "--resume".into(),
        resume_strategy: "flag".into(),
        session_id_field: "session_id".into(),
        ..Default::default()
    }
}

/// A stub CLI: logs its argv and the line it read on stdin to `log`, answers with an init event carrying
/// `sid`, and exits with `code`.
fn stub(dir: &std::path::Path, sid: &str, code: i32) -> std::path::PathBuf {
    let script = dir.join("stub.sh");
    let log = dir.join("log");
    std::fs::write(
        &script,
        format!(
            "read line\necho \"args=$*\" >> '{log}'\necho \"heard=$line\" >> '{log}'\n\
             echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{sid}\"}}'\nexit {code}\n",
            log = log.display()
        ),
    )
    .unwrap();
    script
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("agentmux-turn-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn wait_done(ctrl: &SubprocessController) -> (String, i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let status = ctrl.get_status_snapshot();
        if status.shellprocstatus == STATUS_DONE && !ctrl.run_lock.load(Ordering::SeqCst) {
            return (status.shellprocstatus, status.shellprocexitcode);
        }
        assert!(std::time::Instant::now() < deadline, "the turn did not finish: {status:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn log_lines(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("log")).unwrap_or_default().lines().map(str::to_string).collect()
}

#[tokio::test]
async fn a_turn_gives_the_cli_the_message_on_stdin_captures_its_session_and_ends_done() {
    let dir = scratch("happy");
    let ctrl = controller("turn-happy");
    ctrl.spawn_turn(config(&stub(&dir, "sid-one", 0), "hello there")).unwrap();
    // The turn is running as soon as spawn_turn returns, and holds the run lock.
    assert!(ctrl.run_lock.load(Ordering::SeqCst));
    assert!(ctrl.get_status_snapshot().turn_active);

    let (status, code) = wait_done(&ctrl).await;
    assert_eq!((status.as_str(), code), (STATUS_DONE, 0));
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-one"), "the init event's session id is captured");
    assert_eq!(log_lines(&dir), vec!["args=".to_string(), "heard=hello there".to_string()], "no resume on a first turn; the message is on stdin");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_nonzero_exit_is_recorded_and_the_turn_still_ends_done() {
    let dir = scratch("exit");
    let ctrl = controller("turn-exit");
    ctrl.spawn_turn(config(&stub(&dir, "sid-x", 3), "hi")).unwrap();
    let (status, code) = wait_done(&ctrl).await;
    assert_eq!((status.as_str(), code), (STATUS_DONE, 3));
    assert!(!ctrl.get_status_snapshot().turn_active, "a finished turn is not active");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_turn_with_a_known_session_resumes_it() {
    let dir = scratch("resume");
    let ctrl = controller("turn-resume");
    let mut cfg = config(&stub(&dir, "sid-two", 0), "again");
    cfg.session_id = Some("sid-prior".into());
    ctrl.spawn_turn(cfg).unwrap();
    wait_done(&ctrl).await;
    assert_eq!(log_lines(&dir)[0], "args=--resume sid-prior", "{:?}", log_lines(&dir));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_cli_that_cannot_start_leaves_the_pane_done_with_minus_one_and_the_lock_free() {
    let ctrl = controller("turn-nospawn");
    let mut cfg = config(std::path::Path::new("/nonexistent/script"), "hi");
    cfg.cli_command = "/nonexistent/agentmux-no-such-cli".into();
    let err = ctrl.spawn_turn(cfg).unwrap_err();
    assert!(err.starts_with("failed to spawn subprocess"), "{err}");
    let status = ctrl.get_status_snapshot();
    assert_eq!((status.shellprocstatus.as_str(), status.shellprocexitcode), (STATUS_DONE, -1));
    assert!(!status.turn_active);
    assert!(!ctrl.run_lock.load(Ordering::SeqCst), "the next message is not queued behind a turn that never ran");
}

#[tokio::test]
async fn a_message_sent_during_a_turn_runs_after_it_resuming_the_captured_session() {
    let dir = scratch("queue");
    let ctrl = Arc::new(controller("turn-queue"));
    SubprocessController::set_self_ref(&ctrl);
    let script = stub(&dir, "sid-q", 0);
    ctrl.spawn_turn(config(&script, "first")).unwrap();
    // Queued, not refused: the first turn still holds the lock.
    ctrl.spawn_turn(config(&script, "second")).unwrap();
    assert_eq!(ctrl.inner.lock().unwrap().pending_messages.len(), 1);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while log_lines(&dir).len() < 4 {
        assert!(std::time::Instant::now() < deadline, "the queued message never ran: {:?}", log_lines(&dir));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    wait_done(&ctrl).await;
    assert_eq!(
        log_lines(&dir),
        vec!["args=", "heard=first", "args=--resume sid-q", "heard=second"].into_iter().map(String::from).collect::<Vec<_>>()
    );
    assert!(ctrl.inner.lock().unwrap().pending_messages.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

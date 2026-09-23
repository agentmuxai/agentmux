// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Controller::shutdown` against a real process: a small node script that
//! speaks just enough stream-json to stand in for the Claude CLI, with the
//! behaviour measured in the pane-close spec §5.1 — a running turn survives
//! EOF, the interrupt ends it with `result: error_during_execution`, EOF
//! then exits with code 1.

use super::super::*;
use crate::backend::blockcontroller::{Controller, StopOutcome};

const STUB: &str = r#"
const mode = process.argv[2];
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type === "control_request" && m.request && m.request.subtype === "interrupt") {
    out({ type: "control_response", response: { subtype: "success", request_id: m.request_id } });
    if (mode !== "stubborn") {
      out({ type: "result", subtype: "error_during_execution", is_error: true, session_id: "stub-session" });
    }
    return;
  }
  if (m.type === "user" && mode === "idle") {
    out({ type: "result", subtype: "success", is_error: false, result: "ok", session_id: "stub-session" });
  }
  // "turn" / "stubborn": the turn never finishes on its own.
});
rl.on("close", () => {
  if (mode === "stubborn") { setInterval(() => {}, 1000); } else { process.exit(1); }
});
"#;

/// `None` (test skipped with a note) when `node` isn't on PATH.
fn stub_path() -> Option<std::path::PathBuf> {
    // A PATH lookup, not `node --version`: a probe spawn would be one more
    // unsanitized spawn site for `pane_env`'s I7 inventory to flag.
    let has_node = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    });
    if !has_node {
        eprintln!("shutdown_tests: `node` not on PATH — skipping");
        return None;
    }
    let path = std::env::temp_dir().join(format!("agentmux-shutdown-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&path, STUB).unwrap();
    Some(path)
}

fn start(block_id: &str, mode: &str, stub: &std::path::Path) -> (PersistentSubprocessController, Arc<mps::Broker>) {
    let broker = Arc::new(mps::Broker::new());
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        Some(Arc::clone(&broker)),
        None,
        None,
        None,
    );
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![stub.to_string_lossy().to_string(), mode.to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let msg = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
    c.send_message(msg.to_string(), config).unwrap();
    (c, broker)
}

/// Wait until the stub is actually running: it has printed its init line
/// and the controller captured the session id from it. A bare pid isn't
/// enough — node can take seconds to start on a loaded CI runner, and the
/// timing assertions must not include that. The budget is generous: a
/// Windows runner scanning a freshly written .js file took over 15s once
/// (the #3409 CI run). The panic says what state it was stuck in.
async fn wait_for_pid(c: &PersistentSubprocessController) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let (pid, sid) = {
            let g = c.inner.lock().unwrap();
            (g.current_pid, g.session_id.clone())
        };
        if pid.is_some() && sid.as_deref() == Some("stub-session") {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("stub process never became ready: current_pid={pid:?} session_id={sid:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn failures(broker: &mps::Broker, block_id: &str) -> usize {
    broker
        .read_event_history(mps::EVENT_AGENT_FAILURE, &format!("block:{block_id}"), 10)
        .len()
}

#[tokio::test]
async fn never_spawned_is_not_running() {
    let c = PersistentSubprocessController::new("tab".into(), "blk-never".into(), None, None, None, None);
    let outcome = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(1)).await;
    assert_eq!(outcome, StopOutcome::NotRunning);
}

/// Mid-turn: EOF alone would let the turn run on (§5.1). The interrupt
/// ends it, the process exits well inside the deadline, and the
/// interrupted turn's `is_error` result is NOT reported as a failure.
#[tokio::test]
async fn mid_turn_is_interrupted_and_exits_without_a_failure() {
    let Some(stub) = stub_path() else { return };
    let block_id = "blk-shutdown-midturn";
    let (c, broker) = start(block_id, "turn", &stub);
    wait_for_pid(&c).await;
    assert!(c.health_monitor.is_active_turn(), "precondition: a turn is running");

    let started = std::time::Instant::now();
    let outcome = c.shutdown(started + std::time::Duration::from_secs(5)).await;
    let elapsed = started.elapsed();

    assert_eq!(outcome, StopOutcome::Exited, "must exit on its own, not be killed");
    assert!(elapsed < std::time::Duration::from_secs(3), "took {elapsed:?}");
    assert_eq!(failures(&broker, block_id), 0, "a requested stop is not a failure");
    let _ = std::fs::remove_file(stub);
}

#[tokio::test]
async fn idle_exits_on_eof() {
    let Some(stub) = stub_path() else { return };
    let (c, _broker) = start("blk-shutdown-idle", "idle", &stub);
    wait_for_pid(&c).await;
    // The stub answers the message with a `result`, ending the turn. Node
    // startup on a loaded CI runner can take seconds — wait for it rather
    // than assume.
    for _ in 0..200 {
        if !c.health_monitor.is_active_turn() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!c.health_monitor.is_active_turn(), "precondition: idle");

    let started = std::time::Instant::now();
    let outcome = c.shutdown(started + std::time::Duration::from_secs(5)).await;
    assert_eq!(outcome, StopOutcome::Exited);
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    let _ = std::fs::remove_file(stub);
}

/// A process that ignores both the interrupt and EOF is killed at the
/// deadline — not before, and not long after.
#[tokio::test]
async fn stubborn_process_is_killed_at_the_deadline() {
    let Some(stub) = stub_path() else { return };
    let (c, _broker) = start("blk-shutdown-stubborn", "stubborn", &stub);
    wait_for_pid(&c).await;

    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_millis(1500);
    let outcome = c.shutdown(deadline).await;
    let elapsed = started.elapsed();

    assert_eq!(outcome, StopOutcome::Killed);
    assert!(elapsed >= std::time::Duration::from_millis(1400), "killed early: {elapsed:?}");
    assert!(elapsed < std::time::Duration::from_secs(4), "killed late: {elapsed:?}");
    let _ = std::fs::remove_file(stub);
}

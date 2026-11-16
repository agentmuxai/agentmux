// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The spend cap's interrupt against a real process: a node script that stands in for the CLI, starts a turn that never
//! ends by itself, and ends it with `error_during_execution` when it is asked to interrupt. The turn must end, the
//! interrupt's error result must not show as a failure, and the process must stay up (the Stop button's kill would
//! restart it at the next message).

use super::super::*;

const STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type === "control_request" && m.request && m.request.subtype === "interrupt") {
    out({ type: "control_response", response: { subtype: "success", request_id: m.request_id } });
    out({ type: "result", subtype: "error_during_execution", is_error: true, session_id: "stub-session" });
    return;
  }
  if (m.type === "user") {
    const text = JSON.stringify(m);
    if (text.includes("again")) {
      out({ type: "result", subtype: "success", is_error: false, result: "ok", session_id: "stub-session" });
    } else {
      out({ type: "assistant", message: { content: [{ type: "text", text: "working" }] } });
    }
  }
});
rl.on("close", () => process.exit(0));
"#;

fn node_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file()))
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !done() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_caps_interrupt_ends_the_turn_and_leaves_the_process_up() {
    if !node_on_path() {
        eprintln!("cap_interrupt tests: `node` not on PATH, skipping");
        return;
    }
    let stub = std::env::temp_dir().join(format!("agentmux-cap-interrupt-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&stub, STUB).unwrap();
    let block_id = "blk-cap-interrupt";
    let broker = Arc::new(mps::Broker::new());
    let c = PersistentSubprocessController::new("tab".to_string(), block_id.to_string(), Some(Arc::clone(&broker)), None, None, None);
    let config = || PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![stub.to_string_lossy().to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let message = |text: &str| format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#);
    c.send_message(message("go"), config()).unwrap();
    until("the turn to start", || c.health_monitor.is_active_turn()).await;
    let pid = c.inner.lock().unwrap().current_pid;
    assert!(pid.is_some(), "the process is running");

    Controller::interrupt_turn(&c).expect("the interrupt is sent");
    until("the interrupt to end the turn", || !c.health_monitor.is_active_turn()).await;

    assert_eq!(c.inner.lock().unwrap().current_pid, pid, "the same process is still up");
    let failed = broker.read_event_history(mps::EVENT_AGENT_FAILURE, &format!("block:{block_id}"), 10).len();
    assert_eq!(failed, 0, "the interrupt we sent is not a failure");

    // The process answers the next message itself: nothing was killed.
    c.send_message(message("again"), config()).unwrap();
    until("the next turn to start and end", || !c.health_monitor.is_active_turn()).await;
    assert_eq!(c.inner.lock().unwrap().current_pid, pid, "still the same process after the next turn");
    let _ = std::fs::remove_file(&stub);
}

#[test]
fn nothing_in_flight_is_not_an_error_and_leaves_no_mark_behind() {
    let c = PersistentSubprocessController::new("tab".to_string(), "blk-cap-idle".to_string(), None, None, None, None);
    assert!(Controller::interrupt_turn(&c).is_ok());
    assert!(!stdout_reader::loop_stop::take_cap_end("blk-cap-idle"), "an idle agent's interrupt marks nothing");
}

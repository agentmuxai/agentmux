// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The loop guard against a real process: a node script that stands in for the CLI, answers a user turn
//! with the same failing tool call five times, and ends the turn with `error_during_execution` when it is
//! interrupted. The turn must end, the agent must find a note about it, and the interrupt's error result
//! must not show as a failure.

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
    for (let i = 1; i <= 5; i++) {
      out({ type: "assistant", message: { content: [{ type: "tool_use", id: "t" + i, name: "mcp__agentmux__Tasks", input: { action: "claim" } }] } });
      out({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t" + i, is_error: true, content: "attempt is required" }] } });
    }
  }
});
rl.on("close", () => process.exit(0));
"#;

fn node_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file()))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_repeats_a_failing_call_is_ended_and_the_agent_is_told() {
    if !node_on_path() {
        eprintln!("loop_stop tests: `node` not on PATH, skipping");
        return;
    }
    let stub = std::env::temp_dir().join(format!("agentmux-loop-stop-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&stub, STUB).unwrap();
    let block_id = "blk-loop-stop";
    let broker = Arc::new(mps::Broker::new());
    let c = PersistentSubprocessController::new("tab".to_string(), block_id.to_string(), Some(Arc::clone(&broker)), None, None, None);
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![stub.to_string_lossy().to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let msg = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"go"}]}}"#;
    c.send_message(msg.to_string(), config).unwrap();

    // The turn ends by itself: the stub never sends a result of its own.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let ended = {
            // Dropped unsent, so the notes are held again.
            let (full, _attached) = crate::backend::reactive::held_notes::attach(block_id, "next message");
            full.contains("repeated failing call: Tasks x5") && !c.health_monitor.is_active_turn()
        };
        if ended {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the guard never ended the turn (active turn: {})", c.health_monitor.is_active_turn());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let failed = broker.read_event_history(mps::EVENT_AGENT_FAILURE, &format!("block:{block_id}"), 10).len();
    assert_eq!(failed, 0, "the interrupt we sent is not a failure");
    let _ = std::fs::remove_file(&stub);
}

#[test]
fn a_replaced_processs_reader_does_not_interrupt_its_successor() {
    let c = PersistentSubprocessController::new("tab".to_string(), "blk-loop-fence".to_string(), None, None, None, None);
    let (tx, mut rx) = mpsc::channel::<String>(4);
    {
        let mut g = c.inner.lock().unwrap();
        g.stdin_tx = Some(tx);
        g.spawn_generation = 7;
    }
    let stuck = crate::agents::loop_guard::Stuck::SameThought;

    // The reader of generation 6 saw the loop; generation 7 is the process whose stdin this is now.
    assert!(stdout_reader::loop_stop::end_the_turn(&c.inner, 6, "blk-loop-fence", None, &stuck).is_none());
    assert!(rx.try_recv().is_err(), "nothing was sent to the successor's stdin");
    assert_eq!(crate::backend::reactive::held_notes::attach("blk-loop-fence", "next").0, "next", "no note is held for a turn that did not end");

    assert_eq!(stdout_reader::loop_stop::end_the_turn(&c.inner, 7, "blk-loop-fence", None, &stuck).as_deref(), Some("stuck: repeating the same thought"));
    assert!(rx.try_recv().unwrap().contains("interrupt"), "the current process's reader does interrupt");
}

#[test]
fn the_frame_of_a_turn_we_ended_says_why_instead_of_the_generic_error() {
    let frame = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s1"}"#;
    let plain: serde_json::Value = serde_json::from_str(&stdout_reader::loop_stop::plain_end_frame(frame, "stuck: repeating the same thought")).unwrap();
    assert_eq!(plain["result"], "Stopped by AgentMux: stuck: repeating the same thought.");
    assert_eq!(plain["is_error"], true, "still the turn's error result");
    assert_eq!(plain["session_id"], "s1", "the rest of the frame is kept");
    assert_eq!(stdout_reader::loop_stop::plain_end_frame("not json", "x"), "not json");
}

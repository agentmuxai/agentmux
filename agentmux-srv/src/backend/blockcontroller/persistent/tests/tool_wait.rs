// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Delivery while the agent waits on a tool call
//! (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.7): a message may be written
//! once the model has stopped writing, but never while it is mid-sentence, and
//! only one per tool wait.

use super::super::*;
use crate::backend::storage::filestore::FileStore;
use serde_json::json;

fn busy_controller() -> (PersistentSubprocessController, mpsc::Receiver<String>) {
    let c = PersistentSubprocessController::new("tab".to_string(), "block".to_string(), None, None, None, None);
    let (tx, rx) = mpsc::channel::<String>(16);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    c.health_monitor.set_active_turn(true);
    (c, rx)
}

fn enter(c: &PersistentSubprocessController) -> DeferredFlush {
    signal(c, ToolWaitSignal::Enter)
}

fn signal(c: &PersistentSubprocessController, s: ToolWaitSignal) -> DeferredFlush {
    let mut inner = c.inner.lock().unwrap();
    let generation = inner.spawn_generation;
    PersistentSubprocessController::tool_wait_locked(&mut inner, generation, s, "block")
}

fn queued(c: &PersistentSubprocessController) -> usize {
    c.inner.lock().unwrap().deferred_deliveries.len()
}

// ── Reading the stream ───────────────────────────────────────────────

/// Only the finished message opens a tool wait, not each tool call as it
/// streams in: a message can hold several calls, and one of them can be a
/// question for the operator.
#[test]
fn a_finished_tool_use_message_means_the_model_is_waiting_on_its_tools() {
    let delta = json!({"type": "stream_event", "parent_tool_use_id": null,
        "event": {"type": "message_delta", "delta": {"stop_reason": "tool_use"}}});
    let bash = json!({"type": "assistant", "parent_tool_use_id": null,
        "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}]}});
    assert_eq!(tool_wait_signal(&delta), Some(ToolWaitSignal::Enter));
    assert_eq!(tool_wait_signal(&bash), None, "a single call streaming in is not the whole message");
}

#[test]
fn a_tool_that_waits_on_the_operator_blocks_the_wait() {
    let ask = json!({"type": "assistant", "parent_tool_use_id": null,
        "message": {"content": [{"type": "tool_use", "id": "t1", "name": "AskUserQuestion", "input": {}}]}});
    let text_then_ask = json!({"type": "assistant",
        "message": {"content": [{"type": "text", "text": "one question"},
                                {"type": "tool_use", "id": "t2", "name": "AskUserQuestion", "input": {}}]}});
    assert_eq!(tool_wait_signal(&ask), Some(ToolWaitSignal::Blocked));
    assert_eq!(tool_wait_signal(&text_then_ask), Some(ToolWaitSignal::Blocked));
}

#[test]
fn text_thinking_and_a_new_message_mean_the_model_is_writing() {
    let text = json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "hi"}]}});
    let thinking = json!({"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "hm"}]}});
    let start = json!({"type": "stream_event", "event": {"type": "message_start"}});
    for line in [&text, &thinking, &start] {
        assert_eq!(tool_wait_signal(line), Some(ToolWaitSignal::Leave), "{line}");
    }
}

#[test]
fn a_message_ending_the_turn_is_not_a_tool_wait() {
    let end_turn = json!({"type": "stream_event",
        "event": {"type": "message_delta", "delta": {"stop_reason": "end_turn"}}});
    assert_eq!(tool_wait_signal(&end_turn), None);
}

#[test]
fn deltas_and_unknown_lines_change_nothing() {
    let delta = json!({"type": "stream_event",
        "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "x"}}});
    assert_eq!(tool_wait_signal(&delta), None);
    assert_eq!(tool_wait_signal(&json!({"type": "system", "subtype": "task_started"})), None);
    assert_eq!(tool_wait_signal(&json!({"no": "type"})), None);
}

/// A subagent finishing a tool call says nothing about whether the parent is
/// mid-sentence.
#[test]
fn a_subagents_lines_are_ignored() {
    let sub = json!({"type": "assistant", "parent_tool_use_id": "toolu_parent",
        "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}]}});
    assert_eq!(tool_wait_signal(&sub), None);
}

// ── The gate ─────────────────────────────────────────────────────────

/// While the model is writing, nothing is released: the original guarantee.
#[tokio::test]
async fn a_message_is_held_while_the_model_is_writing() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("jekt".to_string()).unwrap();
    assert!(rx.try_recv().is_err());
    assert_eq!(queued(&c), 1);
}

/// The requested change: a queued message goes out as soon as the agent is
/// waiting on a tool, without waiting for the turn to end.
#[tokio::test]
async fn a_held_message_is_released_when_a_tool_call_starts() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("jekt".to_string()).unwrap();

    let flushed = enter(&c);

    assert!(matches!(flushed, DeferredFlush::Released(ref l) if l.contains("jekt")));
    assert!(rx.try_recv().unwrap().contains("jekt"), "written to stdin during the tool wait");
    assert_eq!(queued(&c), 0);
    assert!(c.health_monitor.is_active_turn(), "the turn is still running");
}

/// A message arriving while the agent is already waiting on a tool is written
/// at once, not parked until the tool returns.
#[tokio::test]
async fn a_message_arriving_during_a_tool_wait_is_written_now() {
    let (c, mut rx) = busy_controller();
    assert_eq!(enter(&c), DeferredFlush::Empty);

    c.send_user_message("jekt".to_string()).unwrap();

    assert!(rx.try_recv().unwrap().contains("jekt"));
    assert_eq!(queued(&c), 0);
}

/// One message per tool wait. The same call is reported twice (its `assistant`
/// line, then the `message_delta`), and parallel calls report once each.
#[tokio::test]
async fn only_one_message_is_released_per_tool_wait() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("first".to_string()).unwrap();
    c.send_user_message("second".to_string()).unwrap();

    assert!(matches!(enter(&c), DeferredFlush::Released(_)));
    assert_eq!(enter(&c), DeferredFlush::Empty, "the message_delta for the same call");
    assert_eq!(enter(&c), DeferredFlush::Empty, "a parallel tool call in the same message");

    assert!(rx.try_recv().unwrap().contains("first"));
    assert!(rx.try_recv().is_err(), "the second message must wait");
    assert_eq!(queued(&c), 1);

    c.send_user_message("third".to_string()).unwrap();
    assert!(rx.try_recv().is_err(), "a spent tool wait holds later arrivals too");
    assert_eq!(queued(&c), 2);
}

/// The next tool wait releases the next message, in the order they arrived.
#[tokio::test]
async fn the_next_tool_wait_releases_the_next_message() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("first".to_string()).unwrap();
    c.send_user_message("second".to_string()).unwrap();
    enter(&c);
    let _ = rx.try_recv();

    assert_eq!(signal(&c, ToolWaitSignal::Leave), DeferredFlush::Empty, "the model writes again");
    assert!(matches!(enter(&c), DeferredFlush::Released(ref l) if l.contains("second")));
    assert!(rx.try_recv().unwrap().contains("second"));
}

/// Back to writing: the gate closes again.
#[tokio::test]
async fn a_message_arriving_after_the_model_starts_writing_again_is_held() {
    let (c, mut rx) = busy_controller();
    enter(&c);
    signal(&c, ToolWaitSignal::Leave);

    c.send_user_message("jekt".to_string()).unwrap();

    assert!(rx.try_recv().is_err());
    assert_eq!(queued(&c), 1);
}

/// ReAgent P1 on #3741. The frames arrive as: the assistant line calling
/// AskUserQuestion, the `message_delta` ending the message, and only then the
/// `control_request` that fills `pending_questions`. The gate must already be
/// closed when the `message_delta` arrives, with the map still empty.
#[tokio::test]
async fn a_question_for_the_operator_keeps_the_gate_closed_before_it_is_parked() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("jekt".to_string()).unwrap();

    let ask = json!({"type": "assistant", "parent_tool_use_id": null,
        "message": {"content": [{"type": "tool_use", "id": "toolu_q", "name": "AskUserQuestion", "input": {}}]}});
    let delta = json!({"type": "stream_event", "parent_tool_use_id": null,
        "event": {"type": "message_delta", "delta": {"stop_reason": "tool_use"}}});

    assert_eq!(signal(&c, tool_wait_signal(&ask).unwrap()), DeferredFlush::Empty);
    assert!(c.inner.lock().unwrap().pending_questions.is_empty(), "not parked yet, as in the real frame order");
    assert_eq!(signal(&c, tool_wait_signal(&delta).unwrap()), DeferredFlush::Empty);
    assert!(rx.try_recv().is_err(), "nothing may be written between the question and its park");
    assert_eq!(queued(&c), 1);

    // A message arriving now is held too, park or no park.
    c.send_user_message("second".to_string()).unwrap();
    assert!(rx.try_recv().is_err());

    // The operator answers, the CLI moves on: the next message starts, and the
    // gate is an ordinary closed one again.
    signal(&c, ToolWaitSignal::Leave);
    assert_eq!(c.inner.lock().unwrap().tool_wait, ToolWait::Writing);
}

/// Belt and braces: a parked question or permission closes the gate even if
/// the state says otherwise.
#[tokio::test]
async fn a_pending_question_or_permission_keeps_the_gate_closed() {
    let (c, mut rx) = busy_controller();
    c.inner.lock().unwrap().pending_questions.insert(
        "toolu_q".to_string(),
        ("req-1".to_string(), json!({"questions": []})),
    );
    c.send_user_message("jekt".to_string()).unwrap();

    assert_eq!(enter(&c), DeferredFlush::Empty);
    assert!(rx.try_recv().is_err());
    assert_eq!(queued(&c), 1);
}

/// A leftover line from a replaced process must not open the gate on the new
/// one, or write into its stdin.
#[tokio::test]
async fn a_replaced_processs_tool_call_is_ignored() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("jekt".to_string()).unwrap();
    let stale = c.inner.lock().unwrap().spawn_generation;
    c.inner.lock().unwrap().spawn_generation += 1;

    let flushed = {
        let mut inner = c.inner.lock().unwrap();
        PersistentSubprocessController::tool_wait_locked(&mut inner, stale, ToolWaitSignal::Enter, "block")
    };

    assert_eq!(flushed, DeferredFlush::Empty);
    assert!(rx.try_recv().is_err());
    assert_eq!(c.inner.lock().unwrap().tool_wait, ToolWait::Writing);
}

/// The turn ends: the wait is over with it, so the next turn starts closed.
#[tokio::test]
async fn a_result_frame_closes_the_gate() {
    let (c, _rx) = busy_controller();
    enter(&c);
    {
        let mut inner = c.inner.lock().unwrap();
        let generation = inner.spawn_generation;
        PersistentSubprocessController::turn_boundary_locked(&mut inner, &c.health_monitor, "block", generation);
    }
    assert_eq!(c.inner.lock().unwrap().tool_wait, ToolWait::Writing);
}

/// A message released at a boundary starts a new turn. Its own first tool call
/// must be able to release the next one, so it starts closed, not spent.
#[tokio::test]
async fn a_message_released_at_a_boundary_starts_a_turn_that_can_open_the_gate() {
    let (c, mut rx) = busy_controller();
    c.send_user_message("first".to_string()).unwrap();
    c.send_user_message("second".to_string()).unwrap();
    {
        let mut inner = c.inner.lock().unwrap();
        let generation = inner.spawn_generation;
        let b = PersistentSubprocessController::turn_boundary_locked(&mut inner, &c.health_monitor, "block", generation)
            .unwrap();
        assert!(matches!(b.flushed, DeferredFlush::Released(_)));
    }
    assert!(rx.try_recv().unwrap().contains("first"));

    assert!(matches!(enter(&c), DeferredFlush::Released(ref l) if l.contains("second")));
}

/// Same for the idle fast path: the message starts a turn whose tool call is
/// still to come.
#[tokio::test]
async fn an_idle_delivery_leaves_the_next_tool_wait_able_to_open() {
    let c = PersistentSubprocessController::new("tab".to_string(), "block".to_string(), None, None, None, None);
    let (tx, mut rx) = mpsc::channel::<String>(16);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    c.send_user_message("starts the turn".to_string()).unwrap();
    assert!(rx.try_recv().is_ok());

    c.send_user_message("second".to_string()).unwrap();
    assert!(rx.try_recv().is_err(), "the turn is running and the model is writing");
    assert!(matches!(enter(&c), DeferredFlush::Released(_)));
}

/// The human operator's own message is never gated.
#[tokio::test]
async fn immediate_policy_is_unchanged() {
    let (c, mut rx) = busy_controller();
    c.send_user_message_with_policy("stop".to_string(), DeliverPolicy::Immediate).unwrap();
    assert!(rx.try_recv().unwrap().contains("stop"));
}

// ── Against a real process ───────────────────────────────────────────

/// One turn that streams text, then runs a tool for 250 ms, then finishes. The
/// stub reports whether it received the jekt while the tool was running. The
/// tool wait is shorter than the 500 ms watchdog tick on purpose: only the
/// stdout reader's own release can land inside it, so this fails if the
/// watchdog is the only thing releasing.
const STUB: &str = r#"
const tool = process.argv[2] || "Bash";
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
let users = 0;
let duringTool = false;
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type !== "user") return;
  if (++users > 1) {
    out({ type: "system", subtype: "jekt_received", during_tool: duringTool });
    return;
  }
  out({ type: "stream_event", event: { type: "message_start" }, parent_tool_use_id: null });
  out({ type: "assistant", message: { content: [{ type: "text", text: "explaining" }] }, session_id: "stub-session" });
  setTimeout(() => {
    duringTool = true;
    out({ type: "assistant", message: { content: [{ type: "tool_use", id: "t1", name: tool, input: {} }] }, session_id: "stub-session" });
    out({ type: "stream_event", event: { type: "message_delta", delta: { stop_reason: "tool_use" } }, parent_tool_use_id: null });
    setTimeout(() => {
      duringTool = false;
      out({ type: "assistant", message: { content: [{ type: "text", text: "all done" }] }, session_id: "stub-session" });
      out({ type: "result", subtype: "success", is_error: false, result: "turn ended", session_id: "stub-session" });
    }, 250);
  }, 700);
});
rl.on("close", () => process.exit(0));
"#;

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !cond() {
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for: {what}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Runs the stub's turn calling `tool`, sends a jekt while the agent is still
/// writing, and returns the transcript as of the moment the agent received it.
async fn run_scenario(tool: &str, block_id: &str) -> Option<String> {
    let has_node = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    });
    if !has_node {
        eprintln!("tool_wait tests: `node` not on PATH — skipping");
        return None;
    }
    let stub = std::env::temp_dir().join(format!("agentmux-tool-wait-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&stub, STUB).unwrap();

    let broker = Arc::new(mps::Broker::new());
    let fs = Arc::new(FileStore::open_in_memory().unwrap());
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        Some(Arc::clone(&broker)),
        None,
        None,
        Some(Arc::clone(&fs)),
    ));
    c.set_self_ref();
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![stub.to_string_lossy().to_string(), tool.to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let first = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"first prompt"}]}}"#;
    c.send_message(first.to_string(), config).unwrap();
    wait_until("the turn to be running", || {
        c.health_monitor.is_active_turn() && !c.inner.lock().unwrap().spawning_in_progress
    })
    .await;

    c.send_user_message("TOOL-WAIT-JEKT".to_string()).unwrap();
    assert_eq!(queued(&c), 1, "held while the agent is still writing");

    let read = || {
        fs.read_file(block_id, crate::backend::agent_session::OUTPUT_FILE)
            .unwrap()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    };
    wait_until("the agent to receive the jekt", || read().contains("jekt_received")).await;
    let transcript = read();
    assert_eq!(queued(&c), 0);

    wait_until("the turn to finish", || read().contains("turn ended")).await;
    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
    Some(transcript)
}

#[tokio::test]
async fn a_jekt_sent_while_the_agent_writes_lands_during_its_tool_call() {
    let Some(transcript) = run_scenario("Bash", "blk-tool-wait").await else { return };
    assert!(
        transcript.contains("\"during_tool\":true"),
        "the agent must receive it while its tool runs, not at the end of the turn:\n{transcript}"
    );
    assert!(!transcript.contains("turn ended"), "{transcript}");
    let lines: Vec<&str> = transcript.lines().collect();
    let position = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
    assert!(
        position("\"tool_use\"") < position("TOOL-WAIT-JEKT"),
        "the jekt follows the tool call it was released by:\n{transcript}"
    );
}

/// ReAgent P1 on #3741, against a real process: the stub calls AskUserQuestion
/// and never sends the `control_request`, so nothing is parked. The gate must
/// still hold the jekt, because the agent is waiting on a person.
#[tokio::test]
async fn a_jekt_is_held_while_the_agent_waits_on_a_question() {
    let Some(transcript) = run_scenario("AskUserQuestion", "blk-tool-wait-ask").await else { return };
    assert!(
        transcript.contains("\"during_tool\":false"),
        "the agent must not receive it while the question is open:\n{transcript}"
    );
    assert!(transcript.contains("turn ended"), "released at the turn boundary instead:\n{transcript}");
}

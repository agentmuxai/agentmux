// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The turn ledger against a real process and the real stdout reader
//! (`SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md` §4.3, §4.7). The
//! stub replays the line order recorded from Claude Code 2.1.112 headless:
//! every pass opens with `system/init`; a background task that finishes
//! reports `task_updated` / `task_notification` after the `result`, then the
//! CLI starts a pass by itself; input that reaches it mid-pass is answered in
//! a pass of its own straight after the `result`. The health tests drive the
//! tracker directly; these check that the reader feeds it those lines.

use super::super::*;
use crate::backend::blockcontroller::health::{TurnEnd, TurnLedger, TurnOrigin};

const STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
// Something at spawn, before any input: never a pass by itself.
out({ type: "system", subtype: "init", session_id: "stub-session" });
let running = false;
const queued = [];
function pass(text, after) {
  running = true;
  out({ type: "system", subtype: "init", session_id: "stub-session" });
  out({ type: "assistant", message: { content: [{ type: "text", text: "on it: " + text }] }, session_id: "stub-session" });
  setTimeout(() => {
    out({ type: "result", subtype: "success", is_error: false, num_turns: 2, duration_api_ms: 900,
          total_cost_usd: 0.01, usage: { input_tokens: 10, output_tokens: 100 }, session_id: "stub-session" });
    running = false;
    // Input that arrived mid-pass: answered in a pass of its own, at once.
    if (queued.length) { pass(queued.shift()); return; }
    if (after) after();
  }, 600);
}
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type !== "user") return;
  const content = m.message && m.message.content;
  const text = typeof content === "string" ? content : JSON.stringify(content);
  if (running) { queued.push(text); return; }
  // "BG": start a background task that finishes after the turn has ended.
  pass(text, text.includes("BG") ? () => setTimeout(() => {
    out({ type: "system", subtype: "task_updated", task_id: "t1", session_id: "stub-session" });
    out({ type: "system", subtype: "task_notification", task_id: "t1", tool_use_id: "toolu_1", status: "completed", session_id: "stub-session" });
    pass("task finished");
  }, 800) : undefined);
});
rl.on("close", () => process.exit(0));
"#;

/// `None` (test skipped with a note) when `node` isn't on PATH.
fn stub_path() -> Option<std::path::PathBuf> {
    let has_node = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    });
    if !has_node {
        eprintln!("turn_ledger tests: `node` not on PATH — skipping");
        return None;
    }
    let path = std::env::temp_dir().join(format!("agentmux-turn-ledger-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&path, STUB).unwrap();
    Some(path)
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !cond() {
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for: {what}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn user_line(text: &str) -> String {
    format!(r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#)
}

/// A controller on the stub, recording every ledger it publishes, with its
/// first turn started by `prompt`.
fn start(block_id: &str, stub: &std::path::Path, prompt: &str) -> (Arc<PersistentSubprocessController>, Arc<Mutex<Vec<TurnLedger>>>) {
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        Some(Arc::new(mps::Broker::new())),
        None,
        None,
        Some(Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap())),
    ));
    c.set_self_ref();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    c.health_monitor.set_ledger_publisher(Arc::new(move |l: &TurnLedger| s.lock().unwrap().push(l.clone())));
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
    // As the pane sends it: labelled as the user's.
    let input = crate::backend::blockcontroller::health::TurnInput {
        origin: TurnOrigin::User,
        text: prompt.to_string(),
        joins_turn: None,
    };
    c.send_message_with_images_from(user_line(prompt), Vec::new(), config, Some(input)).unwrap();
    (c, seen)
}

fn ended_turns(seen: &Mutex<Vec<TurnLedger>>) -> Vec<TurnLedger> {
    let seen = seen.lock().unwrap();
    let mut ids: Vec<u64> = seen.iter().map(|l| l.turn_id).collect();
    ids.dedup();
    ids.iter()
        .filter_map(|id| seen.iter().rev().find(|l| l.turn_id == *id && l.end.is_some()).cloned())
        .collect()
}

/// A background task that finishes after the turn ended: the CLI's own pass
/// is marked active (it used to stay idle to srv for the whole pass) and is
/// an automated turn of its own, with its result counted.
#[tokio::test]
async fn a_background_task_that_wakes_the_cli_is_a_busy_turn_of_its_own() {
    let Some(stub) = stub_path() else { return };
    let (c, seen) = start("blk-ledger-bg", &stub, "start BG work");

    wait_until("two ended turns", || ended_turns(&seen).len() >= 2).await;
    let turns = ended_turns(&seen);
    assert_eq!(turns[0].origin, Some(TurnOrigin::User));
    assert_eq!((turns[0].passes, turns[0].counted_passes, turns[0].output_tokens), (1, 1, 100));
    assert_eq!(turns[0].end, Some(TurnEnd::Completed));
    assert_eq!(turns[1].origin, Some(TurnOrigin::Automated), "the CLI woke for the task");
    assert_eq!((turns[1].passes, turns[1].counted_passes, turns[1].steps), (1, 1, 2));
    let woke_active = seen.lock().unwrap().iter().any(|l| l.turn_id == turns[1].turn_id && l.active);
    assert!(woke_active, "the self-started pass was reported busy");
    assert!(!c.health_monitor.is_active_turn(), "and idle again after its result");

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

/// A jekt that reaches the agent mid-pass and is answered in a pass of its
/// own: one turn of two passes, both counted, never reported idle between.
#[tokio::test]
async fn a_jekt_answered_in_its_own_pass_joins_the_turn() {
    let Some(stub) = stub_path() else { return };
    let (c, seen) = start("blk-ledger-jekt", &stub, "first prompt");
    wait_until("the first pass to run", || {
        c.health_monitor.is_active_turn() && !c.inner.lock().unwrap().spawning_in_progress
    })
    .await;
    c.send_user_message_outcome("JEKT-MID-PASS".to_string()).unwrap();

    wait_until("the turn to end", || ended_turns(&seen).first().is_some_and(|t| t.counted_passes == 2)).await;
    let turn = &ended_turns(&seen)[0];
    assert_eq!((turn.passes, turn.inputs, turn.counted_passes, turn.output_tokens), (2, 1, 2, 200));
    assert_eq!(turn.origin, Some(TurnOrigin::User));
    assert_eq!(ended_turns(&seen).len(), 1, "one turn, not two");

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

/// The CLI's `init` at spawn, before any input, never counts as a pass.
#[tokio::test]
async fn an_init_at_spawn_is_not_a_pass() {
    let Some(stub) = stub_path() else { return };
    let (c, seen) = start("blk-ledger-spawn", &stub, "hello");
    wait_until("the turn to end", || !ended_turns(&seen).is_empty()).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let turns = ended_turns(&seen);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].passes, 1);
    assert!(!c.health_monitor.is_active_turn());

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The turn-boundary flush against a real process: a node stub that ends
//! every turn with a `result` frame shortly after each prompt, so the real
//! stdout reader drives the flush (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`
//! §4.4). The unit tests in `send_input.rs` call the flush directly; this
//! checks what the transcript actually records.

use super::super::*;
use crate::backend::storage::filestore::FileStore;

const STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
let turn = 0;
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type !== "user") return;
  const n = ++turn;
  setTimeout(() => {
    out({ type: "assistant", message: { content: [{ type: "text", text: "reply " + n }] }, session_id: "stub-session" });
    out({ type: "result", subtype: "success", is_error: false, result: "turn " + n, session_id: "stub-session" });
  }, 300);
});
rl.on("close", () => process.exit(0));
"#;

/// `None` (test skipped with a note) when `node` isn't on PATH.
fn stub_path() -> Option<std::path::PathBuf> {
    let has_node = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    });
    if !has_node {
        eprintln!("turn_boundary_tests: `node` not on PATH — skipping");
        return None;
    }
    let path = std::env::temp_dir().join(format!("agentmux-turn-boundary-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&path, STUB).unwrap();
    Some(path)
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !cond() {
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for: {what}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Codex P1 on #3562: the released prompt's stdin write happens at the
/// boundary, but its transcript line must come AFTER the `result` that ended
/// the previous turn. Recorded the other way round, a reopened pane shows
/// `user(N+1)` before `result(N)`, and a live consumer can take turn N's
/// `result` as the end of the turn the released prompt just started.
#[tokio::test]
async fn a_released_prompt_is_recorded_after_the_result_that_released_it() {
    let Some(stub) = stub_path() else { return };
    let block_id = "blk-turn-boundary-order";
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
        cli_args: vec![stub.to_string_lossy().to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let first = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"first prompt"}]}}"#;
    c.send_message(first.to_string(), config).unwrap();
    wait_until("the first turn to be running", || {
        c.health_monitor.is_active_turn() && !c.inner.lock().unwrap().spawning_in_progress
    })
    .await;

    c.send_user_message("DEFERRED-JEKT".to_string()).unwrap();
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "precondition: deferred mid-turn");

    wait_until("the released prompt's own turn to finish", || {
        let text = fs
            .read_file(block_id, crate::backend::agent_session::OUTPUT_FILE)
            .unwrap()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        text.contains("\"turn 2\"")
    })
    .await;

    let transcript = String::from_utf8(
        fs.read_file(block_id, crate::backend::agent_session::OUTPUT_FILE).unwrap().unwrap(),
    )
    .unwrap();
    let lines: Vec<&str> = transcript.lines().collect();
    let position = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not in transcript:\n{transcript}"))
    };
    let result_1 = position("\"turn 1\"");
    let released = position("DEFERRED-JEKT");
    let reply_2 = position("\"reply 2\"");
    assert!(
        result_1 < released && released < reply_2,
        "expected result(1) < user(2) < reply(2), got {result_1}, {released}, {reply_2}:\n{transcript}"
    );

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

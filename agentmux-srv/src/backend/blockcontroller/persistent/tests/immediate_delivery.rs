// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Automated delivery against a real process
//! (`SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md` §2.1): a node stub that
//! reports every stdin line the moment it reads it — and whether a turn was
//! running then — and ends each turn with a `result` frame a while later. The
//! unit tests in `send_input.rs` drive the send path and the watchdog
//! directly; these check what a live agent actually receives, and when.

use super::super::*;
use crate::backend::storage::filestore::FileStore;

const STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
let turn = 0;
let running = 0;
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type !== "user") return;
  const content = m.message && m.message.content;
  const text = typeof content === "string" ? content : JSON.stringify(content);
  out({ type: "system", subtype: "stub_received", busy: running > 0, text, session_id: "stub-session" });
  const n = ++turn;
  running++;
  setTimeout(() => {
    out({ type: "assistant", message: { content: [{ type: "text", text: "reply " + n }] }, session_id: "stub-session" });
    out({ type: "result", subtype: "success", is_error: false, result: "turn " + n, session_id: "stub-session" });
    running--;
  }, 1500);
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
        eprintln!("immediate_delivery tests: `node` not on PATH — skipping");
        return None;
    }
    let path = std::env::temp_dir().join(format!("agentmux-immediate-delivery-stub-{}.js", uuid::Uuid::new_v4()));
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

/// A controller with a real stdout reader, broker and transcript, the
/// watchdog armed, and the stub's first turn started.
fn start(block_id: &str, stub: &std::path::Path) -> (Arc<PersistentSubprocessController>, Arc<FileStore>) {
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
    (c, fs)
}

fn transcript(fs: &FileStore, block_id: &str) -> String {
    fs.read_file(block_id, crate::backend::agent_session::OUTPUT_FILE)
        .unwrap()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// The stub's report of receiving the line containing `needle`.
fn received<'a>(transcript: &'a str, needle: &str) -> Option<(usize, &'a str)> {
    transcript
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("stub_received") && l.contains(needle))
}

/// The change itself, end to end: a jekt sent while the agent's turn is
/// running reaches the agent at once — during that turn, not after its
/// `result` — and the caller is told it was delivered.
#[tokio::test]
async fn a_jekt_sent_mid_turn_reaches_the_running_agent_at_once() {
    let Some(stub) = stub_path() else { return };
    let block_id = "blk-immediate-mid-turn";
    let (c, fs) = start(block_id, &stub);
    wait_until("the first turn to be running", || {
        received(&transcript(&fs, block_id), "first prompt").is_some()
            && !c.inner.lock().unwrap().spawning_in_progress
    })
    .await;
    assert!(c.health_monitor.is_active_turn(), "precondition: a turn is running");

    let outcome = c.send_user_message_outcome("JEKT-MID-TURN".to_string()).unwrap();
    assert_eq!(outcome, SendOutcome::Sent, "a live agent's message is not queued");
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());

    wait_until("the agent to receive the jekt", || received(&transcript(&fs, block_id), "JEKT-MID-TURN").is_some())
        .await;
    let t = transcript(&fs, block_id);
    let (at, line) = received(&t, "JEKT-MID-TURN").unwrap();
    assert!(line.contains("\"busy\":true"), "received while the first turn was running:\n{t}");
    let result_1 = t.lines().position(|l| l.contains("\"turn 1\""));
    assert!(result_1.is_none_or(|r| at < r), "received before the first turn ended:\n{t}");

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

/// The startup race stays fixed against a real spawn: a jekt sent the moment
/// the spawn starts is accepted (queued, not refused or dropped), and reaches
/// the agent once it is up — after the prompt that started it, never ahead.
#[tokio::test]
async fn a_jekt_sent_while_the_agent_starts_up_reaches_it_once_it_is_up() {
    let Some(stub) = stub_path() else { return };
    let block_id = "blk-immediate-startup";
    let (c, fs) = start(block_id, &stub);

    let outcome = c.send_user_message_outcome("JEKT-AT-STARTUP".to_string());
    assert!(outcome.is_ok(), "accepted, not refused: {outcome:?}");

    wait_until("the agent to receive the jekt", || received(&transcript(&fs, block_id), "JEKT-AT-STARTUP").is_some())
        .await;
    let t = transcript(&fs, block_id);
    let (first, _) = received(&t, "first prompt").expect("the first prompt was received");
    let (jekt, _) = received(&t, "JEKT-AT-STARTUP").unwrap();
    assert!(first < jekt, "the prompt that started the process goes first:\n{t}");
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

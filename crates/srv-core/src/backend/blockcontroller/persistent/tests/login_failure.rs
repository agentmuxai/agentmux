// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A turn that ends on an expired login marks the pane signed out, whatever the CLI's wording, against
//! a real process and the real stdout reader. The stub replays the lines Claude Code 2.1.288 wrote for
//! an expired credential: a synthetic assistant message with `"error": "authentication_failed"`, then a
//! `result` that carries only prose. A pane not marked signed out takes every later message into a turn
//! that fails in about 140 ms (the sender is told "Delivered").

use super::super::*;
use crate::agents::failure::FailureClass;
use crate::backend::agent_state::failure_of_block;
use crate::backend::obj::Block;

/// The first argument is the assistant message's `error`; the prose names neither a login nor a token.
const STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
out({ type: "system", subtype: "init", session_id: "stub-session" });
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type !== "user") return;
  out({ type: "system", subtype: "init", session_id: "stub-session" });
  out({ type: "assistant", error: process.argv[2], is_api_error_message: true, session_id: "stub-session",
        message: { model: "<synthetic>", role: "assistant", content: [{ type: "text", text: "Your session lapsed" }] } });
  out({ type: "result", subtype: "success", is_error: true, terminal_reason: "api_error", result: "Your session lapsed",
        duration_ms: 141, session_id: "stub-session" });
});
rl.on("close", () => process.exit(0));
"#;

fn stub_path() -> Option<std::path::PathBuf> {
    let has_node = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("node").is_file() || dir.join("node.exe").is_file())
    });
    if !has_node {
        eprintln!("login_failure tests: `node` not on PATH — skipping");
        return None;
    }
    let path = std::env::temp_dir().join(format!("agentmux-login-failure-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&path, STUB).unwrap();
    Some(path)
}

/// The failure the block has stored once the stub's turn ended (the result frame is the last line), or
/// `None` when the turn ended with none.
async fn stored_failure_after_a_turn(stub: &std::path::Path, error: &str) -> Option<FailureClass> {
    let mstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    mstore.insert(&mut block).unwrap();
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block.oid.clone(),
        Some(Arc::new(mps::Broker::new())),
        None,
        Some(Arc::clone(&mstore)),
        Some(Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap())),
    ));
    c.set_self_ref();
    let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&ended);
    c.health_monitor.set_ledger_publisher(Arc::new(move |l: &crate::backend::blockcontroller::health::TurnLedger| {
        if l.end.is_some() {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }));
    let config = PersistentSpawnConfig {
        cli_command: "node".to_string(),
        cli_args: vec![stub.to_string_lossy().to_string(), error.to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#.to_string();
    c.send_message_with_images_from(line, Vec::new(), config, None, RecordOrigin::new(MessageOrigin::user())).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !ended.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline, "the stub's turn never ended");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let stored = mstore.get::<Block>(&block.oid).ok().flatten().and_then(|b| failure_of_block(&b)).map(|f| f.code);
    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    stored
}

#[tokio::test]
async fn a_turn_that_ends_on_a_failed_login_marks_the_pane_signed_out_whatever_the_wording() {
    let Some(stub) = stub_path() else { return };
    assert_eq!(stored_failure_after_a_turn(&stub, "authentication_failed").await, Some(FailureClass::Auth));
    let _ = std::fs::remove_file(stub);
}

/// The control: the same prose without the marker is not taken for a sign-in failure.
#[tokio::test]
async fn the_same_prose_without_the_marker_is_not_a_sign_in_failure() {
    let Some(stub) = stub_path() else { return };
    assert_ne!(stored_failure_after_a_turn(&stub, "unknown").await, Some(FailureClass::Auth));
    let _ = std::fs::remove_file(stub);
}

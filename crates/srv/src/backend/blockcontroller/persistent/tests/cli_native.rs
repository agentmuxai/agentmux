// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Messages from other sessions through a real process and the real stdout
//! reader (`backend::cli_native`). The stub writes what Claude Code 2.1.288
//! writes under `--replay-user-messages`: the turn it was given replayed, a peer
//! message with its typed origin, the lifecycle lines about it.

use super::super::*;

const STUB: &str = r#"
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const rl = require("readline").createInterface({ input: process.stdin });
const peer = (uuid, msg_id, pid, name) => ({
  type: "user", uuid, isReplay: true, isSynthetic: true, session_id: "stub-session",
  message: { role: "user", content: "<cross-session-message from=\"uds:/x.sock\">\nthe body\n</cross-session-message>" },
  origin: { kind: "peer", from: "uds:/x.sock", verifiedPeerPid: pid, msg_id, name, fromMode: "bypass", body: "look at the failing check" },
});
out({ type: "system", subtype: "init", session_id: "stub-session" });
rl.on("line", (line) => {
  let m;
  try { m = JSON.parse(line); } catch { return; }
  if (m.type !== "user") return;
  out({ type: "system", subtype: "init", session_id: "stub-session" });
  // The turn srv wrote, replayed with the uuid it was given.
  out({ type: "user", uuid: m.uuid, isReplay: true, session_id: "stub-session", message: m.message });
  // A frame of the CLI's own, marked isReplay without the flag: it must stay.
  out({ type: "user", uuid: "u-card", isReplay: true, session_id: "stub-session", message: { role: "user", content: "<local-command-stdout>Compacted </local-command-stdout>" } });
  // From this very CLI process (so: this agent), signing itself as someone else.
  out(peer("u-peer-1", "m-agent", process.pid, "somebody-else"));
  out({ type: "command_lifecycle", command_uuid: "u-peer-1", state: "started", session_id: "stub-session" });
  // From the test process above the CLI: a process outside every agent pane.
  out(peer("u-peer-2", "m-outside", process.ppid, "agentx"));
  out({ type: "assistant", message: { content: [{ type: "text", text: "ANSWER-TEXT" }] }, session_id: "stub-session" });
  out({ type: "command_lifecycle", command_uuid: "u-peer-1", state: "completed", session_id: "stub-session" });
  out({ type: "result", subtype: "success", is_error: false, num_turns: 1, session_id: "stub-session" });
});
rl.on("close", () => process.exit(0));
"#;

fn node_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join("node").is_file() || d.join("node.exe").is_file()))
}

/// The pane's transcript has the answer, the CLI's own card, and none of the replayed turns; the audit has
/// both messages, the sender named from the pid and not from the name the message gave.
#[tokio::test]
async fn a_replayed_turn_stays_out_of_the_transcript_and_a_peer_message_is_audited() {
    if !node_on_path() {
        eprintln!("cli_native tests: `node` not on PATH — skipping");
        return;
    }
    let stub = std::env::temp_dir().join(format!("agentmux-cli-native-stub-{}.js", uuid::Uuid::new_v4()));
    std::fs::write(&stub, STUB).unwrap();
    let block = format!("blk-cli-native-{}", uuid::Uuid::new_v4());
    let filestore = Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap());
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block.clone(),
        Some(Arc::new(mps::Broker::new())),
        None,
        None,
        Some(filestore.clone()),
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
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"go"}]}}"#;
    c.send_message_with_images_from(line.to_string(), Vec::new(), config, None, RecordOrigin::new(MessageOrigin::user())).unwrap();

    let audited = |id: &str| {
        crate::backend::reactive::get_global_handler()
            .get_audit_log(usize::MAX)
            .into_iter()
            .filter(|e| e.event_kind == "cli_native" && e.block_id == block && e.request_id == id)
            .collect::<Vec<_>>()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while audited("m-agent").len() < 2 || audited("m-outside").is_empty() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the audit lines");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let output = filestore
        .read_file(&block, PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    assert!(output.contains("ANSWER-TEXT"), "{output}");
    // What the CLI says about its own turns is kept, as it was before the flag.
    assert!(output.contains(r#""command_uuid":"u-peer-1""#), "{output}");
    // The echo of the turn srv wrote ("go") and the peer messages are not in it; the CLI's own card is.
    assert_eq!(output.matches("isReplay").count(), 1, "only the CLI's own card is a replayed frame: {output}");
    for replayed in ["verifiedPeerPid", "cross-session-message", "the body"] {
        assert!(!output.contains(replayed), "{replayed} reached the transcript: {output}");
    }
    assert!(output.contains("<local-command-stdout>Compacted </local-command-stdout>"), "{output}");

    let mut from_cli = audited("m-agent");
    from_cli.sort_by_key(|e| e.outcome.clone());
    assert_eq!(from_cli.iter().map(|e| e.outcome.as_deref().unwrap()).collect::<Vec<_>>(), ["done", "read"]);
    let read = from_cli.iter().find(|e| e.outcome.as_deref() == Some("read")).unwrap();
    assert_eq!(read.source_agent.as_deref(), Some(block.as_str()), "this pane's own CLI process, however the message signed itself");
    let detail = read.cli_native.as_ref().unwrap();
    assert_eq!((detail.identity.as_str(), detail.claimed_name.as_deref(), detail.seen_by.as_str()), ("agent", Some("somebody-else"), "receiver"));
    assert_eq!(detail.preview, "look at the failing check");

    let outside = &audited("m-outside")[0];
    assert!(outside.source_agent.as_deref().unwrap().starts_with("outside session (pid "), "{:?}", outside.source_agent);
    assert_eq!(outside.cli_native.as_ref().unwrap().identity, "outside");
    assert_eq!(outside.cli_native.as_ref().unwrap().claimed_name.as_deref(), Some("agentx"));

    let _ = c.shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5)).await;
    let _ = std::fs::remove_file(stub);
}

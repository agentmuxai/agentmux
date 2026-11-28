// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The history guarantee, proved with marker strings (`SPEC_HISTORY_COMPACT_FORM_ONLY_2026_11_23.md` 13).
//!
//! A fake provider writes the stream a real one does (the person's message, fragments, the complete reply, a tool
//! result, the turn's end) through the same append path as the stdout reader, into a temporary AgentMux home with
//! real database files, and puts its own session file and a spill file under the agent's provider home. A byte scanner
//! then counts each marker in every file under the home, the database, its log and shared memory included.
//!
//! Each assertion is live when what it checks is built, and `#[ignore = "piece N: ..."]` until its piece of section 15
//! lands. The piece that lands it removes the `ignore` and fills the body; `cargo nextest run --run-ignored
//! ignored-only -E 'test(history_guarantee)'` lists what is left. The tests run in the Rust suite like any other, so a
//! piece that breaks a live assertion fails its own PR.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::storage::filestore::FileStore;
use crate::backend::transcript::append::append_transcript_with;
use crate::backend::transcript::framer;
use crate::backend::transcript::live::{self, LIVE_FILE};

/// The person's message.
const A: &str = "MARKER-A-person-message";
/// A thinking block of the complete reply.
const B: &str = "MARKER-B-thinking";
/// A Bash `tool_use` input.
const C: &str = "MARKER-C-bash-command";
/// The `tool_result` content of that call.
const D: &str = "MARKER-D-tool-output";
/// An Edit's new content.
const E: &str = "MARKER-E-edit-content";
/// A field only the `stream_event` lines carry.
const F: &str = "MARKER-F-fragment-only";
/// What only the provider's own session file holds.
const G: &str = "MARKER-G-provider-file-only";

const DEF: &str = "def-guar";
const ZONE: &str = "agent:def-guar:current";
const BLOCK: &str = "blk-guar";
const SID: &str = "cccccccc-1111-2222-3333-444444444444";
const OTHER_SID: &str = "dddddddd-1111-2222-3333-444444444444";

/// A temporary AgentMux home: the shared transcript store, a channel store, and a provider home.
struct Home {
    dir: tempfile::TempDir,
    gfs: Arc<FileStore>,
    fs: Arc<FileStore>,
}

impl Home {
    fn new() -> Self {
        // The store's own file is read as bytes by the scanner, so no frame may hide a marker (the framer is off).
        framer::set_frames_off_for_test(true);
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared/agents/transcripts");
        let channel = dir.path().join("channels/stable/data/db");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::create_dir_all(&channel).unwrap();
        let gfs = Arc::new(FileStore::open_global_transcripts(&shared.join("filestore.db")).unwrap());
        let fs = Arc::new(FileStore::open(&channel.join("filestore.db")).unwrap());
        Self { dir, gfs, fs }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// One line as the stdout reader appends it, in the default (single copy) mode.
    fn write(&self, line: &Value) {
        let data = format!("{line}\n");
        append_transcript_with(None, BLOCK, data.as_bytes(), Some(&self.fs), Some((&self.gfs, ZONE)), true, None, true);
    }

    /// The provider's session file and a spill file of its session folder, under the agent's provider home.
    fn provider_files(&self, sid: &str) -> (PathBuf, PathBuf) {
        let project = self.path("shared/providers/claude/projects/-work");
        std::fs::create_dir_all(project.join(sid).join("tool-results")).unwrap();
        let session = project.join(format!("{sid}.jsonl"));
        let record = json!({ "type": "user", "sessionId": sid, "uuid": "u-1", "message": { "role": "user", "content": format!("{A} {G}") } });
        std::fs::write(&session, format!("{record}\n")).unwrap();
        let spill = project.join(sid).join("tool-results/spill.txt");
        std::fs::write(&spill, format!("spill {G}")).unwrap();
        (session, spill)
    }

    /// The store's logical text of a file of the agent's zone.
    fn text(&self, name: &str) -> String {
        self.gfs.read_file(ZONE, name).unwrap().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
    }

    /// How many times `marker` is in the bytes of every file under the home, by path.
    fn scan(&self, marker: &str) -> Vec<(PathBuf, usize)> {
        let mut found = Vec::new();
        scan_dir(self.dir.path(), marker.as_bytes(), &mut found);
        found
    }

    fn total(&self, marker: &str) -> usize {
        self.scan(marker).iter().map(|(_, n)| n).sum()
    }
}

fn scan_dir(dir: &Path, needle: &[u8], found: &mut Vec<(PathBuf, usize)>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            scan_dir(&p, needle, found);
        } else if let Ok(bytes) = std::fs::read(&p) {
            let n = bytes.windows(needle.len()).filter(|w| *w == needle).count();
            if n > 0 {
                found.push((p, n));
            }
        }
    }
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// A fragment line. `F` rides in its `uuid`, which the complete lines do not repeat.
fn frag(event: Value) -> Value {
    json!({ "type": "stream_event", "session_id": SID, "uuid": F, "parent_tool_use_id": null, "event": event })
}

fn delta(ix: u64, delta: Value) -> Value {
    frag(json!({ "type": "content_block_delta", "index": ix, "delta": delta }))
}

fn start(ix: u64, block: Value) -> Value {
    frag(json!({ "type": "content_block_start", "index": ix, "content_block": block }))
}

/// One streamed reply with a thinking block, a Bash call and an Edit, the tool result, and the turn's end.
fn reply(home: &Home) {
    let edit = json!({ "file_path": "/work/a.txt", "new_string": E });
    let bash = json!({ "command": C });
    let lines = [
        json!({ "type": "system", "subtype": "init", "session_id": SID }),
        json!({ "type": "user", "session_id": SID, "message": { "role": "user", "content": A } }),
        frag(json!({ "type": "message_start", "message": { "id": "m1", "role": "assistant", "content": [] } })),
        start(0, json!({ "type": "thinking", "thinking": "", "signature": "" })),
        delta(0, json!({ "type": "thinking_delta", "thinking": B })),
        delta(0, json!({ "type": "signature_delta", "signature": "sig-1" })),
        start(1, json!({ "type": "tool_use", "id": "tu1", "name": "Bash", "input": {} })),
        delta(1, json!({ "type": "input_json_delta", "partial_json": bash.to_string() })),
        start(2, json!({ "type": "tool_use", "id": "tu2", "name": "Edit", "input": {} })),
        delta(2, json!({ "type": "input_json_delta", "partial_json": edit.to_string() })),
        json!({ "type": "assistant", "session_id": SID, "uuid": "u-a", "message": { "id": "m1", "role": "assistant", "content": [
            { "type": "thinking", "thinking": B, "signature": "sig-1" },
            { "type": "tool_use", "id": "tu1", "name": "Bash", "input": bash },
            { "type": "tool_use", "id": "tu2", "name": "Edit", "input": edit },
        ] } }),
        frag(json!({ "type": "message_stop" })),
        json!({ "type": "user", "session_id": SID, "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "tu1", "content": D }] } }),
        json!({ "type": "result", "session_id": SID, "subtype": "success" }),
    ];
    for l in &lines {
        home.write(l);
    }
}

// What the harness itself must do, so the assertions below can be trusted.

#[test]
fn history_guarantee_the_scanner_counts_every_file_under_the_home_including_the_database_and_its_log() {
    let h = Home::new();
    std::fs::write(h.path("plain.txt"), format!("x {G} y {G}")).unwrap();
    let (session, spill) = h.provider_files(SID);
    reply(&h);
    let found: Vec<PathBuf> = h.scan(G).into_iter().map(|(p, _)| p).collect();
    assert!(found.contains(&h.path("plain.txt")) && found.contains(&session) && found.contains(&spill));
    assert_eq!(h.total(G), 4, "two in plain.txt, one in the session file, one in the spill file, none made up");
    let in_store: usize = h.scan(A).into_iter().filter(|(p, _)| p.to_string_lossy().contains("filestore.db")).map(|(_, n)| n).sum();
    assert!(in_store >= 1, "the user's message is in the store's own bytes (database or log)");
}

#[test]
fn history_guarantee_the_framer_is_off_for_the_scan() {
    let _h = Home::new();
    assert!(!framer::enabled(), "a frame would hide a marker from the byte scan");
}

// Test 1 of section 13: a new install, one reply.

#[test]
fn history_guarantee_one_reply_keeps_markers_a_to_e_once_and_f_nowhere_in_the_store() {
    let h = Home::new();
    reply(&h);
    let output = h.text("output");
    for m in [A, B, C, D, E] {
        assert_eq!(count(&output, m), 1, "{m} is readable once in the store's output");
    }
    assert_eq!(count(&output, F), 0, "no fragment reached the store's output");
    assert!(!output.contains("stream_event"));
    assert_eq!(h.text(LIVE_FILE), "", "the live file is empty once the reply is complete");
    assert!(h.fs.read_file(BLOCK, "output").unwrap().is_none(), "single copy: no per-channel copy of the output");
    assert!(h.fs.read_file(BLOCK, LIVE_FILE).unwrap().is_none());
}

#[test]
#[ignore = "piece 4: secure_delete and the log scrub; freed pages and the log still hold the fragments"]
fn history_guarantee_f_is_in_no_file_under_the_home_after_the_reply() {
    let h = Home::new();
    reply(&h);
    h.gfs.checkpoint().unwrap();
    assert_eq!(h.scan(F), Vec::<(PathBuf, usize)>::new(), "free pages and the log included");
}

// Test 2: the provider file stays while its session lives.

#[test]
fn history_guarantee_a_provider_file_of_a_live_session_is_not_ended_and_a_second_session_is_untouched() {
    use crate::backend::continuity_segments::{record_end, record_session, record_start, EndReason, Rung, Start};
    use crate::backend::provider_sessions::{ended_sessions, Policy, Roots};
    let h = Home::new();
    let config = h.path("shared/providers/claude");
    let cwd = "/work".to_string();
    let mut begin = |sid: &str, at: i64| {
        let s = Start {
            segment_id: String::new(),
            agent_uid: "uid-guar".to_string(),
            definition_id: Some(DEF.to_string()),
            provider: "claude".to_string(),
            config_dir: Some(config.to_string_lossy().into_owned()),
            provider_session_id: None,
            cwd: cwd.clone(),
            channel: "c".to_string(),
            agentmux_version: "v".to_string(),
            block_id: BLOCK.to_string(),
            zone: Some(ZONE.to_string()),
            byte_start: None,
            started_at_ms: at,
            continuity_rung: Rung::Native,
            predecessor_segment_id: None,
            lease_epoch: None,
            identity_key: None,
            forked_from: None,
        };
        let id = record_start(&h.gfs, s).unwrap();
        record_session(&h.gfs, "uid-guar", &id, sid, at).unwrap();
        id
    };
    let first = begin(SID, 1_000);
    record_end(&h.gfs, "uid-guar", &first, EndReason::Stopped, None, 2_000).unwrap();
    begin(OTHER_SID, 3_000);
    // The files sit where the provider's home keeps them for this working folder.
    for sid in [SID, OTHER_SID] {
        let file = crate::backend::session_backfill::session_file_path(&config.to_string_lossy(), &cwd, sid).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, format!("{G}\n")).unwrap();
    }
    let roots = Roots { shared: Some(h.path("shared")), agentmux_home: Some(h.dir.path().to_path_buf()) };
    let now = agentmux_common::time::now_ms() + 40 * 24 * 60 * 60 * 1000;
    let ended = ended_sessions("uid-guar", &crate::backend::continuity_segments::segments(&h.gfs, "uid-guar"), &roots, &Policy { enabled: true, idle_days: 7 }, now);
    assert_eq!(ended.iter().map(|e| e.sid.as_str()).collect::<Vec<_>>(), vec![SID], "only the session the agent left has ended");
    assert_eq!(h.total(G), 2, "both provider files are still there: a sweep that is not run removes nothing");
}

// Test 5: a kill mid-reply.

#[test]
fn history_guarantee_a_kill_leaves_the_live_file_and_the_next_start_makes_an_interrupted_line() {
    let h = Home::new();
    h.write(&json!({ "type": "user", "session_id": SID, "message": { "role": "user", "content": A } }));
    h.write(&frag(json!({ "type": "message_start", "message": { "id": "m9", "role": "assistant", "content": [] } })));
    h.write(&start(0, json!({ "type": "text", "text": "" })));
    h.write(&delta(0, json!({ "type": "text_delta", "text": "MARKER-I-received-before-the-kill" })));
    assert!(h.text(LIVE_FILE).contains("MARKER-I-received-before-the-kill"));
    assert!(!h.text("output").contains("MARKER-I"));
    live::forget(BLOCK); // a new process: nothing in memory
    h.write(&json!({ "type": "system", "subtype": "init", "session_id": SID }));
    let output = h.text("output");
    assert_eq!(count(&output, "MARKER-I-received-before-the-kill"), 1);
    assert!(output.contains("\"interrupted\":true"));
    assert_eq!(h.text(LIVE_FILE), "");
}

// The assertions whose pieces have not landed. Each names its piece; the piece that lands it fills the body.

#[test]
#[ignore = "piece 3: the session-end sweep removes the provider file by session id, overwritten before unlink, and G is in no provider file; a failing proof removes nothing"]
fn history_guarantee_a_session_end_removes_the_provider_files_and_a_failed_proof_removes_nothing() {
    unimplemented!("test 3 of section 13");
}

#[test]
#[ignore = "piece 3: a provider-only session is imported first, its spill file readable from the store, H in output.provider"]
fn history_guarantee_a_provider_only_session_is_imported_before_it_is_removed() {
    unimplemented!("test 4 of section 13");
}

#[test]
#[ignore = "piece 3: a file named with a session id AgentMux did not start is not deleted"]
fn history_guarantee_a_foreign_session_file_is_not_deleted() {
    unimplemented!("test 6 of section 13");
}

#[test]
#[ignore = "piece 3e: the container path removes session files by session id through the container, or a helper container when it is stopped"]
fn history_guarantee_a_container_agents_session_files_are_removed_by_session_id() {
    unimplemented!("section 5.4, container agents");
}

#[test]
#[ignore = "piece 6: the migration reduces a synthetic raw store, L1 to L5 hold and fail on a corrupted result, a crash at each step leaves the old file"]
fn history_guarantee_the_migration_reduces_a_raw_store_and_its_checks_fail_on_a_corrupted_result() {
    unimplemented!("test 7 of section 13");
}

#[test]
#[ignore = "piece 4: secure_delete on every connection; no marker of removed content in the database, its log, .pre-reclaim or a removed path"]
fn history_guarantee_deleted_content_is_in_no_file_and_the_log_is_truncated() {
    unimplemented!("test 8 of section 13");
}

#[test]
#[ignore = "piece 5: no log at any level carries a marker from a full fake session"]
fn history_guarantee_no_log_carries_a_marker_at_any_level() {
    unimplemented!("test 9 of section 13");
}

#[test]
#[ignore = "piece 6: the same conversation read from a raw store and from its compact result gives the same messages, tool rows and watch output"]
fn history_guarantee_readers_give_the_same_conversation_from_a_raw_and_a_compact_store() {
    unimplemented!("test 10 of section 13");
}

#[test]
#[ignore = "piece 4: owner-only permissions after start, no route serves the store file, quick_check runs at start"]
fn history_guarantee_the_store_is_owner_only_and_checked_at_start() {
    unimplemented!("test 11 of section 13");
}

#[test]
#[ignore = "piece 3: the retention seed is rewritten only after the config directory's sessions are proved, and a user-set value is left alone"]
fn history_guarantee_the_retention_seed_changes_only_after_the_sessions_are_proved() {
    unimplemented!("test 12 of section 13");
}

#[test]
#[ignore = "piece 6: a compact store opens in the previous release's reader and renders the conversation"]
fn history_guarantee_a_compact_store_opens_in_the_previous_releases_reader() {
    unimplemented!("test 13 of section 13");
}

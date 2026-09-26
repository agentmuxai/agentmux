// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! First-spawn continuation (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md
//! §5 P0a): a pane that will render prior history must resume that history's
//! session, not spawn a blank one.

use super::super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

const LIVE_SID: &str = "86800ad0-8c85-40b8-b742-d1936f5677d1";

fn stream(lines: &[&str]) -> Vec<u8> {
    let mut s = lines.join("\n");
    s.push('\n');
    s.into_bytes()
}

// ── last_session_id_in_stream ───────────────────────────────────────────

#[test]
fn the_last_session_id_in_the_stream_wins() {
    let tail = stream(&[
        r#"{"type":"system","subtype":"init","session_id":"old-sid"}"#,
        r#"{"type":"assistant","session_id":"new-sid"}"#,
        r#"{"type":"result","session_id":"new-sid"}"#,
    ]);
    assert_eq!(last_session_id_in_stream(&tail, false, "session_id").as_deref(), Some("new-sid"));
}

/// AgentMux's own synthetic lines (`agentmux_session_outcome` carries
/// `attempted_sid`/`actual_sid`, not the provider field) and non-JSON noise
/// must be skipped, not mistaken for the conversation's id.
#[test]
fn lines_without_the_provider_field_are_skipped() {
    let tail = stream(&[
        r#"{"type":"assistant","session_id":"real-sid"}"#,
        r#"{"type":"system","subtype":"agentmux_session_outcome","outcome":"fresh","attempted_sid":null,"actual_sid":"other"}"#,
        "plain stderr-ish text",
        r#"{"type":"assistant","session_id":""}"#,
    ]);
    assert_eq!(last_session_id_in_stream(&tail, false, "session_id").as_deref(), Some("real-sid"));
}

/// A tail cut out of a larger file starts mid-line; that fragment can parse
/// as garbage or, worse, as a truncated but valid-looking object.
#[test]
fn a_tail_that_starts_mid_line_drops_its_first_fragment() {
    let tail = b"sion_id\":\"fragment-sid\"}\n{\"type\":\"x\"}\n".to_vec();
    assert_eq!(last_session_id_in_stream(&tail, true, "session_id"), None);
    assert_eq!(
        last_session_id_in_stream(b"{\"session_id\":\"whole-sid\"}\n", false, "session_id").as_deref(),
        Some("whole-sid")
    );
}

/// The newest line may still be mid-write; an unparseable last line must
/// fall back to the previous complete one.
#[test]
fn a_half_written_final_line_falls_back_to_the_previous_line() {
    let tail = b"{\"session_id\":\"complete-sid\"}\n{\"session_id\":\"trunc".to_vec();
    assert_eq!(last_session_id_in_stream(&tail, false, "session_id").as_deref(), Some("complete-sid"));
}

#[test]
fn the_provider_field_name_is_respected() {
    let tail = stream(&[r#"{"thread_id":"codex-thread","session_id":"not-this"}"#]);
    assert_eq!(last_session_id_in_stream(&tail, false, "thread_id").as_deref(), Some("codex-thread"));
}

// ── find_continuation_session_id ────────────────────────────────────────

struct Fixture {
    _tmp: tempfile::TempDir,
    config: PersistentSpawnConfig,
    fs: Arc<FileStore>,
}

/// A config dir holding `<LIVE_SID>.jsonl` for `working_dir`, and an
/// in-memory FileStore standing in for the pane's own transcript.
fn fixture(transcript_on_disk: bool) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let working_dir = "/agents/agenty".to_string();
    if transcript_on_disk {
        let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
        let dir = tmp.path().join("projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{LIVE_SID}.jsonl")), b"{}\n").unwrap();
    }
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), tmp.path().to_string_lossy().to_string());
    let config = PersistentSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    Fixture { _tmp: tmp, config, fs: Arc::new(FileStore::open_in_memory().unwrap()) }
}

fn controller_with(fs: &Arc<FileStore>) -> PersistentSubprocessController {
    PersistentSubprocessController::new(
        "tab".to_string(),
        "block".to_string(),
        None,
        None,
        None,
        Some(fs.clone()),
    )
}

fn write_pane_transcript(fs: &FileStore, data: &[u8]) {
    fs.make_file("block", PERSISTENT_OUTPUT_SUBJECT, FileMeta::new(), FileOpts::default())
        .unwrap();
    fs.write_file("block", PERSISTENT_OUTPUT_SUBJECT, data).unwrap();
}

/// The 2026-09-23 incident: the pane shows the prior conversation, the spawn
/// holds no id, and the transcript is reachable under this config dir.
#[test]
fn a_pane_showing_reachable_history_continues_that_session() {
    let f = fixture(true);
    write_pane_transcript(&f.fs, &stream(&[&format!(r#"{{"type":"result","session_id":"{LIVE_SID}"}}"#)]));
    let c = controller_with(&f.fs);
    assert_eq!(c.find_continuation_session_id(&f.config).as_deref(), Some(LIVE_SID));
}

/// `--resume` of an id the CLI can't find is rejected, so an unreachable id
/// must not be adopted (a later phase virtualizes this case instead).
#[test]
fn an_unreachable_session_is_not_adopted() {
    let f = fixture(false);
    write_pane_transcript(&f.fs, &stream(&[&format!(r#"{{"session_id":"{LIVE_SID}"}}"#)]));
    let c = controller_with(&f.fs);
    assert_eq!(c.find_continuation_session_id(&f.config), None);
}

#[test]
fn an_already_poisoned_session_is_not_adopted() {
    let f = fixture(true);
    write_pane_transcript(&f.fs, &stream(&[&format!(r#"{{"session_id":"{LIVE_SID}"}}"#)]));
    let c = controller_with(&f.fs);
    c.inner.lock().unwrap().resume_poisoned = Some(LIVE_SID.to_string());
    assert_eq!(c.find_continuation_session_id(&f.config), None);
}

#[test]
fn a_pane_with_no_prior_transcript_starts_fresh() {
    let f = fixture(true);
    let c = controller_with(&f.fs);
    assert_eq!(c.find_continuation_session_id(&f.config), None);
}

#[test]
fn a_provider_without_resume_is_never_continued() {
    let mut f = fixture(true);
    f.config.resume_flag = String::new();
    write_pane_transcript(&f.fs, &stream(&[&format!(r#"{{"session_id":"{LIVE_SID}"}}"#)]));
    let c = controller_with(&f.fs);
    assert_eq!(c.find_continuation_session_id(&f.config), None);
}

#[test]
fn no_config_dir_means_nothing_to_check_reachability_against() {
    let mut f = fixture(true);
    f.config.env_vars.clear();
    write_pane_transcript(&f.fs, &stream(&[&format!(r#"{{"session_id":"{LIVE_SID}"}}"#)]));
    let c = controller_with(&f.fs);
    assert_eq!(c.find_continuation_session_id(&f.config), None);
}

// ── the continuation packet's source ────────────────────────────────────

const AGENT_ZONE: &str = "agent:d76da857:current";

/// What a new channel's pane file holds when a rejected `--resume` settles:
/// AgentMux's own disclosure line and the CLI's error result. No turns.
fn settled_rejected_resume() -> [&'static str; 2] {
    [
        r#"{"type":"system","subtype":"agentmux_session_outcome","outcome":"fresh","continued":true,"attempted_sid":"49cc350c","actual_sid":null}"#,
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"49cc350c"}"#,
    ]
}

fn write_agent_zone(fs: &FileStore, data: &[u8]) {
    let name = crate::backend::agent_session::OUTPUT_FILE;
    fs.make_file(AGENT_ZONE, name, FileMeta::new(), FileOpts::default()).unwrap();
    fs.write_file(AGENT_ZONE, name, data).unwrap();
}

fn packet_from(local: &FileStore, global: Option<(&FileStore, &str)>) -> Option<String> {
    let (tail, mid) = super::super::resume_retry::packet_history_tail(
        Some(local),
        "block",
        global,
        crate::backend::continuity::PACKET_TAIL_BYTES,
    )?;
    crate::backend::continuity::build_continuation_packet(&tail, mid, None)
}

/// The 2026-09-25 incident (REPORT_AGENT_HISTORY_LOST_ON_NEW_BUILD_2026_09_25.md
/// §4): on a new channel the pane file is created by the settlement's own
/// lines, which must not hide the agent's record of the conversation.
#[test]
fn a_new_channels_pane_file_does_not_hide_the_agents_record() {
    let local = FileStore::open_in_memory().unwrap();
    write_pane_transcript(&local, &stream(&settled_rejected_resume()));
    let global = FileStore::open_in_memory().unwrap();
    let [outcome, error] = settled_rejected_resume();
    let history = [
        r#"{"type":"user","message":{"role":"user","content":"make the history pane open at the latest turn"}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"On it."}]}}"#,
        outcome,
        error,
    ];
    write_agent_zone(&global, &stream(&history));

    let packet = packet_from(&local, Some((&global, AGENT_ZONE))).expect("the agent's record is carried");
    assert!(packet.contains("make the history pane open at the latest turn"), "{packet}");
}

/// A first message already echoed into the new pane file is still not the
/// conversation: the record must come from the agent's zone, not from it.
#[test]
fn a_lone_new_message_in_the_pane_file_does_not_replace_the_agents_record() {
    let local = FileStore::open_in_memory().unwrap();
    let [outcome, error] = settled_rejected_resume();
    let echoed = r#"{"type":"user","message":{"role":"user","content":"u there"}}"#;
    write_pane_transcript(&local, &stream(&[outcome, error, echoed]));
    let global = FileStore::open_in_memory().unwrap();
    let prior = r#"{"type":"user","message":{"role":"user","content":"earlier request"}}"#;
    write_agent_zone(&global, &stream(&[prior, outcome, error, echoed]));

    let packet = packet_from(&local, Some((&global, AGENT_ZONE))).expect("a packet");
    assert!(packet.contains("earlier request"), "{packet}");
}

/// A pane not anchored to an agent has no zone; its own transcript is the
/// whole record.
#[test]
fn without_an_agent_zone_the_packet_reads_the_panes_own_transcript() {
    let local = FileStore::open_in_memory().unwrap();
    let ask = r#"{"type":"user","message":{"role":"user","content":"pane-only request"}}"#;
    write_pane_transcript(&local, &stream(&[ask]));
    let packet = packet_from(&local, None).expect("a packet");
    assert!(packet.contains("pane-only request"), "{packet}");
}

#[test]
fn an_empty_agent_zone_falls_back_to_the_panes_own_transcript() {
    let local = FileStore::open_in_memory().unwrap();
    let ask = r#"{"type":"user","message":{"role":"user","content":"pane-only request"}}"#;
    write_pane_transcript(&local, &stream(&[ask]));
    let global = FileStore::open_in_memory().unwrap();
    let packet = packet_from(&local, Some((&global, AGENT_ZONE))).expect("a packet");
    assert!(packet.contains("pane-only request"), "{packet}");
}

/// Only the tail is read, so a long transcript whose id appears early and
/// then again near the end still resolves.
#[test]
fn a_transcript_longer_than_the_tail_window_still_resolves() {
    let f = fixture(true);
    let filler = format!("{{\"type\":\"assistant\",\"text\":\"{}\"}}", "x".repeat(4096));
    let mut lines: Vec<String> = vec![r#"{"session_id":"early-sid"}"#.to_string()];
    lines.extend(std::iter::repeat(filler).take((CONTINUATION_TAIL_BYTES as usize / 4096) + 8));
    lines.push(format!(r#"{{"type":"result","session_id":"{LIVE_SID}"}}"#));
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    write_pane_transcript(&f.fs, &stream(&refs));
    let c = controller_with(&f.fs);
    assert_eq!(c.find_continuation_session_id(&f.config).as_deref(), Some(LIVE_SID));
}

// ── which conversations the packet may draw on ──────────────────────────

/// A block in an in-memory store, anchored to an agent unless `agent_id` is
/// `None`, archived when `archived_at` is set.
fn store_with_block(agent_id: Option<&str>, archived_at: Option<i64>) -> crate::backend::storage::store::Store {
    let mstore = crate::backend::storage::store::Store::open_in_memory().unwrap();
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("agent"));
    if let Some(id) = agent_id {
        meta.insert("agentId".to_string(), serde_json::json!(id));
    }
    if let Some(at) = archived_at {
        meta.insert(crate::backend::session_archive::META_SESSION_ARCHIVED_AT.to_string(), serde_json::json!(at));
    }
    let mut block = crate::backend::obj::Block {
        oid: "block".to_string(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta,
        subblockids: None,
    };
    mstore.insert(&mut block).expect("insert block");
    mstore
}

/// The zone a pane draws its history from is the agent's, unless the pane is
/// not anchored to an agent or its conversation was archived.
#[test]
fn a_pane_draws_on_its_agents_zone_unless_it_is_unanchored_or_archived() {
    use super::super::resume_retry::global_prior_zone;
    let anchored = store_with_block(Some("d76da857"), None);
    assert_eq!(global_prior_zone(Some(&anchored), "block").as_deref(), Some(AGENT_ZONE));
    assert_eq!(global_prior_zone(Some(&store_with_block(None, None)), "block"), None);
    assert_eq!(global_prior_zone(Some(&store_with_block(Some("d76da857"), Some(1_790_000_000_000))), "block"), None);
    assert_eq!(global_prior_zone(None, "block"), None);
}

/// Every explicit "start fresh" archives the conversation. The record still
/// holds it (the zone is not cleared for other channels), so the archive
/// flag is the only thing keeping a conversation the user walked away from
/// out of the next session's first message. The record is now the packet's
/// primary source, so this is the rule to pin.
#[test]
fn an_archived_conversation_is_not_carried_into_a_fresh_session() {
    use super::super::resume_retry::global_prior_zone;
    let local = FileStore::open_in_memory().unwrap();
    write_pane_transcript(&local, &stream(&settled_rejected_resume()));
    let global = FileStore::open_in_memory().unwrap();
    write_agent_zone(&global, &stream(&[r#"{"type":"user","message":{"role":"user","content":"a conversation the user archived"}}"#]));

    let live = store_with_block(Some("d76da857"), None);
    let zone = global_prior_zone(Some(&live), "block");
    let packet = packet_from(&local, zone.as_deref().map(|z| (&global, z))).expect("the live conversation is carried");
    assert!(packet.contains("a conversation the user archived"), "{packet}");

    let archived = store_with_block(Some("d76da857"), Some(1_790_000_000_000));
    let zone = global_prior_zone(Some(&archived), "block");
    assert_eq!(packet_from(&local, zone.as_deref().map(|z| (&global, z))), None);
}

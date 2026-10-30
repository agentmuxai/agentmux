// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::continuity_segments::{self as ledger, EndReason, Rung, Start};
use crate::backend::reactive::types::CliNativeAudit;

fn audit_entry(ts: u64, from: &str, to: &str) -> AuditLogEntry {
    AuditLogEntry {
        timestamp: ts,
        source_agent: (!from.is_empty()).then(|| from.to_string()),
        target_agent: to.to_string(),
        block_id: format!("block-{to}"),
        message_hash: "00ff00ff00ff00ff".into(),
        message_length: 400,
        success: true,
        error_message: None,
        request_id: format!("r{ts}"),
        outcome: None,
        reason: None,
        event_kind: "delivery".into(),
        evicted_block: None,
        evicted_agent: None,
        audit_source_uid: String::new(),
        lan_claimed_uid: String::new(),
        wan: None,
        cli_native: None,
        turn: None,
    }
}

fn write(dir: &Path, entries: &[AuditLogEntry]) -> std::path::PathBuf {
    let path = dir.join("jekt-audit.jsonl");
    let text: String = entries.iter().map(|e| format!("{}\n", serde_json::to_string(e).unwrap())).collect();
    std::fs::write(&path, text).unwrap();
    path
}

fn start(uid: &str, at: i64, byte: i64) -> Start {
    Start {
        segment_id: String::new(),
        agent_uid: uid.into(),
        definition_id: Some(uid.into()),
        provider: "claude".into(),
        config_dir: None,
        provider_session_id: None,
        cwd: "/work".into(),
        channel: "dev".into(),
        agentmux_version: "0.0.0".into(),
        block_id: format!("block-{uid}"),
        zone: Some(format!("agent:{uid}:current")),
        byte_start: Some(byte),
        started_at_ms: at,
        continuity_rung: Rung::Fresh,
        predecessor_segment_id: None,
        lease_epoch: None,
        identity_key: None,
        forked_from: None,
    }
}

struct World {
    _dir: tempfile::TempDir,
    jekts: std::path::PathBuf,
    store: FileStore,
}

impl World {
    fn new(entries: &[AuditLogEntry]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let jekts = write(dir.path(), entries);
        Self { _dir: dir, jekts, store: FileStore::open_in_memory().unwrap() }
    }

    fn run(&self, req: AuditQueryReq) -> Result<AuditQueryResult, String> {
        let slug = |uid: &str| Some(format!("agent-{uid}"));
        let pane = |slug: &str| (slug == "korp").then(|| "live-korp".to_string());
        let exists = |b: &str| b == "block-Maricon-2";
        let src = Sources { jekt_file: Some(&self.jekts), segments: Some(&self.store), ctx: Ctx { slug_of_uid: &slug, pane_of_agent: &pane, block_exists: &exists } };
        query(&src, &req)
    }

    fn all(&self, req: AuditQueryReq) -> Vec<AuditEntry> {
        self.run(AuditQueryReq { limit: Some(200), ..req }).unwrap().entries
    }
}

fn ids(entries: &[AuditEntry]) -> Vec<String> {
    entries.iter().map(|e| e.id.clone()).collect()
}

fn many() -> Vec<AuditLogEntry> {
    (0..120).map(|n| audit_entry(1_000 + n, if n % 3 == 0 { "Korp" } else { "Loap" }, if n % 2 == 0 { "Maricon-2" } else { "Opaz" })).collect()
}

#[test]
fn pages_follow_each_other_to_the_end_without_repeating_or_skipping() {
    let world = World::new(&many());
    let whole = world.all(AuditQueryReq::default());
    assert_eq!(whole.len(), 120);
    assert!(whole.windows(2).all(|w| w[0].ts_ms >= w[1].ts_ms), "newest first");

    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = world.run(AuditQueryReq { limit: Some(50), cursor: cursor.clone(), ..Default::default() }).unwrap();
        seen.extend(ids(&page.entries));
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen, ids(&whole));
}

#[test]
fn a_jekt_has_both_ends_with_their_own_links_and_no_agent_of_its_own() {
    let world = World::new(&[audit_entry(5_000, "Korp", "Maricon-2")]);
    let entry = world.all(AuditQueryReq::default()).remove(0);
    assert_eq!(entry.kind, "jekt");
    assert_eq!(entry.agent_id, None);
    let (from, to) = (entry.from.unwrap(), entry.to.unwrap());
    assert_eq!((from.agent_id.as_str(), to.agent_id.as_str()), ("korp", "maricon-2"));
    assert_eq!(from.links.pane.unwrap().block_id, "live-korp", "the sender's pane is the one it has open now");
    assert_eq!(from.links.history.unwrap().at_ms, 5_000);
    assert_eq!(to.links.pane.unwrap().block_id, "block-Maricon-2", "the receiver's pane is the block it was delivered to, while it exists");
    assert_eq!(entry.cost.unwrap().input, 100, "400 bytes of injected text, estimated");
}

#[test]
fn the_agent_filter_matches_either_end_of_a_jekt_however_it_is_written() {
    let world = World::new(&many());
    let got = world.all(AuditQueryReq { agent_id: Some("OPAZ".into()), ..Default::default() });
    assert_eq!(got.len(), 60);
    let got = world.all(AuditQueryReq { agent_id: Some("korp".into()), ..Default::default() });
    assert_eq!(got.len(), 40, "Korp sent every third");
    assert!(world.all(AuditQueryReq { agent_id: Some("nobody".into()), ..Default::default() }).is_empty());
}

#[test]
fn time_range_text_and_kind_narrow_the_list() {
    let mut entries = many();
    entries[10].success = false;
    entries[10].error_message = Some("pane gone".into());
    entries[20].event_kind = "register".into();
    let world = World::new(&entries);
    let ranged = world.all(AuditQueryReq { since_ms: Some(1_100), until_ms: Some(1_104), ..Default::default() });
    assert_eq!(ranged.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), [1_104, 1_103, 1_102, 1_101, 1_100]);
    let failed = world.all(AuditQueryReq { text: Some("PANE GONE".into()), ..Default::default() });
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].ts_ms, 1_010);
    let registry = world.all(AuditQueryReq { kinds: Some(vec!["registry".into()]), ..Default::default() });
    assert_eq!(registry.len(), 1);
    assert_eq!(registry[0].agent_id.as_deref(), Some("maricon-2"));
    assert_eq!(world.all(AuditQueryReq { kinds: Some(vec!["jekt".into()]), ..Default::default() }).len(), 119);
}

#[test]
fn agent_starts_and_ends_merge_in_by_time_with_their_byte_and_reason() {
    let world = World::new(&[audit_entry(1_000, "Korp", "Opaz"), audit_entry(3_000, "Korp", "Opaz")]);
    let first = ledger::record_start(&world.store, start("a1", 500, 0)).unwrap();
    ledger::record_end(&world.store, "a1", &first, EndReason::Crashed, Some(4_096), 2_000).unwrap();
    ledger::record_start(&world.store, start("a1", 2_500, 4_096)).unwrap();

    let got = world.all(AuditQueryReq::default());
    assert_eq!(got.iter().map(|e| (e.ts_ms, e.kind.as_str())).collect::<Vec<_>>(), [(3_000, "jekt"), (2_500, "agent_start"), (2_000, "agent_end"), (1_000, "jekt"), (500, "agent_start")]);
    let end = &got[2];
    assert_eq!((end.agent_id.as_deref(), end.reason.as_deref()), (Some("agent-a1"), Some("crashed")));
    let at = end.links.history.as_ref().unwrap();
    assert_eq!((at.at_byte, at.zone.as_deref(), at.at_ms), (Some(4_096), Some("agent:a1:current"), 2_000));
    assert!(end.links.pane.is_none(), "its pane is gone");

    // A page boundary between the sources loses and repeats nothing.
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = world.run(AuditQueryReq { limit: Some(2), cursor, ..Default::default() }).unwrap();
        seen.extend(ids(&page.entries));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, ids(&got));

    let only_ends = world.all(AuditQueryReq { kinds: Some(vec!["agent_end".into()]), agent_id: Some("agent-a1".into()), ..Default::default() });
    assert_eq!(only_ends.len(), 1);
}

#[test]
fn tool_calls_and_thoughts_are_said_not_to_be_recorded() {
    let world = World::new(&many());
    let got = world.run(AuditQueryReq { kinds: Some(vec!["tool".into(), "thought".into()]), ..Default::default() }).unwrap();
    assert!(got.entries.is_empty());
    assert_eq!(got.not_recorded, ["tool", "thought"]);
    assert!(world.run(AuditQueryReq::default()).unwrap().not_recorded.is_empty());
}

#[test]
fn no_entry_holds_any_message_text() {
    let mut cli = audit_entry(9_000, "", "Opaz");
    cli.event_kind = "cli_native".into();
    cli.cli_native = Some(CliNativeAudit {
        seen_by: "receiver".into(),
        identity: "agent".into(),
        claimed_name: None,
        cli_to_name: None,
        sender_pid: None,
        from_mode: None,
        preview: "SECRET-PREVIEW-TEXT".into(),
    });
    let world = World::new(&[cli]);
    let json = serde_json::to_string(&world.all(AuditQueryReq::default())).unwrap();
    assert!(!json.contains("SECRET-PREVIEW-TEXT") && !json.contains("00ff00ff"), "{json}");
    assert!(json.contains("CLI message"));
}

#[test]
fn a_bad_cursor_or_kind_is_refused_and_a_missing_file_is_an_empty_audit() {
    let world = World::new(&[]);
    assert!(world.run(AuditQueryReq { cursor: Some("garbage".into()), ..Default::default() }).is_err());
    assert!(world.run(AuditQueryReq { kinds: Some(vec!["jekts".into()]), ..Default::default() }).unwrap_err().contains("unknown kind"));
    let slug = |_: &str| None;
    let none = |_: &str| None;
    let exists = |_: &str| false;
    let src = Sources { jekt_file: Some(Path::new("/nonexistent/jekt-audit.jsonl")), segments: None, ctx: Ctx { slug_of_uid: &slug, pane_of_agent: &none, block_exists: &exists } };
    assert!(query(&src, &AuditQueryReq::default()).unwrap().entries.is_empty());
}

fn action(ts: u64, actor: &str, command: &str, target: &str) -> AuditLogEntry {
    AuditLogEntry { event_kind: "action".into(), outcome: Some(command.into()), ..audit_entry(ts, actor, target) }
}

#[test]
fn an_action_names_its_actor_only_when_the_actor_is_an_agent() {
    let world = World::new(&[
        action(100, "agent:Korp", "agent.stop", "Opaz"),
        action(200, "operator", "agent.stop", "Opaz"),
        action(300, "unproven", "agent.stop", ""),
        action(400, "", "agent.stop", "Opaz"),
    ]);
    let got = world.all(AuditQueryReq::default());
    assert!(got.iter().all(|e| e.kind == "action"));
    let by_ts = |ts: u64| got.iter().find(|e| e.ts_ms == ts).unwrap();
    let korp = by_ts(100);
    assert_eq!(korp.agent_id.as_deref(), Some("korp"));
    assert_eq!(korp.links.history.as_ref().unwrap().agent_id, "korp");
    assert_eq!(korp.to.as_ref().unwrap().agent_id, "opaz");
    for ts in [200, 300, 400] {
        let e = by_ts(ts);
        assert_eq!((e.agent_id.as_deref(), e.links.clone()), (None, AuditLinks::default()), "{}", e.summary);
    }
    assert!(by_ts(300).to.is_none(), "no target, no link to it");
    assert!(by_ts(200).summary.starts_with("operator agent.stop on opaz"));
    // Korp's own actions are found under its slug; the operator's are not any agent's.
    let mine = world.all(AuditQueryReq { agent_id: Some("korp".into()), ..Default::default() });
    assert_eq!(mine.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), [100]);
    assert_eq!(world.all(AuditQueryReq { agent_id: Some("operator".into()), ..Default::default() }).len(), 0);
}

#[test]
fn a_file_out_of_time_order_neither_repeats_nor_loses_entries_across_pages() {
    // The newest line of the file is not the newest entry: a later write carries an earlier time.
    let world = World::new(&[audit_entry(110, "Korp", "Opaz"), audit_entry(100, "Korp", "Opaz"), audit_entry(90, "Korp", "Opaz")]);
    let l = ledger::record_start(&world.store, start("a1", 105, 0)).unwrap();
    ledger::record_end(&world.store, "a1", &l, EndReason::Closed, None, 95).unwrap();
    let all = world.all(AuditQueryReq::default());
    assert_eq!(all.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), [110, 105, 100, 95, 90], "newest first however the file is ordered");
    let whole = ids(&all);
    for limit in 1..=4u32 {
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = world.run(AuditQueryReq { limit: Some(limit), cursor, ..Default::default() }).unwrap();
            seen.extend(ids(&page.entries));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, whole, "limit {limit}");
    }
}

#[test]
fn many_lines_a_few_seconds_out_of_order_still_page_newest_first() {
    // Several srvs append, each a little behind or ahead of the others.
    let entries: Vec<AuditLogEntry> = (0..200u64).map(|n| audit_entry(1_000_000 + n * 1_000 + (n * 7919) % 9_000, "Korp", "Opaz")).collect();
    let world = World::new(&entries);
    let mut want: Vec<u64> = entries.iter().map(|e| e.timestamp).collect();
    want.sort_by(|a, b| b.cmp(a));
    assert_eq!(world.all(AuditQueryReq::default()).iter().map(|e| e.ts_ms).collect::<Vec<_>>(), want);
    for limit in [1u32, 7, 33] {
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = world.run(AuditQueryReq { limit: Some(limit), cursor, ..Default::default() }).unwrap();
            seen.extend(page.entries.iter().map(|e| e.ts_ms));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, want, "limit {limit}");
    }
    // A range reads back only as far as it needs, and still has all of it.
    let range = world.all(AuditQueryReq { since_ms: Some(1_100_000), until_ms: Some(1_110_000), ..Default::default() });
    assert_eq!(range.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), want.iter().copied().filter(|t| (1_100_000..=1_110_000).contains(t)).collect::<Vec<_>>());
}

#[test]
fn a_start_or_end_points_at_the_zone_its_bytes_are_in_after_new_conversation_archived_them() {
    use crate::backend::storage::filestore::{FileMeta, FileOpts};
    let world = World::new(&[]);
    // The conversation was archived at 1000, and again at 3000.
    for zone in ["agent:a1:archive:1000", "agent:a1:archive:3000", "agent:a1:current"] {
        world.store.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
    }
    let one = ledger::record_start(&world.store, start("a1", 500, 0)).unwrap();
    ledger::record_end(&world.store, "a1", &one, EndReason::Closed, Some(40), 900).unwrap();
    let two = ledger::record_start(&world.store, start("a1", 2_000, 0)).unwrap();
    ledger::record_end(&world.store, "a1", &two, EndReason::Closed, Some(70), 2_500).unwrap();
    ledger::record_start(&world.store, start("a1", 3_500, 0)).unwrap();

    let zones: Vec<(u64, String)> = world
        .all(AuditQueryReq::default())
        .iter()
        .map(|e| (e.ts_ms, e.links.history.as_ref().unwrap().zone.clone().unwrap()))
        .collect();
    assert_eq!(
        zones,
        [
            (3_500, "agent:a1:current".to_string()),
            (2_500, "agent:a1:archive:3000".to_string()),
            (2_000, "agent:a1:archive:3000".to_string()),
            (900, "agent:a1:archive:1000".to_string()),
            (500, "agent:a1:archive:1000".to_string()),
        ]
    );
}

#[test]
fn totals_cover_everything_the_filter_matches_not_the_page_and_only_when_asked() {
    let world = World::new(&many());
    assert!(world.run(AuditQueryReq::default()).unwrap().totals.is_none());
    let page = world.run(AuditQueryReq { limit: Some(5), totals: Some(true), ..Default::default() }).unwrap();
    assert_eq!(page.entries.len(), 5);
    let all = page.totals.unwrap();
    assert_eq!((all.entries, all.input, all.cache_read, all.output, all.usd, all.estimated), (120, 12_000, 0, 0, None, true));
    let opaz = world.run(AuditQueryReq { agent_id: Some("opaz".into()), totals: Some(true), ..Default::default() }).unwrap().totals.unwrap();
    assert_eq!((opaz.entries, opaz.input), (60, 6_000));
    let range = world.run(AuditQueryReq { since_ms: Some(1_100), totals: Some(true), ..Default::default() }).unwrap().totals.unwrap();
    assert_eq!(range.entries, 20);
}

#[test]
fn totals_carry_on_from_where_the_last_scan_stopped_and_start_again_for_a_new_file() {
    let world = World::new(&many());
    let totals = |world: &World| world.run(AuditQueryReq { totals: Some(true), ..Default::default() }).unwrap().totals.unwrap();
    assert_eq!(totals(&world).entries, 120);
    // More is appended, and a line the writer hasn't finished isn't counted yet.
    let mut text = std::fs::read_to_string(&world.jekts).unwrap();
    for e in [audit_entry(2_000, "Korp", "Opaz"), audit_entry(2_001, "Korp", "Opaz")] {
        text.push_str(&serde_json::to_string(&e).unwrap());
        text.push('\n');
    }
    text.push_str("{\"timestamp\":2002");
    std::fs::write(&world.jekts, &text).unwrap();
    assert_eq!(totals(&world).entries, 122);
    assert_eq!(totals(&world).input, 12_200);
    // The file replaced by a shorter one is read from its start.
    std::fs::write(&world.jekts, format!("{}\n", serde_json::to_string(&audit_entry(5, "Korp", "Opaz")).unwrap())).unwrap();
    assert_eq!(totals(&world).entries, 1);
}

#[test]
fn totals_count_a_started_agent_but_cost_it_nothing() {
    let world = World::new(&[audit_entry(1_000, "Korp", "Opaz")]);
    ledger::record_start(&world.store, start("a1", 500, 0)).unwrap();
    let got = world.run(AuditQueryReq { totals: Some(true), ..Default::default() }).unwrap().totals.unwrap();
    assert_eq!((got.entries, got.input), (2, 100));
}

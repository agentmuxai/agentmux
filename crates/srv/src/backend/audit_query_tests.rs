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
        host: None,
        channel: None,
        version: None,
        tier: None,
        trust: None,
        msg_id: None,
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
        let uid = |slug: &str| Some(format!("uid-{slug}"));
        let pane = |slug: &str| (slug == "korp").then(|| "live-korp".to_string());
        let exists = |b: &str| b == "block-Maricon-2";
        let src = Sources { own_file: None, activity_dir: None, own_activity_dir: None, jekt_file: Some(&self.jekts), segments: Some(&self.store), ctx: Ctx { slug_of_uid: &slug, uid_of_slug: &uid, pane_of_agent: &pane, block_exists: &exists } };
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
    let sender_history = from.links.history.unwrap();
    assert_eq!(sender_history.at_ms, 5_000);
    assert_eq!(sender_history.definition_id.as_deref(), Some("uid-korp"), "the History reader opens an agent by its UID, not its slug");
    assert_eq!(to.links.pane.unwrap().block_id, "block-Maricon-2", "the receiver's pane is the block it was delivered to, while it exists");
    assert_eq!(entry.cost.unwrap().input, 100, "400 bytes of injected text, estimated");
}

#[test]
fn a_name_the_registry_does_not_know_gets_no_history_button() {
    let world = World::new(&[audit_entry(5_000, "Korp", "Maricon-2")]);
    let slug = |uid: &str| Some(format!("agent-{uid}"));
    let uid = |slug: &str| (slug == "korp").then(|| "uid-korp".to_string());
    let pane = |_: &str| None;
    let exists = |_: &str| false;
    let src = Sources { own_file: None, activity_dir: None, own_activity_dir: None, jekt_file: Some(&world.jekts), segments: Some(&world.store), ctx: Ctx { slug_of_uid: &slug, uid_of_slug: &uid, pane_of_agent: &pane, block_exists: &exists } };
    let entry = query(&src, &AuditQueryReq::default()).unwrap().entries.remove(0);
    assert!(entry.from.unwrap().links.history.is_some());
    assert!(entry.to.unwrap().links.history.is_none(), "the reader opens an agent by UID: with none there is nothing to open");
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
fn tool_turn_and_thought_rows_are_asked_for_by_kind_and_none_are_not_an_error() {
    // No activity files here: asking for the kinds answers an empty list, and nothing is "not recorded" any more.
    let world = World::new(&many());
    let got = world.run(AuditQueryReq { kinds: Some(vec!["tool".into(), "turn".into(), "thought".into()]), ..Default::default() }).unwrap();
    assert!(got.entries.is_empty());
    assert!(got.not_recorded.is_empty());
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
    let src = Sources { own_file: None, activity_dir: None, own_activity_dir: None, jekt_file: Some(Path::new("/nonexistent/jekt-audit.jsonl")), segments: None, ctx: Ctx { slug_of_uid: &slug, uid_of_slug: &none, pane_of_agent: &none, block_exists: &exists } };
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


/// Two audit files, as a srv that kept its rows in its own file while it did not use the shared folder has: the
/// shared file (`j` ids) and its own (`o` ids). Either may be missing.
struct TwoFiles {
    _dir: tempfile::TempDir,
    shared: Option<std::path::PathBuf>,
    own: Option<std::path::PathBuf>,
}

impl TwoFiles {
    fn new(shared: Option<&[AuditLogEntry]>, own: Option<&[AuditLogEntry]>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let in_dir = |name: &str, entries: &[AuditLogEntry]| {
            let sub = dir.path().join(name);
            std::fs::create_dir_all(&sub).unwrap();
            write(&sub, entries)
        };
        let (shared, own) = (shared.map(|e| in_dir("shared", e)), own.map(|e| in_dir("own", e)));
        Self { _dir: dir, shared, own }
    }

    fn run(&self, req: AuditQueryReq) -> Result<AuditQueryResult, String> {
        let slug = |uid: &str| Some(format!("agent-{uid}"));
        let uid = |slug: &str| Some(format!("uid-{slug}"));
        let none = |_: &str| None;
        let exists = |_: &str| false;
        let src = Sources {
            jekt_file: self.shared.as_deref(),
            own_file: self.own.as_deref(),
            activity_dir: None,
            own_activity_dir: None,
            segments: None,
            ctx: Ctx { slug_of_uid: &slug, uid_of_slug: &uid, pane_of_agent: &none, block_exists: &exists },
        };
        query(&src, &req)
    }
}

#[test]
fn a_srv_without_the_shared_folder_lists_the_rows_it_kept_in_its_own_file() {
    // The shared file is not there (or not readable) and the srv has appended to its own.
    let two = TwoFiles::new(None, Some(&[audit_entry(1_000, "Korp", "Opaz"), audit_entry(2_000, "Opaz", "Korp")]));
    let got = two.run(AuditQueryReq::default()).unwrap().entries;
    assert_eq!(got.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), vec![2_000, 1_000]);
    assert!(got.iter().all(|e| e.id.starts_with('o')), "its own file's ids say so: {:?}", ids(&got));
}

#[test]
fn the_shared_file_and_the_srvs_own_are_merged_newest_first() {
    let two = TwoFiles::new(
        Some(&[audit_entry(1_000, "Korp", "Opaz"), audit_entry(3_000, "Korp", "Opaz")]),
        Some(&[audit_entry(2_000, "Opaz", "Korp"), audit_entry(4_000, "Opaz", "Korp")]),
    );
    let got = two.run(AuditQueryReq::default()).unwrap().entries;
    assert_eq!(got.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), vec![4_000, 3_000, 2_000, 1_000]);
    let from: Vec<char> = got.iter().map(|e| e.id.chars().next().unwrap()).collect();
    assert_eq!(from, vec!['o', 'j', 'o', 'j']);
}

#[test]
fn the_same_byte_offset_in_both_files_is_two_entries_not_one() {
    // The first line of each file starts at byte 0, so only the prefix tells them apart.
    let two = TwoFiles::new(Some(&[audit_entry(1_000, "Korp", "Opaz")]), Some(&[audit_entry(1_000, "Korp", "Opaz")]));
    let got = two.run(AuditQueryReq::default()).unwrap().entries;
    assert_eq!(got.len(), 2);
    assert_ne!(got[0].id, got[1].id);
}

#[test]
fn pages_follow_each_other_across_both_files_without_repeating_or_skipping() {
    let shared: Vec<AuditLogEntry> = (0..60).map(|n| audit_entry(1_000 + 2 * n, "Korp", "Opaz")).collect();
    let own: Vec<AuditLogEntry> = (0..60).map(|n| audit_entry(1_001 + 2 * n, "Opaz", "Korp")).collect();
    let two = TwoFiles::new(Some(&shared), Some(&own));
    let whole = two.run(AuditQueryReq { limit: Some(200), ..Default::default() }).unwrap().entries;
    assert_eq!(whole.len(), 120);
    assert!(whole.windows(2).all(|w| w[0].ts_ms > w[1].ts_ms), "newest first");

    let (mut seen, mut cursor) = (Vec::new(), None);
    loop {
        let page = two.run(AuditQueryReq { limit: Some(25), cursor: cursor.clone(), ..Default::default() }).unwrap();
        seen.extend(ids(&page.entries));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, ids(&whole));
}

#[test]
fn a_cursor_from_before_the_srvs_own_file_still_reads() {
    // The version 3 cursor had no position for it.
    let two = TwoFiles::new(Some(&[audit_entry(1_000, "Korp", "Opaz")]), None);
    let got = two.run(AuditQueryReq { cursor: Some("3|-||-||-".to_string()), ..Default::default() }).unwrap();
    assert_eq!(got.entries.len(), 1);
    assert!(two.run(AuditQueryReq { cursor: Some("9|-".to_string()), ..Default::default() }).is_err());
}

#[test]
fn totals_add_the_two_files() {
    let two = TwoFiles::new(Some(&[audit_entry(1_000, "Korp", "Opaz")]), Some(&[audit_entry(2_000, "Opaz", "Korp")]));
    let got = two.run(AuditQueryReq { totals: Some(true), ..Default::default() }).unwrap().totals.unwrap();
    assert_eq!((got.entries, got.input), (2, 200));
}

// ── activity rows (tool, turn, thought)

use crate::backend::audit_activity::ActivityRow;

fn activity_row(ts: u64, kind: &str, summary: &str) -> ActivityRow {
    ActivityRow {
        ts,
        kind: kind.into(),
        agent: "u1".into(),
        block: "block-Maricon-2".into(),
        zone: "agent:u1:current".into(),
        byte: Some(ts),
        summary: summary.into(),
        tool: (kind == "tool").then(|| "Bash".to_string()),
        outcome: None,
        ms: None,
        cost: None,
        channel: Some("dev".into()),
    }
}

/// A folder of activity day files, and optionally a jekt audit file beside them.
struct Activity {
    _dir: tempfile::TempDir,
    activity_dir: std::path::PathBuf,
    jekts: Option<std::path::PathBuf>,
}

impl Activity {
    fn new(rows: &[ActivityRow], jekts: Option<&[AuditLogEntry]>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let activity_dir = dir.path().join("audit");
        std::fs::create_dir_all(&activity_dir).unwrap();
        let mut days: std::collections::BTreeMap<String, String> = Default::default();
        for r in rows {
            let day = chrono::DateTime::from_timestamp_millis(r.ts as i64).unwrap().format("%Y-%m-%d").to_string();
            days.entry(day).or_default().push_str(&format!("{}\n", serde_json::to_string(r).unwrap()));
        }
        for (day, text) in days {
            std::fs::write(activity_dir.join(format!("activity-{day}.jsonl")), text).unwrap();
        }
        let jekts = jekts.map(|e| write(&activity_dir, e));
        Self { _dir: dir, activity_dir, jekts }
    }

    fn run(&self, req: AuditQueryReq) -> Result<AuditQueryResult, String> {
        let slug = |uid: &str| (uid == "u1").then(|| "maricon-2".to_string());
        let uid = |slug: &str| Some(format!("uid-{slug}"));
        let none = |_: &str| None;
        let exists = |b: &str| b == "block-Maricon-2";
        let src = Sources {
            jekt_file: self.jekts.as_deref(),
            own_file: None,
            activity_dir: Some(&self.activity_dir),
            own_activity_dir: None,
            segments: None,
            ctx: Ctx { slug_of_uid: &slug, uid_of_slug: &uid, pane_of_agent: &none, block_exists: &exists },
        };
        query(&src, &req)
    }
}

const DAY_MS: u64 = 86_400_000;
/// 2023-11-14 22:13:20 UTC: a few seconds on is still that day, and a day on is the next.
const T0: u64 = 1_700_000_000_000;

fn kinds(k: &[&str]) -> Option<Vec<String>> {
    Some(k.iter().map(|s| s.to_string()).collect())
}

#[test]
fn activity_rows_come_only_when_asked_for_by_kind_and_carry_their_links() {
    let a = Activity::new(&[activity_row(T0, "tool", "Bash cargo test"), activity_row(T0 + 1_000, "turn", "turn ended (success)")], Some(&[audit_entry(T0 + 500, "Korp", "Opaz")]));
    let default = a.run(AuditQueryReq::default()).unwrap().entries;
    assert_eq!(default.len(), 1, "the default list is what it was: no tool or turn rows");
    assert_eq!(default[0].kind, "jekt");

    let got = a.run(AuditQueryReq { kinds: kinds(&["tool", "turn", "thought"]), ..Default::default() }).unwrap().entries;
    assert_eq!(got.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["turn", "tool"]);
    let tool = &got[1];
    assert_eq!((tool.agent_id.as_deref(), tool.summary.as_str()), (Some("maricon-2"), "Bash cargo test"));
    let history = tool.links.history.as_ref().unwrap();
    assert_eq!((history.definition_id.as_deref(), history.zone.as_deref(), history.at_byte, history.at_ms), (Some("u1"), Some("agent:u1:current"), Some(T0), T0));
    assert_eq!(tool.links.pane.as_ref().unwrap().block_id, "block-Maricon-2");
    assert!(tool.id.starts_with("as2023-11-14:"), "an activity id names its folder and day: {}", tool.id);
}

#[test]
fn activity_rows_merge_with_jekts_and_the_agent_and_text_filters_apply() {
    let a = Activity::new(
        &[activity_row(T0, "tool", "Bash git status"), activity_row(T0 + 2_000, "tool", "Read /a.rs")],
        Some(&[audit_entry(T0 + 1_000, "Korp", "Maricon-2")]),
    );
    let all = a.run(AuditQueryReq { kinds: kinds(&["tool", "jekt"]), ..Default::default() }).unwrap().entries;
    assert_eq!(all.iter().map(|e| e.ts_ms).collect::<Vec<_>>(), vec![T0 + 2_000, T0 + 1_000, T0]);
    let one = a.run(AuditQueryReq { kinds: kinds(&["tool"]), text: Some("GIT status".into()), ..Default::default() }).unwrap().entries;
    assert_eq!(one.len(), 1);
    assert_eq!(a.run(AuditQueryReq { kinds: kinds(&["tool"]), agent_id: Some("opaz".into()), ..Default::default() }).unwrap().entries.len(), 0);
    assert_eq!(a.run(AuditQueryReq { kinds: kinds(&["tool"]), agent_id: Some("Maricon-2".into()), ..Default::default() }).unwrap().entries.len(), 2);
}

#[test]
fn pages_follow_each_other_across_day_files_and_the_jekt_file_without_repeating_or_skipping() {
    // Three days of rows, jekts in between: a page boundary falls inside a day and between days.
    let rows: Vec<ActivityRow> = (0..90).map(|n| activity_row(T0 + (n % 3) * DAY_MS + (n / 3) * 10, "tool", &format!("Bash {n}"))).collect();
    let jekts: Vec<AuditLogEntry> = (0..20).map(|n| audit_entry(T0 + DAY_MS + n * 7, "Korp", "Opaz")).collect();
    let a = Activity::new(&rows, Some(&jekts));
    let req = |cursor: Option<String>, limit| AuditQueryReq { kinds: kinds(&["tool", "jekt"]), limit: Some(limit), cursor, ..Default::default() };
    let whole = a.run(req(None, 200)).unwrap().entries;
    assert_eq!(whole.len(), 110);
    assert!(whole.windows(2).all(|w| (w[0].ts_ms, &w[0].id) > (w[1].ts_ms, &w[1].id)), "newest first, no repeat");

    let (mut seen, mut cursor, mut pages) = (Vec::new(), None, 0);
    loop {
        let page = a.run(req(cursor.clone(), 17)).unwrap();
        seen.extend(ids(&page.entries));
        pages += 1;
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 7);
    assert_eq!(seen, ids(&whole));
}

#[test]
fn a_window_that_starts_after_a_day_does_not_read_it() {
    let a = Activity::new(&[activity_row(T0, "tool", "old"), activity_row(T0 + 2 * DAY_MS, "tool", "new")], None);
    let got = a.run(AuditQueryReq { kinds: kinds(&["tool"]), since_ms: Some(T0 + DAY_MS), ..Default::default() }).unwrap().entries;
    assert_eq!(got.iter().map(|e| e.summary.as_str()).collect::<Vec<_>>(), vec!["new"]);
}

#[test]
fn a_cursor_with_a_bad_activity_position_is_refused() {
    let a = Activity::new(&[], None);
    // Five stores, each a time, an id and a place: the last two are the activity folders', which name a day and a byte.
    let empty = ["-||-"; 3].join("|");
    let bad_ones = [format!("5|{empty}|-||-|-||x"), format!("5|{empty}|-||2026-11-05:notabyte|-||-"), "5|-||-".to_string(), "6|-||-".to_string()];
    for bad in bad_ones {
        assert!(a.run(AuditQueryReq { cursor: Some(bad.clone()), ..Default::default() }).is_err(), "{bad}");
    }
}

// ── provenance, and the two halves of a message

fn half(ts: u64, msg: &str, sent: bool) -> AuditLogEntry {
    AuditLogEntry {
        event_kind: if sent { "sent" } else { "delivery" }.into(),
        outcome: sent.then(|| "sent".to_string()),
        msg_id: Some(msg.into()),
        request_id: msg.into(),
        tier: Some("host".into()),
        trust: (!sent).then(|| "host-verified".to_string()),
        host: Some("narko".into()),
        channel: Some("dev".into()),
        version: Some("0.60.16".into()),
        ..audit_entry(ts, "Korp", "Opaz")
    }
}

#[test]
fn the_sent_and_delivered_lines_of_a_message_are_one_row_with_both_times_and_a_page_boundary_does_not_split_it() {
    let mut lines = Vec::new();
    for n in 0..30u64 {
        lines.push(half(1_000 + n * 10, &format!("m{n}"), true));
        lines.push(half(1_003 + n * 10, &format!("m{n}"), false));
    }
    let world = World::new(&lines);
    let whole = world.all(AuditQueryReq::default());
    assert_eq!(whole.len(), 30, "one row for each message");
    for e in &whole {
        let p = e.provenance.as_ref().unwrap();
        assert_eq!((p.sent_ms, p.sent_only, p.tier.as_deref(), p.trust.as_deref()), (Some(e.ts_ms - 3), None, Some("host"), Some("host-verified")));
        assert_eq!((p.host.as_deref(), p.channel.as_deref(), p.version.as_deref()), (Some("narko"), Some("dev"), Some("0.60.16")));
    }
    let (mut seen, mut cursor) = (Vec::new(), None);
    loop {
        let page = world.run(AuditQueryReq { limit: Some(7), cursor: cursor.clone(), ..Default::default() }).unwrap();
        seen.extend(ids(&page.entries));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(seen, ids(&whole), "no row repeated, none lost, none split from its other half");
}

#[test]
fn a_sent_line_with_no_delivery_is_a_row_marked_other_end_not_seen_and_is_not_counted_in_the_totals() {
    let world = World::new(&[half(1_000, "alone", true), half(2_000, "both", true), half(2_002, "both", false)]);
    let got = world.run(AuditQueryReq { totals: Some(true), ..Default::default() }).unwrap();
    assert_eq!(got.entries.len(), 2);
    let alone = got.entries.iter().find(|e| e.provenance.as_ref().unwrap().msg_id.as_deref() == Some("alone")).unwrap();
    assert_eq!(alone.provenance.as_ref().unwrap().sent_only, Some(true));
    assert!(alone.summary.contains("sent"), "{}", alone.summary);
    assert_eq!(got.totals.unwrap().entries, 1, "only the receiver's lines are counted, so a message is one entry and a lone sent line none");
}

#[test]
fn old_lines_read_as_before_and_say_their_tier_only_where_they_prove_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jekt-audit.jsonl");
    let old = r#"{"timestamp":1000,"source_agent":"Korp","target_agent":"Opaz","block_id":"b","message_hash":"h","message_length":3,"success":true,"request_id":"r1","channel":"dev"}
{"timestamp":2000,"source_agent":"Korp","target_agent":"Opaz","block_id":"b","message_hash":"h","message_length":3,"success":true,"request_id":"r2","wan":{"verified":true}}
"#;
    std::fs::write(&path, old).unwrap();
    let world = World { _dir: dir, jekts: path, store: FileStore::open_in_memory().unwrap() };
    let got = world.all(AuditQueryReq::default());
    let tiers: Vec<_> = got.iter().map(|e| e.provenance.as_ref().map(|p| (p.tier.clone(), p.tier_inferred, p.channel.clone()))).collect();
    assert_eq!(tiers, vec![Some((Some("cloud".to_string()), Some(true), None)), Some((None, None, Some("dev".to_string())))]);
}

#[test]
fn totals_count_the_activity_rows_the_kinds_ask_for_and_carry_on_as_a_day_file_grows() {
    use crate::backend::rpc_types::AuditCost;
    let measured = |input, output| Some(AuditCost { input, cache_read: 0, output, usd: None, basis: "measured".into() });
    let mut turn = activity_row(T0 + 1_000, "turn", "turn ended (success)");
    turn.cost = measured(100, 50);
    let a = Activity::new(&[activity_row(T0, "tool", "Bash cargo test"), turn], None);
    let totals = |agent: Option<&str>| {
        let req = AuditQueryReq { kinds: kinds(&["tool", "turn"]), agent_id: agent.map(str::to_string), totals: Some(true), ..Default::default() };
        a.run(req).unwrap().totals.unwrap()
    };

    // The default kinds leave them out, so the list's totals are what they were.
    assert_eq!(a.run(AuditQueryReq { totals: Some(true), ..Default::default() }).unwrap().totals.unwrap().entries, 0);
    let both = totals(None);
    assert_eq!((both.entries, both.input, both.output, both.estimated), (2, 100, 50, false), "a turn's cost is in the totals");
    assert_eq!(totals(Some("maricon-2")).entries, 2, "a row names its agent by UID: the filter's slug still finds it");
    assert_eq!(totals(Some("nobody")).entries, 0);

    // The day's file is appended to: the next ask reads only what was written since.
    let mut next = activity_row(T0 + 2_000, "turn", "turn ended (success)");
    next.cost = measured(200, 0);
    let mut file = std::fs::OpenOptions::new().append(true).open(a.activity_dir.join("activity-2023-11-14.jsonl")).unwrap();
    std::io::Write::write_all(&mut file, format!("{}
", serde_json::to_string(&next).unwrap()).as_bytes()).unwrap();
    let grown = totals(None);
    assert_eq!((grown.entries, grown.input, grown.output), (3, 300, 50));
}

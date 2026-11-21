// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

const PARENT: &str = "parent-def";
const FORK: &str = "fork-def";

fn line(sid: &str, n: usize) -> String {
    format!("{{\"type\":\"assistant\",\"session_id\":\"{sid}\",\"n\":{n}}}\n")
}

/// A parent whose transcript holds an older session, then `last`.
fn parent_with(gfs: &FileStore, last: &str) -> Vec<u8> {
    let zone = agent_current_zone(PARENT);
    let mut data = String::new();
    for n in 0..3 {
        data += &line("older-session", n);
    }
    for n in 3..6 {
        data += &line(last, n);
    }
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines_stamped(&zone, OUTPUT_FILE, data.as_bytes(), TSIDX_FILE).unwrap();
    write_zone_file(gfs, &zone, SNAPSHOT_FILE, b"{\"collapsed\":[\"n1\"]}").unwrap();
    data.into_bytes()
}

fn read_all(gfs: &FileStore, zone: &str, name: &str) -> Vec<u8> {
    gfs.read_file(zone, name).unwrap().unwrap_or_default()
}

#[test]
fn a_fork_of_the_last_session_gets_the_parents_history_exactly() {
    let gfs = FileStore::open_in_memory().unwrap();
    let data = parent_with(&gfs, "sess-b");

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host).unwrap();

    let parent = agent_current_zone(PARENT);
    let sidecars = read_all(&gfs, &parent, TSIDX_FILE).len() + read_all(&gfs, &parent, SNAPSHOT_FILE).len();
    assert_eq!(out, ForkHistory::Copied { bytes: (data.len() + sidecars) as u64 });
    let fork = agent_current_zone(FORK);
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), data);
    assert_eq!(read_all(&gfs, &fork, TSIDX_FILE), read_all(&gfs, &agent_current_zone(PARENT), TSIDX_FILE));
    assert_eq!(read_all(&gfs, &fork, SNAPSHOT_FILE), b"{\"collapsed\":[\"n1\"]}");
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("sess-b"));
    assert_eq!(inherited_session(&gfs, &agent_current_zone(PARENT)), None, "the parent is not a fork");
}

#[test]
fn a_last_event_larger_than_the_tail_window_still_names_its_session() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    // A tool result of 700 KB: the first window starts inside it.
    let big = format!("{{\"type\":\"user\",\"session_id\":\"sess-big\",\"text\":\"{}\"}}\n", "x".repeat(700 * 1024));
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, format!("{}{big}", line("sess-big", 0)).as_bytes()).unwrap();

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-big", "session_id", WriterRuntime::Host).unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }), "got {out:?}");
}

#[test]
fn a_fork_of_an_older_session_copies_nothing() {
    let gfs = FileStore::open_in_memory().unwrap();
    parent_with(&gfs, "sess-b");

    let out = copy_parent_history(&gfs, PARENT, FORK, "older-session", "session_id", WriterRuntime::Host).unwrap();

    assert_eq!(out, ForkHistory::NotTheLastSession);
    assert!(gfs.stat(&agent_current_zone(FORK), OUTPUT_FILE).unwrap().is_none());
    assert_eq!(inherited_session(&gfs, &agent_current_zone(FORK)), None);
}

#[test]
fn a_fork_with_history_of_its_own_keeps_it() {
    let gfs = FileStore::open_in_memory().unwrap();
    parent_with(&gfs, "sess-b");
    let fork = agent_current_zone(FORK);
    gfs.make_file(&fork, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&fork, OUTPUT_FILE, line("fork-session", 0).as_bytes()).unwrap();

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host).unwrap();

    assert_eq!(out, ForkHistory::ForkHasHistory);
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), line("fork-session", 0).into_bytes());
}

#[test]
fn nothing_to_copy_without_parent_history_or_a_session() {
    let gfs = FileStore::open_in_memory().unwrap();
    assert_eq!(copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host).unwrap(), ForkHistory::NoParentHistory);
    assert!(copy_parent_history(&gfs, PARENT, FORK, "", "session_id", WriterRuntime::Host).is_err());
    assert!(copy_parent_history(&gfs, PARENT, PARENT, "sess-b", "session_id", WriterRuntime::Host).is_err());
    assert!(copy_parent_history(&gfs, PARENT, FORK, "sess-b", "", WriterRuntime::Host).is_err());
}


/// A parent with `older-session` (3 lines) then `sess-b` (3 lines), and a
/// sidecar with a record for each (written whole: appends close together share
/// one). Returns the bytes and where the older session ends.
fn parent_in_two_batches(gfs: &FileStore) -> (Vec<u8>, usize) {
    let zone = agent_current_zone(PARENT);
    let older: String = (0..3).map(|n| line("older-session", n)).collect();
    let newer: String = (3..6).map(|n| line("sess-b", n)).collect();
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, format!("{older}{newer}").as_bytes()).unwrap();
    let sidecar = format!("{{\"off\":0,\"ms\":1000}}
{{\"off\":{},\"ms\":2000}}
", older.len());
    write_zone_file(gfs, &zone, TSIDX_FILE, sidecar.as_bytes()).unwrap();
    write_zone_file(gfs, &zone, SNAPSHOT_FILE, b"{\"collapsed\":[\"n1\"]}").unwrap();
    (format!("{older}{newer}").into_bytes(), older.len())
}

#[test]
fn a_fork_of_an_older_session_gets_the_history_up_to_where_it_ends() {
    let gfs = FileStore::open_in_memory().unwrap();
    let (data, end) = parent_in_two_batches(&gfs);
    let parent = agent_current_zone(PARENT);
    let fork = agent_current_zone(FORK);

    let out = copy_older_session_history(&gfs, PARENT, FORK, "older-session", "session_id", &parent, end as i64, WriterRuntime::Host).unwrap();

    assert!(matches!(out, ForkHistory::Copied { bytes } if bytes > end as u64), "got {out:?}");
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), &data[..end]);
    // The sidecar stops at the same point: one record (the first batch's).
    let side = read_all(&gfs, &fork, TSIDX_FILE);
    let parent_side = read_all(&gfs, &parent, TSIDX_FILE);
    assert!(!side.is_empty() && side.len() < parent_side.len() && parent_side.starts_with(&side));
    assert_eq!(side.iter().filter(|&&b| b == b'\n').count(), 1);
    // The parent's snapshot is where the parent is now, not here.
    assert!(gfs.stat(&fork, SNAPSHOT_FILE).unwrap().is_none());
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("older-session"));
    // The parent keeps everything.
    assert_eq!(read_all(&gfs, &parent, OUTPUT_FILE), data);
    // The fork continues after the copied history.
    gfs.append_lines(&fork, OUTPUT_FILE, line("fork-session", 0).as_bytes()).unwrap();
    assert!(read_all(&gfs, &fork, OUTPUT_FILE).starts_with(&data[..end]));
}

#[test]
fn a_cut_that_does_not_end_in_the_session_is_undone() {
    let gfs = FileStore::open_in_memory().unwrap();
    let (_, end) = parent_in_two_batches(&gfs);
    let parent = agent_current_zone(PARENT);
    // A stale record: the cut lands in the middle of a line, then at the end
    // of a line of the later session.
    let out = copy_older_session_history(&gfs, PARENT, FORK, "older-session", "session_id", &parent, end as i64 + 1, WriterRuntime::Host).unwrap();
    assert_eq!(out, ForkHistory::NotTheLastSession);
    let second = end + line("sess-b", 3).len();
    let out = copy_older_session_history(&gfs, PARENT, FORK, "older-session", "session_id", &parent, second as i64, WriterRuntime::Host).unwrap();

    assert_eq!(out, ForkHistory::NotTheLastSession);
    assert!(gfs.stat(&agent_current_zone(FORK), OUTPUT_FILE).unwrap().is_none());
}

#[test]
fn an_older_session_needs_a_transcript_a_session_and_a_place_to_end() {
    let gfs = FileStore::open_in_memory().unwrap();
    let parent = agent_current_zone(PARENT);
    assert_eq!(copy_older_session_history(&gfs, PARENT, FORK, "s", "session_id", &parent, 10, WriterRuntime::Host).unwrap(), ForkHistory::NoParentHistory);
    assert!(copy_older_session_history(&gfs, PARENT, FORK, "", "session_id", &parent, 10, WriterRuntime::Host).is_err());
    assert!(copy_older_session_history(&gfs, PARENT, FORK, "s", "session_id", &parent, 0, WriterRuntime::Host).is_err());
    assert!(copy_older_session_history(&gfs, PARENT, PARENT, "s", "session_id", &parent, 10, WriterRuntime::Host).is_err());
}

#[test]
fn a_fork_of_a_session_in_an_archived_conversation_copies_from_the_archive() {
    let gfs = FileStore::open_in_memory().unwrap();
    // After a New conversation the parent's current zone is empty and its
    // sessions are in the archive.
    let archive = "agent:parent-def:archive:1000";
    let data: String = (0..3).map(|n| line("archived-session", n)).collect();
    gfs.make_file(archive, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(archive, OUTPUT_FILE, data.as_bytes()).unwrap();

    assert_eq!(copy_parent_history(&gfs, PARENT, FORK, "archived-session", "session_id", WriterRuntime::Host).unwrap(), ForkHistory::NoParentHistory);
    let out = copy_older_session_history(&gfs, PARENT, FORK, "archived-session", "session_id", archive, data.len() as i64, WriterRuntime::Host).unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }), "got {out:?}");
    assert_eq!(read_all(&gfs, &agent_current_zone(FORK), OUTPUT_FILE), data.as_bytes());
    assert_eq!(read_all(&gfs, archive, OUTPUT_FILE), data.as_bytes(), "the archive is untouched");
}

/// The human's earlier messages are signed for the parent (`origin_sig`); the fork's copy signs the ones that
/// verify for the fork, and leaves a forged one as it was.
#[test]
fn a_fork_shows_the_parents_human_messages_as_the_humans() {
    use crate::backend::transcript::origin_sig::{sign_line, vet_line};
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    let user = |text: &str, session: &str| {
        let mut v = serde_json::json!({ "type": "user", "session_id": session, "message": { "role": "user", "content": text }, "amx": { "v": 1, "origin": { "kind": "user" } } });
        v["amx"]["origin"]["kind"] = "user".into();
        let mut l = v.to_string().into_bytes();
        l.push(b'\n');
        l
    };
    let signed = sign_line(PARENT, &user("typed by the human", "sess-b"), 1_700_000_000_000);
    let forged = user("typed by an agent", "sess-b");
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines_stamped(&zone, OUTPUT_FILE, &[line("sess-b", 0).as_bytes(), &signed, &forged].concat(), TSIDX_FILE).unwrap();

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host).unwrap();
    assert!(matches!(out, ForkHistory::Copied { .. }));

    let copied = read_all(&gfs, &agent_current_zone(FORK), OUTPUT_FILE);
    let kind = |owner: &str, l: &[u8]| {
        let v: serde_json::Value = serde_json::from_slice(&vet_line(owner, l)).unwrap();
        v.pointer("/amx/origin/kind").and_then(|k| k.as_str()).map(str::to_string)
    };
    let lines: Vec<&[u8]> = copied.split(|b| *b == b'\n').filter(|l| !l.is_empty()).collect();
    assert_eq!(kind(FORK, lines[1]).as_deref(), Some("user"), "the human's message is the human's in the fork");
    assert_eq!(kind(FORK, lines[2]), None, "a forged one is not made good by forking");
    assert_eq!(read_all(&gfs, &agent_current_zone(PARENT), OUTPUT_FILE).len(), copied.len(), "same size, so the sidecar still matches");
    assert_eq!(kind(PARENT, &read_all(&gfs, &zone, OUTPUT_FILE).split(|b| *b == b'\n').nth(1).unwrap().to_vec()).as_deref(), Some("user"), "the parent's own copy is untouched");
}

// A parent's transcript is its whole life; a fork copies only a newest part of it, from a turn start.

fn turn_line(sid: &str, n: usize) -> String {
    let mut l = serde_json::json!({ "type": "user", "session_id": sid, "message": { "role": "user", "content": format!("question {n}") } }).to_string();
    l.push('\n');
    l
}

fn work_line(sid: &str, n: usize, pad: usize) -> String {
    format!("{{\"type\":\"assistant\",\"session_id\":\"{sid}\",\"n\":{n},\"pad\":\"{}\"}}\n", "x".repeat(pad))
}

/// `turns` turns of `sid`, each a question and three replies. Returns the bytes and where each turn starts.
fn long_history(sid: &str, turns: usize) -> (String, Vec<usize>) {
    let (mut data, mut starts) = (String::new(), Vec::new());
    for t in 0..turns {
        starts.push(data.len());
        data += &turn_line(sid, t);
        for k in 0..3 {
            data += &work_line(sid, t * 3 + k, 100);
        }
    }
    (data, starts)
}

fn lines_of(bytes: &[u8]) -> usize {
    bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()).count()
}

fn records_of(sidecar: &[u8]) -> Vec<(u64, i64)> {
    sidecar
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_slice(l).unwrap();
            (v["off"].as_u64().unwrap(), v["ms"].as_i64().unwrap())
        })
        .collect()
}

#[test]
fn a_parent_over_the_cap_gives_the_fork_its_newest_part_from_a_turn_start() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    let (data, starts) = long_history("sess-b", 80);
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, data.as_bytes()).unwrap();
    let side: String = starts.iter().enumerate().map(|(i, o)| format!("{{\"off\":{o},\"ms\":{}}}\n", 1000 + i)).collect();
    write_zone_file(&gfs, &zone, TSIDX_FILE, side.as_bytes()).unwrap();
    write_zone_file(&gfs, &zone, SNAPSHOT_FILE, b"{\"highWaterMark\":160}").unwrap();
    // Two summary versions: one that covered only the start of the parent, one that covers all of it.
    let version = |n: u64, based_on: usize| {
        let v = crate::backend::continuity_state::StateVersion {
            version: n,
            created_at_ms: 10 * n as i64,
            based_on: based_on as i64,
            sha256: String::new(),
            text: format!("## Last user request
summary {n}"),
            tokens: None,
        };
        format!("{}
", serde_json::to_string(&v).unwrap())
    };
    let state = format!("{}{}", version(1, 300), version(2, data.len()));
    write_zone_file(&gfs, &zone, crate::backend::continuity_state::STATE_FILE, state.as_bytes()).unwrap();
    let cap = 6_000;
    assert!(data.len() > 4 * cap as usize);

    let out = copy_parent_history_capped(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host, cap).unwrap();

    let fork = agent_current_zone(FORK);
    let copied = read_all(&gfs, &fork, OUTPUT_FILE);
    assert!(matches!(out, ForkHistory::Copied { bytes } if bytes as usize > copied.len()), "got {out:?}");
    assert!(copied.len() as i64 <= cap, "{} bytes over a cap of {cap}", copied.len());
    assert!(copied.len() as i64 > cap - 900, "it keeps what fits, less one turn and a cut line");
    assert!(data.as_bytes().ends_with(&copied), "the newest part, exactly");
    let cut = data.len() - copied.len();
    assert!(starts.contains(&cut), "it starts at a turn start");
    // Counted from its first byte, so the pane opens from line positions at once.
    let counted = gfs.line_state(&fork, OUTPUT_FILE).unwrap().unwrap().counted.expect("counted");
    assert_eq!(counted.lines as usize, lines_of(&copied));
    // The sidecar is the parent's records for those bytes, moved to start at 0.
    let recs = records_of(&read_all(&gfs, &fork, TSIDX_FILE));
    let want: Vec<(u64, i64)> = starts.iter().enumerate().filter(|(_, o)| **o >= cut).map(|(i, o)| ((*o - cut) as u64, 1000 + i as i64)).collect();
    assert_eq!(recs, want);
    // The parent's snapshot counts lines of the whole transcript; its summary is the conversation's.
    assert!(gfs.stat(&fork, SNAPSHOT_FILE).unwrap().is_none());
    // Its coverage is moved to the copy, so the summary is still offered: a version that covered only
    // what was cut away covers none of the copy, the newest covers all of it.
    let versions: Vec<crate::backend::continuity_state::StateVersion> = read_all(&gfs, &fork, crate::backend::continuity_state::STATE_FILE)
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).unwrap())
        .collect();
    assert_eq!(versions.iter().map(|v| v.based_on).collect::<Vec<_>>(), vec![0, copied.len() as i64]);
    let latest = crate::backend::continuity_state::latest_state(&gfs, &fork).expect("the summary is offered to the fork");
    assert_eq!(latest.text, "## Last user request
summary 2");
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("sess-b"));
    assert_eq!(read_all(&gfs, &zone, OUTPUT_FILE), data.as_bytes(), "the parent keeps everything");
    // The fork continues after what it was given.
    gfs.append_lines(&fork, OUTPUT_FILE, work_line("fork-session", 0, 5).as_bytes()).unwrap();
    assert_eq!(gfs.line_state(&fork, OUTPUT_FILE).unwrap().unwrap().counted.unwrap().lines, counted.lines + 1);
}

#[test]
fn a_transcript_at_the_cap_is_copied_whole_as_before() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    let (data, _) = long_history("sess-b", 5);
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, data.as_bytes()).unwrap();
    write_zone_file(&gfs, &zone, SNAPSHOT_FILE, b"{\"highWaterMark\":20}").unwrap();

    let out = copy_parent_history_capped(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host, data.len() as i64).unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }));
    let fork = agent_current_zone(FORK);
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), data.as_bytes());
    assert_eq!(read_all(&gfs, &fork, SNAPSHOT_FILE), b"{\"highWaterMark\":20}");
    assert_eq!(gfs.line_state(&fork, OUTPUT_FILE).unwrap().unwrap().counted.expect("counted").lines as usize, lines_of(data.as_bytes()));
}

#[test]
fn a_window_with_no_turn_start_begins_at_a_whole_line() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    // One long turn: the question, then replies for far more than the cap.
    let mut data = turn_line("sess-b", 0);
    for k in 0..200 {
        data += &work_line("sess-b", k, 100);
    }
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, data.as_bytes()).unwrap();

    let out = copy_parent_history_capped(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host, 5_000).unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }), "got {out:?}");
    let copied = read_all(&gfs, &agent_current_zone(FORK), OUTPUT_FILE);
    assert!(copied.len() <= 5_000 && copied.starts_with(b"{\"type\":\"assistant\""), "a whole line, not a cut one");
    assert!(data.as_bytes().ends_with(&copied));
}

#[test]
fn a_fork_of_an_older_session_over_the_cap_gets_the_newest_part_up_to_where_it_ends() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    let (older, starts) = long_history("older-session", 40);
    let (newer, _) = long_history("sess-b", 10);
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, format!("{older}{newer}").as_bytes()).unwrap();
    let side: String = starts.iter().enumerate().map(|(i, o)| format!("{{\"off\":{o},\"ms\":{}}}\n", 1000 + i)).collect::<String>()
        + &format!("{{\"off\":{},\"ms\":9000}}\n", older.len());
    write_zone_file(&gfs, &zone, TSIDX_FILE, side.as_bytes()).unwrap();
    write_zone_file(&gfs, &zone, SNAPSHOT_FILE, b"{\"highWaterMark\":999}").unwrap();
    let cap = 6_000;

    let out = copy_older_session_history_capped(&gfs, PARENT, FORK, "older-session", "session_id", &zone, older.len() as i64, WriterRuntime::Host, cap).unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }), "got {out:?}");
    let fork = agent_current_zone(FORK);
    let copied = read_all(&gfs, &fork, OUTPUT_FILE);
    assert!(copied.len() as i64 <= cap);
    assert!(older.as_bytes().ends_with(&copied), "the newest part of that session, ending where it ends");
    assert!(starts.contains(&(older.len() - copied.len())), "from a turn start");
    let recs = records_of(&read_all(&gfs, &fork, TSIDX_FILE));
    assert!(recs.iter().all(|(off, _)| (*off as usize) < copied.len()), "no record of the later session");
    assert_eq!(recs[0].0, 0);
    assert!(gfs.stat(&fork, SNAPSHOT_FILE).unwrap().is_none());
    assert!(gfs.stat(&fork, crate::backend::continuity_state::STATE_FILE).unwrap().is_none());
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("older-session"));
}

#[test]
fn the_human_messages_in_a_capped_copy_are_the_humans_in_the_fork() {
    use crate::backend::transcript::origin_sig::{sign_line, vet_line};
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    let human = |n: usize| {
        let v = serde_json::json!({ "type": "user", "session_id": "sess-b", "message": { "role": "user", "content": format!("typed {n}") }, "amx": { "v": 1, "origin": { "kind": "user" } } });
        let mut l = v.to_string().into_bytes();
        l.push(b'\n');
        sign_line(PARENT, &l, 1_700_000_000_000)
    };
    let mut data: Vec<u8> = Vec::new();
    for n in 0..30 {
        data.extend_from_slice(&human(n));
        data.extend_from_slice(work_line("sess-b", n, 200).as_bytes());
    }
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, &data).unwrap();

    let out = copy_parent_history_capped(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Host, 4_000).unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }), "got {out:?}");
    let copied = read_all(&gfs, &agent_current_zone(FORK), OUTPUT_FILE);
    let kind = |owner: &str, l: &[u8]| {
        let v: serde_json::Value = serde_json::from_slice(&vet_line(owner, l)).unwrap();
        v.pointer("/amx/origin/kind").and_then(|k| k.as_str()).map(str::to_string)
    };
    let humans: Vec<&[u8]> = copied.split(|b| *b == b'\n').filter(|l| l.windows(13).any(|w| w == b"\"type\":\"user\"")).collect();
    assert!(humans.len() > 3);
    assert!(humans.iter().all(|l| kind(FORK, l).as_deref() == Some("user") && kind(PARENT, l).is_none()), "signed for the fork, not the parent");
}

// --- Where a fork's copied history came from (the mark's cover, and the stamped lines re-signed for the fork) ---

use crate::backend::transcript::origin_sig::{stamp_writer, vet_line};

fn writer(vetted: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(vetted.strip_suffix(b"\n").unwrap_or(vetted)).ok()?;
    v.pointer("/amx/writer/runtime")?.as_str().map(str::to_string)
}

/// A parent whose transcript holds one line its container wrote and stamped, one written before there were stamps,
/// and the session `sess-b` at the end.
fn stamped_parent(gfs: &FileStore, runtime: WriterRuntime) -> Vec<u8> {
    let zone = agent_current_zone(PARENT);
    let stamped = stamp_writer(PARENT, &line("sess-b", 1).trim_end().to_string(), runtime, 7).unwrap();
    let data = format!("{}{}\n{}", line("sess-b", 0), stamped, line("sess-b", 2));
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines_stamped(&zone, OUTPUT_FILE, data.as_bytes(), TSIDX_FILE).unwrap();
    data.into_bytes()
}

#[test]
fn a_fork_keeps_its_parents_stamped_lines_verifying_for_the_fork_and_its_mark_says_where_the_copy_came_from() {
    for runtime in [WriterRuntime::Container, WriterRuntime::Host] {
        let gfs = FileStore::open_in_memory().unwrap();
        let data = stamped_parent(&gfs, runtime);
        assert!(matches!(copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id", runtime).unwrap(), ForkHistory::Copied { .. }));

        let fork = agent_current_zone(FORK);
        let copied = read_all(&gfs, &fork, OUTPUT_FILE);
        assert_eq!(copied.len(), data.len(), "the offsets did not move");
        let lines: Vec<&[u8]> = copied.split_inclusive(|b| *b == b'\n').collect();
        let name = if runtime == WriterRuntime::Container { "container" } else { "host" };
        assert_eq!(writer(&vet_line(FORK, lines[1])).as_deref(), Some(name), "the stamped line verifies for the fork");
        assert!(writer(&vet_line(PARENT, lines[1])).is_none(), "and no longer for the parent");
        assert!(writer(&vet_line(FORK, lines[0])).is_none() && writer(&vet_line(FORK, lines[2])).is_none(), "a line nobody stamped is not stamped by a copy");

        let cover = fork_history_cover(&gfs, &fork).expect("the mark is sealed");
        assert_eq!((cover.parent.as_str(), cover.runtime, cover.upto), (PARENT, runtime, copied.len() as u64));
        assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("sess-b"), "the mark still says which session it ends in");
    }
}

#[test]
fn a_newest_part_copy_is_sealed_with_the_length_of_what_it_holds() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    let mut data = String::new();
    for n in 0..40 {
        data += &line("sess-b", n);
    }
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines_stamped(&zone, OUTPUT_FILE, data.as_bytes(), TSIDX_FILE).unwrap();
    copy_parent_history_capped(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Container, 400).unwrap();
    let fork = agent_current_zone(FORK);
    let held = read_all(&gfs, &fork, OUTPUT_FILE).len() as u64;
    assert!(held > 0 && (held as usize) < data.len());
    let cover = fork_history_cover(&gfs, &fork).unwrap();
    assert_eq!((cover.runtime, cover.upto), (WriterRuntime::Container, held));
}

#[test]
fn a_mark_that_was_changed_or_made_before_covers_nothing() {
    let gfs = FileStore::open_in_memory().unwrap();
    stamped_parent(&gfs, WriterRuntime::Container);
    copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id", WriterRuntime::Container).unwrap();
    let fork = agent_current_zone(FORK);
    let mut mark: serde_json::Value = serde_json::from_slice(&read_all(&gfs, &fork, FORK_MARK_FILE)).unwrap();
    mark["runtime"] = serde_json::json!("host");
    write_zone_file(&gfs, &fork, FORK_MARK_FILE, mark.to_string().as_bytes()).unwrap();
    assert!(fork_history_cover(&gfs, &fork).is_none(), "a runtime changed after it was signed");
    assert!(fork_history_cover(&gfs, &agent_current_zone(PARENT)).is_none(), "a zone that is no fork's");
}

#[test]
fn a_fork_made_before_its_mark_said_so_is_sealed_once_with_the_runtime_it_is_given() {
    let gfs = FileStore::open_in_memory().unwrap();
    let data = stamped_parent(&gfs, WriterRuntime::Container);
    // A fork as an earlier build made it: the copy and a mark with no runtime.
    let fork = agent_current_zone(FORK);
    gfs.make_file(&fork, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&fork, OUTPUT_FILE, &data).unwrap();
    write_zone_file(&gfs, &fork, FORK_MARK_FILE, br#"{"from":"parent-def","session_id":"sess-b","at_ms":1}"#).unwrap();
    assert!(fork_history_cover(&gfs, &fork).is_none());

    assert!(seal_existing_fork(&gfs, FORK, PARENT, WriterRuntime::Container));
    let cover = fork_history_cover(&gfs, &fork).unwrap();
    assert_eq!((cover.parent.as_str(), cover.runtime, cover.upto), (PARENT, WriterRuntime::Container, data.len() as u64));
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("sess-b"));
    assert!(!seal_existing_fork(&gfs, FORK, PARENT, WriterRuntime::Host), "already sealed: nothing is written, and nothing is loosened");
    assert_eq!(fork_history_cover(&gfs, &fork).unwrap().runtime, WriterRuntime::Container);
    // A zone with no mark at all is no fork's.
    assert!(!seal_existing_fork(&gfs, "other-def", PARENT, WriterRuntime::Host));
}

#[test]
fn the_pass_over_existing_forks_takes_each_runtime_from_srvs_own_record_and_the_less_trusted_of_two() {
    use crate::backend::storage::agents::test_agent_def;
    use crate::backend::storage::store::Store;
    let (gfs, store) = (FileStore::open_in_memory().unwrap(), Store::open_in_memory().unwrap());
    let fork_of = |id: &str, parent: &str, kind: &str| {
        let mut def = test_agent_def(id, id, "claude", kind, 1, "");
        def.parent_id = parent.to_string();
        def.branch_label = "fork".to_string();
        store.agent_def_insert(&mut def).unwrap();
        let zone = agent_current_zone(id);
        gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
        gfs.append_lines(&zone, OUTPUT_FILE, line("s", 0).as_bytes()).unwrap();
        write_zone_file(&gfs, &zone, FORK_MARK_FILE, format!("{{\"from\":\"{parent}\",\"session_id\":\"s\",\"at_ms\":1}}").as_bytes()).unwrap();
    };
    for (id, kind) in [("host-parent", "host"), ("container-parent", "container")] {
        let mut def = test_agent_def(id, id, "claude", kind, 1, "");
        store.agent_def_insert(&mut def).unwrap();
    }
    fork_of("fork-hh", "host-parent", "host");
    fork_of("fork-hc", "host-parent", "container");
    fork_of("fork-ch", "container-parent", "host");
    fork_of("fork-gone", "no-such-parent", "host");

    assert_eq!(seal_existing_forks(&gfs, &store), 4);
    let runtime = |id: &str| fork_history_cover(&gfs, &agent_current_zone(id)).unwrap().runtime;
    assert_eq!(runtime("fork-hh"), WriterRuntime::Host);
    assert_eq!(runtime("fork-hc"), WriterRuntime::Container, "its own runtime is the container's");
    assert_eq!(runtime("fork-ch"), WriterRuntime::Container, "its parent was one");
    assert_eq!(runtime("fork-gone"), WriterRuntime::Container, "a parent srv has no record of");
    assert_eq!(seal_existing_forks(&gfs, &store), 0, "a second pass changes nothing");
}

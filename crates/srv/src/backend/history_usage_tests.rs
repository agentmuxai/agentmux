// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::agent_session::agent_current_zone;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

fn write(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn roots(base: &Path) -> Roots {
    Roots {
        identities: base.join("shared").join("identities"),
        transcripts_db: base.join("shared").join("agents").join("transcripts").join("filestore.db"),
        channels: base.join("channels"),
        archives: base.join("archives"),
        audit: base.join("shared").join("audit").join("jekt-audit.jsonl"),
    }
}

#[test]
fn each_store_is_measured_and_the_total_is_their_sum() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    // Two identities' Claude session files (one nested), and a folder that isn't Claude's.
    write(&r.identities.join("id-a/claude/projects/p1/s1.jsonl"), 1000);
    write(&r.identities.join("id-a/claude/projects/p1/subagents/s2.jsonl"), 500);
    write(&r.identities.join("id-b/claude/projects/p2/s3.jsonl"), 250);
    write(&r.identities.join("id-b/codex/sessions/x.jsonl"), 9999);
    write(&r.transcripts_db, 4000);
    write(Path::new(&format!("{}-wal", r.transcripts_db.display())), 300);
    write(&r.channels.join("local-a/versions/0.59.1/data/db/filestore.db"), 200);
    write(&r.channels.join("local-a/versions/0.59.2/data/db/filestore.db"), 100);
    write(&r.channels.join("stable/versions/0.55.1/data/db/objects.db"), 7777);
    write(&r.archives.join("b1.jsonl.gz"), 60);
    write(&r.audit, 40);

    let u = compute(&r, None, |_| None, 42);

    let by = |k: &str| u.stores.iter().find(|s| s.key == k).unwrap();
    assert_eq!((by("provider-files").bytes, by("provider-files").files), (1750, 3));
    assert_eq!((by("pane-history").bytes, by("pane-history").files), (4300, 2));
    assert_eq!((by("channel-copies").bytes, by("channel-copies").files), (300, 2));
    assert_eq!(by("archives").bytes, 60);
    assert_eq!(by("audit").bytes, 40);
    assert_eq!(u.total_bytes, 1750 + 4300 + 300 + 60 + 40);
    assert_eq!(u.computed_at_ms, 42);
}

#[test]
fn pane_history_counts_the_data_stored_and_shows_free_pages_apart() {
    let dir = tempfile::tempdir().unwrap();
    let r = roots(dir.path());
    std::fs::create_dir_all(r.transcripts_db.parent().unwrap()).unwrap();
    // A store from before new ones were made incremental, so its free pages stay in the file.
    rusqlite::Connection::open(&r.transcripts_db).unwrap().execute_batch("CREATE TABLE seed(x); DROP TABLE seed;").unwrap();
    let fs = FileStore::open_global_transcripts(&r.transcripts_db).unwrap();
    let mut x = 7u64;
    for zone in ["agent:keep:current", "agent:gone:current"] {
        fs.make_file(zone, "output", FileMeta::new(), FileOpts::default()).unwrap();
        let mut data = Vec::new();
        while data.len() < 2_000_000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            data.extend_from_slice(format!("{{\"n\":{x}}}\n").as_bytes());
        }
        fs.append_lines(zone, "output", &data).unwrap();
    }
    fs.delete_zone("agent:gone:current").unwrap();
    fs.checkpoint_truncate_apart().unwrap();
    let file_len = std::fs::metadata(&r.transcripts_db).unwrap().len();

    let u = compute(&r, Some(&fs), |_| None, 1);

    let pane = u.stores.iter().find(|s| s.key == "pane-history").unwrap();
    assert!(u.reclaimable_bytes > 500_000, "the delete left free pages ({})", u.reclaimable_bytes);
    assert_eq!(pane.bytes + u.reclaimable_bytes, file_len, "stored data plus free pages is the file");
    assert!(pane.bytes < file_len);
    assert_eq!(u.total_bytes, pane.bytes, "the free pages are not in the total");
}

#[test]
fn a_missing_store_counts_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let u = compute(&roots(dir.path()), None, |_| None, 1);
    assert_eq!(u.total_bytes, 0);
    assert!(u.agents.is_empty());
    assert_eq!(u.stores.len(), 5, "every store is listed, even when empty");
}

#[test]
fn the_largest_agents_are_named_and_sorted() {
    let fs = FileStore::open_in_memory().unwrap();
    for (id, n) in [("def-small", 10usize), ("def-big", 5000), ("def-mid", 300), ("def-empty", 0)] {
        let zone = agent_current_zone(id);
        fs.make_file(&zone, "output", FileMeta::new(), FileOpts::default()).unwrap();
        if n > 0 {
            fs.append_lines(&zone, "output", format!("{}\n", "x".repeat(n - 1)).as_bytes()).unwrap();
        }
    }
    // Zones of other kinds are not agents' pane history.
    fs.make_file("agent:def-big:archive:1", "output", FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines("agent:def-big:archive:1", "output", b"zzzz\n").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let names = |id: &str| (id == "def-big").then(|| "Lark".to_string());
    let u = compute(&roots(dir.path()), Some(&fs), names, 1);

    let got: Vec<(&str, u64)> = u.agents.iter().map(|a| (a.name.as_str(), a.bytes)).collect();
    assert_eq!(got, vec![("Lark", 5000), ("def-mid", 300), ("def-small", 10)], "no empty zone, no archive zone");
}

#[test]
fn the_largest_agents_leave_out_a_zone_found_corrupt() {
    use crate::backend::transcript::damage::{self, test_support};
    let fs = FileStore::open_in_memory().unwrap();
    for (id, bytes) in [("usage-damage-a", 900usize), ("usage-damage-b", 100)] {
        let zone = agent_current_zone(id);
        fs.make_file(&zone, crate::backend::agent_session::OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
        fs.append_data(&zone, crate::backend::agent_session::OUTPUT_FILE, &vec![b'x'; bytes]).unwrap();
    }
    assert_eq!(largest_agents(Some(&fs), |_| None).len(), 2);
    damage::note_corrupt(&agent_current_zone("usage-damage-a"), "append", &test_support::corrupt_error());
    let rows = largest_agents(Some(&fs), |_| None);
    assert_eq!(rows.iter().map(|r| r.definition_id.as_str()).collect::<Vec<_>>(), ["usage-damage-b"]);
}

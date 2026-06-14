// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-shot backfill of pre-existing agent CONVERSATIONS into the GLOBAL
//! transcript store, so the 9 (and any) agents created before cross-channel
//! transcripts shipped load their history when opened from a fresh channel.
//!
//! The merged feature (#1399) mirrors *new* agent output into the global
//! `agent:<defId>:current` zone, but pre-existing conversations were never
//! mirrored — they live only in the channel they ran in. This scans every
//! channel/dev `objects.db` for blocks belonging to each agent definition,
//! reads the richest `output` from that channel's `filestore.db`, and seeds the
//! global zone with it.
//!
//! **Source = the channel block's `output`, NOT the provider `.jsonl`.** The
//! block `output` is the exact stdout stream-json the renderer already parses
//! (`parseHistoryLines` + the `claude-stream-json` translator); the provider
//! `~/.claude/projects/<slug>/<sid>.jsonl` transcript is a *different* envelope
//! (`queue-operation` / `user`+`parentUuid`/`sessionId`) the renderer can't
//! consume, so copying it would render empty. Using the block output needs zero
//! translation.
//!
//! The seeded snapshot overlay uses `sourceBlockId: ""`, which the frontend
//! treats as "matches any block" — so opening the agent (a fresh local block
//! with no local `output`) takes the v2 fast-path and the
//! `blockfile:read_range` global fallback (#1399) serves the backfilled zone.
//!
//! Marker-gated (`.transcripts_backfilled`, version-stamped) like the
//! definition backfill (`registry::def_migrate`); read-only on every scanned
//! SQLite (the global store is the only thing written).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::backend::agent_session::{
    agent_current_zone, is_valid_definition_id, OUTPUT_FILE, SNAPSHOT_FILE,
};
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

const MARKER: &str = ".transcripts_backfilled";

/// Bump to re-run the backfill once for everyone (e.g. when the source-selection
/// logic improves).
const BACKFILL_VERSION: u32 = 1;

#[derive(Debug, Default, Clone, Copy)]
pub struct BackfillStats {
    pub agents_seen: usize,
    pub data_dirs_scanned: usize,
    pub seeded: usize,
    pub skipped_no_source: usize,
    pub skipped_global_richer: usize,
}

/// Seed the global transcript zone for each agent in `def_ids` from its richest
/// channel/dev block `output`. Marker-gated under `transcripts_dir`. Best-effort
/// — any per-DB failure is skipped, never fatal.
pub fn backfill_transcripts_once(
    home: &Path,
    transcripts_dir: &Path,
    def_ids: &[String],
    global: &FileStore,
) -> BackfillStats {
    let mut stats = BackfillStats::default();
    let marker = transcripts_dir.join(MARKER);
    if marker_version(&marker) >= BACKFILL_VERSION {
        return stats;
    }

    let want: HashSet<&str> = def_ids
        .iter()
        .map(|s| s.as_str())
        .filter(|s| is_valid_definition_id(s))
        .collect();
    if want.is_empty() {
        let _ = write_marker(&marker);
        return stats;
    }

    // def_id -> richest output bytes found across all channels/dev DBs.
    let mut best: HashMap<String, Vec<u8>> = HashMap::new();
    for data in collect_data_dirs(home) {
        let objdb = data.join("db").join("objects.db");
        let fdb = data.join("db").join("filestore.db");
        if !objdb.is_file() || !fdb.is_file() {
            continue;
        }
        stats.data_dirs_scanned += 1;
        let blocks = match read_agent_blocks(&objdb, &want) {
            Ok(b) if !b.is_empty() => b,
            _ => continue,
        };
        let conn = match Connection::open_with_flags(&fdb, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (def_id, block_oid) in blocks {
            if let Some(bytes) = read_zone_output(&conn, &block_oid) {
                let better = best.get(&def_id).map(|b| bytes.len() > b.len()).unwrap_or(true);
                if better {
                    best.insert(def_id, bytes);
                }
            }
        }
    }

    for def_id in def_ids {
        if !is_valid_definition_id(def_id) {
            continue;
        }
        stats.agents_seen += 1;
        let Some(bytes) = best.get(def_id) else {
            stats.skipped_no_source += 1;
            continue;
        };
        let zone = agent_current_zone(def_id);
        // Don't clobber a global zone that already has equal/more content (the
        // live mirror, or a richer prior backfill, may already own it).
        let cur = global
            .stat(&zone, OUTPUT_FILE)
            .ok()
            .flatten()
            .map(|f| f.size)
            .unwrap_or(0);
        if (bytes.len() as i64) <= cur {
            stats.skipped_global_richer += 1;
            continue;
        }
        match seed_global_zone(global, &zone, bytes) {
            Ok(()) => stats.seeded += 1,
            Err(e) => tracing::warn!(zone = %zone, error = %e, "transcript backfill: seed failed"),
        }
    }

    let _ = write_marker(&marker);
    stats
}

/// Write `bytes` as the zone's `output` and a v2 snapshot overlay
/// (`sourceBlockId: ""`) so the open path restores it cross-channel.
fn seed_global_zone(global: &FileStore, zone: &str, bytes: &[u8]) -> Result<(), String> {
    overwrite_zone_file(global, zone, OUTPUT_FILE, bytes)?;
    let hwm = count_nonblank_lines(bytes);
    let overlay = serde_json::json!({
        "schemaVersion": 2,
        "savedAt": "",
        "highWaterMark": hwm,
        "sourceBlockId": "",
        "documentState": {
            "collapsedNodeIds": [],
            "pinnedNodeIds": [],
            "scrollPosition": 0,
            "filter": {
                "showThinking": false,
                "showSuccessfulTools": true,
                "showFailedTools": true,
                "showIncoming": true,
                "showOutgoing": true
            }
        },
        "paneState": { "detailsOpen": false }
    });
    let overlay_bytes = serde_json::to_vec(&overlay).map_err(|e| format!("overlay json: {e}"))?;
    overwrite_zone_file(global, zone, SNAPSHOT_FILE, &overlay_bytes)?;
    Ok(())
}

/// Create-or-replace a zone file with exactly `bytes` (FileStore `write_file`
/// requires the file to exist first).
fn overwrite_zone_file(fs: &FileStore, zone: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
    use crate::backend::storage::error::StoreError;
    if fs.stat(zone, name).map_err(|e| format!("stat: {e}"))?.is_none() {
        match fs.make_file(zone, name, FileMeta::default(), FileOpts::default()) {
            Ok(()) => {}
            Err(StoreError::AlreadyExists) => {}
            Err(e) => return Err(format!("make_file: {e}")),
        }
    }
    fs.write_file(zone, name, bytes).map_err(|e| format!("write_file: {e}"))
}

fn count_nonblank_lines(bytes: &[u8]) -> u64 {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u64
}

/// Read all blocks from `objdb` whose `meta.agentId` is in `want`, returning
/// `(def_id, block_oid)` pairs. Read-only.
fn read_agent_blocks(
    objdb: &Path,
    want: &HashSet<&str>,
) -> Result<Vec<(String, String)>, rusqlite::Error> {
    let conn = Connection::open_with_flags(objdb, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare("SELECT data FROM db_block")?;
    // `db_block.data` is declared TEXT but the wstore stores the JSON as a BLOB
    // (`typeof = 'blob'`), so read raw bytes — `get::<String>` would error on
    // every row and silently skip the whole DB. `from_slice` parses either.
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut out = Vec::new();
    for r in rows {
        let Ok(bytes) = r else { continue };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue };
        let meta = v.get("meta");
        // The agent definition id is stored under either `agentId` (current
        // shape) or the legacy `agent:id` — match both, exactly like the
        // canonical block scan in `agent_session.rs`. Legacy blocks are a core
        // part of the pre-existing population this backfill targets, so missing
        // them would strand those conversations permanently (the marker is
        // written after the scan). (reagent P1 / codex P2 #1403.)
        let agent_id = meta
            .and_then(|m| m.get("agentId"))
            .and_then(|x| x.as_str())
            .or_else(|| meta.and_then(|m| m.get("agent:id")).and_then(|x| x.as_str()));
        let oid = v.get("oid").and_then(|x| x.as_str());
        if let (Some(aid), Some(oid)) = (agent_id, oid) {
            if want.contains(aid) {
                out.push((aid.to_string(), oid.to_string()));
            }
        }
    }
    Ok(out)
}

/// Reassemble a zone's `output` file bytes directly from `db_file_data`
/// (read-only, avoids opening a second writable FileStore on another channel).
/// Returns `None` when absent or empty. Agent `output` is non-circular, so
/// concatenating parts in `partidx` order matches `FileStore::read_file`.
fn read_zone_output(conn: &Connection, block_oid: &str) -> Option<Vec<u8>> {
    let size: Option<i64> = conn
        .query_row(
            "SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            rusqlite::params![block_oid, OUTPUT_FILE],
            |r| r.get(0),
        )
        .ok();
    if size.unwrap_or(0) <= 0 {
        return None;
    }
    let mut stmt = conn
        .prepare("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 ORDER BY partidx")
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![block_oid, OUTPUT_FILE], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .ok()?;
    let mut buf = Vec::new();
    for r in rows {
        if let Ok(part) = r {
            buf.extend_from_slice(&part);
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// Every `<home>/channels/*/versions/*/data` and `<home>/dev/<branch>[/<sub>]/data`
/// (mirrors `registry::def_migrate::collect_scan_dbs`, returning the data dir).
fn collect_data_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut add = |d: PathBuf| {
        if d.join("db").join("objects.db").is_file() {
            dirs.push(d);
        }
    };
    for ch in dir_subdirs(&home.join("channels")) {
        for v in dir_subdirs(&ch.join("versions")) {
            add(v.join("data"));
        }
    }
    for br in dir_subdirs(&home.join("dev")) {
        add(br.join("data"));
        for sub in dir_subdirs(&br) {
            add(sub.join("data"));
        }
    }
    dirs
}

fn dir_subdirs(p: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                out.push(path);
            }
        }
    }
    out
}

fn marker_version(marker: &Path) -> u32 {
    std::fs::read_to_string(marker)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

fn write_marker(marker: &Path) -> std::io::Result<()> {
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(marker, BACKFILL_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_session::OUTPUT_FILE as OUT;
    use std::sync::Arc;

    // Build a channel data dir at <home>/channels/<ch>/versions/<v>/data with a
    // db_block row (agentId=def) and a filestore.db holding that block's output.
    fn seed_channel(home: &Path, ch: &str, def_id: &str, block_oid: &str, output: &[u8]) {
        seed_channel_keyed(home, ch, "agentId", def_id, block_oid, output);
    }

    fn seed_channel_keyed(
        home: &Path,
        ch: &str,
        meta_key: &str,
        def_id: &str,
        block_oid: &str,
        output: &[u8],
    ) {
        let data = home
            .join("channels")
            .join(ch)
            .join("versions")
            .join("0.44.1")
            .join("data");
        let db = data.join("db");
        std::fs::create_dir_all(&db).unwrap();
        // objects.db with a minimal db_block(data) row.
        let oc = Connection::open(db.join("objects.db")).unwrap();
        oc.execute("CREATE TABLE db_block (data TEXT)", []).unwrap();
        let block_json = serde_json::json!({"oid": block_oid, "meta": {"view":"agent", meta_key: def_id}});
        // Store as a BLOB to mirror the wstore (db_block.data is declared TEXT
        // but holds blob-typed JSON) — the reader reads raw bytes.
        oc.execute(
            "INSERT INTO db_block (data) VALUES (?1)",
            rusqlite::params![block_json.to_string().into_bytes()],
        )
        .unwrap();
        drop(oc);
        // filestore.db with the block's output written via FileStore.
        let fs = FileStore::open(&db.join("filestore.db")).unwrap();
        fs.make_file(block_oid, OUT, FileMeta::default(), FileOpts::default()).unwrap();
        fs.append_data(block_oid, OUT, output).unwrap();
    }

    #[test]
    fn backfill_seeds_global_zone_from_richest_channel_block() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let tdir = home.join("shared").join("agents").join("transcripts");
        std::fs::create_dir_all(&tdir).unwrap();
        let def_id = "11111111-2222-3333-4444-555555555555";

        // Two channels for the same agent; the richer (bigger) output wins.
        let small = b"{\"type\":\"system\"}\n";
        let big = b"{\"type\":\"system\"}\n{\"type\":\"assistant\"}\n{\"type\":\"result\"}\n";
        seed_channel(home, "verify-a", def_id, "blk-small", small);
        seed_channel(home, "verify-b", def_id, "blk-big", big);

        let global = Arc::new(FileStore::open(&tdir.join("filestore.db")).unwrap());
        let stats = backfill_transcripts_once(home, &tdir, &[def_id.to_string()], &global);

        assert_eq!(stats.seeded, 1, "should seed the agent zone");
        let zone = agent_current_zone(def_id);
        let out = global.read_file(&zone, OUT).unwrap().unwrap();
        assert_eq!(out, big, "global zone seeded with the RICHEST channel output");
        // Snapshot overlay: v2, hwm = non-blank line count, sourceBlockId empty.
        let snap = global.read_file(&zone, SNAPSHOT_FILE).unwrap().unwrap();
        let o: serde_json::Value = serde_json::from_slice(&snap).unwrap();
        assert_eq!(o["schemaVersion"], 2);
        assert_eq!(o["highWaterMark"], 3);
        assert_eq!(o["sourceBlockId"], "");

        // Idempotent: second run is a no-op (marker written).
        let stats2 = backfill_transcripts_once(home, &tdir, &[def_id.to_string()], &global);
        assert_eq!(stats2.seeded, 0);
        assert_eq!(stats2.data_dirs_scanned, 0, "marker short-circuits the re-run");
    }

    #[test]
    fn backfill_matches_legacy_agent_id_key() {
        // Blocks from older builds stamp the legacy `agent:id` meta key instead
        // of `agentId`; the backfill must still recover them. (reagent P1 #1403.)
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let tdir = home.join("shared").join("agents").join("transcripts");
        std::fs::create_dir_all(&tdir).unwrap();
        let def_id = "deadbeef-0000-1111-2222-333344445555";
        seed_channel_keyed(home, "legacy-ch", "agent:id", def_id, "blk-legacy", b"{\"type\":\"system\"}\n{\"type\":\"result\"}\n");

        let global = Arc::new(FileStore::open(&tdir.join("filestore.db")).unwrap());
        let stats = backfill_transcripts_once(home, &tdir, &[def_id.to_string()], &global);
        assert_eq!(stats.seeded, 1, "legacy agent:id block must be recovered");
        let out = global.read_file(&agent_current_zone(def_id), OUT).unwrap().unwrap();
        assert!(out.starts_with(b"{\"type\":\"system\"}"));
    }

    #[test]
    fn backfill_does_not_clobber_richer_global_zone() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let tdir = home.join("shared").join("agents").join("transcripts");
        std::fs::create_dir_all(&tdir).unwrap();
        let def_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        seed_channel(home, "verify-a", def_id, "blk1", b"{\"type\":\"system\"}\n");

        // Global already holds MORE than the channel source → must not clobber.
        let global = Arc::new(FileStore::open(&tdir.join("filestore.db")).unwrap());
        let zone = agent_current_zone(def_id);
        global.make_file(&zone, OUT, FileMeta::default(), FileOpts::default()).unwrap();
        global.write_file(&zone, OUT, b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n{\"d\":4}\n").unwrap();

        let stats = backfill_transcripts_once(home, &tdir, &[def_id.to_string()], &global);
        assert_eq!(stats.seeded, 0);
        assert_eq!(stats.skipped_global_richer, 1);
        let out = global.read_file(&zone, OUT).unwrap().unwrap();
        assert!(out.starts_with(b"{\"a\":1}"), "richer global content preserved");
    }
}

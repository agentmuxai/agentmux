// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `cross_channel_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;
use crate::backend::agent_session::OUTPUT_FILE;
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

fn mem_store() -> Arc<FileStore> {
    Arc::new(FileStore::open_in_memory().unwrap())
}

fn seed_output(fs: &Arc<FileStore>, zone: &str, body: &[u8]) {
    fs.make_file(zone, OUTPUT_FILE, FileMeta::default(), FileOpts::default())
        .unwrap();
    fs.append_data(zone, OUTPUT_FILE, body).unwrap();
}

fn insert_agent_block(mstore: &Arc<Store>, def_id: &str) -> String {
    let oid = uuid::Uuid::new_v4().to_string();
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("agent"));
    meta.insert("agentId".to_string(), serde_json::json!(def_id));
    let mut block = Block {
        oid: oid.clone(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta,
        subblockids: None,
    };
    mstore.insert(&mut block).expect("insert block");
    oid
}

#[test]
fn global_output_source_falls_back_when_local_empty() {
    let per_channel = mem_store();
    let global = mem_store();
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let block_id = insert_agent_block(&mstore, "def-cc-1");

    // No local output for the block, but the global zone has content.
    seed_output(&global, "agent:def-cc-1:current", b"{\"type\":\"user\"}\n");

    let resolved = global_output_source(
        &per_channel,
        &Some(global.clone()),
        &mstore,
        &block_id,
        "output",
    );
    let (_store, zone) = resolved.expect("should fall back to global");
    assert_eq!(zone, "agent:def-cc-1:current");
}

#[test]
fn global_output_source_prefers_global_even_when_local_present() {
    // After the Bug-1 fix: even when the local output is non-empty (current
    // session started writing), the global zone is still returned so that
    // cross-channel history load sees the FULL record, not just the current
    // session's lines.
    let per_channel = mem_store();
    let global = mem_store();
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let block_id = insert_agent_block(&mstore, "def-cc-2");

    seed_output(&per_channel, &block_id, b"{\"type\":\"local\"}\n");
    seed_output(&global, "agent:def-cc-2:current", b"{\"type\":\"global\"}\n");

    let resolved = global_output_source(
        &per_channel,
        &Some(global.clone()),
        &mstore,
        &block_id,
        "output",
    );
    let (_, zone) = resolved.expect("global always preferred when available");
    assert_eq!(zone, "agent:def-cc-2:current");
}

#[test]
fn global_output_source_only_for_output_and_with_global_store() {
    let per_channel = mem_store();
    let global = mem_store();
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let block_id = insert_agent_block(&mstore, "def-cc-3");
    seed_output(&global, "agent:def-cc-3:current", b"{\"x\":1}\n");

    // Non-"output" filename is never globalized.
    assert!(global_output_source(&per_channel, &Some(global.clone()), &mstore, &block_id, "term").is_none());
    // No global store configured → None.
    assert!(global_output_source(&per_channel, &None, &mstore, &block_id, "output").is_none());
    // Non-agent block id → None.
    assert!(global_output_source(&per_channel, &Some(global), &mstore, "not-a-block", "output").is_none());
}

#[test]
fn global_output_source_suppressed_for_archived_block() {
    // A block archived via the UI/sweep (`session:archived_at` set, local
    // output deleted) must NOT resurrect from the global mirror — it should
    // reopen archived/empty as pre-PR. (reagent P1 #1399.)
    let per_channel = mem_store();
    let global = mem_store();
    let mstore = Arc::new(Store::open_in_memory().unwrap());

    let oid = uuid::Uuid::new_v4().to_string();
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("agent"));
    meta.insert("agentId".to_string(), serde_json::json!("def-cc-arch"));
    meta.insert(
        crate::backend::session_archive::META_SESSION_ARCHIVED_AT.to_string(),
        serde_json::json!(1_700_000_000_000i64),
    );
    let mut block = Block {
        oid: oid.clone(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta,
        subblockids: None,
    };
    mstore.insert(&mut block).expect("insert block");

    // Global zone has content, but the block is archived → no fallback.
    seed_output(&global, "agent:def-cc-arch:current", b"{\"type\":\"user\"}\n");
    assert!(
        global_output_source(&per_channel, &Some(global), &mstore, &oid, "output").is_none(),
        "archived block must not fall back to the global mirror",
    );
}

#[test]
fn global_zone_line_count_counts_non_blank_lines() {
    let global = mem_store();
    let zone = "agent:def-cc-4:current";
    seed_output(&global, zone, b"{\"a\":1}\n{\"b\":2}\n\n{\"c\":3}\n");
    // 3 non-blank NDJSON lines (the blank line is ignored, matching read_range).
    assert_eq!(global_zone_line_count(&global, zone), Some(3));

    // Empty / absent zone → Some(0) / None respectively.
    let empty_zone = "agent:def-empty:current";
    assert_eq!(global_zone_line_count(&global, empty_zone), None);
}

#[test]
fn global_zone_line_count_reuses_fresh_index_instead_of_rescanning() {
    // Same fresh-index reuse the read_range path already does
    // (blockfile.rs) — a header whose covered_size matches the current
    // `output` size must be trusted as-is, not rebuilt from a full scan.
    // Proven here by seeding a deliberately-wrong-but-size-matching index
    // (2 entries) alongside real `output` content that would scan to 3
    // non-blank lines: a fix that trusts the fresh header returns the
    // cached 2; the old always-rebuild code returns the rescanned 3.
    let global = mem_store();
    let zone = "agent:def-cc-5:current";
    let body: &[u8] = b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n";
    seed_output(&global, zone, body);

    let mut idx = Vec::new();
    idx.extend_from_slice(&(body.len() as u64).to_le_bytes()); // covered_size == output size
    idx.extend_from_slice(&0u64.to_le_bytes()); // fabricated entry 0
    idx.extend_from_slice(&9u64.to_le_bytes()); // fabricated entry 1 (only 2, not 3)
    // Labelled with `output`'s current generation, as the builder labels
    // a real index (5a-2b) — so it is trusted as fresh.
    let gen = global.line_state(zone, OUTPUT_FILE).unwrap().unwrap().counted.unwrap().gen;
    let mut label = FileMeta::default();
    label.insert("for_gen".into(), serde_json::json!(gen));
    global.put_file_with_meta(zone, "output.idx", &idx, label).unwrap();

    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(2),
        "must trust the fresh cached index rather than rescanning `output`",
    );
}

#[test]
fn global_zone_line_count_is_exact_when_another_instance_rebuilt_the_index() {
    // Codex P1 on #3634: store A holds a stale cached `output.idx` row.
    // Another srv instance (store B, same database file) appends and
    // rebuilds the index. A's output size comes from the database; the
    // index size must too, or A under-reports the count.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("global.db");
    let a = Arc::new(FileStore::open(&path).unwrap());
    let b = Arc::new(FileStore::open(&path).unwrap());
    let zone = "agent:def-cc-11:current";
    seed_output(&a, zone, b"{\"a\":1}\n");
    assert_eq!(global_zone_line_count(&a, zone), Some(1));
    // A caches a row for the index (as an older build's make_file would).
    a.write_file(zone, "output.idx", &[0u8; 16]).ok();
    a.make_file(zone, "output.idx", FileMeta::default(), FileOpts::default()).ok();
    let _ = a.stat(zone, "output.idx");

    b.append_data(zone, OUTPUT_FILE, b"{\"b\":2}\n{\"c\":3}\n{\"d\":4}\n").unwrap();
    let b_size = b.line_state(zone, OUTPUT_FILE).unwrap().unwrap().size as u64;
    assert_eq!(crate::backend::blockcontroller::shell::rebuild_output_idx(&b, zone, b_size, crate::backend::blockcontroller::shell::output_now(&b, zone).and_then(|(_, g)| g)), Some(4));

    assert_eq!(global_zone_line_count(&a, zone), Some(4), "A must not count from its stale cached index size");
}

#[test]
fn global_zone_line_count_does_not_trust_an_index_of_replaced_content_of_the_same_size() {
    // The coincidence the covered_size check alone can't see: `output`
    // replaced (restore, backfill) by different content of exactly the
    // same byte size. The old index then "matches" and serves the old
    // file's count and offsets. Since 5a-2b the index is labelled with
    // the generation it was built for, and a replace mints a new one.
    let global = mem_store();
    let zone = "agent:def-cc-10:current";
    let three: &[u8] = b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n";
    let two: &[u8] = b"{\"x\":11111}\n{\"y\":22222}\n";
    assert_eq!(three.len(), two.len(), "precondition: same size");
    seed_output(&global, zone, three);
    assert_eq!(global_zone_line_count(&global, zone), Some(3));

    replace_output(&global, zone, two);
    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(2),
        "an index built for another generation of `output` must be rebuilt",
    );
}

/// `seed_output` creates the file, so it can only be called once per zone.
/// These tests need to grow (and shrink) an existing `output`.
fn append_output(fs: &Arc<FileStore>, zone: &str, body: &[u8]) {
    fs.append_data(zone, OUTPUT_FILE, body).unwrap();
}

fn replace_output(fs: &Arc<FileStore>, zone: &str, body: &[u8]) {
    fs.write_file(zone, OUTPUT_FILE, body).unwrap();
}

#[test]
fn global_zone_line_count_is_exact_after_an_append() {
    // The count MUST stay exact. Returning a stale index's count was the
    // first attempt at this fix and is wrong: the count feeds
    // useHistoryPagination's tail window (offset = total - PAGE_SIZE), so
    // an undercount silently drops the newest history on reopen (codex P1
    // on PR #2838). Only the cost of exactness was negotiable.
    let global = mem_store();
    let zone = "agent:def-cc-6:current";
    seed_output(&global, zone, b"{\"a\":1}\n{\"b\":2}\n");
    assert_eq!(global_zone_line_count(&global, zone), Some(2));

    append_output(&global, zone, b"{\"c\":3}\n{\"d\":4}\n");
    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(4),
        "an append must be reflected exactly, not answered from the stale index",
    );
}

#[test]
fn global_zone_line_count_handles_a_line_completed_after_the_previous_build() {
    // The boundary case that makes a naive "scan from covered_size"
    // incremental index wrong. When the previous build ran the file ended
    // MID-LINE (no trailing newline), so that partial line already has an
    // index entry. Bytes appended since continue that same line rather
    // than starting a new one — so the extend re-scans from the last
    // entry's offset and re-derives it instead of treating the
    // continuation as new. Getting this wrong double-counts the
    // straddling line (5 instead of 4 here).
    let global = mem_store();
    let zone = "agent:def-cc-8:current";
    seed_output(&global, zone, b"{\"a\":1}\n{\"b\":2}\n{\"par");
    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(3),
        "the partial trailing line is itself indexed",
    );

    append_output(&global, zone, b"tial\":3}\n{\"d\":4}\n");
    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(4),
        "the completed line must not be counted twice",
    );
}

#[test]
fn global_zone_line_count_rebuilds_when_output_shrank() {
    // Rotation/truncation: the existing index covers more than the file
    // now holds, so it can't be used as a base at all.
    let global = mem_store();
    let zone = "agent:def-cc-9:current";
    seed_output(&global, zone, b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n{\"d\":4}\n");
    assert_eq!(global_zone_line_count(&global, zone), Some(4));

    replace_output(&global, zone, b"{\"z\":9}\n");
    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(1),
        "a shrunken output must be rebuilt from scratch, not extended",
    );
}

#[test]
fn global_zone_line_count_still_builds_when_no_index_exists() {
    // The one case that must still pay for a build: without it the
    // line_count handler falls through to `session:line_count` (absent for
    // a zone this channel never wrote) and then the capped MPS ring, so a
    // fresh cross-channel open would under-report and render a near-empty
    // pane. Paid once per zone, not once per 30-second poll.
    let global = mem_store();
    let zone = "agent:def-cc-7:current";
    seed_output(&global, zone, b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n");

    assert_eq!(
        global_zone_line_count(&global, zone),
        Some(3),
        "a missing index must still be built so cross-channel opens count correctly",
    );
    // And the build must have persisted an index for subsequent calls.
    let idx = global.stat(zone, "output.idx").expect("stat ok");
    assert!(idx.is_some(), "the build should leave an index behind");
}

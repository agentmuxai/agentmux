// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `blockfile:line_count` / `blockfile:read_range` name the transcript stream
//! and generation they served (Phase 5a-3,
//! SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7).

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

async fn call(state: &AppState, command: &str, data: serde_json::Value) -> serde_json::Value {
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage {
        command: command.to_string(),
        reqid: "req-1".to_string(),
        data: Some(data),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(resp.error.is_empty(), "{command}: {}", resp.error);
    resp.data.expect("result data")
}

fn seed_lines(fs: &FileStore, zone: &str, body: &[u8]) {
    fs.make_file(zone, "output", FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines(zone, "output", body).unwrap();
}

fn gen(fs: &FileStore, zone: &str) -> String {
    fs.line_state(zone, "output").unwrap().unwrap().counted.unwrap().gen
}

fn read(block: &str, offset: u64, expect_gen: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({"block_id": block, "filename": "output", "offset": offset, "limit": 10});
    if let Some(g) = expect_gen {
        v["expect_gen"] = serde_json::json!(g);
    }
    v
}

#[tokio::test]
async fn a_blocks_own_output_is_counted_and_read_with_its_stream_and_generation() {
    let state = crate::server::tests::test_state();
    let block = "blk-rpc-own";
    seed_lines(&state.filestore, block, b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n");
    let g = gen(&state.filestore, block);

    let count = call(&state, COMMAND_BLOCKFILE_LINE_COUNT, serde_json::json!({"block_id": block, "filename": "output"})).await;
    assert_eq!(count, serde_json::json!({"count": 3, "stream": "b:blk-rpc-own", "gen": g}));

    let r = call(&state, COMMAND_BLOCKFILE_READ_RANGE, read(block, 1, None)).await;
    assert_eq!(r["lines"], serde_json::json!(["{\"b\":2}", "{\"c\":3}"]));
    assert_eq!(r["total"], serde_json::json!(3));
    assert_eq!(r["stream"], serde_json::json!("b:blk-rpc-own"));
    assert_eq!(r["gen"], serde_json::json!(g));
    assert!(r.get("gen_mismatch").is_none());
}

#[tokio::test]
async fn a_read_expecting_a_replaced_generation_gets_a_mismatch_not_other_lines() {
    let state = crate::server::tests::test_state();
    let block = "blk-rpc-replaced";
    seed_lines(&state.filestore, block, b"{\"old\":1}\n{\"old\":2}\n");
    let old = gen(&state.filestore, block);
    state.filestore.write_file(block, "output", b"{\"new\":1}\n").unwrap();
    let new = gen(&state.filestore, block);
    assert_ne!(old, new);

    let stale = call(&state, COMMAND_BLOCKFILE_READ_RANGE, read(block, 0, Some(&old))).await;
    assert_eq!(stale["gen_mismatch"], serde_json::json!(true));
    assert_eq!(stale["lines"], serde_json::json!([]));
    assert_eq!(stale["gen"], serde_json::json!(new));

    let current = call(&state, COMMAND_BLOCKFILE_READ_RANGE, read(block, 0, Some(&new))).await;
    assert_eq!(current["lines"], serde_json::json!(["{\"new\":1}"]));
    assert!(current.get("gen_mismatch").is_none());
}

#[tokio::test]
async fn an_uncounted_output_gets_an_epoch_on_its_first_count() {
    let state = crate::server::tests::test_state();
    let block = "blk-rpc-legacy";
    seed_lines(&state.filestore, block, b"{\"a\":1}\n{\"b\":2}\n");
    // As if written by a build without the counter.
    state.filestore.conn().lock().unwrap().execute("UPDATE db_wave_file SET modts = modts + 1", []).unwrap();
    assert_eq!(state.filestore.line_state(block, "output").unwrap().unwrap().counted, None);

    let count = call(&state, COMMAND_BLOCKFILE_LINE_COUNT, serde_json::json!({"block_id": block, "filename": "output"})).await;
    assert_eq!(count["count"], serde_json::json!(2));
    assert_eq!(count["gen"], serde_json::json!(gen(&state.filestore, block)));
}

#[tokio::test]
async fn the_agents_global_zone_is_named_as_its_own_stream() {
    let mut state = crate::server::tests::test_state();
    let gfs = Arc::new(FileStore::open_in_memory().unwrap());
    state.global_transcript_store = Some(gfs.clone());
    let block_id = uuid::Uuid::new_v4().to_string();
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert("view".to_string(), serde_json::json!("agent"));
    meta.insert("agentId".to_string(), serde_json::json!("def-rpc-g"));
    let mut block = Block {
        oid: block_id.clone(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta,
        subblockids: None,
    };
    state.mstore.insert(&mut block).unwrap();
    let zone = "agent:def-rpc-g:current";
    seed_lines(&gfs, zone, b"{\"x\":1}\n{\"y\":2}\n");

    let count = call(&state, COMMAND_BLOCKFILE_LINE_COUNT, serde_json::json!({"block_id": block_id, "filename": "output"})).await;
    assert_eq!(count, serde_json::json!({"count": 2, "stream": format!("g:{zone}"), "gen": gen(&gfs, zone)}));

    let r = call(&state, COMMAND_BLOCKFILE_READ_RANGE, read(&block_id, 0, None)).await;
    assert_eq!(r["stream"], serde_json::json!(format!("g:{zone}")));
    assert_eq!(r["lines"], serde_json::json!(["{\"x\":1}", "{\"y\":2}"]));
}

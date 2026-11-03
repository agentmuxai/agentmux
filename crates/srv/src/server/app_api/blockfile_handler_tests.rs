// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `blockfile:line_count`, `blockfile:read_state` and `blockfile:write_state` handlers, called without
//! the RPC engine.

use super::*;
use crate::server::tests::test_state;

fn add_block(state: &AppState, meta: &[(&str, serde_json::Value)]) -> String {
    let oid = uuid::Uuid::new_v4().to_string();
    let mut block = Block { oid: oid.clone(), ..Default::default() };
    for (k, v) in meta {
        block.meta.insert(k.to_string(), v.clone());
    }
    state.mstore.insert(&mut block).unwrap();
    oid
}

fn count_req(block_id: &str, filename: &str) -> CommandBlockfileLineCountData {
    CommandBlockfileLineCountData { block_id: block_id.into(), filename: filename.into(), archive_zone: None }
}

fn write_req(block_id: &str, filename: &str, content: &str) -> CommandBlockfileWriteStateData {
    CommandBlockfileWriteStateData { block_id: block_id.into(), filename: filename.into(), content: content.into() }
}

fn read_req(block_id: &str, filename: &str) -> CommandBlockfileReadStateData {
    CommandBlockfileReadStateData { block_id: block_id.into(), filename: filename.into() }
}

#[tokio::test]
async fn a_block_with_the_session_line_count_meta_reports_it() {
    let state = test_state();
    let block_id = add_block(&state, &[("session:line_count", serde_json::json!(7))]);
    let result = blockfile_line_count(&state, count_req(&block_id, "output")).await.unwrap();
    assert_eq!(result.count, 7);
}

#[tokio::test]
async fn a_file_nothing_was_written_to_counts_zero() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    let result = blockfile_line_count(&state, count_req(&block_id, "notes.txt")).await.unwrap();
    assert_eq!(result.count, 0);
}

#[tokio::test]
async fn an_archive_zone_that_is_not_the_agents_own_is_refused() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    let mut req = count_req(&block_id, "output");
    req.archive_zone = Some("agent:someone-else:archive:123".into());
    let err = blockfile_line_count(&state, req).await.unwrap_err();
    assert!(err.contains("is not an archived conversation of this block's agent"), "{err}");
}

#[tokio::test]
async fn a_state_file_reads_back_what_was_written() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    assert_eq!(blockfile_write_state(&state.filestore, write_req(&block_id, "pane.json", "{\"a\":1}")).unwrap().bytes_written, 7);
    assert_eq!(blockfile_read_state(&state.filestore, read_req(&block_id, "pane.json")).unwrap().content.as_deref(), Some("{\"a\":1}"));
    // A second write replaces the first.
    blockfile_write_state(&state.filestore, write_req(&block_id, "pane.json", "{}")).unwrap();
    assert_eq!(blockfile_read_state(&state.filestore, read_req(&block_id, "pane.json")).unwrap().content.as_deref(), Some("{}"));
}

#[tokio::test]
async fn a_state_file_that_was_never_written_reads_as_none() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    assert!(blockfile_read_state(&state.filestore, read_req(&block_id, "missing.json")).unwrap().content.is_none());
}

#[tokio::test]
async fn a_state_filename_with_a_path_in_it_is_refused_both_ways() {
    let state = test_state();
    for name in ["a/b", "a\\b", "..", "x..y"] {
        let read = blockfile_read_state(&state.filestore, read_req("b", name)).err().unwrap();
        assert_eq!(read, "blockfile:read_state: filename must not contain path separators", "{name}");
        let write = blockfile_write_state(&state.filestore, write_req("b", name, "x")).err().unwrap();
        assert_eq!(write, "blockfile:write_state: filename must not contain path separators", "{name}");
    }
}

#[tokio::test]
async fn the_transcript_and_its_sidecars_cannot_be_written_as_state() {
    let state = test_state();
    for name in ["output", "output.idx", "output.tsidx"] {
        let err = blockfile_write_state(&state.filestore, write_req("b", name, "x")).err().unwrap();
        assert!(err.contains("is a transcript file and can't be written through this RPC"), "{name}: {err}");
    }
}

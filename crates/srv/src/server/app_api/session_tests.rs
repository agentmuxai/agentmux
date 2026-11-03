// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `session:*` handlers, called without the RPC engine.

use std::io::Write;

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};
use crate::server::tests::test_state;

const OUTPUT: &str = "output";

fn add_block(state: &AppState, meta: &[(&str, serde_json::Value)]) -> String {
    let oid = uuid::Uuid::new_v4().to_string();
    let mut block = Block { oid: oid.clone(), ..Default::default() };
    for (k, v) in meta {
        block.meta.insert(k.to_string(), v.clone());
    }
    state.mstore.insert(&mut block).unwrap();
    oid
}

fn write_output(state: &AppState, block_id: &str, data: &[u8]) {
    state.filestore.make_file(block_id, OUTPUT, FileMeta::default(), FileOpts::default()).unwrap();
    state.filestore.append_data(block_id, OUTPUT, data).unwrap();
}

fn gz_file(dir: &std::path::Path, data: &[u8]) -> std::path::PathBuf {
    let path = dir.join("block.gz");
    let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&path).unwrap(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap();
    path
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("agentmux-session-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn export_returns_the_pane_transcript_encoded_with_its_line_and_byte_counts() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    write_output(&state, &block_id, b"one\n\ntwo\n");

    let result = export_session(&state, CommandSessionExportData { block_id }).await.unwrap();
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(result.content).unwrap(), b"one\n\ntwo\n");
    assert_eq!(result.line_count, 2, "blank lines are not counted");
    assert_eq!(result.byte_count, 9);
}

#[tokio::test]
async fn export_of_a_block_that_does_not_exist_is_an_error() {
    let state = test_state();
    let err = export_session(&state, CommandSessionExportData { block_id: "no-such-block".into() }).await.unwrap_err();
    assert!(err.contains("BLOCK_NOT_FOUND") || err.contains("mstore.get"), "{err}");
}

#[tokio::test]
async fn archive_and_restore_are_refused_for_an_agent_that_does_not_own_the_pane() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    let stranger = RpcContext { agent_id: "someone-else".into(), ..Default::default() };

    let err = archive_session(&state, &stranger, CommandSessionArchiveData { block_id: block_id.clone() }).await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
    let err = restore_session(&state, &stranger, CommandSessionRestoreData { block_id }).await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
}

#[tokio::test]
async fn archive_and_restore_are_refused_for_a_connection_that_is_not_the_window() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    let window_less = RpcContext::default();
    let err = archive_session(&state, &window_less, CommandSessionArchiveData { block_id: block_id.clone() }).await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
    let err = restore_session(&state, &window_less, CommandSessionRestoreData { block_id }).await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");
}

#[tokio::test]
async fn restore_puts_the_archived_transcript_back_and_reports_its_size() {
    let state = test_state();
    let dir = scratch("restore");
    let archive = gz_file(&dir, b"first\nsecond\n");
    let block_id = add_block(&state, &[("session:archive_path", serde_json::json!(archive.to_string_lossy()))]);

    let result = restore_session(&state, &RpcContext::operator_for_tests(), CommandSessionRestoreData { block_id: block_id.clone() })
        .await
        .unwrap();
    assert_eq!((result.block_id.as_str(), result.restored_bytes), (block_id.as_str(), 13));
    let (bytes, lines) = crate::backend::session_archive::read_session_output(&state.mstore, &state.filestore, &block_id).unwrap();
    assert_eq!((bytes.as_slice(), lines), (b"first\nsecond\n".as_slice(), 2));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn restore_without_an_archive_path_says_which_block_has_none() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    let err = restore_session(&state, &RpcContext::operator_for_tests(), CommandSessionRestoreData { block_id: block_id.clone() })
        .await
        .unwrap_err();
    assert!(err.contains("session:archive_path not set") && err.contains(&block_id), "{err}");
}

#[tokio::test]
async fn resume_preflight_of_a_block_that_does_not_exist_names_the_command() {
    let state = test_state();
    let err = resume_preflight(&state, CommandSessionResumePreflightData { block_id: "no-such-block".into() }).await.unwrap_err();
    assert!(err.starts_with("session:resume_preflight: "), "{err}");
}

#[tokio::test]
async fn resume_preflight_of_a_pane_with_no_session_answers_with_a_verdict_and_its_steps() {
    let state = test_state();
    let block_id = add_block(&state, &[]);
    let result = resume_preflight(&state, CommandSessionResumePreflightData { block_id: block_id.clone() }).await.unwrap();
    assert_eq!(result.block_id, block_id);
    assert!(["resume", "recover", "fresh", "unknown"].contains(&result.verdict.as_str()), "{}", result.verdict);
}

#[tokio::test]
async fn the_next_prompt_suggestion_of_a_block_that_does_not_exist_is_an_error_or_empty_never_a_panic() {
    let state = test_state();
    let cmd = CommandNextPromptSuggestionData { block_id: "no-such-block".into(), generation: 1, activity: None };
    match next_prompt_suggestion(&state, cmd).await {
        Ok(result) => assert!(result.suggestion.is_empty()),
        Err(err) => assert!(err.contains("BLOCK_NOT_FOUND"), "{err}"),
    }
}

#[tokio::test]
async fn the_activity_summary_of_a_block_that_does_not_exist_is_an_error_or_empty_never_a_panic() {
    let state = test_state();
    let cmd = CommandActivitySummaryData { block_id: "no-such-block".into(), word_target: None, generation: 1, user_message: Some("hi".into()) };
    match activity_summary(&state, cmd).await {
        Ok(result) => assert!(result.summary.is_empty()),
        Err(err) => assert!(err.contains("BLOCK_NOT_FOUND"), "{err}"),
    }
}

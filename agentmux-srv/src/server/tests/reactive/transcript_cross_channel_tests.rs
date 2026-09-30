// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `transcript_cross_channel_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use crate::server::tests::test_state;

/// The regression this test guards: before Phase A
/// (`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`),
/// an unknown agent name always 404d straight out of the host-tier
/// lookup. The refactor routes that miss through
/// `handle_reactive_transcript_cross_channel` instead — this proves the
/// common case (nothing found on any tier, which is what a fresh
/// `test_state()` with no shared registry entries looks like) still
/// ends in the same 404, not a panic or a wrongly-200'd empty body.
#[tokio::test]
async fn unknown_agent_still_404s_when_not_on_any_tier() {
    let state = test_state();
    let resp = handle_reactive_transcript(
        State(state),
        Query(TranscriptQuery {
            agent: "no-such-agent-anywhere".to_string(),
            max_lines: 100,
            forwarded: false,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn empty_agent_param_is_bad_request_before_any_tier_lookup() {
    let state = test_state();
    let resp = handle_reactive_transcript(
        State(state),
        Query(TranscriptQuery {
            agent: String::new(),
            max_lines: 100,
            forwarded: false,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn host_tier_hit_is_labeled_with_its_tier() {
    let state = test_state();
    let unique = uuid::Uuid::new_v4();
    let agent_id = format!("transcript-tier-test-{unique}");
    let block_id = format!("transcript-tier-block-{unique}");
    state
        .reactive_handler
        .register_agent(&agent_id, &block_id, None)
        .unwrap();
    state
        .filestore
        .make_file(
            &block_id,
            "output",
            crate::backend::storage::filestore::FileMeta::default(),
            crate::backend::storage::filestore::FileOpts::default(),
        )
        .expect("make_file");
    state
        .filestore
        .append_data(&block_id, "output", b"hello\n")
        .expect("append_data");
    let mut block = crate::backend::obj::Block {
        oid: block_id.clone(),
        parentoref: String::new(),
        version: 1,
        runtimeopts: None,
        stickers: None,
        meta: Default::default(),
        subblockids: None,
    };
    state.mstore.insert(&mut block).expect("mstore insert");

    let resp = handle_reactive_transcript(
        State(state),
        Query(TranscriptQuery {
            agent: agent_id,
            max_lines: 100,
            forwarded: false,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["tier"], "host");
}

/// The P1 regression this guards (reagent + codex on PR #2715): a
/// forwarded request (`forwarded: true`) must 404 immediately on a
/// host-tier miss, never attempt a second cross-channel lookup/forward
/// — that's what caps a stale/cyclic shared-registry entry at exactly
/// one hop instead of looping (self-forward) or chaining indefinitely
/// (multi-instance cycle).
#[tokio::test]
async fn forwarded_request_404s_without_a_second_hop() {
    let state = test_state();
    let resp = handle_reactive_transcript(
        State(state),
        Query(TranscriptQuery {
            agent: "no-such-agent-anywhere".to_string(),
            max_lines: 100,
            forwarded: true,
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

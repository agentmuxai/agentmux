// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `req_shape_tests`, moved out of server/native_memory_handlers.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;
use serde_json::json;

// agent-native-memory-model.ts:128
#[test]
fn list_req_accepts_the_payload_the_stub_sends() {
    serde_json::from_value::<CommandNativeMemoryListData>(json!({"agent_id": "a1"}))
        .expect("agent:memory:list must accept {agent_id}");
}

// agent-native-memory-model.ts:167, native-memory-history-model.ts:176
#[test]
fn read_file_req_accepts_the_payload_the_stub_sends() {
    serde_json::from_value::<CommandNativeMemoryReadFileData>(
        json!({"agent_id": "a1", "filename": "MEMORY.md"}),
    )
    .expect("agent:memory:read_file must accept {agent_id, filename}");
}

// agent-native-memory-model.ts:201 and :237 — BOTH call sites send
// `provenance: { source: "human" }` and neither sends `detail`, so the
// nested `default_detail` path is the only one the UI ever exercises.
#[test]
fn write_file_req_accepts_provenance_without_detail() {
    let req: CommandNativeMemoryWriteFileData = serde_json::from_value(json!({
        "agent_id": "a1",
        "filename": "MEMORY.md",
        "content": "hello",
        "provenance": {"source": "human"},
    }))
    .expect("agent:memory:write_file must accept a detail-less provenance");
    let prov = req.provenance.expect("provenance should round-trip");
    assert_eq!(prov.source, "human");
    // `default_detail` must yield `{}`, not `null` — a null here would be
    // written into the version row's source_detail as the string "null".
    assert_eq!(prov.detail, json!({}));
}

// `provenance` is `Option<_>` and generated as `provenance?`, so an omitted
// key must parse even though no current caller omits it.
#[test]
fn write_file_req_accepts_an_omitted_provenance() {
    let req: CommandNativeMemoryWriteFileData = serde_json::from_value(json!({
        "agent_id": "a1",
        "filename": "MEMORY.md",
        "content": "hello",
    }))
    .expect("agent:memory:write_file must accept an omitted provenance");
    assert!(req.provenance.is_none());
    assert!(req.base_sha256.is_none(), "an omitted base means an unconditional write");
}

// The Armory / Stash editors send the hash of the content their draft
// started from (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4).
#[test]
fn write_file_req_accepts_a_base_sha256() {
    let req: CommandNativeMemoryWriteFileData = serde_json::from_value(json!({
        "agent_id": "a1",
        "filename": "MEMORY.md",
        "content": "hello",
        "provenance": {"source": "human"},
        "base_sha256": "abc123",
    }))
    .expect("agent:memory:write_file must accept base_sha256");
    assert_eq!(req.base_sha256.as_deref(), Some("abc123"));
}

// native-memory-history-model.ts:196
#[test]
fn history_req_accepts_the_payload_the_stub_sends() {
    serde_json::from_value::<CommandNativeMemoryHistoryData>(
        json!({"agent_id": "a1", "filename": "MEMORY.md"}),
    )
    .expect("agent:memory:history must accept {agent_id, filename}");
}

// native-memory-history-model.ts:242. `agent_id` is required here on
// purpose (reagent P1: without it any caller could read another agent's
// memory by version id), so a payload missing it must be REJECTED.
#[test]
fn diff_req_accepts_the_payload_the_stub_sends_and_requires_agent_id() {
    serde_json::from_value::<CommandNativeMemoryDiffData>(
        json!({"agent_id": "a1", "from_version_id": "v1", "to_version_id": "v2"}),
    )
    .expect("agent:memory:diff must accept {agent_id, from_version_id, to_version_id}");
    assert!(
        serde_json::from_value::<CommandNativeMemoryDiffData>(
            json!({"from_version_id": "v1", "to_version_id": "v2"}),
        )
        .is_err(),
        "agent:memory:diff must reject a payload with no agent_id — that is the              ownership check the P1 fix added"
    );
}

// native-memory-history-model.ts:270
#[test]
fn revert_req_accepts_the_payload_the_stub_sends() {
    serde_json::from_value::<CommandNativeMemoryRevertData>(
        json!({"agent_id": "a1", "filename": "MEMORY.md", "target_version_id": "v1"}),
    )
    .expect("agent:memory:revert must accept {agent_id, filename, target_version_id}");
}

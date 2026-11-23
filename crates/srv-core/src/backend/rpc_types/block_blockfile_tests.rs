// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Request-shape tests for the four `blockfile:*` commands.
//
// Nothing else catches a Req/payload mismatch: `tsc` only checks the frontend
// against the GENERATED types, and `scripts/check-rpc-bindings.sh` only checks
// that a generated type exists per command and is current. Neither ever
// deserializes a real payload into the Rust struct.

use super::*;
use serde_json::json;

// AgentHistoryView.tsx's `load`, useHistoryPagination.ts's
// `needsHwmWidening` probe
#[test]
fn line_count_accepts_the_payload_the_stub_sends() {
    serde_json::from_value::<CommandBlockfileLineCountData>(
        json!({"block_id": "b1", "filename": "output"}),
    )
    .expect("blockfile:line_count must accept block_id and filename");
}

// AgentHistoryView.tsx's `load`
#[test]
fn read_range_accepts_the_payload_the_stub_sends() {
    let r: CommandBlockfileReadRangeData = serde_json::from_value(
        json!({"block_id": "b1", "filename": "output", "offset": 0, "limit": 500}),
    )
    .expect("blockfile:read_range must accept the full range payload");
    assert_eq!((r.offset, r.limit), (0, 500));
}

// `offset`/`limit` are u64 carrying JS numbers, which is why the generated
// binding overrides them to `number` rather than `bigint`. A negative
// offset must be rejected rather than wrapping to a huge positive.
#[test]
fn read_range_rejects_a_negative_offset() {
    assert!(
        serde_json::from_value::<CommandBlockfileReadRangeData>(
            json!({"block_id": "b1", "filename": "output", "offset": -1, "limit": 10}),
        )
        .is_err(),
        "a negative offset must fail loudly, not wrap into a huge u64"
    );
}

#[test]
fn the_state_commands_accept_their_payloads() {
    serde_json::from_value::<CommandBlockfileReadStateData>(
        json!({"block_id": "b1", "filename": "output.state.json"}),
    )
    .expect("blockfile:read_state");
    serde_json::from_value::<CommandBlockfileWriteStateData>(
        json!({"block_id": "b1", "filename": "output.state.json", "content": "{}"}),
    )
    .expect("blockfile:write_state");
}

// `stamps` is `skip_serializing_if = "Option::is_none"`, so the key is
// OMITTED rather than nulled when absent -- which is why the generated
// binding says `stamps?: number[]`. The doc comment on the field promises
// old frontends can ignore it and new ones tolerate absence; that promise
// only holds while it is genuinely omitted.
#[test]
fn read_range_result_omits_stamps_rather_than_nulling_them() {
    let without = serde_json::to_value(BlockfileReadRangeResult {
        lines: vec!["a".to_string()],
        total: 1,
        ..Default::default()
    })
    .expect("serializable");
    assert_eq!(without, json!({"lines": ["a"], "total": 1}));

    let with = serde_json::to_value(BlockfileReadRangeResult {
        lines: vec!["a".to_string()],
        total: 1,
        stamps: Some(vec![7]),
        ..Default::default()
    })
    .expect("serializable");
    assert_eq!(with, json!({"lines": ["a"], "total": 1, "stamps": [7]}));

    // The Phase 5a-3 fields follow the same rule: omitted when unset.
    let positioned = serde_json::to_value(BlockfileReadRangeResult {
        lines: vec![],
        total: 3,
        stream: Some("g:agent:x:current".to_string()),
        gen: Some("0123456789abcdef".to_string()),
        gen_mismatch: Some(true),
        ..Default::default()
    })
    .expect("serializable");
    assert_eq!(
        positioned,
        json!({"lines": [], "total": 3, "stream": "g:agent:x:current", "gen": "0123456789abcdef", "gen_mismatch": true})
    );
}

// `content` is a plain `Option<String>` with NO skip_serializing_if, so the
// key is always present and carries null -- a different shape from
// `stamps` above, and the generated binding says `string | null` for
// exactly that reason. The two live side by side in one domain, so pin
// both rather than assuming they behave alike.
#[test]
fn read_state_result_nulls_content_rather_than_omitting_it() {
    let v = serde_json::to_value(BlockfileReadStateResult { content: None })
        .expect("serializable");
    assert_eq!(v, json!({"content": null}));
}

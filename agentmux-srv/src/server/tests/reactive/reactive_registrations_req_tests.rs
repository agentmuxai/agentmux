// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `reactive_registrations_req_tests`, moved out of server/reactive.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;

#[test]
fn params_accept_the_payload_the_stub_sends() {
    // frontend/app/store/rpc-api/reactive.ts sends exactly this shape.
    let sent_by_stub = serde_json::json!({ "agent_id": "manoz" });
    let params: ReactiveRegistrationsParams = serde_json::from_value(sent_by_stub)
        .expect("reactive.registrations must accept { agent_id }");
    assert_eq!(params.agent_id, "manoz");
}

#[test]
fn params_reject_a_missing_agent_id() {
    let empty = serde_json::json!({});
    assert!(serde_json::from_value::<ReactiveRegistrationsParams>(empty).is_err());
}

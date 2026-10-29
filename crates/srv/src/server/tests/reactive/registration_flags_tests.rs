// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `recent_mismatch_for`: which audit line the Swarm's registration badge reports.

use super::registration_flags::recent_mismatch_for;
use crate::backend::reactive::AuditLogEntry;

fn line(timestamp: u64, target: &str, outcome: Option<&str>, block: &str) -> AuditLogEntry {
    serde_json::from_value(serde_json::json!({
        "timestamp": timestamp,
        "target_agent": target,
        "block_id": block,
        "message_hash": "h",
        "message_length": 1,
        "success": false,
        "error_message": "different live identity",
        "request_id": "r",
        "outcome": outcome,
    }))
    .unwrap()
}

#[test]
fn the_newest_identity_refusal_for_the_agent_is_reported_by_name_in_any_case() {
    // The audit log is newest first.
    let audit = vec![
        line(30, "Bob", Some("identity-mismatch"), "b-bob"),
        line(20, "Ann", Some("held_delivered"), "b-held"),
        line(10, "ann", Some("identity-mismatch"), "b-ann"),
    ];
    let found = recent_mismatch_for(&audit, "ANN", None).expect("ann's refusal");
    assert_eq!((found.timestamp, found.block_id.as_str()), (10, "b-ann"));
    assert_eq!(
        found.error_message.as_deref(),
        Some("different live identity")
    );
}

#[test]
fn an_agent_with_no_refusal_has_none() {
    let audit = vec![
        line(20, "ann", None, "b1"),
        line(10, "bob", Some("identity-mismatch"), "b2"),
    ];
    assert!(recent_mismatch_for(&audit, "ann", None).is_none());
    assert!(recent_mismatch_for(&[], "ann", None).is_none());
}

#[test]
fn a_refusal_of_a_message_addressed_by_uid_is_found_through_the_block_it_resolved_to() {
    let audit = vec![
        line(20, "uid-1234", Some("identity-mismatch"), "b-ann"),
        line(10, "bob", Some("identity-mismatch"), "b-bob"),
    ];
    assert!(recent_mismatch_for(&audit, "ann", None).is_none());
    let found = recent_mismatch_for(&audit, "ann", Some("b-ann")).expect("ann's refusal by block");
    assert_eq!(found.timestamp, 20);
}

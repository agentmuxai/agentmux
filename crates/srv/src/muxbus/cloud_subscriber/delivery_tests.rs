// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::{wan_request, PendingInj, PendingResp};
use crate::backend::reactive::types::JektTier;

fn pulled(extra: serde_json::Value) -> PendingInj {
    let mut row = serde_json::json!({
        "id": "inj-1",
        "source_agent": "camper",
        "message": "hello",
        "priority": "normal",
    });
    row.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(row).unwrap()
}

#[test]
fn a_declared_tier_travels_with_a_wan_jekt() {
    for (raw, want) in [("sensitive", JektTier::Sensitive), ("info", JektTier::Info), ("coord", JektTier::Coord)] {
        let req = wan_request(&pulled(serde_json::json!({ "jekt_tier": raw })), "agent2");
        assert_eq!(req.jekt_tier, Some(want), "{raw}");
        assert_eq!(req.delivery_tier.as_deref(), Some("wan"));
        assert_eq!(req.target_agent, "agent2");
        assert_eq!(req.request_id.as_deref(), Some("inj-1"));
    }
}

#[test]
fn an_unknown_tier_is_read_as_none_declared() {
    for raw in [
        serde_json::json!("SENSITIVE"),
        serde_json::json!("urgent"),
        serde_json::json!(""),
        serde_json::json!(null),
    ] {
        let req = wan_request(&pulled(serde_json::json!({ "jekt_tier": raw })), "agent2");
        assert_eq!(req.jekt_tier, None, "{raw}");
    }
}

#[test]
fn a_row_without_a_tier_declares_none() {
    let req = wan_request(&pulled(serde_json::json!({})), "agent2");
    assert_eq!(req.jekt_tier, None);
}

#[test]
fn a_pull_with_fields_this_build_does_not_know_still_parses() {
    let body: PendingResp = serde_json::from_value(serde_json::json!({
        "injections": [{ "id": "inj-1", "message": "hello", "jekt_tier": "sensitive", "some_later_field": 1 }],
        "count": 1,
    }))
    .unwrap();
    assert_eq!(body.injections.len(), 1);
}

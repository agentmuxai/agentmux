// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the fleet-control commands (`fleet.broadcast`,
//! `fleet.bulk-stop`, `fleet.group.*`) — see
//! docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

use serde::{Deserialize, Serialize};

/// One outcome bucket per bulk action, mirroring `ImportAgentDefinitionsResult`'s
/// partial-success shape — the spec's §3 finding that silent partial failure
/// is the single most commonly-cited fleet-ops pitfall means this must never
/// collapse to a single bool/count.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FleetActionResult {
    pub succeeded: Vec<String>,
    pub failed: Vec<FleetActionFailure>,
    /// True only when a staged run's `max_fail_percentage` was crossed and
    /// remaining batches were skipped — lets the frontend show "aborted
    /// early" distinctly from "ran to completion, some failed."
    #[serde(default)]
    pub aborted_early: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FleetActionFailure {
    pub id: String,
    pub error: String,
}

/// Input for fleet.broadcast. `targets` are block ids (Swarm's native
/// per-agent selection unit) — resolved server-side to each block's
/// registered agent name before delivery.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandFleetBroadcastData {
    pub targets: Vec<String>,
    pub message: String,
}

/// Caps blast radius on a bulk-stop: targets are stopped in batches of
/// `batch_size`; if the failure rate within a completed batch exceeds
/// `max_fail_percentage`, remaining batches are skipped. A simplified,
/// fixed-batch-size take on Ansible's `serial` + `max_fail_percentage`
/// (spec §3/§5.3) — not the full canary-then-widen ladder, which is more
/// generality than a first version needs.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct StagePlanInput {
    #[ts(type = "number")]
    pub batch_size: usize,
    pub max_fail_percentage: u8,
}

/// Input for fleet.bulk-stop. `targets` are block ids, same as broadcast.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandFleetBulkStopData {
    pub targets: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub signal: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub staged: Option<StagePlanInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FleetGroup {
    pub id: String,
    pub name: String,
    pub member_ids: Vec<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandFleetGroupCreateData {
    pub name: String,
    #[serde(default)]
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FleetGroupListResult {
    pub groups: Vec<FleetGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandFleetGroupUpdateData {
    pub id: String,
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub member_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandFleetGroupDeleteData {
    pub id: String,
}

/// Input for `fleet.group.list`. The handler ignores its payload entirely, but
/// this must still be a struct rather than `()`: the stub calls it with `{}`
/// (swarm-model.ts:1588), and serde deserializes `()` ONLY from JSON `null`, so
/// a unit Req would reject every real call at runtime while compiling and
/// passing every CI gate. That is the exact bug found on `bookmarks.list`.
/// An empty struct generates `Record<string, never>`, which is what the
/// hand-written stub already declared.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandFleetGroupListData {}

/// Result of `fleet.group.delete`. Previously an ad-hoc
/// `json!({ "ok": deleted })` built inline in the handler — the kind of
/// untyped literal that cannot be checked against the frontend at all.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FleetGroupDeleteResult {
    /// False when no group with that id existed — the delete is idempotent,
    /// so this is "was something actually removed", not an error flag.
    pub ok: bool,
}

// Request-shape tests for the six `fleet.*` commands.
//
// Nothing else catches a Req/payload mismatch: `tsc` only checks the frontend
// against the GENERATED types, and `scripts/check-rpc-bindings.sh` only checks
// that a generated type exists per command and is current. Neither ever
// deserializes a real payload into the Rust struct, so a `Req` that cannot
// parse what the stub sends compiles, typechecks, passes the binding gate, and
// then fails on every call — exactly the `bookmarks.list` bug. Each case below
// is pinned to the literal JSON its call site sends.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    // swarm-model.ts:1556
    #[test]
    fn broadcast_accepts_the_payload_the_stub_sends() {
        let r: CommandFleetBroadcastData =
            serde_json::from_value(json!({"targets": ["b1", "b2"], "message": "go"}))
                .expect("fleet.broadcast must accept targets and message");
        assert_eq!(r.targets.len(), 2);
    }

    // swarm-model.ts:1573 spreads `...opts`, so bulk-stop is called both with
    // and without the two optional fields. Both must parse.
    #[test]
    fn bulk_stop_accepts_the_bare_and_the_fully_staged_payload() {
        let bare: CommandFleetBulkStopData =
            serde_json::from_value(json!({"targets": ["b1"]}))
                .expect("fleet.bulk-stop must accept targets alone");
        assert!(bare.signal.is_none() && bare.staged.is_none());

        let staged: CommandFleetBulkStopData = serde_json::from_value(json!({
            "targets": ["b1", "b2"],
            "signal": "SIGTERM",
            "staged": {"batch_size": 2, "max_fail_percentage": 50},
        }))
        .expect("fleet.bulk-stop must accept a full staged payload");
        let plan = staged.staged.expect("staged should round-trip");
        assert_eq!((plan.batch_size, plan.max_fail_percentage), (2, 50));
    }

    // `max_fail_percentage` is a u8. A value past 255 must be REJECTED rather
    // than silently wrapping, since it caps the blast radius of a bulk stop.
    #[test]
    fn bulk_stop_rejects_an_out_of_range_max_fail_percentage() {
        assert!(
            serde_json::from_value::<CommandFleetBulkStopData>(json!({
                "targets": ["b1"],
                "staged": {"batch_size": 1, "max_fail_percentage": 300},
            }))
            .is_err(),
            "a percentage past u8 range must fail loudly, not wrap"
        );
    }

    // swarm-model.ts:1597
    #[test]
    fn group_create_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandFleetGroupCreateData>(
            json!({"name": "g", "member_ids": ["b1"]}),
        )
        .expect("fleet.group.create must accept name and member_ids");
    }

    // swarm-model.ts:1588 calls this with `{}`. This is the case that broke
    // `bookmarks.list`: serde deserializes `()` ONLY from JSON `null`, so a
    // unit Req would reject every real call. The assertion below pins both
    // halves — that the struct accepts `{}`, and that `()` would not have.
    #[test]
    fn group_list_accepts_the_empty_object_the_stub_sends() {
        serde_json::from_value::<CommandFleetGroupListData>(json!({}))
            .expect("fleet.group.list must accept the {} payload the frontend sends");
        assert!(
            serde_json::from_value::<()>(json!({})).is_err(),
            "the unit type must still reject an empty object — this is why the empty struct exists"
        );
    }

    // No frontend caller today, but the command is registered and reachable.
    #[test]
    fn group_update_accepts_a_partial_payload() {
        let only_name: CommandFleetGroupUpdateData =
            serde_json::from_value(json!({"id": "g1", "name": "renamed"}))
                .expect("fleet.group.update must accept an id plus one field");
        assert!(only_name.member_ids.is_none());

        let only_id: CommandFleetGroupUpdateData = serde_json::from_value(json!({"id": "g1"}))
            .expect("both update fields are optional");
        assert!(only_id.name.is_none() && only_id.member_ids.is_none());
    }

    // swarm-model.ts:1602
    #[test]
    fn group_delete_accepts_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandFleetGroupDeleteData>(json!({"id": "g1"}))
            .expect("fleet.group.delete must accept an id");
    }

    // The delete result used to be an inline `json!({"ok": deleted})`. Pin the
    // serialized shape so the new struct cannot drift from what the frontend
    // reads.
    #[test]
    fn group_delete_result_serializes_as_the_frontend_expects() {
        let v = serde_json::to_value(FleetGroupDeleteResult { ok: false }).expect("serializable");
        assert_eq!(v, json!({"ok": false}));
    }
}

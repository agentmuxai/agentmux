// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


//! Bundle command shapes (v7). A Bundle is a db_bundles row; the type names
//! here still carry the pre-rename "Memory" wording because they are wire-
//! adjacent (two generate frontend bindings) and rename in a later phase.



use serde::{Deserialize, Serialize};

// ---- v7 Bundle command shapes ----

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandGetBundleData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandDeleteBundleData {
    pub id: String,
}

/// Response for `deletememory` and `deletesystemmemory` — both take the
/// same request (`CommandDeleteBundleData`, above) and answer with the
/// same shape, so they share this response type too. Was an anonymous
/// `json!({"deleted": ..})` before this type existed to name it for the
/// RPC bindings generator.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DeleteBundleResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandReorderGlobalBundlesData {
    /// Full ordered list of global bundle ids. Each id's `sort_order`
    /// becomes its position in this list.
    pub ids: Vec<String>,
}


/// Input for `listmemories`. The handler ignores its payload, but this must be
/// a struct rather than `()`: the stub defaults the argument to `{}`, and serde
/// deserializes `()` ONLY from JSON `null`, so a unit Req would reject every
/// real call at runtime while passing every CI gate (the `bookmarks.list` bug).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandListBundlesData {}

/// Input for `getclaudeglobalconfig`. Same reasoning as
/// `CommandListBundlesData` — read-only, no parameters, but still not `()`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandGetClaudeGlobalConfigData {}

/// Result of `reorderglobalbrain`. Was an inline `json!({"updated": n})`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ReorderGlobalBundlesResult {
    /// How many rows had their `sort_order` rewritten. `usize` to match
    /// `Store::bundle_reorder`'s own return type rather than casting at the
    /// boundary — the old inline `json!({"updated": updated})` serialized the
    /// same value.
    #[ts(type = "number")]
    pub updated: usize,
}

// Request-shape tests for the eight bundle CRUD commands.
//
// Nothing else catches a Req/payload mismatch: `tsc` only checks the frontend
// against the GENERATED types, and `scripts/check-rpc-bindings.sh` only checks
// that a generated type exists per command and is current. Neither ever
// deserializes a real payload into the Rust struct.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use crate::backend::storage::store::Bundle;
    use serde_json::json;

    // `listmemories` and `getclaudeglobalconfig` both ignore their payload,
    // but the stub defaults the argument to `{}`. serde deserializes `()` ONLY
    // from JSON null, so a unit Req would have rejected every real call while
    // compiling and passing every gate -- the `bookmarks.list` bug.
    #[test]
    fn the_payload_ignoring_commands_accept_the_empty_object_the_stub_sends() {
        serde_json::from_value::<CommandListBundlesData>(json!({})).expect("listmemories");
        serde_json::from_value::<CommandGetClaudeGlobalConfigData>(json!({}))
            .expect("getclaudeglobalconfig");
        assert!(
            serde_json::from_value::<()>(json!({})).is_err(),
            "the unit type still rejects an empty object -- this is why both structs exist"
        );
    }

    #[test]
    fn the_id_only_commands_accept_the_payload_the_stub_sends() {
        serde_json::from_value::<CommandGetBundleData>(json!({"id": "b1"})).expect("getmemory");
        serde_json::from_value::<CommandDeleteBundleData>(json!({"id": "b1"}))
            .expect("deletememory / deletesystemmemory");
        serde_json::from_value::<CommandReorderGlobalBundlesData>(json!({"ids": ["a", "b"]}))
            .expect("reorderglobalbrain");
    }

    // THE POINT OF THIS DOMAIN. `upsertmemory` deserializes straight into
    // `Bundle`, whose `id` and `name` are the only two fields WITHOUT
    // `#[serde(default)]`. The stub typed this request as `Partial<Bundle>`,
    // which told callers every field was omittable -- including `name`, which
    // serde rejects. `BundleUpsertInput` now says id+name required, rest
    // optional, which is exactly what these two assertions pin.
    #[test]
    fn upsert_requires_id_and_name_but_defaults_everything_else() {
        let minimal: Bundle = serde_json::from_value(json!({"id": "b1", "name": "n"}))
            .expect("upsertmemory must accept just id and name");
        assert_eq!(minimal.instructions_by_provider, "{}");
        assert_eq!(minimal.context_files, "[]");
        assert_eq!(minimal.mcp_servers, "[]");
        assert_eq!(minimal.skills, "[]");
        assert_eq!((minimal.created_at, minimal.updated_at, minimal.sort_order), (0, 0, 0));

        assert!(
            serde_json::from_value::<Bundle>(json!({"id": "b1", "description": "d"})).is_err(),
            "a payload with no name must be rejected -- Partial<Bundle> wrongly allowed it"
        );
        assert!(
            serde_json::from_value::<Bundle>(json!({"name": "n"})).is_err(),
            "a payload with no id must be rejected too"
        );
    }

    // `Bundle` is the RESPONSE shape and is all-required in TypeScript because
    // every field is `#[serde(default)]` but none is `skip_serializing_if` --
    // the server never omits one. This pins that, since the generated
    // all-required binding is only correct while it holds.
    #[test]
    fn a_bundle_response_always_carries_every_field() {
        let v = serde_json::to_value(
            serde_json::from_value::<Bundle>(json!({"id": "b1", "name": "n"})).unwrap(),
        )
        .expect("serializable");
        let obj = v.as_object().expect("object");
        for key in [
            "id",
            "name",
            "description",
            "is_blank",
            "is_global",
            "provider",
            "model",
            "instructions",
            "instructions_by_provider",
            "context_files",
            "mcp_servers",
            "skills",
            "sort_order",
            "is_system",
            "created_at",
            "updated_at",
        ] {
            assert!(obj.contains_key(key), "response omitted {key}; the generated Bundle marks it required");
        }
    }

    // Both delete commands answered with an inline `json!({"deleted": ..})`
    // before `DeleteBundleResult` named the shape; reorder still did until
    // this change. Pin both so they cannot drift from what the frontend reads.
    #[test]
    fn the_result_shapes_serialize_as_the_frontend_expects() {
        assert_eq!(
            serde_json::to_value(DeleteBundleResult { deleted: true }).unwrap(),
            json!({"deleted": true})
        );
        assert_eq!(
            serde_json::to_value(ReorderGlobalBundlesResult { updated: 3 }).unwrap(),
            json!({"updated": 3})
        );
    }
}

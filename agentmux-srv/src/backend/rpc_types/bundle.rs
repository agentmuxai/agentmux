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

/// Request for `upsertmemory` / `upsertsystemmemory`: the `Bundle` row itself
/// (flattened, so the wire shape every existing caller sends is unchanged)
/// plus an optional `base_sha256`. When present, the save is refused with a
/// `conflict:` error unless the stored row's `name` + `instructions` still
/// hash to it — `bundle_versions::content_hash`, i.e. SHA-256 of
/// `name + "\0" + instructions`, the same value as the latest version's
/// `content_hash`. Absent = today's unconditional save. See
/// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandUpsertBundleData {
    #[serde(flatten)]
    pub bundle: crate::backend::storage::store::Bundle,
    #[serde(default)]
    #[ts(optional)]
    pub base_sha256: Option<String>,
}

// ---- Global Memory version history for the Armory UI —
// globalmemory:history / diff / revert. The WebSocket counterparts of the
// GlobalMemoryHistory/Diff/Revert MCP tools (#3448), over the same
// db_bundle_versions rows. SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3.

/// One `db_bundle_versions` row's metadata, no `name`/`instructions` —
/// the same fields `bundle_version_meta_json` (app_api) puts on the MCP wire.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GlobalMemoryVersionMeta {
    pub id: String,
    pub content_hash: String,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub source_detail: String,
    /// The trusted writer: an agent's own id, or `"armory-ui"`.
    pub written_by: String,
    pub written_by_uid: String,
    // ts-rs maps 64-bit integers to `bigint`; every consumer treats this as a
    // plain JS number. See BrowserBookmark::created_at (PR #3293).
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandGlobalMemoryHistoryData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GlobalMemoryHistoryResult {
    /// Newest first.
    pub versions: Vec<GlobalMemoryVersionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandGlobalMemoryDiffData {
    /// Both versions must belong to this entry.
    pub id: String,
    pub from_version_id: String,
    pub to_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GlobalMemoryDiffResult {
    /// Same format as `NativeMemoryDiffResult::diff`, preceded by a
    /// `- name:` / `+ name:` pair when the entry was renamed.
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandGlobalMemoryRevertData {
    pub id: String,
    pub target_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GlobalMemoryRevertResult {
    /// The new `source: "revert"` version. `null` only for a system-tier
    /// entry whose revert target is byte-identical to what's stored — that
    /// path deliberately records no version for a no-op (see
    /// `Store::bundle_upsert_system_if_changed`).
    pub version: Option<GlobalMemoryVersionMeta>,
}

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

    // The upsert request wraps `Bundle` with `#[serde(flatten)]` so the
    // optional `base_sha256` can ride alongside without changing what any
    // existing caller sends: a bare Bundle payload still parses, `name`/`id`
    // are still required, and the base is picked out when present.
    #[test]
    fn upsert_request_flattens_the_bundle_and_takes_an_optional_base() {
        let bare: CommandUpsertBundleData = serde_json::from_value(json!({"id": "b1", "name": "n"}))
            .expect("a bare Bundle payload must still parse");
        assert_eq!(bare.bundle.id, "b1");
        assert_eq!(bare.bundle.context_files, "[]");
        assert!(bare.base_sha256.is_none());

        let based: CommandUpsertBundleData = serde_json::from_value(json!({
            "id": "b1", "name": "n", "instructions": "body", "is_global": true, "sort_order": 3,
            "base_sha256": "abc",
        }))
        .expect("base_sha256 rides alongside the Bundle fields");
        assert_eq!(based.base_sha256.as_deref(), Some("abc"));
        assert_eq!(based.bundle.instructions, "body");
        assert!(based.bundle.is_global);
        assert_eq!(based.bundle.sort_order, 3);

        assert!(
            serde_json::from_value::<CommandUpsertBundleData>(json!({"id": "b1"})).is_err(),
            "name is still required through the flatten"
        );
    }

    #[test]
    fn the_global_memory_history_commands_accept_the_payloads_the_stub_sends() {
        serde_json::from_value::<CommandGlobalMemoryHistoryData>(json!({"id": "b1"}))
            .expect("globalmemory:history");
        serde_json::from_value::<CommandGlobalMemoryDiffData>(
            json!({"id": "b1", "from_version_id": "v1", "to_version_id": "v2"}),
        )
        .expect("globalmemory:diff");
        serde_json::from_value::<CommandGlobalMemoryRevertData>(
            json!({"id": "b1", "target_version_id": "v1"}),
        )
        .expect("globalmemory:revert");
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

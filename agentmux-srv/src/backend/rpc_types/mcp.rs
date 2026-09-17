// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the v1 standalone MCP Server primitive (`mcp.*` and
//! `mcp.catalog.*`).
//!
//! Same situation as `rpc_types/skill.rs`: these were function-local anonymous
//! `struct Req` declarations inside each handler closure in
//! `server/app_api/mcp.rs` — seventeen of them — invisible to the bindings
//! generator, so the frontend's inline shapes were hand-maintained against
//! types it could not see.
//!
//! `mcp` and `skill` are the "twin primitives" the 09-06 DRY audit flagged
//! (cause 4). That audit recommended *not* abstracting them into a shared
//! generic, and this file deliberately keeps that separation: the shapes rhyme
//! (`{agent_id, mcp_id}` vs `{agent_id, skill_id}`) but the field NAMES differ,
//! and collapsing them would mean renaming a wire field to serve an
//! abstraction. Sharing within each primitive is real; sharing across them
//! would not be.

use serde::{Deserialize, Serialize};

/// `mcp.list`, `mcp.catalog.list_for_agent`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpAgentScopeData {
    pub agent_id: String,
}

/// `mcp.get`, `mcp.delete`, `mcp.probe` — one server row within an agent.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpAgentItemData {
    pub agent_id: String,
    pub id: String,
}

/// `mcp.bind`, `mcp.unbind`, `mcp.catalog.bind`, `mcp.catalog.unbind`.
/// `mcp_id`, not `id`: the row being addressed is the server, not the binding.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpAgentBindingData {
    pub agent_id: String,
    pub mcp_id: String,
}

/// `mcp.catalog.bind_to_bundle`, `mcp.catalog.unbind_from_bundle`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpBundleBindingData {
    pub bundle_id: String,
    pub mcp_id: String,
}

/// `mcp.catalog.list_for_bundle`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpBundleScopeData {
    pub bundle_id: String,
}

/// `mcp.catalog.delete`, `mcp.catalog.probe` — window-scoped, so no agent key.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpCatalogItemData {
    pub id: String,
}

/// `mcp.catalog.list` — window-scoped with no parameters. A struct rather than
/// `()`, because the stub calls it with `{}` and serde deserializes `()` only
/// from JSON `null`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpCatalogListData {}

fn default_transport() -> String {
    "stdio".to_string()
}

fn default_config() -> String {
    "{}".to_string()
}

/// `mcp.upsert` — agent-scoped create-or-update.
///
/// `id`, `transport` and `config` are all defaulted, so they are omittable on
/// the wire, and ts-rs generates them as **required** (it cannot express an
/// optional property for a non-`Option` field). The frontend derives the
/// accurate shape from this type rather than hand-listing the optional fields;
/// see `McpUpsertInput` in `rpc-api/mcp.ts`.
///
/// The defaults are load-bearing and are why these cannot become
/// `Option<String>`: an omitted `transport` is `"stdio"` and an omitted
/// `config` is `"{}"`, not `null` — and `config` is parsed as JSON downstream,
/// so a null would be a different failure.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandMcpUpsertData {
    pub agent_id: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_config")]
    pub config: String,
}

/// `mcp.catalog.upsert` — window-scoped. Same field rules.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandMcpCatalogUpsertData {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_config")]
    pub config: String,
}

/// `mcp.catalog.upsert_for_bundle` — bundle-scoped. Same field rules.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandMcpCatalogUpsertForBundleData {
    pub bundle_id: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_config")]
    pub config: String,
}

/// Result of `mcp.delete` and `mcp.catalog.delete`. Both were inline
/// `json!({ "deleted": .. })`. False means no row with that id existed — the
/// delete is idempotent, so this is "was something removed", not an error.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpDeleteResult {
    pub deleted: bool,
}

/// Result of the three bind commands. Was an inline `json!({ "bound": true })`
/// — a literal, never computed, because binding is idempotent and any real
/// failure returns `Err`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpBindResult {
    pub bound: bool,
}

/// Result of the three unbind commands. Unlike `bound`, this one IS computed:
/// false means there was no binding to remove.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct McpUnbindResult {
    pub unbound: bool,
}

// Request-shape tests for the eighteen `mcp.*` / `mcp.catalog.*` commands.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_shared_scope_shapes_accept_their_payloads() {
        serde_json::from_value::<McpAgentScopeData>(json!({"agent_id": "a1"}))
            .expect("mcp.list / catalog.list_for_agent");
        serde_json::from_value::<McpAgentItemData>(json!({"agent_id": "a1", "id": "m1"}))
            .expect("mcp.get / delete / probe");
        serde_json::from_value::<McpAgentBindingData>(json!({"agent_id": "a1", "mcp_id": "m1"}))
            .expect("the four agent bind/unbind commands");
        serde_json::from_value::<McpBundleBindingData>(json!({"bundle_id": "b1", "mcp_id": "m1"}))
            .expect("the two bundle bind/unbind commands");
        serde_json::from_value::<McpBundleScopeData>(json!({"bundle_id": "b1"}))
            .expect("catalog.list_for_bundle");
        serde_json::from_value::<McpCatalogItemData>(json!({"id": "m1"}))
            .expect("catalog.delete / catalog.probe");
    }

    // The binding shapes use `mcp_id`, their skill counterparts use `skill_id`.
    // That is the reason the two primitives do NOT share a type, so pin it:
    // a skill-shaped payload must be rejected here.
    #[test]
    fn the_binding_shape_is_keyed_on_mcp_id_not_skill_id() {
        assert!(
            serde_json::from_value::<McpAgentBindingData>(
                json!({"agent_id": "a1", "skill_id": "s1"}),
            )
            .is_err(),
            "mcp bindings take mcp_id; sharing a type with skill would require renaming a wire field"
        );
    }

    #[test]
    fn catalog_list_accepts_the_empty_object_the_stub_sends() {
        serde_json::from_value::<McpCatalogListData>(json!({})).expect("catalog.list");
        assert!(serde_json::from_value::<()>(json!({})).is_err());
    }

    // The defaults are why these fields cannot become `Option<String>`: an
    // omitted transport is "stdio" and an omitted config is "{}", not null --
    // and config is parsed as JSON downstream, so a null would fail differently.
    #[test]
    fn upsert_defaults_transport_and_config_rather_than_nulling_them() {
        let minimal: CommandMcpUpsertData =
            serde_json::from_value(json!({"agent_id": "a1", "name": "n"}))
                .expect("mcp.upsert must accept just agent_id and name");
        assert_eq!(minimal.id, "");
        assert_eq!(minimal.transport, "stdio");
        assert_eq!(minimal.config, "{}");

        assert!(
            serde_json::from_value::<CommandMcpUpsertData>(json!({"agent_id": "a1"})).is_err(),
            "name has no default"
        );
    }

    #[test]
    fn the_other_two_upserts_have_the_same_defaulting_rules() {
        let catalog: CommandMcpCatalogUpsertData =
            serde_json::from_value(json!({"name": "n"})).expect("catalog.upsert is window-scoped");
        assert_eq!((catalog.transport.as_str(), catalog.config.as_str()), ("stdio", "{}"));

        let for_bundle: CommandMcpCatalogUpsertForBundleData =
            serde_json::from_value(json!({"bundle_id": "b1", "name": "n"}))
                .expect("catalog.upsert_for_bundle");
        assert_eq!(for_bundle.transport, "stdio");
        assert!(
            serde_json::from_value::<CommandMcpCatalogUpsertForBundleData>(json!({"name": "n"}))
                .is_err(),
            "bundle_id has no default"
        );
    }

    #[test]
    fn the_result_shapes_serialize_as_the_frontend_expects() {
        assert_eq!(
            serde_json::to_value(McpDeleteResult { deleted: false }).unwrap(),
            json!({"deleted": false})
        );
        assert_eq!(serde_json::to_value(McpBindResult { bound: true }).unwrap(), json!({"bound": true}));
        assert_eq!(
            serde_json::to_value(McpUnbindResult { unbound: false }).unwrap(),
            json!({"unbound": false})
        );
    }
}

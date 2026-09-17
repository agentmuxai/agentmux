// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the v1 standalone Skill primitive (`skill.*` and
//! `skill.catalog.*`).
//!
//! These previously existed only as function-local anonymous `struct Req`
//! declarations inside each handler closure in `server/app_api/skill.rs` —
//! sixteen of them — so there was nothing for the bindings generator to read
//! and the frontend's inline request shapes were hand-maintained against types
//! it could not see.
//!
//! Several commands genuinely share a shape, so they share a type here rather
//! than getting sixteen near-identical ones. The names describe the *shape*
//! (what is being addressed) rather than any single command, because that is
//! what makes the sharing honest: `skill.bind` and `skill.catalog.unbind` take
//! the same pair for the same reason.

use serde::{Deserialize, Serialize};

/// `skill.list`, `skill.catalog.list_for_agent` — everything scoped to one
/// agent. Both are `check_s1`-gated on this field.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillAgentScopeData {
    pub agent_id: String,
}

/// `skill.get`, `skill.delete` — one skill row addressed within an agent.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillAgentItemData {
    pub agent_id: String,
    pub id: String,
}

/// `skill.bind`, `skill.unbind`, `skill.catalog.bind`, `skill.catalog.unbind` —
/// attach/detach a catalog skill to an agent. `skill_id` rather than `id`
/// because the row being addressed is the skill, not the binding.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillAgentBindingData {
    pub agent_id: String,
    pub skill_id: String,
}

/// `skill.catalog.bind_to_bundle`, `skill.catalog.unbind_from_bundle`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillBundleBindingData {
    pub bundle_id: String,
    pub skill_id: String,
}

/// `skill.catalog.list_for_bundle`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillBundleScopeData {
    pub bundle_id: String,
}

/// `skill.catalog.delete` — window-scoped, so no agent or bundle key.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillCatalogItemData {
    pub id: String,
}

/// `skill.catalog.list` — window-scoped with no parameters. Still a struct
/// rather than `()`: the stub calls it with `{}` and serde deserializes `()`
/// only from JSON `null`, so a unit Req would reject every real call while
/// compiling and passing every CI gate (the `bookmarks.list` bug).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillCatalogListData {}

fn default_skill_type() -> String {
    "prompt".to_string()
}

/// `skill.upsert` — agent-scoped create-or-update.
///
/// Every field except `agent_id` and `name` is `#[serde(default)]`, so all of
/// them are omittable on the wire. ts-rs generates them as **required**,
/// because it cannot express an optional property for a non-`Option` field.
/// That is not papered over here: the frontend derives the accurate shape from
/// this generated type with
/// `Pick<…, "agent_id" | "name"> & Partial<Omit<…, "agent_id" | "name">>`,
/// the same approach `BundleUpsertInput` uses. Deriving it rather than
/// hand-listing the optional fields means a new field added below flows
/// through automatically.
///
/// Changing these to `Option<String>` instead would alter handler behaviour —
/// `default_skill_type` exists so an omitted `skill_type` becomes `"prompt"`,
/// and the rest treat `""` as "not provided".
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandSkillUpsertData {
    pub agent_id: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

/// `skill.catalog.upsert` — window-scoped, so no agent key. Same field rules as
/// `CommandSkillUpsertData`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandSkillCatalogUpsertData {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

/// `skill.catalog.upsert_for_bundle` — bundle-scoped. Same field rules again.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandSkillCatalogUpsertForBundleData {
    pub bundle_id: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: String,
}

/// Result of `skill.delete` and `skill.catalog.delete`. Both were an inline
/// `json!({ "deleted": .. })`.
///
/// `deleted` is false when no row with that id existed — the delete is
/// idempotent, so this is "was something actually removed", not an error flag.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillDeleteResult {
    pub deleted: bool,
}

/// Result of the three bind commands. Was an inline `json!({ "bound": true })`
/// — a literal `true`, never computed, because binding is idempotent and any
/// real failure returns `Err`. The field is kept (rather than making these
/// return nothing) because the frontend already reads it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillBindResult {
    pub bound: bool,
}

/// Result of the three unbind commands. Unlike `bound` above this one IS
/// computed: false means there was no binding to remove.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct SkillUnbindResult {
    pub unbound: bool,
}

// Request-shape tests for the sixteen `skill.*` / `skill.catalog.*` commands.
//
// These shapes were function-local anonymous structs until now, so the
// frontend's inline copies were hand-maintained against types it could not see.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_shared_scope_shapes_accept_their_payloads() {
        serde_json::from_value::<SkillAgentScopeData>(json!({"agent_id": "a1"}))
            .expect("skill.list / catalog.list_for_agent");
        serde_json::from_value::<SkillAgentItemData>(json!({"agent_id": "a1", "id": "s1"}))
            .expect("skill.get / skill.delete");
        serde_json::from_value::<SkillAgentBindingData>(
            json!({"agent_id": "a1", "skill_id": "s1"}),
        )
        .expect("the four agent bind/unbind commands");
        serde_json::from_value::<SkillBundleBindingData>(
            json!({"bundle_id": "b1", "skill_id": "s1"}),
        )
        .expect("the two bundle bind/unbind commands");
        serde_json::from_value::<SkillBundleScopeData>(json!({"bundle_id": "b1"}))
            .expect("catalog.list_for_bundle");
        serde_json::from_value::<SkillCatalogItemData>(json!({"id": "s1"}))
            .expect("catalog.delete");
    }

    // `skill.catalog.list` is called with `{}`. A unit Req would reject that.
    #[test]
    fn catalog_list_accepts_the_empty_object_the_stub_sends() {
        serde_json::from_value::<SkillCatalogListData>(json!({})).expect("catalog.list");
        assert!(serde_json::from_value::<()>(json!({})).is_err());
    }

    // THE POINT OF THIS DOMAIN. Every upsert field except the scope key and
    // `name` is `#[serde(default)]`, so a minimal payload must parse. The
    // generated binding marks them required because ts-rs cannot express an
    // optional non-`Option` property -- the frontend derives the accurate
    // shape from it (`SkillUpsertInput`). This pins the wire half of that
    // arrangement, which is the half TypeScript cannot check.
    #[test]
    fn upsert_requires_only_its_scope_key_and_name() {
        let minimal: CommandSkillUpsertData =
            serde_json::from_value(json!({"agent_id": "a1", "name": "n"}))
                .expect("skill.upsert must accept just agent_id and name");
        assert_eq!(minimal.id, "");
        assert_eq!(minimal.trigger, "");
        assert_eq!(minimal.description, "");
        assert_eq!(minimal.content, "");
        // default_skill_type, not the empty string -- an omitted skill_type is
        // "prompt", which is why these cannot become Option<String>.
        assert_eq!(minimal.skill_type, "prompt");

        assert!(
            serde_json::from_value::<CommandSkillUpsertData>(json!({"agent_id": "a1"})).is_err(),
            "name has no default and must be required"
        );
        assert!(
            serde_json::from_value::<CommandSkillUpsertData>(json!({"name": "n"})).is_err(),
            "agent_id has no default and must be required"
        );
    }

    #[test]
    fn the_other_two_upserts_have_the_same_defaulting_rules() {
        let catalog: CommandSkillCatalogUpsertData =
            serde_json::from_value(json!({"name": "n"})).expect("catalog.upsert is window-scoped");
        assert_eq!(catalog.skill_type, "prompt");

        let for_bundle: CommandSkillCatalogUpsertForBundleData =
            serde_json::from_value(json!({"bundle_id": "b1", "name": "n"}))
                .expect("catalog.upsert_for_bundle");
        assert_eq!(for_bundle.skill_type, "prompt");
        assert!(
            serde_json::from_value::<CommandSkillCatalogUpsertForBundleData>(json!({"name": "n"}))
                .is_err(),
            "bundle_id has no default"
        );
    }

    // The three result shapes were inline json! literals. Pin their serialized
    // form so they cannot drift from what the frontend reads.
    #[test]
    fn the_result_shapes_serialize_as_the_frontend_expects() {
        assert_eq!(
            serde_json::to_value(SkillDeleteResult { deleted: false }).unwrap(),
            json!({"deleted": false})
        );
        assert_eq!(
            serde_json::to_value(SkillBindResult { bound: true }).unwrap(),
            json!({"bound": true})
        );
        assert_eq!(
            serde_json::to_value(SkillUnbindResult { unbound: false }).unwrap(),
            json!({"unbound": false})
        );
    }
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Proves the `optionalFields` in `frontend/app/store/rpc-api/generated-manifest.json`.
//!
//! A request field with `#[serde(default)]` that is not an `Option` is one srv accepts the
//! frontend leaving out, but ts-rs can't say so (`#[ts(optional)]` works only on `Option<T>`),
//! so its generated type still requires it. The manifest names those fields per command and
//! `scripts/gen-rpc-stubs.mjs` makes them optional in the stub. This module is what makes that
//! a fact rather than a claim: each test deserializes the request with every listed field left
//! out, and the last test fails if the manifest lists optional fields for a command that has no
//! test here.

use serde_json::json;

use super::agent::{CommandCreateAgentDefinitionData, CommandCreateAgentSkillData, CommandUpdateAgentDefinitionData};
use super::block::{CommandBlockInputData, CommandResolveCliData};
use super::bundle::CommandUpsertBundleData;
use super::identity::CommandUnlinkAgentIdentityData;
use super::instance::CommandCreateAgentInstanceData;
use super::mcp::{CommandMcpCatalogUpsertData, CommandMcpUpsertData};
use super::skill::{CommandSkillCatalogUpsertData, CommandSkillCatalogUpsertForBundleData, CommandSkillUpsertData};
use crate::server::identity_handlers::StartProviderAuthReq;
use crate::server::widget_handlers::WidgetInstallReq;

/// Commands whose `optionalFields` this module proves.
const PROVEN: &[&str] = &[
    "auth.start",
    "controllerinput",
    "createagent",
    "createagentinstance",
    "createagentskill",
    "mcp.catalog.upsert",
    "mcp.upsert",
    "resolvecli",
    "skill.catalog.upsert",
    "skill.catalog.upsert_for_bundle",
    "skill.upsert",
    "unlinkagentidentity",
    "updateagent",
    "upsertmemory",
    "upsertsystemmemory",
    "widgets.install",
];

#[test]
fn controllerinput_takes_a_signal_a_resize_or_input_alone() {
    // What useAgentCommands (a stop) and usePtyWidth (a resize) send.
    let sig: CommandBlockInputData = serde_json::from_value(json!({ "blockid": "b", "signame": "SIGINT" })).unwrap();
    assert_eq!(sig.inputdata64, "");
    let resize: CommandBlockInputData =
        serde_json::from_value(json!({ "blockid": "b", "termsize": { "rows": 24, "cols": 80 } })).unwrap();
    assert_eq!((resize.inputdata64.as_str(), resize.signame.as_str()), ("", ""));
}

#[test]
fn resolvecli_takes_a_request_without_a_block() {
    // Only block_id is optional in the stub. The install commands are defaulted too, but
    // stay required there: resolving with a silently empty install command is a worse
    // failure than a compile error.
    let r: CommandResolveCliData = serde_json::from_value(json!({
        "provider_id": "claude",
        "cli_command": "claude",
        "npm_package": "@anthropic-ai/claude-code",
        "pinned_version": "latest",
        "windows_install_command": "irm https://claude.ai/install.ps1 | iex",
        "unix_install_command": "curl -fsSL https://claude.ai/install.sh | bash",
    }))
    .unwrap();
    assert_eq!(r.block_id, "");
}

#[test]
fn skill_catalog_upsert_for_bundle_takes_just_bundle_and_name() {
    let r: CommandSkillCatalogUpsertForBundleData =
        serde_json::from_value(json!({ "bundle_id": "bundle-1", "name": "Review" })).unwrap();
    assert_eq!(r.id, "");
    assert_eq!(r.skill_type, "prompt", "skill_type has a non-empty default");
}

#[test]
fn updateagent_takes_just_the_required_four() {
    let r: CommandUpdateAgentDefinitionData =
        serde_json::from_value(json!({ "id": "a1", "name": "Agent", "icon": "robot", "provider": "claude" })).unwrap();
    assert_eq!(r.description, "");
    assert_eq!(r.auto_continue_enabled, None);
    assert_eq!(r.model_vendor_base_url, None);
}

/// The requests whose hand-written stubs had these fields optional (srv defaults every one of
/// them), so the generated stubs keep them optional: each takes just its required fields.
#[test]
fn creates_and_upserts_take_just_their_required_fields() {
    let _: CommandCreateAgentDefinitionData = serde_json::from_value(json!({ "name": "A", "provider": "claude" })).unwrap();
    let _: CommandCreateAgentInstanceData = serde_json::from_value(json!({ "definition_id": "d1" })).unwrap();
    let _: CommandCreateAgentSkillData = serde_json::from_value(json!({ "agent_id": "a1", "name": "Review" })).unwrap();
    let s: CommandSkillUpsertData = serde_json::from_value(json!({ "agent_id": "a1", "name": "Review" })).unwrap();
    assert_eq!(s.skill_type, "prompt");
    let c: CommandSkillCatalogUpsertData = serde_json::from_value(json!({ "name": "Review" })).unwrap();
    assert_eq!(c.id, "");
    let u: CommandUnlinkAgentIdentityData = serde_json::from_value(json!({ "agent_id": "a1", "provider": "claude" })).unwrap();
    assert!(!u.silent);
    let w: WidgetInstallReq = serde_json::from_value(json!({ "path": "/tmp/w" })).unwrap();
    assert!(!w.replace);
}

#[test]
fn auth_start_takes_a_request_without_account_or_env() {
    // What login-backend.ts sends.
    let r: StartProviderAuthReq = serde_json::from_value(json!({
        "providerId": "claude",
        "cliPath": "claude",
        "authLoginArgs": ["auth", "login"],
        "authCheckArgs": ["auth", "status"],
    }))
    .unwrap();
    assert!(!r.direct_account && !r.requires_tty);
    assert_eq!(r.existing_account_id, "");
    assert!(r.auth_env.is_empty());
}

#[test]
fn mcp_upserts_take_a_new_server_without_id_transport_or_config() {
    let r: CommandMcpUpsertData = serde_json::from_value(json!({ "agent_id": "a1", "name": "files" })).unwrap();
    assert_eq!((r.id.as_str(), r.transport.as_str()), ("", "stdio"));
    let c: CommandMcpCatalogUpsertData = serde_json::from_value(json!({ "name": "files" })).unwrap();
    assert_eq!(c.transport, "stdio");
}

#[test]
fn upsert_bundle_takes_just_id_and_name() {
    // upsertmemory and upsertsystemmemory share this request (a Bundle, flattened).
    let r: CommandUpsertBundleData = serde_json::from_value(json!({ "id": "b1", "name": "Bundle" })).unwrap();
    assert_eq!(r.bundle.description, "");
    assert_eq!(r.bundle.context_files, "[]", "context_files has a non-empty default");
    assert_eq!(r.base_sha256, None);
}

#[test]
fn every_manifest_command_with_optional_fields_is_proven_here() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/app/store/rpc-api/generated-manifest.json");
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut listed: Vec<String> = manifest["stubs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s.get("optionalFields").is_some())
        .map(|s| s["command"].as_str().unwrap().to_string())
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        PROVEN.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "a command in generated-manifest.json has optionalFields without a test here proving srv accepts the request without them (or a test here is stale)"
    );
}

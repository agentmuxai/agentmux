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

use super::agent::CommandUpdateAgentDefinitionData;
use super::block::{CommandBlockInputData, CommandResolveCliData};
use super::skill::CommandSkillCatalogUpsertForBundleData;

/// Commands whose `optionalFields` this module proves.
const PROVEN: &[&str] = &["controllerinput", "resolvecli", "skill.catalog.upsert_for_bundle", "updateagent"];

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
fn resolvecli_takes_a_request_without_install_commands_or_block() {
    let r: CommandResolveCliData = serde_json::from_value(json!({
        "provider_id": "claude",
        "cli_command": "claude",
        "npm_package": "@anthropic-ai/claude-code",
        "pinned_version": "latest",
    }))
    .unwrap();
    assert_eq!((r.windows_install_command.as_str(), r.unix_install_command.as_str(), r.block_id.as_str()), ("", "", ""));
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

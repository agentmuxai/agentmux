// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each typed `bundle.*` response serializes to exactly the JSON its
//! handler built by hand before it had a type.

use super::super::*;
use crate::backend::bundle_export::BundleExportFile;
use crate::backend::storage::project_instructions::InstructionChange;

#[test]
fn bundle_list_response_keeps_its_wire_shape() {
    let item = BundleListItem {
        id: "b1".into(),
        name: "Name".into(),
        description: "Desc".into(),
        provider: "claude".into(),
        model: "anthropic".into(),
        is_blank: false,
        updated_at: 42,
    };
    let resp = BundleListResponse { bundles: vec![item.clone()], presets: vec![item] };
    let row = json!({
        "id": "b1", "name": "Name", "description": "Desc",
        "provider": "claude", "model": "anthropic", "is_blank": false, "updated_at": 42,
    });
    assert_eq!(serde_json::to_value(&resp).unwrap(), json!({ "bundles": [row.clone()], "presets": [row] }));
}

#[test]
fn bundle_delete_response_keeps_its_wire_shape() {
    assert_eq!(serde_json::to_value(DeleteBundleResult { deleted: true }).unwrap(), json!({ "deleted": true }));
}

#[test]
fn bundle_export_response_keeps_both_wire_shapes() {
    let files = BundleExportResponse {
        root_slug: "my-bundle".into(),
        files: Some(vec![BundleExportFile { path: "bundle.json".into(), content: "{}".into() }]),
        skipped_skills: vec!["legacy".into()],
        warnings: vec!["w".into()],
        missing_skill_ids: vec![],
        zip_base64: None,
    };
    assert_eq!(
        serde_json::to_value(&files).unwrap(),
        json!({
            "root_slug": "my-bundle",
            "files": [{ "path": "bundle.json", "content": "{}" }],
            "skipped_skills": ["legacy"],
            "warnings": ["w"],
            "missing_skill_ids": [],
        })
    );

    let zip = BundleExportResponse { files: None, zip_base64: Some("UEsDBA==".into()), ..files };
    assert_eq!(
        serde_json::to_value(&zip).unwrap(),
        json!({
            "root_slug": "my-bundle",
            "skipped_skills": ["legacy"],
            "warnings": ["w"],
            "missing_skill_ids": [],
            "zip_base64": "UEsDBA==",
        })
    );
}

#[test]
fn bundle_export_with_history_response_keeps_its_wire_shape() {
    let resp = BundleExportWithHistoryResponse {
        root_slug: "my-bundle".into(),
        skipped_skills: vec![],
        warnings: vec!["history: included 1 of 1 known session(s)".into()],
        missing_skill_ids: vec![],
        history_session_count: 1,
        zip_base64: "UEsDBA==".into(),
    };
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        json!({
            "root_slug": "my-bundle",
            "skipped_skills": [],
            "warnings": ["history: included 1 of 1 known session(s)"],
            "missing_skill_ids": [],
            "history_session_count": 1,
            "zip_base64": "UEsDBA==",
        })
    );
}

#[test]
fn bundle_import_response_keeps_its_wire_shape() {
    let resp = BundleImportResponse {
        bundle_id: "b1".into(),
        imported_skill_ids: vec!["s1".into()],
        skipped_skills: vec!["dup".into()],
        resolved_requirement_ids: vec!["r1".into()],
        unresolved_requirements: vec![crate::backend::rpc_types::BundleImportUnresolvedRequirement {
            id: "r2".into(),
            provider: "codex".into(),
            env: "OPENAI_API_KEY".into(),
            match_count: 0,
        }],
        warnings: vec![],
    };
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        json!({
            "bundle_id": "b1",
            "imported_skill_ids": ["s1"],
            "skipped_skills": ["dup"],
            "resolved_requirement_ids": ["r1"],
            "unresolved_requirements": [
                { "id": "r2", "provider": "codex", "env": "OPENAI_API_KEY", "match_count": 0 },
            ],
            "warnings": [],
        })
    );
}

#[test]
fn bundle_import_for_agent_response_keeps_its_wire_shape() {
    let resp = BundleImportForAgentResponse {
        bundle_id: "b1".into(),
        memory_files_written: 2,
        skipped_skills: vec![],
        warnings: vec!["w".into()],
    };
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        json!({ "bundle_id": "b1", "memory_files_written": 2, "skipped_skills": [], "warnings": ["w"] })
    );
}

#[test]
fn agent_project_instructions_response_keeps_its_wire_shape() {
    let first_seen = AgentProjectInstructionFile {
        path: "CLAUDE.md".into(),
        exists: true,
        size_bytes: 12,
        content_hash: "abc".into(),
        owner: "foreign".into(),
        content: "House rules.".into(),
        truncated: false,
        error: None,
        change: InstructionChange::FirstSeen,
        last_observed_at: None,
    };
    let removed = AgentProjectInstructionFile {
        path: ".github/instructions/old.instructions.md".into(),
        exists: false,
        size_bytes: 0,
        content_hash: String::new(),
        owner: "foreign".into(),
        content: String::new(),
        truncated: false,
        error: None,
        change: InstructionChange::Removed,
        last_observed_at: Some(7),
    };
    let resp = AgentProjectInstructionsResponse {
        provider: "claude".into(),
        working_directory: "/work/proj".into(),
        files: vec![first_seen, removed],
    };
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        json!({
            "provider": "claude",
            "working_directory": "/work/proj",
            "files": [
                {
                    "path": "CLAUDE.md", "exists": true, "size_bytes": 12, "content_hash": "abc",
                    "owner": "foreign", "content": "House rules.", "truncated": false,
                    "error": null, "change": "first_seen", "last_observed_at": null,
                },
                {
                    "path": ".github/instructions/old.instructions.md", "exists": false,
                    "size_bytes": 0, "content_hash": "", "owner": "foreign", "content": "",
                    "truncated": false, "error": null, "change": "removed", "last_observed_at": 7,
                },
            ],
        })
    );
}
